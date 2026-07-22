//! Semantic aliases for the model-facing action tools.
//!
//! `Bash`, `File`, `Git`, `Run`, and `Web` deliberately keep their canonical
//! names at the execution and audit boundaries.  Presentation and policy
//! consumers, however, still understand the older per-action names.  Resolve
//! that semantic name in one place so live calls and saved legacy transcripts
//! receive identical downstream behavior without rewriting the original call.

use serde_json::Value;

pub(crate) const CANONICAL_ACTION_ALIASES: &[(&str, &str, &str)] = &[
    ("Bash", "run", "exec_shell"),
    ("Bash", "wait", "exec_shell_wait"),
    ("Bash", "interact", "exec_shell_interact"),
    ("Bash", "cancel", "exec_shell_cancel"),
    ("File", "read", "read_file"),
    ("File", "list", "list_dir"),
    ("File", "search_name", "file_search"),
    ("File", "search_content", "grep_files"),
    ("File", "write", "write_file"),
    ("File", "edit", "edit_file"),
    ("File", "patch", "apply_patch"),
    ("Git", "status", "git_status"),
    ("Git", "diff", "git_diff"),
    ("Git", "log", "git_log"),
    ("Git", "show", "git_show"),
    ("Git", "blame", "git_blame"),
    ("Run", "tests", "run_tests"),
    ("Run", "verifiers", "run_verifiers"),
    ("Web", "search", "web_search"),
    ("Web", "fetch", "fetch_url"),
    ("Web", "wait", "wait_for_dev_server"),
];

/// Resolve a canonical action tool to the legacy name for that exact action.
///
/// Missing actions follow each wrapper's execution default. Unknown actions
/// stay canonical so policy remains conservative and the eventual tool error
/// is attributed to the call the model actually made.
#[must_use]
pub(crate) fn canonical_action_alias<'a>(tool_name: &'a str, input: &Value) -> &'a str {
    let default_action = match tool_name {
        "Bash" => Some("run"),
        "File" => Some("read"),
        "Git" => Some("status"),
        "Run" => Some("tests"),
        "Web" => Some("search"),
        _ => None,
    };
    let Some(default_action) = default_action else {
        return tool_name;
    };
    let action = input
        .get("action")
        .and_then(Value::as_str)
        .unwrap_or(default_action);

    CANONICAL_ACTION_ALIASES
        .iter()
        .find_map(|(family, candidate_action, alias)| {
            (*family == tool_name && *candidate_action == action).then_some(*alias)
        })
        .unwrap_or(tool_name)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn every_canonical_action_resolves_to_its_legacy_semantic_alias() {
        for (family, action, alias) in CANONICAL_ACTION_ALIASES {
            assert_eq!(
                canonical_action_alias(family, &json!({"action": action})),
                *alias,
                "{family}.{action}"
            );
        }
    }

    #[test]
    fn canonical_defaults_match_wrapper_execution_defaults() {
        for (family, alias) in [
            ("Bash", "exec_shell"),
            ("File", "read_file"),
            ("Git", "git_status"),
            ("Run", "run_tests"),
            ("Web", "web_search"),
        ] {
            assert_eq!(
                canonical_action_alias(family, &json!({})),
                alias,
                "{family}"
            );
        }
    }

    #[test]
    fn legacy_unknown_and_invalid_calls_keep_their_original_names() {
        for name in ["exec_shell", "read_file", "future_tool"] {
            assert_eq!(canonical_action_alias(name, &json!({})), name);
        }
        assert_eq!(
            canonical_action_alias("File", &json!({"action": "delete"})),
            "File"
        );
        assert_eq!(
            canonical_action_alias("Bash", &json!({"action": 42})),
            "exec_shell"
        );
    }
}
