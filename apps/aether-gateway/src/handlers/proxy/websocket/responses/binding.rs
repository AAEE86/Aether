//! Identity of the physical upstream connection backing a Responses session.
//!
//! A Responses continuation carries state that lives on one provider socket.
//! Comparing only the selected key is therefore not sufficient: transport
//! settings and native backend can all change
//! the connection that would receive the next event. Rotating bearer values
//! are intentionally excluded because they do not change an already-upgraded
//! socket's physical binding.

use std::collections::{BTreeMap, BTreeSet};
use std::fmt;

use aether_contracts::{ProxySnapshot, ResolvedTransportProfile};

use super::backend::NativeResponsesWebSocketBackend;
use crate::ai_serving::AiExecutionDecision;
use crate::handlers::proxy::websocket::transport::{
    websocket_handshake_headers, websocket_upstream_url,
};
use crate::orchestration::ResponsesWebSocketBackendKind;

/// Stable, comparable identity for the actual WebSocket connection target.
///
/// The identity deliberately owns the normalized handshake values rather than
/// retaining a reference to the planner decision.  A later re-plan can then
/// be compared without accidentally ignoring a field that changes the
/// physical connection.
#[derive(Clone, PartialEq)]
pub(super) struct UpstreamBindingIdentity {
    backend_kind: ResponsesWebSocketBackendKind,
    provider_id: Option<String>,
    endpoint_id: Option<String>,
    key_id: Option<String>,
    upstream_url: String,
    handshake_headers: BTreeMap<String, String>,
    /// Non-secret provider-transport credential generation. This deliberately
    /// differs from the final Authorization value: OAuth access tokens and
    /// Agent Identity assertions may rotate without replacing the account.
    credential_binding_fingerprint: String,
    proxy: Option<ProxySnapshot>,
    transport_profile: Option<ResolvedTransportProfile>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum UpstreamBindingIdentityError {
    MissingUpstreamUrl,
    InvalidUpstreamUrl,
    InvalidHandshakeHeaders,
}

impl UpstreamBindingIdentity {
    /// Builds an identity from the same normalized URL and headers used by
    /// the WebSocket transport client.
    pub(super) fn from_decision(
        backend: &'static dyn NativeResponsesWebSocketBackend,
        decision: &AiExecutionDecision,
        credential_binding_fingerprint: &str,
    ) -> Result<Self, UpstreamBindingIdentityError> {
        let raw_url = decision
            .upstream_url
            .as_deref()
            .filter(|value| !value.trim().is_empty())
            .ok_or(UpstreamBindingIdentityError::MissingUpstreamUrl)?;
        let upstream_url = websocket_upstream_url(raw_url, "invalid")
            .map_err(|_| UpstreamBindingIdentityError::InvalidUpstreamUrl)?
            .to_string();

        let headers = websocket_handshake_headers(&decision.provider_request_headers, "invalid")
            .map_err(|_| UpstreamBindingIdentityError::InvalidHandshakeHeaders)?;
        let authentication_header_names = authentication_header_names(decision);
        let mut handshake_headers = BTreeMap::new();
        for (name, value) in &headers {
            let name = name.as_str().to_ascii_lowercase();
            let value = value
                .to_str()
                .map_err(|_| UpstreamBindingIdentityError::InvalidHandshakeHeaders)?;
            if authentication_header_names.contains(name.as_str()) {
                continue;
            } else {
                handshake_headers.insert(name, value.to_string());
            }
        }

        Ok(Self {
            backend_kind: backend.kind(),
            provider_id: decision.provider_id.clone(),
            endpoint_id: decision.endpoint_id.clone(),
            key_id: decision.key_id.clone(),
            upstream_url,
            handshake_headers,
            credential_binding_fingerprint: credential_binding_fingerprint.to_string(),
            proxy: effective_proxy_snapshot(decision.proxy.as_ref()),
            transport_profile: decision.transport_profile.clone(),
        })
    }

    /// Compares a fresh turn decision with the already-upgraded socket.
    ///
    /// Codex derives `session-id` and `thread-id` from optional request-body
    /// cache metadata. Responses continuations are allowed to omit that
    /// metadata, but an upgraded socket necessarily retains the values from
    /// its original handshake. Treat an omitted derived header as inheritance;
    /// an explicitly different value remains a binding change.
    pub(super) fn matches_turn_decision(
        &self,
        backend: &'static dyn NativeResponsesWebSocketBackend,
        decision: &AiExecutionDecision,
        credential_binding_fingerprint: &str,
    ) -> Result<bool, UpstreamBindingIdentityError> {
        let mut fresh = Self::from_decision(backend, decision, credential_binding_fingerprint)?;
        if decision
            .provider_type
            .as_deref()
            .is_some_and(|provider_type| provider_type.trim().eq_ignore_ascii_case("codex"))
        {
            for header_name in ["session-id", "thread-id"] {
                if !fresh.handshake_headers.contains_key(header_name) {
                    if let Some(value) = self.handshake_headers.get(header_name) {
                        fresh
                            .handshake_headers
                            .insert(header_name.to_string(), value.clone());
                    }
                }
            }
        }
        Ok(self == &fresh)
    }
}

/// Header names that carry credentials in the provider handshake.  The
/// planner's explicit `auth_header` extends this list for provider-specific
/// schemes; unknown headers remain part of the stable handshake identity.
fn authentication_header_names(decision: &AiExecutionDecision) -> BTreeSet<String> {
    let mut names = BTreeSet::from([
        "authorization".to_string(),
        "proxy-authorization".to_string(),
        "x-api-key".to_string(),
        "api-key".to_string(),
        "x-goog-api-key".to_string(),
        "x-azure-api-key".to_string(),
    ]);
    if let Some(name) = decision
        .auth_header
        .as_deref()
        .map(str::trim)
        .filter(|name| !name.is_empty())
    {
        names.insert(name.to_ascii_lowercase());
    }
    names
}

/// Normalize only values that are provably direct transport.  Keep node/tunnel
/// fields even though the current WebSocket builder rejects those proxies: a
/// re-plan must not accidentally reuse an already-bound direct socket for a
/// decision that selected a different proxy topology.
fn effective_proxy_snapshot(proxy: Option<&ProxySnapshot>) -> Option<ProxySnapshot> {
    let proxy = proxy?;
    if proxy.enabled == Some(false) {
        return None;
    }
    let mut normalized = proxy.clone();
    normalized.url = normalized
        .url
        .take()
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty());
    normalized.mode = normalized
        .mode
        .take()
        .map(|value| value.trim().to_ascii_lowercase())
        .filter(|value| !value.is_empty());
    normalized.node_id = normalized
        .node_id
        .take()
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty());
    normalized.label = normalized
        .label
        .take()
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty());

    let has_effective_proxy = normalized.url.is_some()
        || normalized.node_id.is_some()
        || normalized.mode.is_some()
        || normalized.extra.is_some();
    has_effective_proxy.then_some(normalized)
}

impl fmt::Debug for UpstreamBindingIdentity {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("UpstreamBindingIdentity")
            .field("backend_kind", &self.backend_kind)
            .field("provider_id", &self.provider_id)
            .field("endpoint_id", &self.endpoint_id)
            .field("key_id", &self.key_id)
            .field("upstream_url", &self.upstream_url)
            .field(
                "handshake_header_names",
                &self.handshake_headers.keys().collect::<Vec<_>>(),
            )
            .field("proxy_configured", &self.proxy.is_some())
            .field(
                "transport_profile_id",
                &self
                    .transport_profile
                    .as_ref()
                    .map(|profile| profile.profile_id.as_str()),
            )
            .finish()
    }
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use serde_json::json;

    use super::{UpstreamBindingIdentity, UpstreamBindingIdentityError};
    use crate::ai_serving::AiExecutionDecision;
    use crate::handlers::proxy::websocket::responses::backend::resolve_native_responses_websocket_backend;
    use crate::orchestration::ResponsesWebSocketBackendKind;

    fn backend() -> &'static dyn crate::handlers::proxy::websocket::responses::backend::NativeResponsesWebSocketBackend{
        resolve_native_responses_websocket_backend(
            ResponsesWebSocketBackendKind::NativeResponsesWebSocket,
        )
    }

    fn decision() -> AiExecutionDecision {
        AiExecutionDecision {
            action: "execute".to_string(),
            decision_kind: None,
            execution_strategy: None,
            conversion_mode: None,
            request_id: Some("request-1".to_string()),
            candidate_id: Some("candidate-1".to_string()),
            provider_name: Some("provider".to_string()),
            provider_type: Some("openai".to_string()),
            provider_id: Some("provider-1".to_string()),
            endpoint_id: Some("endpoint-1".to_string()),
            key_id: Some("key-1".to_string()),
            upstream_base_url: Some("https://api.example.test".to_string()),
            upstream_url: Some("https://api.example.test/v1/responses".to_string()),
            provider_request_method: Some("POST".to_string()),
            auth_header: Some("authorization".to_string()),
            auth_value: Some("Bearer secret".to_string()),
            provider_api_format: Some("openai:responses".to_string()),
            client_api_format: Some("openai:responses".to_string()),
            provider_contract: None,
            client_contract: None,
            model_name: Some("gpt-5.6-sol".to_string()),
            mapped_model: None,
            prompt_cache_key: None,
            extra_headers: BTreeMap::new(),
            provider_request_headers: BTreeMap::from([
                ("Authorization".to_string(), "Bearer secret".to_string()),
                ("X-Client".to_string(), "aether".to_string()),
                ("Connection".to_string(), "keep-alive".to_string()),
            ]),
            provider_request_body: Some(json!({"model": "gpt-5.6-sol"})),
            provider_request_body_base64: None,
            content_type: Some("application/json".to_string()),
            content_encoding: None,
            request_gzip: None,
            proxy: None,
            transport_profile: None,
            timeouts: None,
            upstream_is_stream: true,
            report_kind: None,
            report_context: None,
            auth_context: None,
        }
    }

    #[test]
    fn identity_normalizes_url_and_hop_by_hop_headers() {
        let identity =
            UpstreamBindingIdentity::from_decision(backend(), &decision(), "credential-1").unwrap();

        assert_eq!(identity.upstream_url, "wss://api.example.test/v1/responses");
        assert_eq!(
            identity.handshake_headers,
            BTreeMap::from([("x-client".to_string(), "aether".to_string())])
        );
    }

    #[test]
    fn identity_changes_when_physical_binding_changes() {
        let base = decision();
        let identity =
            UpstreamBindingIdentity::from_decision(backend(), &base, "credential-1").unwrap();

        for mutate in [
            |decision: &mut AiExecutionDecision| {
                decision.key_id = Some("key-2".to_string());
            },
            |decision: &mut AiExecutionDecision| {
                decision.upstream_url = Some("https://other.example.test/v1/responses".to_string());
            },
            |decision: &mut AiExecutionDecision| {
                decision
                    .provider_request_headers
                    .insert("X-Client".to_string(), "other".to_string());
            },
            |decision: &mut AiExecutionDecision| {
                decision.proxy = Some(aether_contracts::ProxySnapshot {
                    enabled: Some(true),
                    url: Some("http://proxy.example.test:8080".to_string()),
                    ..Default::default()
                });
            },
            |decision: &mut AiExecutionDecision| {
                decision.transport_profile = Some(aether_contracts::ResolvedTransportProfile {
                    profile_id: "chrome136".to_string(),
                    ..Default::default()
                });
            },
        ] {
            let mut changed = base.clone();
            mutate(&mut changed);
            let changed_identity =
                UpstreamBindingIdentity::from_decision(backend(), &changed, "credential-1")
                    .unwrap();
            assert_ne!(identity, changed_identity);
        }

        let mut rotated = base.clone();
        rotated
            .provider_request_headers
            .insert("Authorization".to_string(), "Bearer rotated".to_string());
        assert_eq!(
            identity,
            UpstreamBindingIdentity::from_decision(backend(), &rotated, "credential-1").unwrap()
        );
    }

    #[test]
    fn beta_only_header_differences_do_not_change_the_stable_binding() {
        let bound = super::super::upstream::canonicalize_responses_websocket_decision(decision());
        let identity =
            UpstreamBindingIdentity::from_decision(backend(), &bound, "credential-1").unwrap();
        let mut fresh = decision();
        fresh.provider_request_headers.insert(
            "OpEnAI-BeTa".to_string(),
            "responses_multi_agent=v1".to_string(),
        );
        let fresh = super::super::upstream::canonicalize_responses_websocket_decision(fresh);

        assert!(identity
            .matches_turn_decision(backend(), &fresh, "credential-1")
            .unwrap());
    }

    #[test]
    fn codex_turn_inherits_omitted_session_headers_but_rejects_explicit_changes() {
        let mut bound = decision();
        bound.provider_type = Some("codex".to_string());
        bound
            .provider_request_headers
            .insert("session-id".to_string(), "session-1".to_string());
        bound
            .provider_request_headers
            .insert("thread-id".to_string(), "thread-1".to_string());
        let identity =
            UpstreamBindingIdentity::from_decision(backend(), &bound, "credential-1").unwrap();

        let mut omitted = bound.clone();
        omitted.provider_request_headers.remove("session-id");
        omitted.provider_request_headers.remove("thread-id");
        assert!(identity
            .matches_turn_decision(backend(), &omitted, "credential-1")
            .unwrap());

        let mut changed = omitted;
        changed
            .provider_request_headers
            .insert("session-id".to_string(), "session-2".to_string());
        assert!(!identity
            .matches_turn_decision(backend(), &changed, "credential-1")
            .unwrap());
    }

    #[test]
    fn non_codex_turn_cannot_omit_bound_session_headers() {
        let mut bound = decision();
        bound
            .provider_request_headers
            .insert("session-id".to_string(), "session-1".to_string());
        let identity =
            UpstreamBindingIdentity::from_decision(backend(), &bound, "credential-1").unwrap();

        let mut omitted = bound;
        omitted.provider_request_headers.remove("session-id");
        assert!(!identity
            .matches_turn_decision(backend(), &omitted, "credential-1")
            .unwrap());
    }

    #[test]
    fn stable_key_identity_rejects_custom_auth_value_rotation() {
        let mut base = decision();
        base.auth_header = Some("X-Provider-Token".to_string());
        base.provider_request_headers.remove("Authorization");
        base.provider_request_headers.insert(
            "X-Provider-Token".to_string(),
            "provider-token-1".to_string(),
        );
        let identity =
            UpstreamBindingIdentity::from_decision(backend(), &base, "credential-1").unwrap();
        assert!(!identity.handshake_headers.contains_key("x-provider-token"));

        let mut rotated = base;
        rotated.provider_request_headers.insert(
            "X-Provider-Token".to_string(),
            "provider-token-2".to_string(),
        );
        assert_eq!(
            identity,
            UpstreamBindingIdentity::from_decision(backend(), &rotated, "credential-1").unwrap()
        );
        assert_ne!(
            identity,
            UpstreamBindingIdentity::from_decision(backend(), &rotated, "credential-2").unwrap()
        );
    }

    #[test]
    fn missing_key_identity_still_uses_explicit_credential_generation() {
        let mut first = decision();
        first.key_id = None;
        let first_identity =
            UpstreamBindingIdentity::from_decision(backend(), &first, "credential-1").unwrap();

        let mut same_account_rotation = first.clone();
        same_account_rotation.provider_request_headers.insert(
            "Authorization".to_string(),
            "Bearer different-account-or-token".to_string(),
        );
        let changed_identity = UpstreamBindingIdentity::from_decision(
            backend(),
            &same_account_rotation,
            "credential-1",
        )
        .unwrap();
        assert_eq!(first_identity, changed_identity);
        assert_ne!(
            first_identity,
            UpstreamBindingIdentity::from_decision(
                backend(),
                &same_account_rotation,
                "credential-2"
            )
            .unwrap()
        );

        let mut non_auth_change = first;
        non_auth_change
            .provider_request_headers
            .insert("X-Client".to_string(), "other-client".to_string());
        assert_ne!(
            first_identity,
            UpstreamBindingIdentity::from_decision(backend(), &non_auth_change, "credential-1")
                .unwrap()
        );
    }

    #[test]
    fn disabled_proxy_is_equivalent_to_direct_transport() {
        let direct = decision();
        let direct_identity =
            UpstreamBindingIdentity::from_decision(backend(), &direct, "credential-1").unwrap();
        let mut explicitly_disabled = direct;
        explicitly_disabled.proxy = Some(aether_contracts::ProxySnapshot {
            enabled: Some(false),
            url: Some("http://ignored.example.test:8080".to_string()),
            ..Default::default()
        });

        assert_eq!(
            direct_identity,
            UpstreamBindingIdentity::from_decision(backend(), &explicitly_disabled, "credential-1")
                .unwrap()
        );
    }

    #[test]
    fn identity_rejects_missing_or_invalid_connection_fields() {
        let mut missing = decision();
        missing.upstream_url = None;
        assert_eq!(
            UpstreamBindingIdentity::from_decision(backend(), &missing, "credential-1"),
            Err(UpstreamBindingIdentityError::MissingUpstreamUrl)
        );

        let mut invalid = decision();
        invalid.upstream_url = Some("file:///tmp/responses".to_string());
        assert_eq!(
            UpstreamBindingIdentity::from_decision(backend(), &invalid, "credential-1"),
            Err(UpstreamBindingIdentityError::InvalidUpstreamUrl)
        );
    }
}
