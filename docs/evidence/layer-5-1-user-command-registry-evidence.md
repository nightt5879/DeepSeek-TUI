# Layer 5.1: User Command Registry And Loading Boundary — Evidence Summary

**EPIC-002**: Staged command-boundary refactor (Hmbown/CodeWhale#2870)

**FEAT-010**: Layer 5.1 — User Command Registry And Loading Boundary

**Status**: ✅ Verified on production/test commit

**Date**: 2026-07-21

## Provenance And Correction

Paulo Aboim Pinto's exact evidence commit from PR #4046,
`75e08e5c67f555b5cda8511742c824bb74bb6c1d`, remains in this integration's
ancestry. That document correctly identified the dedicated registry boundary,
hidden-command behavior, and runtime `allowed-tools` propagation, but its claim
that the written Layer 5.1 contract was already complete was too broad.

Paulo's follow-up audit on #2870 identified the missing `name`, `usage`, and
`arguments` frontmatter fields and the missing explicit proof that a malformed
file does not block a valid sibling. The production-and-test follow-up
`2717843394050ff1b4f651f931e3894432ace8f8` implements and verifies that written
contract. This revision corrects the claim while preserving the original commit
and authorship as the provenance record.

## Acceptance Criteria Verification

| Criterion | Status | Evidence |
|-----------|--------|----------|
| User commands load through a dedicated boundary separate from built-ins | Implemented | `commands::execute()` calls `user_registry::try_dispatch()` before the built-in registry; `UserCommandRegistry` owns user metadata and errors. |
| Parse `name`, `description`, `usage`, `arguments`, `hidden`, and `allowed-tools` | Implemented | `registry_loads_markdown_metadata`, `frontmatter_name_replaces_filename_canonical_name`, and `parser_preserves_layer_5_1_frontmatter_fields`. |
| Invalid frontmatter is recoverable per file and does not block valid commands | Implemented | `malformed_file_is_recoverable_and_valid_sibling_still_dispatches`; malformed losing duplicates are isolated by `malformed_losing_name_override_does_not_poison_valid_winner`. |
| Hidden commands dispatch directly but are filtered from palette and slash completion | Implemented | `hidden_user_commands_still_dispatch_directly`, `hidden_frontmatter_name_override_suppresses_shadowed_builtin`, and `hidden_name_override_filters_shadowed_builtin_from_slash_completion`. |
| `allowed-tools` reaches runtime dispatch, including explicit-empty deny-all | Implemented | `dispatch_uses_frontmatter_name_arguments_and_allowed_tools` and `empty_allowed_tools_frontmatter_blocks_all_tools`. |
| Reload reflects command-file changes | Implemented | `registry_reloads_when_existing_command_file_changes`. |
| Name, alias, palette, completion, and dispatch override behavior is deterministic | Implemented | Frontmatter-name, directory-precedence, canonical/alias collision, palette, completion, and dispatch tests in `user_registry.rs`, `command_palette.rs`, `widgets/mod.rs`, and `ui/tests.rs`. |

## Final Contract

- The normalized filename is the default canonical command name.
- A valid frontmatter `name` replaces the filename default. The filename is not
  an implicit alias; retain it explicitly with `alias` or `aliases` when needed.
- Directory precedence is resolved first. Within a directory, normalized
  filenames are ordered. Distinct files that resolve to the same canonical
  name use first-wins order and record a recoverable error for each loser.
- An alias cannot replace any canonical user-command name. Duplicate aliases
  also use first-wins order.
- A malformed winning command reports its error on direct dispatch and never
  falls through to a built-in. A malformed losing duplicate cannot poison a
  valid winner, and malformed siblings cannot block valid commands.
- Palette and completion presentation use non-empty `usage`, then legacy
  `argument-hint`, then `arguments`. These are presentation/input hints, not
  runtime validation.
- Existing template behavior is unchanged: `$ARGUMENTS` receives the complete
  argument tail and `$1`, `$2`, and later positionals use whitespace splitting.
- Hidden commands still shadow built-ins and dispatch directly, but do not
  appear in palette or slash completion.
- `allowed-tools` continues to normalize tool names into runtime state; an
  explicitly empty value remains `Some(Vec::new())`, blocking every tool.

The public version of this contract is recorded in
`docs/architecture/command-dispatch.md`.

## Verification Receipts

These commands ran serially against the shared target with incremental
compilation disabled and one build job. Every focused test command emitted the
same macOS linker warning that `__eh_frame` exceeded the compact-unwind table's
16 MB encoding limit; the warning did not fail linking or any test.

| Command | Result |
|---------|--------|
| `CARGO_TARGET_DIR=/Volumes/VIXinSSD/CW/codewhale/target CARGO_INCREMENTAL=0 CARGO_BUILD_JOBS=1 cargo test -p codewhale-tui --bin codewhale-tui --locked user_registry --jobs 1 -- --test-threads=1` | ✅ PASS — 25 passed, 0 failed, 7812 filtered out |
| `CARGO_TARGET_DIR=/Volumes/VIXinSSD/CW/codewhale/target CARGO_INCREMENTAL=0 CARGO_BUILD_JOBS=1 cargo test -p codewhale-tui --bin codewhale-tui --locked user_commands --jobs 1 -- --test-threads=1` | ✅ PASS — 25 passed, 0 failed, 7812 filtered out |
| `CARGO_TARGET_DIR=/Volumes/VIXinSSD/CW/codewhale/target CARGO_INCREMENTAL=0 CARGO_BUILD_JOBS=1 cargo test -p codewhale-tui --bin codewhale-tui --locked command_palette --jobs 1 -- --test-threads=1` | ✅ PASS — 27 passed, 0 failed, 7810 filtered out |
| `CARGO_TARGET_DIR=/Volumes/VIXinSSD/CW/codewhale/target CARGO_INCREMENTAL=0 CARGO_BUILD_JOBS=1 cargo test -p codewhale-tui --bin codewhale-tui --locked slash_completion --jobs 1 -- --test-threads=1` | ✅ PASS — 22 passed, 0 failed, 7815 filtered out |
| `CARGO_TARGET_DIR=/Volumes/VIXinSSD/CW/codewhale/target CARGO_INCREMENTAL=0 CARGO_BUILD_JOBS=1 cargo test -p codewhale-tui --bin codewhale-tui --locked apply_slash_menu_selection --jobs 1 -- --test-threads=1` | ✅ PASS — 5 passed, 0 failed, 7832 filtered out |
| `CARGO_TARGET_DIR=/Volumes/VIXinSSD/CW/codewhale/target CARGO_INCREMENTAL=0 CARGO_BUILD_JOBS=1 cargo check -p codewhale-tui --bin codewhale-tui --locked --jobs 1` | ✅ PASS |
| `CARGO_TARGET_DIR=/Volumes/VIXinSSD/CW/codewhale/target CARGO_INCREMENTAL=0 CARGO_BUILD_JOBS=1 cargo fmt --all -- --check` | ✅ PASS |
| `git diff --check` | ✅ PASS |
