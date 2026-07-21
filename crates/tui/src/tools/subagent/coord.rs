//! Coordination records for delegated Work (#4647).
//!
//! Decision records, write-scope claims, and contention detection for parallel
//! agent work. Parallel work may proceed only when scopes and contracts do not
//! collide silently.

use serde::{Deserialize, Serialize};

/// Status of a coordination decision.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DecisionStatus {
    Proposed,
    Accepted,
    Superseded,
}

/// A bounded coordination decision record (#4647).
///
/// Persisted with stable subject, concise constraints, one active owner,
/// applicability scope, evidence handles, and sequence/version.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DecisionRecord {
    pub decision_id: String,
    pub subject: String,
    pub status: DecisionStatus,
    pub owner: String,
    pub scope: Vec<String>,
    pub constraints: Vec<String>,
    pub evidence_handles: Vec<String>,
    pub version: u32,
    pub sequence: u64,
}

/// A write-scope claim for a write-capable child (#4647).
///
/// Declares expected repo-relative paths/trees and named contracts.
/// This is coordination metadata, not another approval system.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WriteScopeClaim {
    pub owner: String,
    pub roots: Vec<String>,
    pub exact_files: Vec<String>,
    pub contracts: Vec<String>,
}

impl WriteScopeClaim {
    /// Check whether this claim overlaps with another. A claim overlaps when
    /// either normalized tree contains the other or exact files collide.
    #[must_use]
    pub fn overlaps(&self, other: &WriteScopeClaim) -> bool {
        for root_a in &self.roots {
            for root_b in &other.roots {
                if root_a.starts_with(root_b.as_str()) || root_b.starts_with(root_a.as_str()) {
                    return true;
                }
            }
        }
        for file_a in &self.exact_files {
            if other.exact_files.iter().any(|f| f == file_a) {
                return true;
            }
        }
        false
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn overlapping_roots_detected() {
        let a = WriteScopeClaim {
            owner: "agent-a".into(),
            roots: vec!["src/tui/".into()],
            exact_files: vec![],
            contracts: vec![],
        };
        let b = WriteScopeClaim {
            owner: "agent-b".into(),
            roots: vec!["src/tui/widgets/".into()],
            exact_files: vec![],
            contracts: vec![],
        };
        assert!(a.overlaps(&b));
    }

    #[test]
    fn disjoint_roots_no_overlap() {
        let a = WriteScopeClaim {
            owner: "agent-a".into(),
            roots: vec!["src/tui/".into()],
            exact_files: vec![],
            contracts: vec![],
        };
        let b = WriteScopeClaim {
            owner: "agent-b".into(),
            roots: vec!["src/core/".into()],
            exact_files: vec![],
            contracts: vec![],
        };
        assert!(!a.overlaps(&b));
    }

    #[test]
    fn exact_file_collision_detected() {
        let a = WriteScopeClaim {
            owner: "agent-a".into(),
            roots: vec![],
            exact_files: vec!["src/main.rs".into()],
            contracts: vec![],
        };
        let b = WriteScopeClaim {
            owner: "agent-b".into(),
            roots: vec![],
            exact_files: vec!["src/main.rs".into()],
            contracts: vec![],
        };
        assert!(a.overlaps(&b));
    }
}
