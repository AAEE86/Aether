use crate::ai_serving::planner::candidate_resolution::{
    candidate_auth_channel_skip_reason, candidate_transport_policy_facts,
    provider_transport_uses_pool, EligibleLocalExecutionCandidate, LocalExecutionCandidateKind,
};
use crate::ai_serving::planner::common::endpoint_config_forces_body_stream_field;
use crate::ai_serving::planner::plan_builders::{AiStreamAttempt, AiSyncAttempt};
use crate::ai_serving::planner::spec_metadata::local_openai_responses_spec_metadata;
use crate::ai_serving::planner::standard::codex::codex_model_capabilities_for_transport;
use crate::ai_serving::planner::standard::normalize::build_local_openai_responses_request_body_with_codex_model_capabilities;
use crate::ai_serving::GatewayControlDecision;
use crate::orchestration::{
    codex_quota_breaker_blocks_candidate, log_codex_quota_breaker_check_failure,
    responses_websocket_capability, LocalExecutionCandidateMetadata, ResponsesProviderObserverKind,
    ResponsesWebSocketBackendKind,
};
use crate::{AiExecutionDecision, AppState, GatewayError};
use aether_data_contracts::repository::candidate_selection::{
    StoredMinimalCandidateSelectionRow, StoredPoolKeyCandidateRowsByKeyIdsQuery,
};
use aether_runtime_state::RuntimeLockLease;
use aether_scheduler_core::SchedulerMinimalCandidateSelectionCandidate;
use std::collections::BTreeSet;
use uuid::Uuid;

mod decision;
mod plans;

use self::decision::{
    build_local_openai_responses_candidate_attempt_source,
    maybe_build_local_openai_responses_decision_payload_for_candidate,
    resolve_local_openai_responses_decision_input,
    resolve_local_openai_responses_decision_input_strong,
    resolve_local_openai_responses_decision_input_with_auth_snapshot,
    LocalOpenAiResponsesCandidateAttempt, LocalOpenAiResponsesDecisionInput,
};
use self::plans::{
    build_local_stream_attempt_source, build_local_stream_plan_and_reports,
    build_local_sync_attempt_source, build_local_sync_plan_and_reports, resolve_stream_spec,
    resolve_sync_spec,
};

pub(crate) async fn build_local_openai_responses_sync_plan_and_reports_for_kind(
    state: &AppState,
    parts: &http::request::Parts,
    trace_id: &str,
    decision: &GatewayControlDecision,
    body_json: &serde_json::Value,
    plan_kind: &str,
) -> Result<Vec<AiSyncAttempt>, GatewayError> {
    let Some(spec) = resolve_sync_spec(plan_kind) else {
        return Ok(Vec::new());
    };

    build_local_sync_plan_and_reports(state, parts, trace_id, decision, body_json, spec).await
}

pub(crate) async fn build_local_openai_responses_stream_plan_and_reports_for_kind(
    state: &AppState,
    parts: &http::request::Parts,
    trace_id: &str,
    decision: &GatewayControlDecision,
    body_json: &serde_json::Value,
    plan_kind: &str,
) -> Result<Vec<AiStreamAttempt>, GatewayError> {
    let Some(spec) = resolve_stream_spec(plan_kind) else {
        return Ok(Vec::new());
    };

    build_local_stream_plan_and_reports(state, parts, trace_id, decision, body_json, spec).await
}

pub(crate) async fn build_local_openai_responses_sync_attempt_source_for_kind<'a>(
    state: &'a AppState,
    parts: &'a http::request::Parts,
    trace_id: &'a str,
    decision: &'a GatewayControlDecision,
    body_json: &'a serde_json::Value,
    plan_kind: &str,
) -> Result<
    Option<(
        impl crate::ai_serving::planner::LocalExecutionAttemptSource<AiSyncAttempt> + 'a,
        usize,
    )>,
    GatewayError,
> {
    let Some(spec) = resolve_sync_spec(plan_kind) else {
        return Ok(None);
    };

    build_local_sync_attempt_source(state, parts, trace_id, decision, body_json, spec).await
}

pub(crate) async fn build_local_openai_responses_stream_attempt_source_for_kind<'a>(
    state: &'a AppState,
    parts: &'a http::request::Parts,
    trace_id: &'a str,
    decision: &'a GatewayControlDecision,
    body_json: &'a serde_json::Value,
    plan_kind: &str,
) -> Result<
    Option<(
        impl crate::ai_serving::planner::LocalExecutionAttemptSource<AiStreamAttempt> + 'a,
        usize,
    )>,
    GatewayError,
> {
    let Some(spec) = resolve_stream_spec(plan_kind) else {
        return Ok(None);
    };

    build_local_stream_attempt_source(state, parts, trace_id, decision, body_json, spec).await
}

pub(crate) async fn maybe_build_sync_local_openai_responses_decision_payload(
    state: &AppState,
    parts: &http::request::Parts,
    trace_id: &str,
    decision: &GatewayControlDecision,
    body_json: &serde_json::Value,
    plan_kind: &str,
) -> Result<Option<AiExecutionDecision>, GatewayError> {
    let Some(spec) = resolve_sync_spec(plan_kind) else {
        return Ok(None);
    };

    let Some(input) = resolve_local_openai_responses_decision_input(
        state, parts, trace_id, decision, body_json, plan_kind,
    )
    .await?
    else {
        return Ok(None);
    };
    let body_json = input.effective_body_json(body_json);

    let (mut source, _) = build_local_openai_responses_candidate_attempt_source(
        state, trace_id, &input, body_json, spec,
    )
    .await?;

    while let Some(attempt) = source.next_attempt().await? {
        if let Some(payload) = maybe_build_local_openai_responses_decision_payload_for_candidate(
            state, parts, trace_id, body_json, &input, attempt, spec,
        )
        .await?
        {
            return Ok(Some(payload));
        }
    }

    Ok(None)
}

pub(crate) async fn maybe_build_stream_local_openai_responses_decision_payload(
    state: &AppState,
    parts: &http::request::Parts,
    trace_id: &str,
    decision: &GatewayControlDecision,
    body_json: &serde_json::Value,
    plan_kind: &str,
) -> Result<Option<AiExecutionDecision>, GatewayError> {
    let Some(spec) = resolve_stream_spec(plan_kind) else {
        return Ok(None);
    };

    let Some(input) = resolve_local_openai_responses_decision_input(
        state, parts, trace_id, decision, body_json, plan_kind,
    )
    .await?
    else {
        return Ok(None);
    };
    let body_json = input.effective_body_json(body_json);

    let (mut source, _) = build_local_openai_responses_candidate_attempt_source(
        state, trace_id, &input, body_json, spec,
    )
    .await?;

    while let Some(attempt) = source.next_attempt().await? {
        if let Some(payload) = maybe_build_local_openai_responses_decision_payload_for_candidate(
            state, parts, trace_id, body_json, &input, attempt, spec,
        )
        .await?
        {
            return Ok(Some(payload));
        }
    }

    Ok(None)
}

/// One eligible upstream plus the backend and provider observer selected for
/// the public Responses WebSocket session.
///
/// The public wire protocol is not part of this choice: it remains OpenAI
/// Responses WebSocket for every backend. The current planner only returns the
/// native fast path; a future HTTP-stream conversion backend can be added as a
/// second backend without changing the public session state machine.
pub(crate) struct ResponsesWebSocketDecision {
    pub(crate) execution: AiExecutionDecision,
    /// The concrete provider/endpoint/key selected after pool expansion.
    /// Continuation turns must revalidate this exact identity and may never
    /// migrate to a sibling key while retaining `previous_response_id`.
    pub(crate) bound_candidate: SchedulerMinimalCandidateSelectionCandidate,
    /// Non-secret credential-generation fence for the concrete transport.
    /// Unlike the final Authorization header, this remains stable across an
    /// ordinary OAuth refresh and Agent Identity assertion/task rotation.
    pub(crate) credential_binding_fingerprint: String,
    pub(crate) backend: ResponsesWebSocketBackendKind,
    pub(crate) provider_observer: ResponsesProviderObserverKind,
    pub(crate) normalization: ResponsesWebSocketBodyNormalization,
}

#[derive(Debug, Clone)]
pub(crate) enum BoundResponsesCandidateRevalidation {
    Prepared {
        decision: AiExecutionDecision,
        credential_binding_fingerprint: String,
    },
    Denied {
        reason: &'static str,
    },
    CapacityLimited {
        reason: &'static str,
    },
    Unavailable {
        reason: &'static str,
    },
}

impl BoundResponsesCandidateRevalidation {
    fn denied(reason: &'static str) -> Self {
        Self::Denied { reason }
    }

    fn capacity_limited(reason: &'static str) -> Self {
        Self::CapacityLimited { reason }
    }

    fn unavailable(reason: &'static str) -> Self {
        Self::Unavailable { reason }
    }
}

/// Revalidates the exact physical candidate retained by a Responses
/// continuation. This function never invokes candidate selection and can
/// therefore never migrate a `previous_response_id` chain to a sibling key.
#[allow(clippy::too_many_arguments)]
pub(crate) async fn revalidate_bound_responses_candidate(
    state: &AppState,
    parts: &http::request::Parts,
    trace_id: &str,
    decision: &GatewayControlDecision,
    auth_snapshot: &crate::data::auth::GatewayAuthApiKeySnapshot,
    client_event: &serde_json::Value,
    bound_candidate: &SchedulerMinimalCandidateSelectionCandidate,
    expected_provider_model: &str,
    expected_backend: ResponsesWebSocketBackendKind,
    expected_provider_observer: ResponsesProviderObserverKind,
) -> BoundResponsesCandidateRevalidation {
    let runtime_miss_key =
        crate::ai_serving::runtime_miss_diagnostic_key_from_parts(parts, trace_id).to_string();
    let _runtime_miss_cleanup =
        crate::ai_serving::RuntimeMissDiagnosticCleanupGuard::new(state, runtime_miss_key);

    let Some(spec) = resolve_stream_spec(crate::ai_serving::OPENAI_RESPONSES_STREAM_PLAN_KIND)
    else {
        return BoundResponsesCandidateRevalidation::unavailable(
            "responses_websocket_plan_unavailable",
        );
    };
    let input = match resolve_local_openai_responses_decision_input_strong(
        state,
        parts,
        trace_id,
        decision,
        client_event,
        spec.decision_kind,
        auth_snapshot,
    )
    .await
    {
        Ok(Some(input)) => input,
        Ok(None) => {
            return BoundResponsesCandidateRevalidation::denied(
                "responses_websocket_candidate_not_authorized",
            )
        }
        Err(GatewayError::Client { .. }) => {
            return BoundResponsesCandidateRevalidation::denied(
                "responses_websocket_routing_denied",
            )
        }
        Err(_) => {
            return BoundResponsesCandidateRevalidation::unavailable(
                "responses_websocket_revalidation_unavailable",
            )
        }
    };

    if input.routing_policy.as_ref().is_some_and(|policy| {
        !policy
            .ranking_overlay
            .provider_allowed(bound_candidate.provider_id.as_str())
            || !policy
                .ranking_overlay
                .key_allowed(bound_candidate.key_id.as_str())
    }) {
        return BoundResponsesCandidateRevalidation::denied(
            "responses_websocket_routing_candidate_revoked",
        );
    }

    let effective_body = input.effective_body_json(client_event);
    let request_operation =
        crate::ai_serving::openai_responses_request_operation("openai:responses", effective_body);
    let query = StoredPoolKeyCandidateRowsByKeyIdsQuery {
        api_format: "openai:responses".to_string(),
        provider_id: bound_candidate.provider_id.clone(),
        endpoint_id: bound_candidate.endpoint_id.clone(),
        model_id: bound_candidate.model_id.clone(),
        selected_provider_model_name: bound_candidate.selected_provider_model_name.clone(),
        key_ids: vec![bound_candidate.key_id.clone()],
    };
    let rows = match state
        .list_pool_key_candidate_rows_for_group_key_ids_strong(&query)
        .await
    {
        Ok(rows) => rows,
        Err(_) => {
            return BoundResponsesCandidateRevalidation::unavailable(
                "responses_websocket_catalog_unavailable",
            )
        }
    };
    let Some(candidate) =
        resolve_exact_bound_responses_candidate(rows, &input, bound_candidate, request_operation)
    else {
        return BoundResponsesCandidateRevalidation::denied(
            "responses_websocket_candidate_revoked",
        );
    };

    let transport = match state
        .read_provider_transport_snapshot_strong(
            candidate.provider_id.as_str(),
            candidate.endpoint_id.as_str(),
            candidate.key_id.as_str(),
        )
        .await
    {
        Ok(Some(transport)) => transport,
        Ok(None) => {
            return BoundResponsesCandidateRevalidation::denied(
                "responses_websocket_transport_revoked",
            )
        }
        Err(_) => {
            return BoundResponsesCandidateRevalidation::unavailable(
                "responses_websocket_transport_unavailable",
            )
        }
    };
    if crate::ai_serving::candidate_common_transport_skip_reason(
        &transport,
        candidate_transport_policy_facts(&candidate),
        Some(input.requested_model.as_str()),
    )
    .is_some()
        || candidate_auth_channel_skip_reason(&transport, input.request_auth_channel.as_deref())
            .is_some()
    {
        return BoundResponsesCandidateRevalidation::denied(
            "responses_websocket_transport_revoked",
        );
    }
    let Some(capability) = responses_websocket_capability(
        transport.provider.provider_type.as_str(),
        transport.provider.config.as_ref(),
    ) else {
        return BoundResponsesCandidateRevalidation::denied(
            "responses_websocket_capability_revoked",
        );
    };
    if capability.backend != expected_backend
        || capability.provider_observer != expected_provider_observer
        || !capability.supports_provider_type(transport.provider.provider_type.as_str())
        || crate::ai_serving::normalize_api_format_alias(transport.endpoint.api_format.as_str())
            != "openai:responses"
    {
        return BoundResponsesCandidateRevalidation::denied(
            "responses_websocket_binding_contract_changed",
        );
    }
    let credential_binding_fingerprint =
        crate::ai_serving::transport::provider_transport_credential_binding_fingerprint(&transport);

    let strong_runtime = state.strong_scheduler_runtime_state();
    match crate::scheduler::candidate::concrete_candidate_runtime_skip_reason(
        &strong_runtime,
        &candidate,
        Some(auth_snapshot),
        crate::clock::current_unix_secs(),
    )
    .await
    {
        Ok(Some(reason)) if runtime_skip_reason_is_authorization_failure(reason) => {
            return BoundResponsesCandidateRevalidation::denied(reason)
        }
        Ok(Some(reason)) => return BoundResponsesCandidateRevalidation::capacity_limited(reason),
        Ok(None) => {}
        Err(_) => {
            return BoundResponsesCandidateRevalidation::unavailable(
                "responses_websocket_runtime_unavailable",
            )
        }
    }
    if let Some(pool_config) =
        crate::handlers::shared::provider_pool::admin_provider_pool_config_from_config_value(
            transport.provider.config.as_ref(),
        )
    {
        let policy = crate::scheduler::candidate::ConcretePoolRuntimePolicy {
            cost_window_seconds: pool_config.cost_window_seconds,
            cost_limit_per_key_tokens: pool_config.cost_limit_per_key_tokens,
            probing_enabled: pool_config.probing_enabled,
        };
        match crate::scheduler::candidate::concrete_pool_candidate_runtime_skip_reason(
            &strong_runtime,
            &candidate,
            policy,
        )
        .await
        {
            Ok(Some(reason)) => {
                return BoundResponsesCandidateRevalidation::capacity_limited(reason)
            }
            Ok(None) => {}
            Err(_) => {
                return BoundResponsesCandidateRevalidation::unavailable(
                    "responses_websocket_pool_runtime_unavailable",
                )
            }
        }
    }

    let provider_api_format = transport.endpoint.api_format.trim().to_ascii_lowercase();
    let kind = if provider_transport_uses_pool(&transport) {
        LocalExecutionCandidateKind::PoolGroup
    } else {
        LocalExecutionCandidateKind::SingleKey
    };
    let attempt = LocalOpenAiResponsesCandidateAttempt {
        eligible: EligibleLocalExecutionCandidate {
            kind,
            candidate,
            transport: std::sync::Arc::new(transport),
            provider_api_format,
            orchestration: LocalExecutionCandidateMetadata {
                scheduler_affinity_epoch: Some(state.scheduler_affinity_epoch()),
                ..LocalExecutionCandidateMetadata::default()
            },
            ranking: None,
        },
        candidate_index: 0,
        retry_index: 0,
        candidate_id: new_bound_responses_revalidation_candidate_id(),
    };
    let effective_body = input.effective_body_json(client_event);
    let fresh_decision = match maybe_build_local_openai_responses_decision_payload_for_candidate(
        state,
        parts,
        trace_id,
        effective_body,
        &input,
        attempt,
        spec,
    )
    .await
    {
        Ok(Some(decision)) => decision,
        Ok(None) => {
            return BoundResponsesCandidateRevalidation::denied(
                "responses_websocket_provider_request_revoked",
            )
        }
        Err(GatewayError::Client { .. }) => {
            return BoundResponsesCandidateRevalidation::denied(
                "responses_websocket_provider_request_denied",
            )
        }
        Err(_) => {
            return BoundResponsesCandidateRevalidation::unavailable(
                "responses_websocket_provider_request_unavailable",
            )
        }
    };
    let fresh_provider_model = fresh_decision
        .provider_request_body
        .as_ref()
        .and_then(|body| body.get("model"))
        .and_then(serde_json::Value::as_str)
        .or(fresh_decision.mapped_model.as_deref())
        .map(str::trim)
        .filter(|model| !model.is_empty());
    if fresh_provider_model != Some(expected_provider_model.trim()) {
        return BoundResponsesCandidateRevalidation::denied(
            "responses_websocket_provider_model_changed",
        );
    }

    BoundResponsesCandidateRevalidation::Prepared {
        decision: fresh_decision,
        credential_binding_fingerprint,
    }
}

fn resolve_exact_bound_responses_candidate(
    rows: Vec<StoredMinimalCandidateSelectionRow>,
    input: &LocalOpenAiResponsesDecisionInput,
    bound: &SchedulerMinimalCandidateSelectionCandidate,
    request_operation: Option<&str>,
) -> Option<SchedulerMinimalCandidateSelectionCandidate> {
    let auth_constraints =
        crate::data::candidate_selection::auth_snapshot_constraints(&input.auth_snapshot);
    let directive_resolution = input
        .model_directive_policy
        .resolve_reasoning("openai:responses", Some(input.requested_model.as_str()));
    let routing_model = directive_resolution
        .base_model()
        .unwrap_or(input.requested_model.as_str());
    let resolved_global_model = aether_scheduler_core::resolve_requested_global_model_name_with_model_directives_and_request_operation(
        &rows,
        routing_model,
        "openai:responses",
        false,
        request_operation,
    )?;
    aether_scheduler_core::enumerate_minimal_candidate_selection_with_model_directives(
        aether_scheduler_core::EnumerateMinimalCandidateSelectionInput {
            rows,
            normalized_api_format: "openai:responses",
            request_operation,
            requested_model_name: routing_model,
            resolved_global_model_name: resolved_global_model.as_str(),
            require_streaming: true,
            required_capabilities: input.required_capabilities.as_ref(),
            auth_constraints: Some(&auth_constraints),
        },
        false,
    )
    .ok()?
    .into_iter()
    .find(|candidate| bound_responses_candidate_identity_matches(candidate, bound))
}

fn bound_responses_candidate_identity_matches(
    current: &SchedulerMinimalCandidateSelectionCandidate,
    bound: &SchedulerMinimalCandidateSelectionCandidate,
) -> bool {
    current.provider_id == bound.provider_id
        && current.provider_type == bound.provider_type
        && current.endpoint_id == bound.endpoint_id
        && crate::ai_serving::normalize_api_format_alias(&current.endpoint_api_format)
            == crate::ai_serving::normalize_api_format_alias(&bound.endpoint_api_format)
        && current.key_id == bound.key_id
        && current.key_auth_type == bound.key_auth_type
        && current.model_id == bound.model_id
        && current.global_model_id == bound.global_model_id
        && current.global_model_name == bound.global_model_name
        && current.selected_provider_model_name == bound.selected_provider_model_name
        && current.mapping_matched_model == bound.mapping_matched_model
}

fn runtime_skip_reason_is_authorization_failure(reason: &str) -> bool {
    matches!(reason, "oauth_invalid" | "pool_account_blocked")
}

fn new_bound_responses_revalidation_candidate_id() -> String {
    Uuid::new_v4().to_string()
}

/// Everything needed to re-run provider-body normalization for the candidate a
/// socket is already bound to.
///
/// A continuation turn (`previous_response_id` on the bound upstream) cannot
/// re-enter the planner, because planning selects a candidate and a different
/// key would break the response chain. Without this, such turns reached the
/// provider with only their `model` rewritten — skipping model directives,
/// endpoint body rules, and the Codex body contract that turn 1 received.
///
/// This value holds cloned scalars and JSON only: no candidate, no pool key
/// lease, no `AppState`. It cannot influence selection.
#[derive(Debug, Clone)]
pub(crate) struct ResponsesWebSocketBodyNormalization {
    provider_type: String,
    provider_api_format: String,
    client_api_format: String,
    mapped_model: String,
    requested_model: String,
    upstream_is_stream: bool,
    force_body_stream_field: bool,
    body_rules: Option<serde_json::Value>,
    request_headers: http::HeaderMap,
    codex_model_capabilities: Option<crate::ai_serving::CodexResponsesModelCapabilities>,
    model_directive_patch: Option<serde_json::Value>,
}

impl ResponsesWebSocketBodyNormalization {
    /// Builds a normalizer for a plain `openai:responses` upstream with no
    /// endpoint body rules, directives or Codex capabilities, so relay tests can
    /// construct a bound connection without standing up a provider snapshot.
    #[cfg(test)]
    pub(crate) fn for_tests(mapped_model: &str) -> Self {
        Self {
            provider_type: "openai".to_string(),
            provider_api_format: "openai:responses".to_string(),
            client_api_format: "openai:responses".to_string(),
            mapped_model: mapped_model.to_string(),
            requested_model: mapped_model.to_string(),
            upstream_is_stream: true,
            force_body_stream_field: false,
            body_rules: None,
            request_headers: http::HeaderMap::new(),
            codex_model_capabilities: None,
            model_directive_patch: None,
        }
    }

    #[cfg(test)]
    pub(crate) fn with_provider_type_for_tests(mut self, provider_type: &str) -> Self {
        self.provider_type = provider_type.to_string();
        self
    }

    #[cfg(test)]
    pub(crate) fn with_model_directive_patch_for_tests(mut self, patch: serde_json::Value) -> Self {
        self.model_directive_patch = Some(patch);
        self
    }

    #[cfg(test)]
    pub(crate) fn with_body_rules_for_tests(mut self, rules: serde_json::Value) -> Self {
        self.body_rules = Some(rules);
        self
    }

    /// Applies the same body transformations the planner applied on the turn
    /// that bound this upstream.
    ///
    /// Mirrors the same-format branch of
    /// `resolve_local_openai_responses_candidate_payload_parts`. This
    /// normalizer belongs specifically to the native Responses WebSocket
    /// backend. Cross-format providers will use a per-turn backend plan and the
    /// shared canonical request converter instead of retaining this value on a
    /// physical provider socket.
    ///
    /// Returns `None` when normalization fails. The native WebSocket backend
    /// treats that as a rejected turn because forwarding the unnormalized
    /// event would bypass the selected provider contract.
    pub(crate) fn normalize_response_create(
        &self,
        client_event: &serde_json::Value,
    ) -> Option<serde_json::Value> {
        use crate::ai_serving::planner::common::{
            enforce_provider_body_stream_policy, request_requires_body_stream_field,
        };

        let source_model = client_event
            .get("model")
            .and_then(serde_json::Value::as_str)
            .unwrap_or(self.requested_model.as_str());
        let require_body_stream_field =
            request_requires_body_stream_field(client_event, self.force_body_stream_field);
        let mut body = build_local_openai_responses_request_body_with_codex_model_capabilities(
            client_event,
            &self.mapped_model,
            self.upstream_is_stream,
            self.force_body_stream_field,
            self.provider_type.as_str(),
            self.provider_api_format.as_str(),
            self.body_rules.as_ref(),
            &self.request_headers,
            self.codex_model_capabilities.as_ref(),
            false,
        )?;
        if let Some(patch) = self.model_directive_patch.as_ref() {
            crate::ai_serving::apply_model_directive_mapping_patch(&mut body, patch);
            // The patch is a deep merge and may reintroduce `stream`.
            enforce_provider_body_stream_policy(
                &mut body,
                self.provider_api_format.as_str(),
                self.upstream_is_stream,
                require_body_stream_field,
            );
        }
        crate::ai_serving::finalize_openai_provider_request_with_codex_model_capabilities(
            &mut body,
            crate::ai_serving::OpenAiProviderRequestFinalization {
                source_api_format: self.client_api_format.as_str(),
                provider_api_format: self.provider_api_format.as_str(),
                provider_type: self.provider_type.as_str(),
                provider_model: self.mapped_model.as_str(),
                source_model,
                body_rules: self.body_rules.as_ref(),
                upstream_is_stream: self.upstream_is_stream,
                require_body_stream_field,
            },
            self.codex_model_capabilities.as_ref(),
        )
        .ok()?;
        Some(body)
    }
}

/// Builds one upstream decision for a Responses WebSocket turn. The session
/// reuses this decision for same-model turns and invokes the planner again when
/// a later `response.create` changes the public model.
pub(crate) async fn maybe_build_responses_websocket_decision(
    state: &AppState,
    parts: &http::request::Parts,
    trace_id: &str,
    decision: &GatewayControlDecision,
    body_json: &serde_json::Value,
    excluded_key_ids: Option<&BTreeSet<String>>,
    excluded_codex_account_ids: Option<&BTreeSet<String>>,
) -> Result<Option<ResponsesWebSocketDecision>, GatewayError> {
    maybe_build_responses_websocket_decision_inner(
        state,
        parts,
        trace_id,
        decision,
        body_json,
        excluded_key_ids,
        excluded_codex_account_ids,
        None,
    )
    .await
}

pub(crate) async fn maybe_build_responses_websocket_decision_with_auth_snapshot(
    state: &AppState,
    parts: &http::request::Parts,
    trace_id: &str,
    decision: &GatewayControlDecision,
    body_json: &serde_json::Value,
    excluded_key_ids: Option<&BTreeSet<String>>,
    excluded_codex_account_ids: Option<&BTreeSet<String>>,
    auth_snapshot: &crate::data::auth::GatewayAuthApiKeySnapshot,
) -> Result<Option<ResponsesWebSocketDecision>, GatewayError> {
    maybe_build_responses_websocket_decision_inner(
        state,
        parts,
        trace_id,
        decision,
        body_json,
        excluded_key_ids,
        excluded_codex_account_ids,
        Some(auth_snapshot),
    )
    .await
}

async fn maybe_build_responses_websocket_decision_inner(
    state: &AppState,
    parts: &http::request::Parts,
    trace_id: &str,
    decision: &GatewayControlDecision,
    body_json: &serde_json::Value,
    excluded_key_ids: Option<&BTreeSet<String>>,
    excluded_codex_account_ids: Option<&BTreeSet<String>>,
    auth_snapshot: Option<&crate::data::auth::GatewayAuthApiKeySnapshot>,
) -> Result<Option<ResponsesWebSocketDecision>, GatewayError> {
    let Some(spec) = resolve_stream_spec(crate::ai_serving::OPENAI_RESPONSES_STREAM_PLAN_KIND)
    else {
        return Ok(None);
    };
    let input = match auth_snapshot {
        Some(auth_snapshot) => {
            resolve_local_openai_responses_decision_input_with_auth_snapshot(
                state,
                parts,
                trace_id,
                decision,
                body_json,
                spec.decision_kind,
                auth_snapshot,
            )
            .await?
        }
        None => {
            resolve_local_openai_responses_decision_input(
                state,
                parts,
                trace_id,
                decision,
                body_json,
                spec.decision_kind,
            )
            .await?
        }
    };
    let Some(input) = input else {
        return Ok(None);
    };
    let body_json = input.effective_body_json(body_json);
    let (mut source, _) = build_local_openai_responses_candidate_attempt_source(
        state, trace_id, &input, body_json, spec,
    )
    .await?;

    while let Some(attempt) = source.next_attempt().await? {
        let pool_key_lease = attempt.eligible.orchestration.pool_key_lease.clone();
        if excluded_key_ids
            .is_some_and(|key_ids| key_ids.contains(attempt.eligible.candidate.key_id.as_str()))
        {
            release_responses_websocket_planning_lease(state, pool_key_lease.as_ref()).await;
            continue;
        }
        let Some(capability) = responses_websocket_capability(
            &attempt.eligible.transport.provider.provider_type,
            attempt.eligible.transport.provider.config.as_ref(),
        ) else {
            release_responses_websocket_planning_lease(state, pool_key_lease.as_ref()).await;
            continue;
        };
        // Captured before `attempt` is consumed so a later continuation turn can
        // reproduce this candidate's body normalization without re-planning.
        let bound_candidate = attempt.eligible.candidate.clone();
        let transport = std::sync::Arc::clone(&attempt.eligible.transport);
        let credential_binding_fingerprint =
            crate::ai_serving::transport::provider_transport_credential_binding_fingerprint(
                &transport,
            );
        let candidate_provider_api_format = attempt.eligible.provider_api_format.clone();
        let payload = match maybe_build_local_openai_responses_decision_payload_for_candidate(
            state, parts, trace_id, body_json, &input, attempt, spec,
        )
        .await
        {
            Ok(Some(payload)) => payload,
            Ok(None) => {
                release_responses_websocket_planning_lease(state, pool_key_lease.as_ref()).await;
                continue;
            }
            Err(error) => {
                release_responses_websocket_planning_lease(state, pool_key_lease.as_ref()).await;
                return Err(error);
            }
        };
        if payload
            .provider_type
            .as_deref()
            .is_some_and(|value| value.trim().eq_ignore_ascii_case("codex"))
            && crate::orchestration::codex_account_id_from_headers(
                &payload.provider_request_headers,
            )
            .is_some_and(|account_id| {
                excluded_codex_account_ids
                    .is_some_and(|account_ids| account_ids.contains(account_id))
            })
        {
            release_responses_websocket_planning_lease(state, pool_key_lease.as_ref()).await;
            continue;
        }
        match codex_quota_breaker_blocks_candidate(
            state,
            payload.provider_type.as_deref(),
            payload.key_id.as_deref(),
            &payload.provider_request_headers,
        )
        .await
        {
            Ok(true) => {
                release_responses_websocket_planning_lease(state, pool_key_lease.as_ref()).await;
                continue;
            }
            Ok(false) => {}
            Err(error) => log_codex_quota_breaker_check_failure(&error),
        }
        if payload
            .provider_type
            .as_deref()
            .is_some_and(|value| capability.supports_provider_type(value))
            && payload.provider_api_format.as_deref().is_some_and(|value| {
                crate::ai_serving::normalize_api_format_alias(value) == "openai:responses"
            })
        {
            let mapped_model = payload.mapped_model.clone().unwrap_or_default();
            let source_model = body_json
                .get("model")
                .and_then(serde_json::Value::as_str)
                .unwrap_or(input.requested_model.as_str());
            let normalization = ResponsesWebSocketBodyNormalization {
                provider_type: transport.provider.provider_type.clone(),
                provider_api_format: candidate_provider_api_format.clone(),
                client_api_format: local_openai_responses_spec_metadata(spec)
                    .api_format
                    .to_string(),
                requested_model: input.requested_model.clone(),
                upstream_is_stream: payload.upstream_is_stream,
                force_body_stream_field: endpoint_config_forces_body_stream_field(
                    transport.endpoint.config.as_ref(),
                ),
                body_rules: transport.endpoint.body_rules.clone(),
                request_headers: input.effective_headers(&parts.headers).clone(),
                codex_model_capabilities: codex_model_capabilities_for_transport(
                    &transport,
                    candidate_provider_api_format.as_str(),
                    mapped_model.as_str(),
                    source_model,
                ),
                model_directive_patch: input
                    .model_directive_policy
                    .resolve_reasoning(
                        candidate_provider_api_format.as_str(),
                        Some(&input.requested_model),
                    )
                    .mapping_patch_for_mapped_model(mapped_model.as_str())
                    .ok()
                    .flatten(),
                mapped_model,
            };
            return Ok(Some(ResponsesWebSocketDecision {
                execution: payload,
                bound_candidate,
                credential_binding_fingerprint,
                backend: capability.backend,
                provider_observer: capability.provider_observer,
                normalization,
            }));
        }
        release_responses_websocket_planning_lease(state, pool_key_lease.as_ref()).await;
    }

    Ok(None)
}

async fn release_responses_websocket_planning_lease(
    state: &AppState,
    lease: Option<&RuntimeLockLease>,
) {
    let Some(lease) = lease else {
        return;
    };
    if let Err(error) =
        crate::handlers::shared::provider_pool::release_admin_provider_pool_key_lease(
            state.runtime_state.as_ref(),
            lease,
        )
        .await
    {
        tracing::warn!(
            error = ?error,
            "gateway Responses WebSocket planner failed to release an unused pool key lease"
        );
    }
}

#[cfg(test)]
mod tests {
    use super::new_bound_responses_revalidation_candidate_id;

    #[test]
    fn bound_revalidation_candidate_ids_are_unique_uuids() {
        let first = new_bound_responses_revalidation_candidate_id();
        let second = new_bound_responses_revalidation_candidate_id();

        assert_ne!(first, second);
        assert!(uuid::Uuid::parse_str(&first).is_ok());
        assert!(uuid::Uuid::parse_str(&second).is_ok());
    }
}
