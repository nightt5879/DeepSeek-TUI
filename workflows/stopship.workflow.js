// Live acceptance on DeepSeek Flash and GLM-5-Turbo measured the inherited
// read-only prompt/tool envelope at 17,457 and 17,550 tokens before the first
// useful tool turn completed. Budget 24k per intended evidence turn, then add
// token-neutral max_steps headroom for the required final verdict. The token
// ceiling remains independent and the five role caps still total 360k.
export default workflow({
  "id": "stopship-release-acceptance",
  "goal": "Verify the current Codewhale Fleet, Workflow, Lane, Runtime, and gate receipt path without changing the workspace",
  "description": "Version-neutral, read-only release acceptance fixture. Every Fleet role inspects checked-in runtime evidence; no step creates branches, edits files, installs dependencies, or publishes anything.",
  "gates": [
    {
      "id": "scout-evidence",
      "role": "scout",
      "on": "role_complete",
      "gate": "approve",
      "on_fail": "block",
      "blocks_role": "implementer",
      "max_retries": 0,
      "artifact_kind": "source_evidence",
      "require_explicit_verdict": true
    },
    {
      "id": "implementation-plan",
      "role": "implementer",
      "on": "role_complete",
      "gate": "approve",
      "on_fail": "block",
      "blocks_role": "reviewer",
      "max_retries": 0,
      "artifact_kind": "verification_plan",
      "require_explicit_verdict": true
    },
    {
      "id": "review-findings",
      "role": "reviewer",
      "on": "role_complete",
      "gate": "review",
      "on_fail": "block",
      "blocks_role": "verifier",
      "max_retries": 0,
      "artifact_kind": "review_report",
      "require_explicit_verdict": true
    },
    {
      "id": "verifier-evidence",
      "role": "verifier",
      "on": "role_complete",
      "gate": "verify",
      "on_fail": "block",
      "blocks_role": "release_lead",
      "max_retries": 0,
      "artifact_kind": "verification_report",
      "require_explicit_verdict": true
    }
  ],
  "nodes": [
    {
      "sequence": {
        "id": "acceptance-chain",
        "children": [
          {
            "agent": {
              "id": "scout-runtime",
              "prompt": "Verify the runtime release-orchestration owners using only the five files in File scope. The host's typed run_started receipt already owns the compiled Workflow id and source path; do not re-verify the Workflow alias. You have at most six model responses and must reserve the verdict. Response 1 must make exactly one `grep_files` call with `path` set to `.` and this high-signal alternation pattern: `name = \"stopship\"|load_named_fleet|start_lane|resolve_workflow_agent|record_task_started|WorkflowUiEventKind::GateUpdated|WorkflowUiEventKind::RunCompleted|stopship_acceptance_fixture_emits_role_gate_and_terminal_receipts`. Set at most 80 results and 2 context lines. Do not add generic field names such as `resolved_profile` to the pattern. Do not call `grep_files` more than once and do not call `read_file`, `list_dir`, `file_search`, or any other tool. Response 2 must return the verdict with no tool calls; any later reserved response must do the same instead of gathering more evidence. Treat an exact match naming a call site, typed event constructor, or test assertion in a scoped file as source-owner evidence. If the single grep does not cover all six runtime owners, return BLOCK. The first non-empty line of your response must be exactly APPROVE or exactly BLOCK. Do not put any words before that verdict: no confirmation, summary, heading, or phrase such as `Here is the verdict`. After the verdict, include a `SOURCE EVIDENCE` section with concise `path: symbol` evidence for named Fleet loading, role-to-profile resolution, tmux Lane launch, typed task_started, gate_updated, and terminal run_completed receipts. A bare verdict is invalid. Do not edit files, create branches, run shell commands, access GitHub, or infer success where source evidence is absent.",
              "agent_type": "explore",
              "role": "scout",
              "mode": "read_only",
              "file_scope": [
                "workflows/stopship.workflow.js",
                "fleets/stopship.toml",
                "crates/cli/src/lib.rs",
                "crates/workflow/src/role_resolve.rs",
                "crates/tui/src/tools/workflow.rs"
              ],
              "budget": { "max_steps": 6, "timeout_secs": 480, "max_tokens": 96000 }
            }
          },
          {
            "agent": {
              "id": "plan-verification",
              "prompt": "Act as the Fleet implementer role for a verification-only acceptance run. Use only the promoted scout source_evidence handoff to produce a no-edit verification plan for the Fleet/Workflow/Lane/Runtime contract. Tools are intentionally unavailable because the promoted handoff is the evidence boundary; do not request source reads or gather new evidence. You have at most four model responses, but the first response must return APPROVE or BLOCK with no tool calls. After that verdict, include a `PLAN` section that copies at least three exact `path: symbol` citations from source_evidence and maps them to role resolution, gate promotion or blocking, and terminal Lane reconciliation. A bare verdict is invalid. The first non-empty line of your response must be exactly APPROVE or exactly BLOCK. Do not put any words before that verdict: no confirmation, summary, heading, or phrase such as `Here is the verdict`. Use APPROVE only when that concrete plan can be produced from the handoff; otherwise use BLOCK. This is deliberately not an implementation task: do not edit files, create branches, run shell commands, or propose fixes unrelated to missing acceptance evidence.",
              "agent_type": "implementer",
              "role": "implementer",
              "mode": "read_only",
              "permissions": { "deny_all_tools": true },
              "file_scope": [
                "crates/cli/src/lib.rs",
                "crates/lane/src/registry.rs",
                "crates/tui/src/tools/workflow.rs"
              ],
              "budget": { "max_steps": 4, "timeout_secs": 420, "max_tokens": 72000 }
            }
          },
          {
            "agent": {
              "id": "review-contract",
              "prompt": "Review only the promoted verification_plan handoff against the source_evidence citations it carries. Tools are intentionally unavailable because promoted handoffs are the evidence boundary; do not request source reads or gather new evidence. You have at most four model responses, but the first response must return APPROVE or BLOCK with no tool calls. After that verdict, include an `EVIDENCE REVIEW` section that carries forward the concrete `path: symbol` citations for declared role versus resolved profile, gate state versus prose verdict, tmux process exit versus terminal workflow receipt, and completed Lane child evidence. A bare verdict is invalid. The first non-empty line of your response must be exactly APPROVE or exactly BLOCK. Do not put any words before that verdict: no confirmation, summary, heading, or phrase such as `Here is the verdict`. Use APPROVE only when each claimed receipt has a concrete source owner in the promoted evidence; otherwise use BLOCK and list the missing evidence. Remain read-only and do not run shell commands or edit anything.",
              "agent_type": "review",
              "role": "reviewer",
              "mode": "read_only",
              "permissions": { "deny_all_tools": true },
              "file_scope": [
                "crates/cli/src/lib.rs",
                "crates/lane/src/registry.rs",
                "crates/tui/src/tools/workflow.rs"
              ],
              "budget": { "max_steps": 4, "timeout_secs": 420, "max_tokens": 72000 }
            }
          },
          {
            "agent": {
              "id": "verify-receipts",
              "prompt": "Statically verify only the promoted review_report and the source-evidence citations it carries against the required receipt contract. Tools are intentionally unavailable because promoted handoffs are the evidence boundary; do not request source reads or gather new evidence. You have at most four model responses, but the first response must return APPROVE or BLOCK with no tool calls. After that verdict, include an `EVIDENCE MATRIX` with a concrete test name or `path: symbol` owner for role-resolved task_started, gate_updated, run_completed, metadata, and the Lane exit receipt. A bare verdict is invalid. The first non-empty line of your response must be exactly APPROVE or exactly BLOCK. Do not put any words before that verdict: no confirmation, summary, heading, or phrase such as `Here is the verdict`. Use APPROVE only when every required receipt is covered; otherwise use BLOCK. Do not run commands, edit files, or create build artifacts; the host gate interprets the explicit first-line verdict.",
              "agent_type": "verifier",
              "role": "verifier",
              "mode": "read_only",
              "permissions": { "deny_all_tools": true },
              "file_scope": [
                "crates/cli/src/lib.rs",
                "crates/lane/src/registry.rs",
                "crates/tui/src/tools/workflow.rs"
              ],
              "budget": { "max_steps": 4, "timeout_secs": 420, "max_tokens": 72000 }
            }
          },
          {
            "agent": {
              "id": "release-receipt",
              "prompt": "Use only the promoted verification_report handoff to produce the final acceptance receipt for the Fleet/Workflow/Lane/Runtime contract. Tools are intentionally unavailable because promoted handoffs are the evidence boundary; do not request source reads or gather new evidence. You have at most three model responses, but the first response must return APPROVE or BLOCK with no tool calls. After that verdict, include a `FINAL RECEIPT` section carrying forward declared Fleet role and resolved profile evidence, every observed gate state, the required terminal workflow status, and their concrete source owners. A bare verdict is invalid. The first non-empty line of your response must be exactly APPROVE or exactly BLOCK. Do not put any words before that verdict: no confirmation, summary, heading, or phrase such as `Here is the verdict`. Use APPROVE only when that receipt can be produced from the handoff; otherwise use BLOCK and name the closure blocker. Never claim that source inspection substitutes for a live Lane log. Do not edit, publish, close issues, run shell commands, or mutate the workspace.",
              "agent_type": "general",
              "role": "release_lead",
              "mode": "read_only",
              "permissions": { "deny_all_tools": true },
              "file_scope": [
                "crates/cli/src/lib.rs",
                "crates/lane/src/registry.rs",
                "crates/tui/src/tools/workflow.rs"
              ],
              "budget": { "max_steps": 3, "timeout_secs": 300, "max_tokens": 48000 }
            }
          }
        ]
      }
    }
  ]
});
