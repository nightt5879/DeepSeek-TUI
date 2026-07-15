//! Truthful, session-local provider readiness.
//!
//! Static configuration can prove that credential material exists, but not
//! that an endpoint is reachable or an OAuth token is still entitled to a
//! model. `Ready` is therefore reserved for observed success in this session.

use std::borrow::Cow;

use crate::config::ApiProvider;
use crate::error_taxonomy::{ErrorCategory, ErrorEnvelope};
use codewhale_config::route::{LogicalModelRef, RouteRequest, RouteResolver};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum CredentialState {
    MissingKey,
    MissingLogin,
    Saved,
    Local,
    Legacy,
}

/// Credential route whose observed health may be reused. A provider can
/// expose more than one auth route (notably xAI and Moonshot), so provider id
/// alone is not a safe cache key: a successful API-key request must not make a
/// newly selected OAuth login appear verified.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ProviderAuthClass {
    ApiKey,
    OAuth,
    Local,
    Legacy,
}

/// Exact route whose observed health may be reused. This deliberately keeps
/// custom provider id, endpoint, model, and auth class together: success on
/// one private endpoint or model entitlement is not evidence for another.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ProviderRouteIdentity {
    provider: ApiProvider,
    provider_id: String,
    endpoint: String,
    model: String,
    auth_class: ProviderAuthClass,
}

pub(crate) fn route_identity_for_model(
    config: &crate::config::Config,
    provider: ApiProvider,
    model: &str,
) -> ProviderRouteIdentity {
    let configured = config.provider_config_for(provider);
    let provider_id = if provider == ApiProvider::Custom {
        config
            .provider
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .unwrap_or(provider.as_str())
    } else {
        provider.as_str()
    };
    let endpoint = configured
        .and_then(|entry| entry.base_url.as_deref())
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .or_else(|| {
            (matches!(provider, ApiProvider::Deepseek | ApiProvider::DeepseekCN)
                || (provider == ApiProvider::Custom && config.uses_legacy_literal_custom_route()))
            .then_some(config.base_url.as_deref())
            .flatten()
            .map(str::trim)
            .filter(|value| !value.is_empty())
        })
        .unwrap_or_else(|| provider.default_base_url())
        .trim_end_matches('/')
        .to_string();
    ProviderRouteIdentity {
        provider,
        provider_id: provider_id.to_string(),
        endpoint,
        model: model.trim().to_string(),
        auth_class: auth_class_for_provider(config, provider),
    }
}

pub(crate) fn auth_class_for_provider(
    config: &crate::config::Config,
    provider: ApiProvider,
) -> ProviderAuthClass {
    if provider == ApiProvider::OpenaiCodex {
        return ProviderAuthClass::OAuth;
    }
    let auth_mode = config
        .provider_config_for(provider)
        .and_then(|entry| entry.auth_mode.as_deref())
        .map(|mode| mode.trim().to_ascii_lowercase().replace(['-', ' '], "_"));
    if provider == ApiProvider::Moonshot
        && auth_mode
            .as_deref()
            .is_some_and(|mode| matches!(mode, "kimi" | "kimi_oauth" | "kimi_cli" | "oauth"))
    {
        return ProviderAuthClass::OAuth;
    }
    if provider == ApiProvider::Xai
        && auth_mode
            .as_deref()
            .is_some_and(crate::xai_oauth::auth_mode_uses_xai_oauth)
    {
        return ProviderAuthClass::OAuth;
    }
    match credential_state_for_provider(config, provider) {
        CredentialState::Local => ProviderAuthClass::Local,
        CredentialState::Legacy => ProviderAuthClass::Legacy,
        _ => ProviderAuthClass::ApiKey,
    }
}

pub(crate) fn credential_state_for_provider(
    config: &crate::config::Config,
    provider: ApiProvider,
) -> CredentialState {
    // DeepSeek CN is a TUI compatibility alias without a shared
    // `ProviderKind`, but it is still a live route handled by the runtime.
    // Treating it as `Legacy` makes setup claim it cannot run at all.
    if provider == ApiProvider::DeepseekCN {
        return if crate::config::has_api_key_for(config, provider) {
            CredentialState::Saved
        } else {
            CredentialState::MissingKey
        };
    }
    if provider.kind().is_none() {
        return CredentialState::Legacy;
    }
    if provider == ApiProvider::Custom {
        if config.uses_legacy_literal_custom_route() {
            if config
                .base_url
                .as_deref()
                .is_some_and(crate::config::base_url_uses_local_host)
            {
                return CredentialState::Local;
            }
            return if crate::config::has_api_key_for(config, provider) {
                CredentialState::Saved
            } else {
                CredentialState::MissingKey
            };
        }
        let Some(configured) = config.provider_config_for(provider) else {
            return CredentialState::MissingKey;
        };
        let auth_optional = configured.auth_mode.as_deref().is_some_and(|mode| {
            matches!(
                mode.trim()
                    .to_ascii_lowercase()
                    .replace(['-', ' '], "_")
                    .as_str(),
                "none" | "off" | "disabled" | "no_auth" | "noapi" | "no_api_key" | "anonymous"
            )
        }) || configured
            .base_url
            .as_deref()
            .is_some_and(crate::config::base_url_uses_local_host);
        if auth_optional {
            return CredentialState::Local;
        }
        let has_auth = (provider == config.api_provider()
            && crate::config::explicit_cli_api_key_override().is_some())
            || configured
                .api_key
                .as_deref()
                .is_some_and(|value| !value.trim().is_empty())
            || configured
                .api_key_env
                .as_deref()
                .map(str::trim)
                .filter(|name| !name.is_empty())
                .is_some_and(|name| {
                    std::env::var(name).is_ok_and(|value| !value.trim().is_empty())
                })
            || configured
                .auth
                .as_ref()
                .is_some_and(|auth| auth.validate().is_ok());
        return if has_auth {
            CredentialState::Saved
        } else {
            CredentialState::MissingKey
        };
    }
    if provider.is_self_hosted() {
        return CredentialState::Local;
    }

    let configured = config.provider_config_for(provider);
    let auth_mode = configured
        .and_then(|entry| entry.auth_mode.as_deref())
        .map(|mode| mode.trim().to_ascii_lowercase().replace(['-', ' '], "_"));
    let uses_kimi_oauth = provider == ApiProvider::Moonshot
        && auth_mode
            .as_deref()
            .is_some_and(|mode| matches!(mode, "kimi" | "kimi_oauth" | "kimi_cli" | "oauth"));
    if uses_kimi_oauth {
        return if crate::config::kimi_cli_credentials_valid() {
            CredentialState::Saved
        } else {
            CredentialState::MissingLogin
        };
    }
    if provider == ApiProvider::OpenaiCodex {
        return if crate::config::has_api_key_for(config, provider) {
            CredentialState::Saved
        } else {
            CredentialState::MissingLogin
        };
    }
    let xai_oauth_selected = provider == ApiProvider::Xai
        && auth_mode
            .as_deref()
            .is_some_and(crate::xai_oauth::auth_mode_uses_xai_oauth);
    if xai_oauth_selected {
        return if crate::xai_oauth::credentials_valid() {
            CredentialState::Saved
        } else {
            CredentialState::MissingLogin
        };
    }
    if provider == ApiProvider::Xai && explicit_provider_credential_present(config, provider) {
        return CredentialState::Saved;
    }
    if provider == ApiProvider::Xai {
        return CredentialState::MissingKey;
    }

    if crate::config::has_api_key_for(config, provider) {
        CredentialState::Saved
    } else {
        CredentialState::MissingKey
    }
}

fn explicit_provider_credential_present(
    config: &crate::config::Config,
    provider: ApiProvider,
) -> bool {
    (provider == config.api_provider() && crate::config::explicit_cli_api_key_override().is_some())
        || provider
            .env_vars()
            .iter()
            .any(|name| std::env::var(name).is_ok_and(|value| !value.trim().is_empty()))
        || config.provider_config_for(provider).is_some_and(|entry| {
            entry
                .api_key
                .as_deref()
                .is_some_and(|value| !value.trim().is_empty())
                || entry
                    .auth
                    .as_ref()
                    .is_some_and(|auth| auth.validate().is_ok())
        })
}

/// Validate the configured provider/model/endpoint route without making a
/// network request. This is shared by model inventory, `/model`, and Fleet so
/// none of them can mark a route selectable when `/provider` would reject it.
pub(crate) fn route_is_valid_for_model(
    config: &crate::config::Config,
    provider: ApiProvider,
    model: Option<&str>,
) -> bool {
    let compatibility_kind =
        (provider == ApiProvider::DeepseekCN).then_some(codewhale_config::ProviderKind::Deepseek);
    let Some(kind) = provider.kind().or(compatibility_kind) else {
        return true;
    };
    let configured = config.provider_config_for(provider);
    let configured_model = model
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string)
        .or_else(|| {
            configured
                .and_then(|entry| entry.model.as_deref())
                .map(str::trim)
                .filter(|value| !value.is_empty())
                .map(str::to_string)
        });
    let active_model = (provider == config.api_provider())
        .then(|| config.default_model())
        .filter(|model| !model.trim().is_empty() && !model.eq_ignore_ascii_case("auto"));
    let request = RouteRequest {
        explicit_provider: Some(kind),
        model_selector: configured_model.or(active_model).map(LogicalModelRef::from),
        saved_provider_model: None,
        base_url_override: if provider == ApiProvider::DeepseekCN {
            None
        } else if provider == ApiProvider::Custom && config.uses_legacy_literal_custom_route() {
            config
                .base_url
                .as_deref()
                .map(str::trim)
                .filter(|value| !value.is_empty())
                .map(str::to_string)
        } else {
            configured
                .and_then(|entry| entry.base_url.as_deref())
                .map(str::trim)
                .filter(|value| !value.is_empty())
                .map(str::to_string)
        },
    };
    RouteResolver::new()
        .resolve(&request)
        .is_ok_and(|candidate| candidate.validation.ok)
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum LastProviderCheck {
    Passed,
    Failed {
        category: ErrorCategory,
        message: String,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum ResolvedProviderReadiness {
    MissingKey,
    MissingLogin,
    SavedUnchecked,
    LocalUnchecked,
    Ready,
    SavedLastCheckFailed {
        category: ErrorCategory,
        message: String,
    },
    InvalidRoute,
    Legacy,
}

impl ResolvedProviderReadiness {
    pub(crate) fn label(&self) -> Cow<'static, str> {
        match self {
            Self::MissingKey => Cow::Borrowed("missing key"),
            Self::MissingLogin => Cow::Borrowed("missing login"),
            Self::SavedUnchecked => Cow::Borrowed("key saved · not checked"),
            Self::LocalUnchecked => Cow::Borrowed("local · not checked"),
            Self::Ready => Cow::Borrowed("ready"),
            Self::SavedLastCheckFailed { category, .. } => {
                Cow::Owned(format!("last check failed ({category})"))
            }
            Self::InvalidRoute => Cow::Borrowed("invalid route"),
            Self::Legacy => Cow::Borrowed("legacy"),
        }
    }

    pub(crate) fn detail(&self) -> Option<&str> {
        match self {
            Self::SavedLastCheckFailed { message, .. } => Some(message),
            _ => None,
        }
    }

    pub(crate) fn can_attempt(&self) -> bool {
        matches!(
            self,
            Self::SavedUnchecked
                | Self::LocalUnchecked
                | Self::Ready
                | Self::SavedLastCheckFailed { .. }
        )
    }
}

#[derive(Debug, Clone, Default)]
pub(crate) struct ProviderReadinessSnapshot {
    checks: Vec<(ProviderRouteIdentity, LastProviderCheck)>,
}

impl ProviderReadinessSnapshot {
    fn last(&self, identity: &ProviderRouteIdentity) -> Option<&LastProviderCheck> {
        self.checks
            .iter()
            .rev()
            .find_map(|(candidate, check)| (candidate == identity).then_some(check))
    }

    pub(crate) fn record_success(
        &mut self,
        config: &crate::config::Config,
        provider: ApiProvider,
        model: &str,
    ) {
        self.replace(
            route_identity_for_model(config, provider, model),
            LastProviderCheck::Passed,
        );
    }

    pub(crate) fn record_failure(
        &mut self,
        config: &crate::config::Config,
        provider: ApiProvider,
        model: &str,
        envelope: &ErrorEnvelope,
    ) {
        if !provider_owned_failure(envelope) {
            return;
        }
        self.replace(
            route_identity_for_model(config, provider, model),
            LastProviderCheck::Failed {
                category: envelope.category,
                message: sanitize_message(&envelope.message),
            },
        );
    }

    #[cfg(test)]
    pub(crate) fn record_failure_message(
        &mut self,
        config: &crate::config::Config,
        provider: ApiProvider,
        model: &str,
        category: ErrorCategory,
        message: &str,
    ) {
        self.replace(
            route_identity_for_model(config, provider, model),
            LastProviderCheck::Failed {
                category,
                message: sanitize_message(message),
            },
        );
    }

    fn replace(&mut self, identity: ProviderRouteIdentity, check: LastProviderCheck) {
        self.checks.retain(|(candidate, _)| candidate != &identity);
        self.checks.push((identity, check));
    }
}

pub(crate) fn resolve_with_identity(
    identity: &ProviderRouteIdentity,
    credentials: CredentialState,
    route_ok: bool,
    checks: &ProviderReadinessSnapshot,
) -> ResolvedProviderReadiness {
    if !route_ok {
        return ResolvedProviderReadiness::InvalidRoute;
    }
    match credentials {
        CredentialState::Legacy => ResolvedProviderReadiness::Legacy,
        CredentialState::MissingKey => ResolvedProviderReadiness::MissingKey,
        CredentialState::MissingLogin => ResolvedProviderReadiness::MissingLogin,
        CredentialState::Saved | CredentialState::Local => match checks.last(identity) {
            Some(LastProviderCheck::Passed) => ResolvedProviderReadiness::Ready,
            Some(LastProviderCheck::Failed { category, message }) => {
                ResolvedProviderReadiness::SavedLastCheckFailed {
                    category: *category,
                    message: message.clone(),
                }
            }
            None if credentials == CredentialState::Local => {
                ResolvedProviderReadiness::LocalUnchecked
            }
            None => ResolvedProviderReadiness::SavedUnchecked,
        },
    }
}

pub(crate) fn resolve_for_model(
    config: &crate::config::Config,
    provider: ApiProvider,
    model: &str,
    checks: &ProviderReadinessSnapshot,
) -> ResolvedProviderReadiness {
    resolve_with_identity(
        &route_identity_for_model(config, provider, model),
        credential_state_for_provider(config, provider),
        route_is_valid_for_model(config, provider, Some(model)),
        checks,
    )
}

fn provider_owned_failure(envelope: &ErrorEnvelope) -> bool {
    matches!(
        envelope.category,
        ErrorCategory::Network
            | ErrorCategory::Authentication
            | ErrorCategory::Authorization
            | ErrorCategory::RateLimit
            | ErrorCategory::Timeout
    )
}

fn sanitize_message(message: &str) -> String {
    crate::utils::truncate_with_ellipsis(message.trim(), 120, "…")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::error_taxonomy::ErrorSeverity;

    fn resolve_test_route(
        config: &crate::config::Config,
        provider: ApiProvider,
        model: &str,
        credentials: CredentialState,
        route_ok: bool,
        checks: &ProviderReadinessSnapshot,
    ) -> ResolvedProviderReadiness {
        resolve_with_identity(
            &route_identity_for_model(config, provider, model),
            credentials,
            route_ok,
            checks,
        )
    }

    #[test]
    fn saved_credentials_are_never_ready_without_observed_success() {
        let config = crate::config::Config::default();
        let checks = ProviderReadinessSnapshot::default();
        assert_eq!(
            resolve_test_route(
                &config,
                ApiProvider::Deepseek,
                "deepseek-v4-pro",
                CredentialState::Saved,
                true,
                &checks,
            ),
            ResolvedProviderReadiness::SavedUnchecked
        );
    }

    #[test]
    fn deepseek_cn_compatibility_alias_uses_real_key_readiness() {
        let _lock = crate::test_support::lock_test_env();
        let _key = crate::test_support::EnvVarGuard::remove("DEEPSEEK_API_KEY");
        let missing = crate::config::Config {
            provider: Some("deepseek-cn".to_string()),
            ..Default::default()
        };
        assert_eq!(
            credential_state_for_provider(&missing, ApiProvider::DeepseekCN),
            CredentialState::MissingKey
        );

        let configured = crate::config::Config {
            provider: Some("deepseek-cn".to_string()),
            providers: Some(crate::config::ProvidersConfig {
                deepseek_cn: crate::config::ProviderConfig {
                    api_key: Some("deepseek-cn-test-key".to_string()),
                    ..Default::default()
                },
                ..Default::default()
            }),
            ..Default::default()
        };
        assert_eq!(
            credential_state_for_provider(&configured, ApiProvider::DeepseekCN),
            CredentialState::Saved
        );
    }

    #[test]
    fn custom_readiness_identity_preserves_case_sensitive_route_parts() {
        let custom = std::collections::HashMap::from([
            (
                "CUSTOM".to_string(),
                crate::config::ProviderConfig {
                    kind: Some("openai-compatible".to_string()),
                    base_url: Some("https://example.test/TenantA/v1".to_string()),
                    model: Some("Vendor/ModelA".to_string()),
                    api_key: Some("test-key-a".to_string()),
                    ..Default::default()
                },
            ),
            (
                "custom".to_string(),
                crate::config::ProviderConfig {
                    kind: Some("openai-compatible".to_string()),
                    base_url: Some("https://example.test/tenanta/v1".to_string()),
                    model: Some("vendor/modela".to_string()),
                    api_key: Some("test-key-b".to_string()),
                    ..Default::default()
                },
            ),
        ]);
        let upper = crate::config::Config {
            provider: Some("CUSTOM".to_string()),
            providers: Some(crate::config::ProvidersConfig {
                custom: custom.clone(),
                ..Default::default()
            }),
            ..Default::default()
        };
        let lower = crate::config::Config {
            provider: Some("custom".to_string()),
            providers: Some(crate::config::ProvidersConfig {
                custom,
                ..Default::default()
            }),
            ..Default::default()
        };
        let upper_identity = route_identity_for_model(&upper, ApiProvider::Custom, "Vendor/ModelA");
        let lower_identity = route_identity_for_model(&lower, ApiProvider::Custom, "vendor/modela");

        assert_ne!(upper_identity, lower_identity);
        assert_eq!(upper_identity.provider_id, "CUSTOM");
        assert_eq!(upper_identity.endpoint, "https://example.test/TenantA/v1");
        assert_eq!(upper_identity.model, "Vendor/ModelA");

        let mut checks = ProviderReadinessSnapshot::default();
        checks.record_success(&upper, ApiProvider::Custom, "Vendor/ModelA");
        assert_eq!(
            resolve_for_model(&lower, ApiProvider::Custom, "vendor/modela", &checks),
            ResolvedProviderReadiness::SavedUnchecked
        );
    }

    #[test]
    fn success_and_provider_failure_replace_session_evidence() {
        let config = crate::config::Config::default();
        let mut checks = ProviderReadinessSnapshot::default();
        checks.record_success(&config, ApiProvider::Zai, "glm-5.2");
        assert_eq!(
            resolve_test_route(
                &config,
                ApiProvider::Zai,
                "glm-5.2",
                CredentialState::Saved,
                true,
                &checks,
            ),
            ResolvedProviderReadiness::Ready
        );

        checks.record_failure(
            &config,
            ApiProvider::Zai,
            "glm-5.2",
            &ErrorEnvelope::new(
                ErrorCategory::Authentication,
                ErrorSeverity::Error,
                false,
                "auth_failed",
                "token rejected",
            ),
        );
        let resolved = resolve_test_route(
            &config,
            ApiProvider::Zai,
            "glm-5.2",
            CredentialState::Saved,
            true,
            &checks,
        );
        assert!(matches!(
            resolved,
            ResolvedProviderReadiness::SavedLastCheckFailed {
                category: ErrorCategory::Authentication,
                ..
            }
        ));
        assert!(resolved.can_attempt());
    }

    #[test]
    fn tool_failures_do_not_poison_provider_health() {
        let config = crate::config::Config::default();
        let mut checks = ProviderReadinessSnapshot::default();
        checks.record_success(&config, ApiProvider::Deepseek, "deepseek-v4-pro");
        checks.record_failure(
            &config,
            ApiProvider::Deepseek,
            "deepseek-v4-pro",
            &ErrorEnvelope::new(
                ErrorCategory::Tool,
                ErrorSeverity::Error,
                false,
                "tool_failed",
                "shell failed",
            ),
        );
        assert!(matches!(
            resolve_test_route(
                &config,
                ApiProvider::Deepseek,
                "deepseek-v4-pro",
                CredentialState::Saved,
                true,
                &checks,
            ),
            ResolvedProviderReadiness::Ready
        ));
    }

    #[test]
    fn route_and_missing_auth_states_dominate_health() {
        let config = crate::config::Config {
            provider: Some("openai-codex".to_string()),
            ..Default::default()
        };
        let mut checks = ProviderReadinessSnapshot::default();
        checks.record_success(&config, ApiProvider::OpenaiCodex, "gpt-5.5");
        assert_eq!(
            resolve_test_route(
                &config,
                ApiProvider::OpenaiCodex,
                "gpt-5.5",
                CredentialState::MissingLogin,
                true,
                &checks
            ),
            ResolvedProviderReadiness::MissingLogin
        );
        assert_eq!(
            resolve_test_route(
                &config,
                ApiProvider::OpenaiCodex,
                "gpt-5.5",
                CredentialState::Saved,
                false,
                &checks
            ),
            ResolvedProviderReadiness::InvalidRoute
        );
    }

    #[test]
    fn api_key_success_does_not_verify_new_xai_oauth_route() {
        let _lock = crate::test_support::lock_test_env();
        let temp = tempfile::tempdir().expect("oauth fixture root");
        let oauth_path = temp.path().join("grok-auth.json");
        std::fs::write(
            &oauth_path,
            serde_json::to_vec(&serde_json::json!({
                "test-scope": {
                    "key": "expired-access-token",
                    "refresh_token": "saved-refresh-token",
                    "expires_at": "2000-01-01T00:00:00Z",
                    "auth_mode": "oidc"
                }
            }))
            .expect("oauth json"),
        )
        .expect("oauth fixture");
        let _oauth_path = crate::test_support::EnvVarGuard::set("GROK_AUTH_PATH", &oauth_path);

        let api_key_config = crate::config::Config {
            provider: Some("xai".to_string()),
            providers: Some(crate::config::ProvidersConfig {
                xai: crate::config::ProviderConfig {
                    api_key: Some("xai-test-key".to_string()),
                    auth_mode: Some("api_key".to_string()),
                    ..Default::default()
                },
                ..Default::default()
            }),
            ..Default::default()
        };
        let mut checks = ProviderReadinessSnapshot::default();
        let api_key_model = api_key_config.default_model();
        checks.record_success(&api_key_config, ApiProvider::Xai, &api_key_model);

        let mut oauth_config = api_key_config;
        oauth_config
            .providers
            .as_mut()
            .expect("providers")
            .xai
            .auth_mode = Some("oauth".to_string());
        assert_eq!(
            credential_state_for_provider(&oauth_config, ApiProvider::Xai),
            CredentialState::Saved,
            "fixture must have structurally valid OAuth material"
        );
        let model = oauth_config.default_model();
        assert_eq!(
            resolve_for_model(&oauth_config, ApiProvider::Xai, &model, &checks),
            ResolvedProviderReadiness::SavedUnchecked,
            "API-key evidence must not cross the auth-class boundary"
        );
    }

    #[test]
    fn observed_success_is_scoped_to_exact_model_endpoint_and_custom_provider() {
        let deepseek = crate::config::Config {
            api_key: Some("deepseek-test-key".to_string()),
            ..Default::default()
        };
        let mut checks = ProviderReadinessSnapshot::default();
        checks.record_success(&deepseek, ApiProvider::Deepseek, "deepseek-v4-pro");
        assert_eq!(
            resolve_for_model(
                &deepseek,
                ApiProvider::Deepseek,
                "deepseek-v4-flash",
                &checks,
            ),
            ResolvedProviderReadiness::SavedUnchecked,
            "one model entitlement must not verify a sibling model"
        );

        let custom_config = |id: &str, endpoint: &str| crate::config::Config {
            provider: Some(id.to_string()),
            providers: Some(crate::config::ProvidersConfig {
                custom: std::collections::HashMap::from([(
                    id.to_string(),
                    crate::config::ProviderConfig {
                        kind: Some("openai-compatible".to_string()),
                        base_url: Some(endpoint.to_string()),
                        model: Some("private-coder".to_string()),
                        api_key: Some("custom-test-key".to_string()),
                        ..Default::default()
                    },
                )]),
                ..Default::default()
            }),
            ..Default::default()
        };
        let alpha = custom_config("alpha", "https://alpha.example/v1");
        checks.record_success(&alpha, ApiProvider::Custom, "private-coder");

        let beta = custom_config("beta", "https://alpha.example/v1");
        assert_eq!(
            resolve_for_model(&beta, ApiProvider::Custom, "private-coder", &checks),
            ResolvedProviderReadiness::SavedUnchecked,
            "named custom providers must not share observed health"
        );

        let alpha_moved = custom_config("alpha", "https://other.example/v1");
        assert_eq!(
            resolve_for_model(&alpha_moved, ApiProvider::Custom, "private-coder", &checks,),
            ResolvedProviderReadiness::SavedUnchecked,
            "changing endpoints must invalidate observed health"
        );
    }

    #[test]
    fn malformed_oauth_files_are_missing_login_not_ready() {
        let _lock = crate::test_support::lock_test_env();
        let temp = tempfile::tempdir().expect("oauth fixture root");
        let kimi_home = temp.path().join("kimi");
        std::fs::create_dir_all(kimi_home.join("credentials")).expect("kimi credentials dir");
        std::fs::write(kimi_home.join("credentials/kimi-code.json"), "{not-json")
            .expect("malformed kimi fixture");
        let _kimi_home = crate::test_support::EnvVarGuard::set(
            "KIMI_CODE_HOME",
            kimi_home.to_str().expect("utf8 path"),
        );
        let grok_path = temp.path().join("grok-auth.json");
        std::fs::write(&grok_path, "{}").expect("empty grok fixture");
        let _grok_path = crate::test_support::EnvVarGuard::set(
            "GROK_AUTH_PATH",
            grok_path.to_str().expect("utf8 path"),
        );

        let config = crate::config::Config {
            providers: Some(crate::config::ProvidersConfig {
                moonshot: crate::config::ProviderConfig {
                    auth_mode: Some("kimi_oauth".to_string()),
                    ..Default::default()
                },
                xai: crate::config::ProviderConfig {
                    auth_mode: Some("oauth".to_string()),
                    ..Default::default()
                },
                ..Default::default()
            }),
            ..Default::default()
        };

        assert_eq!(
            credential_state_for_provider(&config, ApiProvider::Moonshot),
            CredentialState::MissingLogin
        );
        assert_eq!(
            credential_state_for_provider(&config, ApiProvider::Xai),
            CredentialState::MissingLogin
        );

        let api_key_config = crate::config::Config {
            providers: Some(crate::config::ProvidersConfig {
                xai: crate::config::ProviderConfig {
                    api_key: Some("explicit-xai-key".to_string()),
                    ..Default::default()
                },
                ..Default::default()
            }),
            ..Default::default()
        };
        assert_eq!(
            credential_state_for_provider(&api_key_config, ApiProvider::Xai),
            CredentialState::Saved,
            "an unrelated stale Grok OAuth file must not shadow an explicit xAI API key"
        );
        assert_eq!(
            credential_state_for_provider(&crate::config::Config::default(), ApiProvider::Xai),
            CredentialState::MissingKey,
            "a Grok file is not active until xAI OAuth is selected in config"
        );
        let stale_root_config = crate::config::Config {
            provider: Some("xai".to_string()),
            api_key: Some("legacy-deepseek-root-key".to_string()),
            ..Default::default()
        };
        assert_eq!(
            credential_state_for_provider(&stale_root_config, ApiProvider::Xai),
            CredentialState::MissingKey,
            "the legacy root DeepSeek key is not an xAI credential"
        );

        let _cli_source = crate::test_support::EnvVarGuard::set("DEEPSEEK_API_KEY_SOURCE", "cli");
        let _cli_key =
            crate::test_support::EnvVarGuard::set("CODEWHALE_CLI_API_KEY", "explicit-cli-key");
        let cli_config = crate::config::Config {
            provider: Some("xai".to_string()),
            ..Default::default()
        };
        assert_eq!(
            credential_state_for_provider(&cli_config, ApiProvider::Xai),
            CredentialState::Saved,
            "the source-marked CLI override is valid for the active xAI provider"
        );
    }

    #[test]
    fn custom_provider_env_and_local_no_auth_states_match_runtime() {
        let _lock = crate::test_support::lock_test_env();
        let _custom_key = crate::test_support::EnvVarGuard::set("ACME_CUSTOM_KEY", "custom-secret");
        let remote = crate::config::Config {
            provider: Some("acme".to_string()),
            providers: Some(crate::config::ProvidersConfig {
                custom: std::collections::HashMap::from([(
                    "acme".to_string(),
                    crate::config::ProviderConfig {
                        kind: Some("openai-compatible".to_string()),
                        base_url: Some("https://api.acme.test/v1".to_string()),
                        model: Some("acme-coder".to_string()),
                        api_key_env: Some("ACME_CUSTOM_KEY".to_string()),
                        ..Default::default()
                    },
                )]),
                ..Default::default()
            }),
            ..Default::default()
        };
        assert_eq!(
            credential_state_for_provider(&remote, ApiProvider::Custom),
            CredentialState::Saved
        );

        let local = crate::config::Config {
            provider: Some("local-acme".to_string()),
            providers: Some(crate::config::ProvidersConfig {
                custom: std::collections::HashMap::from([(
                    "local-acme".to_string(),
                    crate::config::ProviderConfig {
                        kind: Some("openai-compatible".to_string()),
                        base_url: Some("http://127.0.0.1:8080/v1".to_string()),
                        model: Some("local-model".to_string()),
                        auth_mode: Some("none".to_string()),
                        ..Default::default()
                    },
                )]),
                ..Default::default()
            }),
            ..Default::default()
        };
        assert_eq!(
            credential_state_for_provider(&local, ApiProvider::Custom),
            CredentialState::Local
        );
    }
}
