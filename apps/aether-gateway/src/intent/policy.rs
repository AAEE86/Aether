use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};

use aether_contracts::ExecutionPlan;
use axum::body::Bytes;

use crate::gateway::headers::header_value_str;
use crate::gateway::provider_transport::is_codex_cli_backend_url;
use crate::gateway::{
    AppState, GatewayControlDecision, GatewayControlSyncDecisionResponse,
    DIRECT_PLAN_BYPASS_MAX_ENTRIES, DIRECT_PLAN_BYPASS_TTL,
};

pub(crate) fn should_bypass_execution_runtime_decision(
    payload: &GatewayControlSyncDecisionResponse,
) -> bool {
    let provider_api_format = payload
        .provider_api_format
        .as_deref()
        .map(str::trim)
        .unwrap_or_default()
        .to_ascii_lowercase();
    if !matches!(
        provider_api_format.as_str(),
        "openai:cli" | "openai:compact"
    ) {
        return false;
    }

    payload
        .upstream_url
        .as_deref()
        .or(payload.upstream_base_url.as_deref())
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .is_some_and(is_codex_cli_backend_url)
}

pub(crate) fn should_bypass_execution_runtime_plan(plan: &ExecutionPlan) -> bool {
    let provider_api_format = plan.provider_api_format.trim().to_ascii_lowercase();
    if !matches!(
        provider_api_format.as_str(),
        "openai:cli" | "openai:compact"
    ) {
        return false;
    }

    is_codex_cli_backend_url(&plan.url)
}

pub(crate) fn build_direct_plan_bypass_cache_key(
    plan_kind: &str,
    parts: &http::request::Parts,
    body_bytes: &Bytes,
    decision: &GatewayControlDecision,
) -> String {
    let mut hasher = DefaultHasher::new();
    plan_kind.hash(&mut hasher);
    parts.method.as_str().hash(&mut hasher);
    parts.uri.path().hash(&mut hasher);
    parts.uri.query().unwrap_or_default().hash(&mut hasher);
    decision
        .route_family
        .as_deref()
        .unwrap_or_default()
        .hash(&mut hasher);
    decision
        .route_kind
        .as_deref()
        .unwrap_or_default()
        .hash(&mut hasher);
    decision
        .auth_endpoint_signature
        .as_deref()
        .unwrap_or_default()
        .hash(&mut hasher);
    header_value_str(&parts.headers, http::header::AUTHORIZATION.as_str())
        .unwrap_or_default()
        .hash(&mut hasher);
    header_value_str(&parts.headers, "x-api-key")
        .unwrap_or_default()
        .hash(&mut hasher);
    header_value_str(&parts.headers, "api-key")
        .unwrap_or_default()
        .hash(&mut hasher);
    header_value_str(&parts.headers, http::header::CONTENT_TYPE.as_str())
        .unwrap_or_default()
        .hash(&mut hasher);
    body_bytes.hash(&mut hasher);
    format!("{plan_kind}:{:x}", hasher.finish())
}

pub(crate) fn should_skip_direct_plan(state: &AppState, cache_key: &str) -> bool {
    state
        .direct_plan_bypass_cache
        .should_skip(cache_key, DIRECT_PLAN_BYPASS_TTL)
}

pub(crate) fn mark_direct_plan_bypass(state: &AppState, cache_key: String) {
    state.direct_plan_bypass_cache.mark(
        cache_key,
        DIRECT_PLAN_BYPASS_TTL,
        DIRECT_PLAN_BYPASS_MAX_ENTRIES,
    );
}
