//! GitHub context and guarded write tools backed by the `gh` CLI.
//!
//! Unified surface (piagent phase B): the model sees one tool, `github`,
//! with an `action` parameter routing to the per-action logic. The legacy
//! `github_*` names stay registered as hidden compat aliases that force the
//! action so saved transcripts replay correctly — the pattern `BashTool`
//! established for `exec_shell*` in #4625.

use std::path::{Path, PathBuf};
use std::process::Command;

use crate::dependencies::ExternalTool;
use async_trait::async_trait;
use chrono::Utc;
use serde_json::{Value, json};
use uuid::Uuid;

use crate::task_manager::{TaskArtifactRef, TaskGithubEvent};
use crate::tools::spec::{
    ApprovalRequirement, ToolCapability, ToolContext, ToolError, ToolResult, ToolSpec,
    optional_bool, optional_str, required_str, required_u64,
};

const DEFAULT_GH: &str = "/opt/homebrew/bin/gh";
const FALLBACK_GH_PATHS: &[&str] = &[
    "/usr/bin/gh",                       // Linux system package manager
    "/usr/local/bin/gh",                 // macOS Intel Homebrew / manual install
    "/home/linuxbrew/.linuxbrew/bin/gh", // Linux Homebrew (official prefix)
    "/opt/homebrew/bin/gh",              // macOS Apple Silicon Homebrew
];
const BODY_ARTIFACT_THRESHOLD: usize = 4_000;
const DIFF_ARTIFACT_THRESHOLD: usize = 8_000;

/// Actions the Plan-mode read-only surface exposes.
const READ_ACTIONS: &[&str] = &["issue_context", "pr_context"];
const ALL_ACTIONS: &[&str] = &[
    "issue_context",
    "pr_context",
    "comment",
    "close_issue",
    "close_pr",
];

/// Unified GitHub tool.
///
/// One struct, one input schema per surface: the canonical `github` tool
/// (all actions, or the read-only subset via [`GithubTool::read_only`]) plus
/// hidden legacy aliases carrying a `forced_action`.
pub struct GithubTool {
    name: &'static str,
    forced_action: Option<&'static str>,
    read_only: bool,
}

impl GithubTool {
    pub const fn new(name: &'static str) -> Self {
        Self {
            name,
            forced_action: None,
            read_only: false,
        }
    }

    /// Plan-mode variant: only the read-only actions are advertised and routed.
    pub const fn read_only(name: &'static str) -> Self {
        Self {
            name,
            forced_action: None,
            read_only: true,
        }
    }

    pub const fn alias(name: &'static str, action: &'static str) -> Self {
        Self {
            name,
            forced_action: Some(action),
            read_only: false,
        }
    }

    fn allowed_actions(&self) -> &'static [&'static str] {
        if self.read_only { READ_ACTIONS } else { ALL_ACTIONS }
    }

    fn resolve_action<'a>(&'a self, input: &'a Value) -> Result<&'a str, ToolError> {
        let action = match self.forced_action {
            Some(action) => action,
            None => input
                .get("action")
                .and_then(Value::as_str)
                .ok_or_else(|| {
                    ToolError::invalid_input(format!(
                        "github: missing `action` (one of: {})",
                        self.allowed_actions().join(", ")
                    ))
                })?,
        };
        if self.allowed_actions().contains(&action) {
            Ok(action)
        } else {
            Err(ToolError::invalid_input(format!(
                "github: invalid action `{action}` (one of: {})",
                self.allowed_actions().join(", ")
            )))
        }
    }

    fn action_is_read(action: &str) -> bool {
        READ_ACTIONS.contains(&action)
    }
}

#[async_trait]
impl ToolSpec for GithubTool {
    fn name(&self) -> &'static str {
        self.name
    }

    fn model_visible(&self) -> bool {
        self.forced_action.is_none()
    }

    fn description(&self) -> &'static str {
        match self.forced_action {
            Some("issue_context") => "Read GitHub issue context using gh. Read-only: body/comments/labels/state are summarized and large bodies become task artifacts when a durable task is active.",
            Some("pr_context") => "Read GitHub PR context using gh: body/comments/reviews/check status/files and optional diff artifact. Read-only; no push/merge/close.",
            Some("comment") => "Post an evidence-backed GitHub issue/PR comment with gh. Requires approval. Use blocker comments for partial work; do not claim closure without evidence.",
            Some("close_issue") => "Close a GitHub issue only when structured acceptance evidence is present and approved. For pull requests use github_close_pr; do not call PRs issues in user-facing output. Never close merely because the agent is stopping.",
            Some("close_pr") => "Close a GitHub pull request only when structured acceptance evidence is present and approved. Use this for PRs instead of github_close_issue so the UI, audit trail, and comments keep PR wording clear.",
            _ if self.read_only => "Read GitHub issue/PR context using gh. Actions: \"issue_context\" and \"pr_context\"; bodies/comments/labels/state are summarized and large bodies become task artifacts when a durable task is active.",
            _ => "Read and guardedly mutate GitHub issues/PRs using gh. Actions: \"issue_context\", \"pr_context\" (read-only; large bodies become task artifacts when a durable task is active), \"comment\" (approval; evidence-backed), \"close_issue\", \"close_pr\" (approval; only with structured acceptance evidence — never close merely because the agent is stopping). No push/merge.",
        }
    }

    fn input_schema(&self) -> Value {
        if let Some(action) = self.forced_action {
            return legacy_action_schema(action);
        }
        let actions: Vec<&str> = self.allowed_actions().to_vec();
        let mut properties = serde_json::Map::new();
        properties.insert(
            "action".to_string(),
            json!({
                "type": "string",
                "enum": actions,
                "description": "Action to perform."
            }),
        );
        properties.insert(
            "number".to_string(),
            json!({ "type": "integer", "minimum": 1, "description": "Issue/PR number (all actions)." }),
        );
        properties.insert(
            "include_comments".to_string(),
            json!({ "type": "boolean", "default": true, "description": "(action=issue_context)" }),
        );
        properties.insert(
            "include_diff".to_string(),
            json!({ "type": "boolean", "default": false, "description": "(action=pr_context)" }),
        );
        if !self.read_only {
            properties.insert(
                "target".to_string(),
                json!({ "type": "string", "enum": ["issue", "pr"], "description": "(action=comment)" }),
            );
            properties.insert(
                "body".to_string(),
                json!({ "type": "string", "description": "Comment body (action=comment)." }),
            );
            properties.insert(
                "evidence".to_string(),
                json!({
                    "type": "object",
                    "description": "Evidence object (action=comment/close_*). Close actions require files_changed, tests_run, final_status.",
                    "properties": {
                        "files_changed": { "type": "array", "items": { "type": "string" } },
                        "tests_run": { "type": "array", "items": { "type": "string" } },
                        "commits": { "type": "array", "items": { "type": "string" } },
                        "final_status": { "type": "string" }
                    }
                }),
            );
            properties.insert(
                "acceptance_criteria".to_string(),
                json!({ "type": "array", "items": { "type": "string" }, "minItems": 1, "description": "(action=close_issue/close_pr)" }),
            );
            properties.insert(
                "comment".to_string(),
                json!({ "type": "string", "description": "Optional closing comment (action=close_issue/close_pr)." }),
            );
            properties.insert(
                "allow_dirty".to_string(),
                json!({ "type": "boolean", "default": false, "description": "(action=close_issue/close_pr)" }),
            );
            properties.insert(
                "dry_run".to_string(),
                json!({ "type": "boolean", "default": false, "description": "(action=comment/close_issue/close_pr)" }),
            );
        }
        json!({
            "type": "object",
            "properties": properties,
            "additionalProperties": false
        })
    }

    fn capabilities(&self) -> Vec<ToolCapability> {
        match self.forced_action {
            Some(action) if Self::action_is_read(action) => {
                vec![ToolCapability::ReadOnly, ToolCapability::Network]
            }
            Some(_) => vec![
                ToolCapability::Network,
                ToolCapability::RequiresApproval,
            ],
            None if self.read_only => vec![ToolCapability::ReadOnly, ToolCapability::Network],
            None => vec![
                ToolCapability::Network,
                ToolCapability::RequiresApproval,
            ],
        }
    }

    fn approval_requirement(&self) -> ApprovalRequirement {
        match self.forced_action {
            Some(action) if Self::action_is_read(action) => ApprovalRequirement::Auto,
            Some(_) => ApprovalRequirement::Required,
            None if self.read_only => ApprovalRequirement::Auto,
            None => ApprovalRequirement::Required,
        }
    }

    fn approval_requirement_for(&self, input: &Value) -> ApprovalRequirement {
        match self.resolve_action(input) {
            Ok(action) if Self::action_is_read(action) => ApprovalRequirement::Auto,
            _ => ApprovalRequirement::Required,
        }
    }

    fn is_read_only_for(&self, input: &Value) -> bool {
        match self.resolve_action(input) {
            Ok(action) => Self::action_is_read(action),
            Err(_) => self.is_read_only(),
        }
    }

    async fn execute(&self, input: Value, context: &ToolContext) -> Result<ToolResult, ToolError> {
        match self.resolve_action(&input)? {
            "issue_context" => self.execute_issue_context(&input, context).await,
            "pr_context" => self.execute_pr_context(&input, context).await,
            "comment" => self.execute_comment(&input, context).await,
            "close_issue" => close_github_thread(input, context, GithubCloseTarget::Issue),
            "close_pr" => close_github_thread(input, context, GithubCloseTarget::Pr),
            action => Err(ToolError::invalid_input(format!(
                "github: invalid action `{action}`"
            ))),
        }
    }
}

impl GithubTool {
    async fn execute_issue_context(
        &self,
        input: &Value,
        context: &ToolContext,
    ) -> Result<ToolResult, ToolError> {
        ensure_github_repo(context)?;
        let number = required_u64(input, "number")?;
        let include_comments = optional_bool(input, "include_comments", true);
        let fields = if include_comments {
            "number,title,state,author,labels,assignees,milestone,body,comments,url,createdAt,updatedAt"
        } else {
            "number,title,state,author,labels,assignees,milestone,body,url,createdAt,updatedAt"
        };
        let number_s = number.to_string();
        let raw = run_gh_json(context, &["issue", "view", &number_s, "--json", fields])?;
        let shaped = shape_large_text(context, raw, "issue_body", BODY_ARTIFACT_THRESHOLD)?;
        let mut result = ToolResult::json(&json!({
            "summary": format!("Issue #{number}: {}", shaped["title"].as_str().unwrap_or("")),
            "issue": shaped,
        }))
        .map_err(|e| ToolError::execution_failed(e.to_string()))?;
        let artifacts = artifact_refs_from_context(&result.content, "github_issue_body");
        if !artifacts.is_empty() {
            result = result.with_metadata(json!({ "task_updates": { "artifacts": artifacts } }));
        }
        Ok(result)
    }

    async fn execute_pr_context(
        &self,
        input: &Value,
        context: &ToolContext,
    ) -> Result<ToolResult, ToolError> {
        ensure_github_repo(context)?;
        let number = required_u64(input, "number")?;
        let number_s = number.to_string();
        let raw = run_gh_json(
            context,
            &[
                "pr",
                "view",
                &number_s,
                "--json",
                "number,title,state,author,body,comments,reviews,reviewDecision,statusCheckRollup,baseRefName,headRefName,headRefOid,baseRefOid,files,url,createdAt,updatedAt",
            ],
        )?;
        let mut shaped = shape_large_text(context, raw, "pr_body", BODY_ARTIFACT_THRESHOLD)?;
        if optional_bool(input, "include_diff", false) {
            let diff = run_gh_text(context, &["pr", "diff", &number_s, "--patch"])?;
            let diff_ref =
                write_artifact_if_needed(context, "pr_diff", &diff, DIFF_ARTIFACT_THRESHOLD)?;
            shaped["diff_summary"] = json!(summarize(&diff, 900));
            shaped["diff_artifact"] = json!(diff_ref);
        }
        let mut result = ToolResult::json(&json!({
            "summary": format!("PR #{number}: {}", shaped["title"].as_str().unwrap_or("")),
            "pr": shaped,
        }))
        .map_err(|e| ToolError::execution_failed(e.to_string()))?;
        let mut artifacts = artifact_refs_from_context(&result.content, "github_pr_body");
        artifacts.extend(artifact_refs_from_context(
            &result.content,
            "github_pr_diff",
        ));
        if !artifacts.is_empty() {
            result = result.with_metadata(json!({ "task_updates": { "artifacts": artifacts } }));
        }
        Ok(result)
    }

    async fn execute_comment(
        &self,
        input: &Value,
        context: &ToolContext,
    ) -> Result<ToolResult, ToolError> {
        validate_evidence(input, false)?;
        let target = required_str(input, "target")?;
        let number = required_u64(input, "number")?;
        let body = required_str(input, "body")?;
        if optional_bool(input, "dry_run", false) {
            return Ok(ToolResult::success(format!(
                "Dry run: would comment on {target} #{number}."
            )));
        }
        let subcmd = if target == "pr" { "pr" } else { "issue" };
        let number_s = number.to_string();
        run_gh_text(context, &[subcmd, "comment", &number_s, "--body", body])?;
        let metadata = github_event_metadata(
            "comment",
            target,
            number,
            summarize(body, 240),
            None,
            write_artifact_if_needed(context, "github_comment", body, BODY_ARTIFACT_THRESHOLD)?,
        );
        Ok(
            ToolResult::success(format!("Commented on {target} #{number}."))
                .with_metadata(metadata),
        )
    }
}

/// The exact schema the legacy per-action tool exposed, kept so hidden alias
/// registrations report an identical contract to the pre-unification tools.
fn legacy_action_schema(action: &str) -> Value {
    match action {
        "issue_context" => json!({
            "type": "object",
            "properties": {
                "number": { "type": "integer", "minimum": 1 },
                "include_comments": { "type": "boolean", "default": true }
            },
            "required": ["number"],
            "additionalProperties": false
        }),
        "pr_context" => json!({
            "type": "object",
            "properties": {
                "number": { "type": "integer", "minimum": 1 },
                "include_diff": { "type": "boolean", "default": false }
            },
            "required": ["number"],
            "additionalProperties": false
        }),
        "comment" => json!({
            "type": "object",
            "properties": {
                "target": { "type": "string", "enum": ["issue", "pr"] },
                "number": { "type": "integer", "minimum": 1 },
                "body": { "type": "string" },
                "evidence": { "type": "object" },
                "dry_run": { "type": "boolean", "default": false }
            },
            "required": ["target", "number", "body", "evidence"],
            "additionalProperties": false
        }),
        // close_issue / close_pr share the close schema.
        _ => close_input_schema(),
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum GithubCloseTarget {
    Issue,
    Pr,
}

impl GithubCloseTarget {
    fn cli_subcommand(self) -> &'static str {
        match self {
            Self::Issue => "issue",
            Self::Pr => "pr",
        }
    }

    fn metadata_target(self) -> &'static str {
        match self {
            Self::Issue => "issue",
            Self::Pr => "pr",
        }
    }

    fn display(self) -> &'static str {
        match self {
            Self::Issue => "issue",
            Self::Pr => "PR",
        }
    }

    fn summary_subject(self) -> &'static str {
        match self {
            Self::Issue => "Issue",
            Self::Pr => "PR",
        }
    }
}

fn close_input_schema() -> Value {
    json!({
        "type": "object",
        "properties": {
            "number": { "type": "integer", "minimum": 1 },
            "acceptance_criteria": { "type": "array", "items": { "type": "string" }, "minItems": 1 },
            "evidence": {
                "type": "object",
                "properties": {
                    "files_changed": { "type": "array", "items": { "type": "string" } },
                    "tests_run": { "type": "array", "items": { "type": "string" } },
                    "commits": { "type": "array", "items": { "type": "string" } },
                    "final_status": { "type": "string" }
                },
                "required": ["files_changed", "tests_run", "final_status"]
            },
            "comment": { "type": "string" },
            "allow_dirty": { "type": "boolean", "default": false },
            "dry_run": { "type": "boolean", "default": false }
        },
        "required": ["number", "acceptance_criteria", "evidence"],
        "additionalProperties": false
    })
}

fn close_github_thread(
    input: Value,
    context: &ToolContext,
    target: GithubCloseTarget,
) -> Result<ToolResult, ToolError> {
    validate_evidence(&input, true)?;
    if !optional_bool(&input, "allow_dirty", false) {
        let status = git_status_porcelain(context)?;
        if !status.trim().is_empty() {
            return Ok(ToolResult::error(format!(
                "Refusing to close {}: worktree is dirty and allow_dirty was false.",
                target.display()
            ))
            .with_metadata(json!({ "dirty_status": status })));
        }
    }
    let number = required_u64(&input, "number")?;
    if optional_bool(&input, "dry_run", false) {
        return Ok(ToolResult::success(format!(
            "Dry run: would close {} #{number}.",
            target.display()
        )));
    }
    let subcmd = target.cli_subcommand();
    let number_s = number.to_string();
    if let Some(comment) = optional_str(&input, "comment") {
        run_gh_text(context, &[subcmd, "comment", &number_s, "--body", comment])?;
    }
    let close_args: Vec<&str> = match target {
        GithubCloseTarget::Issue => vec!["issue", "close", &number_s, "--reason", "completed"],
        GithubCloseTarget::Pr => vec!["pr", "close", &number_s],
    };
    run_gh_text(context, &close_args)?;
    let metadata = github_event_metadata(
        "close",
        target.metadata_target(),
        number,
        format!(
            "{} closed as completed with structured evidence",
            target.summary_subject()
        ),
        None,
        optional_str(&input, "comment")
            .and_then(|comment| {
                write_artifact_if_needed(
                    context,
                    "github_close_comment",
                    comment,
                    BODY_ARTIFACT_THRESHOLD,
                )
                .ok()
            })
            .flatten(),
    );
    Ok(
        ToolResult::success(format!("Closed {} #{number}.", target.display()))
            .with_metadata(metadata),
    )
}

fn gh_bin() -> String {
    if let Ok(bin) = std::env::var("CODEWHALE_GH_BIN").or_else(|_| std::env::var("DEEPSEEK_GH_BIN"))
    {
        return bin;
    }
    for path in FALLBACK_GH_PATHS {
        if std::path::Path::new(path).is_file() {
            return path.to_string();
        }
    }
    DEFAULT_GH.to_string()
}

fn run_gh_text(context: &ToolContext, args: &[&str]) -> Result<String, ToolError> {
    let out = Command::new(gh_bin())
        .args(args)
        .current_dir(&context.workspace)
        .output()
        .map_err(|e| {
            if e.kind() == std::io::ErrorKind::NotFound {
                ToolError::not_available("gh CLI not found; install it or set DEEPSEEK_GH_BIN")
            } else {
                ToolError::execution_failed(format!("failed to run gh: {e}"))
            }
        })?;
    if !out.status.success() {
        return Err(ToolError::execution_failed(format!(
            "gh {} failed: {}",
            args.join(" "),
            String::from_utf8_lossy(&out.stderr).trim()
        )));
    }
    Ok(String::from_utf8_lossy(&out.stdout).to_string())
}

fn run_gh_json(context: &ToolContext, args: &[&str]) -> Result<Value, ToolError> {
    let text = run_gh_text(context, args)?;
    serde_json::from_str(&text).map_err(|e| ToolError::execution_failed(e.to_string()))
}

fn ensure_github_repo(context: &ToolContext) -> Result<(), ToolError> {
    let out = crate::dependencies::Git::output(
        &["rev-parse", "--is-inside-work-tree"],
        &context.workspace,
    )
    .map_err(|e| ToolError::execution_failed(format!("failed to run git: {e}")))?;
    if out.status.success() {
        Ok(())
    } else {
        Err(ToolError::not_available(
            "current workspace is not a git repository",
        ))
    }
}

fn git_status_porcelain(context: &ToolContext) -> Result<String, ToolError> {
    let out = crate::dependencies::Git::output(&["status", "--porcelain"], &context.workspace)
        .map_err(|e| ToolError::execution_failed(format!("failed to run git status: {e}")))?;
    Ok(String::from_utf8_lossy(&out.stdout).to_string())
}

fn shape_large_text(
    context: &ToolContext,
    mut value: Value,
    label: &str,
    threshold: usize,
) -> Result<Value, ToolError> {
    let body = value
        .get("body")
        .and_then(Value::as_str)
        .map(ToString::to_string);
    if let Some(body) = body
        && body.len() > threshold
    {
        let artifact = write_artifact_if_needed(context, label, &body, threshold)?;
        value["body_summary"] = json!(summarize(&body, 900));
        value["body_artifact"] = json!(artifact);
        value["body"] = json!(summarize(&body, 1200));
    }
    Ok(value)
}

fn write_artifact_if_needed(
    context: &ToolContext,
    label: &str,
    content: &str,
    threshold: usize,
) -> Result<Option<PathBuf>, ToolError> {
    if content.len() <= threshold {
        return Ok(None);
    }
    let Some(task_id) = context.runtime.active_task_id.as_deref() else {
        return Ok(None);
    };
    if let Some(manager) = context.runtime.task_manager.as_ref() {
        return manager
            .write_task_artifact(task_id, label, content)
            .map(Some)
            .map_err(|e| ToolError::execution_failed(e.to_string()));
    }
    let Some(data_dir) = context.runtime.task_data_dir.as_ref() else {
        return Ok(None);
    };
    let dir = data_dir.join("artifacts").join(task_id);
    std::fs::create_dir_all(&dir)
        .map_err(|e| ToolError::execution_failed(format!("create artifact dir: {e}")))?;
    let absolute = dir.join(format!(
        "{}_{}.txt",
        Utc::now().format("%Y%m%dT%H%M%S%.3fZ"),
        sanitize_filename(label)
    ));
    std::fs::write(&absolute, content)
        .map_err(|e| ToolError::execution_failed(format!("write artifact: {e}")))?;
    Ok(Some(
        absolute
            .strip_prefix(data_dir)
            .map(Path::to_path_buf)
            .unwrap_or(absolute),
    ))
}

fn artifact_refs_from_context(content: &str, label: &str) -> Vec<TaskArtifactRef> {
    let Ok(value) = serde_json::from_str::<Value>(content) else {
        return Vec::new();
    };
    let (path_key, summary_key) = if label.ends_with("_diff") {
        ("diff_artifact", "diff_summary")
    } else {
        ("body_artifact", "body_summary")
    };
    let mut refs = Vec::new();
    collect_artifact_refs(&value, path_key, summary_key, label, &mut refs);
    refs
}

fn collect_artifact_refs(
    value: &Value,
    path_key: &str,
    summary_key: &str,
    label: &str,
    refs: &mut Vec<TaskArtifactRef>,
) {
    match value {
        Value::Object(map) => {
            if let Some(path) = map.get(path_key).and_then(Value::as_str) {
                let summary = map
                    .get(summary_key)
                    .and_then(Value::as_str)
                    .map(ToString::to_string)
                    .unwrap_or_else(|| format!("GitHub {label} artifact"));
                refs.push(TaskArtifactRef {
                    label: label.to_string(),
                    path: PathBuf::from(path),
                    summary,
                    created_at: Utc::now(),
                });
            }
            for child in map.values() {
                collect_artifact_refs(child, path_key, summary_key, label, refs);
            }
        }
        Value::Array(items) => {
            for child in items {
                collect_artifact_refs(child, path_key, summary_key, label, refs);
            }
        }
        _ => {}
    }
}

fn github_event_metadata(
    action: &str,
    target: &str,
    number: u64,
    summary: String,
    url: Option<String>,
    artifact: Option<PathBuf>,
) -> Value {
    let artifacts = artifact
        .map(|path| {
            json!([TaskArtifactRef {
                label: format!("github_{action}"),
                path,
                summary: summary.clone(),
                created_at: Utc::now(),
            }])
        })
        .unwrap_or_else(|| json!([]));
    json!({
        "task_updates": {
            "github_event": TaskGithubEvent {
                id: format!("gh_{}", &Uuid::new_v4().to_string()[..8]),
                action: action.to_string(),
                target: target.to_string(),
                number,
                summary,
                url,
                recorded_at: Utc::now(),
            },
            "artifacts": artifacts
        }
    })
}

fn validate_evidence(input: &Value, closing: bool) -> Result<(), ToolError> {
    let evidence = input
        .get("evidence")
        .and_then(Value::as_object)
        .ok_or_else(|| ToolError::invalid_input("evidence object is required"))?;
    if closing {
        let criteria = input
            .get("acceptance_criteria")
            .and_then(Value::as_array)
            .filter(|items| !items.is_empty())
            .ok_or_else(|| ToolError::invalid_input("acceptance_criteria must be non-empty"))?;
        if criteria
            .iter()
            .any(|item| item.as_str().unwrap_or("").trim().is_empty())
        {
            return Err(ToolError::invalid_input(
                "acceptance_criteria entries must be non-empty",
            ));
        }
        for key in ["files_changed", "tests_run", "final_status"] {
            if !evidence.contains_key(key) {
                return Err(ToolError::invalid_input(format!(
                    "closure evidence missing {key}"
                )));
            }
        }
    }
    Ok(())
}

fn summarize(text: &str, limit: usize) -> String {
    let mut out = String::new();
    for (idx, ch) in text.chars().enumerate() {
        if idx >= limit.saturating_sub(3) {
            out.push_str("...");
            return out;
        }
        if ch.is_control() && ch != '\n' && ch != '\t' {
            continue;
        }
        out.push(ch);
    }
    out
}

fn sanitize_filename(input: &str) -> String {
    let mut out = String::new();
    for ch in input.chars() {
        if ch.is_ascii_alphanumeric() || ch == '_' || ch == '-' {
            out.push(ch);
        } else {
            out.push('_');
        }
    }
    if out.is_empty() {
        "artifact".to_string()
    } else {
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tools::spec::ToolSpec;

    #[test]
    fn close_schema_requires_structured_evidence() {
        let schema = GithubTool::alias("github_close_issue", "close_issue").input_schema();
        assert!(
            schema["properties"]["evidence"]["required"]
                .as_array()
                .expect("required")
                .contains(&json!("tests_run"))
        );
    }

    #[test]
    fn close_pr_schema_requires_structured_evidence() {
        let schema = GithubTool::alias("github_close_pr", "close_pr").input_schema();
        assert!(
            schema["properties"]["evidence"]["required"]
                .as_array()
                .expect("required")
                .contains(&json!("tests_run"))
        );
    }

    #[test]
    fn close_tools_distinguish_issue_and_pr_wording() {
        assert_eq!(GithubCloseTarget::Issue.display(), "issue");
        assert_eq!(GithubCloseTarget::Pr.display(), "PR");
        assert!(
            GithubTool::alias("github_close_issue", "close_issue")
                .description()
                .contains("github_close_pr")
        );
        assert!(
            GithubTool::alias("github_close_pr", "close_pr")
                .description()
                .contains("pull request")
        );
    }

    #[test]
    fn missing_close_evidence_refuses() {
        let input = json!({
            "number": 1,
            "acceptance_criteria": ["done"],
            "evidence": { "files_changed": [] }
        });
        let err = validate_evidence(&input, true).expect_err("should refuse");
        assert!(err.to_string().contains("tests_run"));
    }

    #[test]
    fn canonical_schema_lists_all_actions() {
        let schema = GithubTool::new("github").input_schema();
        let actions = schema["properties"]["action"]["enum"]
            .as_array()
            .expect("action enum");
        for action in [
            "issue_context",
            "pr_context",
            "comment",
            "close_issue",
            "close_pr",
        ] {
            assert!(
                actions.iter().any(|value| value.as_str() == Some(action)),
                "canonical schema must offer action {action}"
            );
        }
        for field in ["number", "target", "body", "evidence", "acceptance_criteria"] {
            assert!(
                schema["properties"][field].is_object(),
                "canonical schema must carry union field {field}"
            );
        }
        assert_eq!(schema["additionalProperties"], json!(false));
    }

    #[test]
    fn read_only_variant_only_offers_read_actions() {
        let tool = GithubTool::read_only("github");
        let schema = tool.input_schema();
        assert_eq!(
            schema["properties"]["action"]["enum"],
            json!(["issue_context", "pr_context"])
        );
        assert!(!schema["properties"]["body"].is_object());
        assert_eq!(tool.approval_requirement(), ApprovalRequirement::Auto);
        assert!(tool.is_read_only());
    }

    #[test]
    fn aliases_hide_from_model_and_force_action() {
        let comment = GithubTool::alias("github_comment", "comment");
        assert!(!comment.model_visible());
        assert_eq!(comment.name(), "github_comment");
        assert_eq!(comment.approval_requirement(), ApprovalRequirement::Required);
        assert!(
            comment
                .capabilities()
                .contains(&ToolCapability::Network)
        );

        let issue = GithubTool::alias("github_issue_context", "issue_context");
        assert_eq!(issue.approval_requirement(), ApprovalRequirement::Auto);
        assert!(issue.is_read_only_for(&json!({})));

        let canonical = GithubTool::new("github");
        assert!(canonical.model_visible());
        assert_eq!(
            canonical.approval_requirement_for(&json!({"action": "pr_context"})),
            ApprovalRequirement::Auto
        );
        assert_eq!(
            canonical.approval_requirement_for(&json!({"action": "close_pr"})),
            ApprovalRequirement::Required
        );
        assert!(canonical.is_read_only_for(&json!({"action": "issue_context"})));
        assert!(!canonical.is_read_only_for(&json!({"action": "comment"})));
    }

    #[test]
    fn canonical_rejects_unknown_or_missing_action() {
        let tool = GithubTool::new("github");
        let err = tool
            .resolve_action(&json!({}))
            .expect_err("missing action must fail");
        assert!(err.to_string().contains("missing `action`"));
        let err = tool
            .resolve_action(&json!({"action": "merge"}))
            .expect_err("unknown action must fail");
        assert!(err.to_string().contains("invalid action"));

        let read_only = GithubTool::read_only("github");
        let err = read_only
            .resolve_action(&json!({"action": "close_pr"}))
            .expect_err("read-only surface must reject write actions");
        assert!(err.to_string().contains("invalid action"));
    }
}
