//! Worker runtime profile — the per-role capability contract for a CodeWhale
//! worker (#3217, #3211, #3213, and the child-permission-intersection issues
//! #414 / #426 / #1186).
//!
//! This is the **WhaleFlow substrate**: every detached worker — whether launched
//! as a `agent_open` sub-agent or a Fleet worker — should run under a profile
//! that bounds what it may do (permissions, shell access, tool scope, model
//! route, recursion budget, foreground/background). A child profile is always
//! **derived** from its parent and can never escalate beyond it.
//!
//! Scope: this module is the **foundation only**. It defines the contract and
//! the parent→child derivation with tests. Wiring `agent_open` and the Fleet
//! worker launch to *build and enforce* these profiles — and mapping the legacy
//! `AgentWorkerToolProfile` / per-worker shell boolean onto `ShellPolicy` /
//! `ToolScope` — is a follow-up pass (#3217). Nothing here changes existing
//! behavior yet.

#![allow(dead_code)] // foundation: consumers are wired in a follow-up (#3217).

use crate::tools::subagent::SubAgentType;

/// Coarse capability classes a worker may exercise, beyond read access (reads
/// are always permitted). A child may only ever hold a *subset* of its parent's
/// capabilities.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PermissionSet {
    /// May modify the workspace (`write_file` / `edit_file` / `apply_patch`).
    pub write: bool,
    /// May use network-capable tools (web search/fetch, networked MCP servers).
    pub network: bool,
}

impl PermissionSet {
    /// Full capabilities (write + network).
    pub const fn full() -> Self {
        Self {
            write: true,
            network: true,
        }
    }

    /// Read-only: no write, no network.
    pub const fn read_only() -> Self {
        Self {
            write: false,
            network: false,
        }
    }

    /// Intersection: a capability is granted only if **both** sets grant it.
    /// This is the core non-escalation primitive — `parent.intersect(child)`
    /// can never produce a capability the parent lacks.
    #[must_use]
    pub fn intersect(self, other: Self) -> Self {
        Self {
            write: self.write && other.write,
            network: self.network && other.network,
        }
    }
}

/// Shell access policy — the replacement for the legacy per-worker shell boolean
/// (#3217). Ordered from most to least restrictive so `min` yields the safer of
/// two policies.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum ShellPolicy {
    /// No shell access.
    None,
    /// Read-only / non-mutating commands only (the policy enforcement lives in
    /// the exec/sandbox layer; this is the declared intent).
    ReadOnly,
    /// Full shell access.
    Full,
}

impl ShellPolicy {
    /// The more restrictive (safer) of two policies. A child can never exceed
    /// its parent's shell policy.
    #[must_use]
    pub fn min_with(self, other: Self) -> Self {
        if self <= other { self } else { other }
    }
}

/// Which tools a worker may call. Mirrors the existing `AgentWorkerToolProfile`
/// (`Inherited` / `Explicit`) so the two can be reconciled when this is wired in.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ToolScope {
    /// Inherit the parent's tool surface.
    Inherit,
    /// Only the explicitly listed tool names.
    Explicit(Vec<String>),
}

/// How a worker's model is selected — the heterogeneous-model piece that makes
/// WhaleFlow distinct from a single-model sub-agent fan-out (#3217, #2027,
/// #1768). A scout/tool role can ride a cheap flash route while a synthesis role
/// inherits the (typically larger) session model.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ModelRoute {
    /// Same model as the parent / session.
    Inherit,
    /// Provider-aware automatic routing (e.g. flash for scouts, pro for synthesis).
    Auto,
    /// An explicit model id, validated against the active provider at spawn time.
    Fixed(String),
}

/// The capability contract a single worker runs under.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WorkerRuntimeProfile {
    pub role: SubAgentType,
    pub permissions: PermissionSet,
    pub shell: ShellPolicy,
    pub tools: ToolScope,
    pub model: ModelRoute,
    /// Explicit provider override; `None` inherits the parent/session provider.
    pub provider: Option<String>,
    /// Remaining nested-delegation budget. A worker may spawn children while
    /// `max_spawn_depth > 0`; each level decrements it. Clamped to the workspace
    /// ceiling.
    pub max_spawn_depth: u32,
    /// Whether the worker runs detached (background) or inline (foreground).
    pub background: bool,
}

impl WorkerRuntimeProfile {
    /// The default profile for a role — the per-role posture. Mirrors the role
    /// stances documented in `docs/SUBAGENTS.md` (explore/plan/review are
    /// read-only; verifier runs tests; implementer/general/tool write).
    #[must_use]
    pub fn for_role(role: SubAgentType) -> Self {
        let (permissions, shell) = match role {
            // Read-only investigators.
            SubAgentType::Explore | SubAgentType::Review => {
                (PermissionSet::read_only(), ShellPolicy::ReadOnly)
            }
            // Planner: analysis only, no shell.
            SubAgentType::Plan => (PermissionSet::read_only(), ShellPolicy::None),
            // Verifier: doesn't modify code, but runs the test suite.
            SubAgentType::Verifier => (PermissionSet::read_only(), ShellPolicy::Full),
            // Doers.
            SubAgentType::Implementer | SubAgentType::General | SubAgentType::ToolAgent => {
                (PermissionSet::full(), ShellPolicy::Full)
            }
            // Custom starts locked down; the caller opens specific tools explicitly.
            SubAgentType::Custom => (PermissionSet::read_only(), ShellPolicy::None),
        };
        // Cheap, machine-bound roles default to auto-routing (flash lane); the
        // rest inherit the session model.
        let model = match role {
            SubAgentType::Explore | SubAgentType::ToolAgent => ModelRoute::Auto,
            _ => ModelRoute::Inherit,
        };
        Self {
            role,
            permissions,
            shell,
            tools: ToolScope::Inherit,
            model,
            provider: None,
            max_spawn_depth: codewhale_config::DEFAULT_SPAWN_DEPTH,
            background: true,
        }
    }

    /// Derive a child profile from this (parent) profile and a `requested` child
    /// profile. The result is the **intersection** of the two — it can never
    /// grant the child something the parent lacks (#414 / #426 / #1186):
    ///
    /// - permissions are AND-ed,
    /// - shell takes the more restrictive policy,
    /// - an explicit parent tool set bounds the child's tool set,
    /// - the spawn-depth budget decrements by one level and clamps to the ceiling.
    ///
    /// The child keeps its own requested role, model route, and
    /// foreground/background preference (these don't grant capability), but its
    /// provider falls back to the parent's when unset.
    #[must_use]
    pub fn derive_child(&self, requested: &WorkerRuntimeProfile) -> WorkerRuntimeProfile {
        let permissions = self.permissions.intersect(requested.permissions);
        let shell = self.shell.min_with(requested.shell);
        let tools = match (&self.tools, &requested.tools) {
            // Parent restricts to a set → the child can only narrow within it.
            (ToolScope::Explicit(parent), ToolScope::Explicit(child)) => ToolScope::Explicit(
                child
                    .iter()
                    .filter(|name| parent.contains(name))
                    .cloned()
                    .collect(),
            ),
            (ToolScope::Explicit(parent), ToolScope::Inherit) => {
                ToolScope::Explicit(parent.clone())
            }
            // Parent inherits the full surface → the child's request stands.
            (ToolScope::Inherit, child) => child.clone(),
        };
        // The child gets at most one level less budget than the parent, and never
        // more than it requested, clamped to the hard ceiling.
        let max_spawn_depth = requested
            .max_spawn_depth
            .min(self.max_spawn_depth.saturating_sub(1))
            .min(codewhale_config::MAX_SPAWN_DEPTH_CEILING);
        WorkerRuntimeProfile {
            role: requested.role.clone(),
            permissions,
            shell,
            tools,
            model: requested.model.clone(),
            provider: requested.provider.clone().or_else(|| self.provider.clone()),
            max_spawn_depth,
            background: requested.background,
        }
    }

    /// Whether this worker may still spawn a child (budget remaining).
    #[must_use]
    pub fn can_spawn_child(&self) -> bool {
        self.max_spawn_depth > 0
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn permission_intersection_never_escalates() {
        let parent = PermissionSet::read_only();
        let greedy_child = PermissionSet::full();
        // Even though the child asks for everything, the read-only parent wins.
        let got = parent.intersect(greedy_child);
        assert_eq!(got, PermissionSet::read_only());
    }

    #[test]
    fn shell_policy_min_takes_the_safer() {
        assert_eq!(
            ShellPolicy::ReadOnly.min_with(ShellPolicy::Full),
            ShellPolicy::ReadOnly
        );
        assert_eq!(
            ShellPolicy::None.min_with(ShellPolicy::ReadOnly),
            ShellPolicy::None
        );
        assert_eq!(
            ShellPolicy::Full.min_with(ShellPolicy::Full),
            ShellPolicy::Full
        );
    }

    #[test]
    fn for_role_postures_match_role_stances() {
        let explore = WorkerRuntimeProfile::for_role(SubAgentType::Explore);
        assert!(!explore.permissions.write, "explore must not write");
        assert_eq!(explore.shell, ShellPolicy::ReadOnly);
        assert_eq!(
            explore.model,
            ModelRoute::Auto,
            "explore rides the cheap auto lane"
        );

        let implementer = WorkerRuntimeProfile::for_role(SubAgentType::Implementer);
        assert!(implementer.permissions.write, "implementer writes");
        assert_eq!(implementer.shell, ShellPolicy::Full);

        let verifier = WorkerRuntimeProfile::for_role(SubAgentType::Verifier);
        assert!(
            !verifier.permissions.write,
            "verifier reports, does not patch"
        );
        assert_eq!(
            verifier.shell,
            ShellPolicy::Full,
            "verifier runs the test suite"
        );
    }

    #[test]
    fn child_cannot_escalate_beyond_a_readonly_parent() {
        let parent = WorkerRuntimeProfile::for_role(SubAgentType::Explore); // read-only
        let greedy = WorkerRuntimeProfile::for_role(SubAgentType::Implementer); // wants write + full shell
        let child = parent.derive_child(&greedy);
        assert!(
            !child.permissions.write,
            "a read-only parent cannot bear a writing child"
        );
        assert!(!child.permissions.network);
        assert_eq!(
            child.shell,
            ShellPolicy::ReadOnly,
            "child shell clamped to parent's"
        );
    }

    #[test]
    fn child_explicit_tools_are_bounded_by_parent() {
        let mut parent = WorkerRuntimeProfile::for_role(SubAgentType::General);
        parent.tools = ToolScope::Explicit(vec!["read_file".into(), "grep_files".into()]);
        let mut requested = WorkerRuntimeProfile::for_role(SubAgentType::General);
        requested.tools = ToolScope::Explicit(vec!["read_file".into(), "write_file".into()]);
        let child = parent.derive_child(&requested);
        match child.tools {
            ToolScope::Explicit(names) => {
                assert_eq!(
                    names,
                    vec!["read_file".to_string()],
                    "write_file not in parent set is dropped"
                );
            }
            ToolScope::Inherit => panic!("expected explicit tool scope"),
        }
    }

    #[test]
    fn spawn_depth_decrements_and_clamps() {
        let mut parent = WorkerRuntimeProfile::for_role(SubAgentType::General);
        parent.max_spawn_depth = 2;
        let mut requested = WorkerRuntimeProfile::for_role(SubAgentType::General);
        requested.max_spawn_depth = 99; // tries to grab more than the parent has
        let child = parent.derive_child(&requested);
        assert_eq!(
            child.max_spawn_depth, 1,
            "child budget is at most parent-1, never the requested 99"
        );
        assert!(child.can_spawn_child());

        let mut leaf_parent = WorkerRuntimeProfile::for_role(SubAgentType::General);
        leaf_parent.max_spawn_depth = 1;
        let grandchild = leaf_parent.derive_child(&requested);
        assert_eq!(grandchild.max_spawn_depth, 0);
        assert!(
            !grandchild.can_spawn_child(),
            "budget exhausted at the leaf"
        );
    }

    #[test]
    fn child_provider_falls_back_to_parent() {
        let mut parent = WorkerRuntimeProfile::for_role(SubAgentType::General);
        parent.provider = Some("moonshot".to_string());
        let requested = WorkerRuntimeProfile::for_role(SubAgentType::Explore); // provider None
        let child = parent.derive_child(&requested);
        assert_eq!(child.provider.as_deref(), Some("moonshot"));
    }
}
