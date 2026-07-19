//! Route-scoped capability facts.
//!
//! Capability state is deliberately three-valued: an absent catalog fact is
//! unknown, not unsupported, and must never be promoted to supported by a
//! transport/protocol heuristic. These values travel with the exact provider
//! offering selected by [`super::resolver::RouteResolver`].

use serde::{Deserialize, Serialize};

/// Whether a resolved provider/model offering supports one capability.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CapabilityState {
    /// The selected offering explicitly reports support.
    Supported,
    /// The selected offering explicitly reports no support.
    Unsupported,
    /// The selected offering did not state the fact.
    #[default]
    Unknown,
}

impl CapabilityState {
    /// Preserve a sourced optional boolean as a three-state fact.
    #[must_use]
    pub const fn from_optional_bool(value: Option<bool>) -> Self {
        match value {
            Some(true) => Self::Supported,
            Some(false) => Self::Unsupported,
            None => Self::Unknown,
        }
    }

    /// Whether the source explicitly reports support.
    #[must_use]
    pub const fn is_supported(self) -> bool {
        matches!(self, Self::Supported)
    }
}

/// Capability facts owned by one provider/model route offering.
///
/// Fields without a current authoritative catalog source remain `Unknown`.
/// They are present now so live/provider-native facts can be added without
/// changing the candidate contract or guessing from request protocol.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct RouteCapabilities {
    #[serde(default)]
    pub attachments: CapabilityState,
    #[serde(default)]
    pub reasoning: CapabilityState,
    #[serde(default)]
    pub native_tool_calls: CapabilityState,
    #[serde(default)]
    pub structured_output: CapabilityState,
    #[serde(default)]
    pub parallel_tool_calls: CapabilityState,
    #[serde(default)]
    pub streaming: CapabilityState,
    #[serde(default)]
    pub prompt_caching: CapabilityState,
    #[serde(default)]
    pub server_side_web_search: CapabilityState,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn optional_boolean_preserves_unknown_and_false() {
        assert_eq!(
            CapabilityState::from_optional_bool(None),
            CapabilityState::Unknown
        );
        assert_eq!(
            CapabilityState::from_optional_bool(Some(false)),
            CapabilityState::Unsupported
        );
        assert_eq!(
            CapabilityState::from_optional_bool(Some(true)),
            CapabilityState::Supported
        );
    }

    #[test]
    fn unsourced_route_capabilities_default_to_unknown() {
        let capabilities = RouteCapabilities::default();
        assert_eq!(capabilities.streaming, CapabilityState::Unknown);
        assert_eq!(
            capabilities.server_side_web_search,
            CapabilityState::Unknown
        );
    }
}
