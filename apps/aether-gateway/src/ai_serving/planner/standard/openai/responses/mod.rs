use crate::ai_serving::planner::plan_builders::{AiStreamAttempt, AiSyncAttempt};
use crate::ai_serving::GatewayControlDecision;
use crate::{AiExecutionDecision, AppState, GatewayError};

mod decision;
mod plans;

use self::decision::{
    build_local_openai_responses_candidate_attempt_source,
    maybe_build_local_openai_responses_decision_payload_for_candidate,
    resolve_local_openai_responses_decision_input,
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

/// Builds one upstream decision for a Codex Responses WebSocket turn. The
/// bridge reuses this decision for same-model turns and invokes the planner
/// again when a later `response.create` changes the public model.
pub(crate) async fn maybe_build_codex_responses_websocket_decision(
    state: &AppState,
    parts: &http::request::Parts,
    trace_id: &str,
    decision: &GatewayControlDecision,
    body_json: &serde_json::Value,
) -> Result<Option<AiExecutionDecision>, GatewayError> {
    let Some(spec) = resolve_stream_spec(aether_ai_formats::api::OPENAI_RESPONSES_STREAM_PLAN_KIND)
    else {
        return Ok(None);
    };
    let Some(input) = resolve_local_openai_responses_decision_input(
        state,
        parts,
        trace_id,
        decision,
        body_json,
        spec.decision_kind,
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
        if !attempt
            .eligible
            .transport
            .provider
            .provider_type
            .trim()
            .eq_ignore_ascii_case("codex")
        {
            continue;
        }
        if !crate::orchestration::codex_responses_websocket_enabled(
            &attempt.eligible.transport.provider.provider_type,
            attempt.eligible.transport.provider.config.as_ref(),
        ) {
            continue;
        }
        let Some(payload) = maybe_build_local_openai_responses_decision_payload_for_candidate(
            state, parts, trace_id, body_json, &input, attempt, spec,
        )
        .await?
        else {
            continue;
        };
        if payload
            .provider_type
            .as_deref()
            .is_some_and(|value| value.trim().eq_ignore_ascii_case("codex"))
            && payload.provider_api_format.as_deref().is_some_and(|value| {
                crate::ai_serving::normalize_api_format_alias(value) == "openai:responses"
            })
        {
            return Ok(Some(payload));
        }
    }

    Ok(None)
}
