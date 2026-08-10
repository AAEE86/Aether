//! Quota exhaustion, replay safety, and upstream replacement policy.

use axum::extract::ws::WebSocket;
use futures_util::SinkExt;
use serde_json::Value;

use super::adapter::{
    resolve_responses_provider_observer, ResponsesProviderObserver,
    ResponsesWebSocketDrainDirective, ResponsesWebSocketRebindSafety,
};
use super::backend::resolve_native_responses_websocket_backend;
use super::frame::ParsedResponsesWebSocketFrame;
use super::lifecycle::{queue_turn_finalization, PreviousAttemptSettled};
use super::ownership::{
    await_owned_responses_websocket_plan, await_owned_responses_websocket_turn, disarm_owned_turn,
    spawn_owned_responses_websocket_plan, spawn_owned_responses_websocket_turn,
};
use super::request::{
    build_planning_parts, planned_response_create_event, response_create_has_previous_response_id,
};
use super::state::BoundResponsesConnection;
use super::turn::{prepare_responses_websocket_turn_decision, ResponsesWebSocketTurnOutcome};
use super::upstream::{
    bind_responses_upstream, canonicalize_responses_websocket_decision, close_bound_upstream,
    ResponsesUpstreamBindError,
};
use crate::clock::current_unix_secs;
use crate::handlers::proxy::websocket::ingress::WebSocketRequestContext;
use crate::handlers::proxy::websocket::session::WEBSOCKET_LOG_TRANSPORT;
use crate::handlers::proxy::websocket::transport::send_responses_websocket_error;
use crate::AppState;

const PREVIOUS_RESPONSE_NOT_FOUND_MESSAGE: &str =
    "Previous response was not found. Retrying the full request.";
const LOG_TARGET: &str = "aether_gateway::handlers::proxy::responses_ws";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum QuotaRetryDisposition {
    Retried,
    Failed,
    ExecutionReservationLost,
}

macro_rules! debug {
    ($($arg:tt)*) => {
        tracing::debug!(target: LOG_TARGET, $($arg)*)
    };
}

macro_rules! warn {
    ($($arg:tt)*) => {
        tracing::warn!(target: LOG_TARGET, $($arg)*)
    };
}

pub(super) async fn detach_exhausted_upstream(
    bound: &mut BoundResponsesConnection,
    directive: ResponsesWebSocketDrainDirective,
    trace_id: &str,
) {
    let exclusion = record_exhausted_bound_key(bound, directive.retry_exclusion_until_unix_secs);
    close_bound_upstream(bound).await;
    // 调用方必须先结束当前 logical turn 再 detach：拆掉上游后 attempt 已经不可能
    // 收到终态，留着它只会等 deadline 或 drop guard 兜底。
    debug_assert!(
        !bound.turn_state.response_in_flight(),
        "an exhausted upstream must be detached after its logical turn ended"
    );
    bound.pending_provider_drain = None;
    let now_unix_secs = current_unix_secs();
    debug!(
        event_name = "responses_websocket_upstream_detached",
        log_type = "event",
        transport = WEBSOCKET_LOG_TRANSPORT,
        websocket = true,
        trace_id = %trace_id,
        reason = directive.error_code,
        exhausted_key_id = ?exclusion.as_ref().map(|(key_id, _)| key_id),
        retry_exclusion_until_unix_secs = ?exclusion.as_ref().map(|(_, until)| until),
        exhausted_exclusion_count = bound.exhausted_exclusions.len(now_unix_secs),
        "gateway detached an exhausted Responses WebSocket upstream while preserving the client socket"
    );
}

pub(super) fn record_exhausted_bound_key(
    bound: &mut BoundResponsesConnection,
    reset_at_unix_secs: Option<u64>,
) -> Option<(String, u64)> {
    let key_id = bound
        .decision_template
        .key_id
        .as_deref()
        .map(str::trim)
        .filter(|key_id| !key_id.is_empty())?
        .to_string();
    let provider_account_id = bound
        .provider_observer
        .exhaustion_exclusion_identity(&bound.decision_template)
        .and_then(|identity| identity.account_id);
    let exclusion_until = bound.exhausted_exclusions.exclude(
        key_id.clone(),
        provider_account_id,
        reset_at_unix_secs,
        current_unix_secs(),
    );
    Some((key_id, exclusion_until))
}

/// 为同一个 logical turn 规划并绑定下一个 attempt。
///
/// `_previous_settled` 不被使用，它只是把「上一个 attempt 已经结算完毕」这个
/// 前置条件写进签名：规划要读 health / adaptive / pool 状态，而这些是上一个
/// attempt 结算时才投射的；它的 pool key lease 也要先释放，否则替代 key 的挑选
/// 会看到一把仍被占用的 key。
pub(super) async fn retry_active_turn_after_quota_exhaustion(
    bound: &mut BoundResponsesConnection,
    state: &AppState,
    context: &WebSocketRequestContext,
    _previous_settled: PreviousAttemptSettled,
) -> QuotaRetryDisposition {
    let Some(active) = bound.turn_state.logical_mut() else {
        return QuotaRetryDisposition::Failed;
    };
    if let Some(reason) = active.quota_retry_block_reason() {
        debug!(
            event_name = "responses_websocket_quota_retry_skipped",
            log_type = "event",
            transport = WEBSOCKET_LOG_TRANSPORT,
            websocket = true,
            trace_id = %context.trace_id,
            turn_index = active.turn_index,
            logical_turn_id = %active.logical_turn_id,
            turn_attempt = active.turn_attempt,
            reason,
            "gateway will not transparently replay an unsafe Responses WebSocket turn"
        );
        return QuotaRetryDisposition::Failed;
    }
    active.retry_attempted = true;
    active.turn_attempt = active.turn_attempt.saturating_add(1);
    let client_event = active.client_event.clone();
    let turn_control_decision = active.control_decision.clone();
    let turn_auth_snapshot = active.auth_snapshot.clone();
    let turn_index = active.turn_index;
    let logical_turn_id = active.logical_turn_id.clone();
    let parent_request_id = active.request_id.clone();
    let turn_request_id = crate::execution_identity::ExecutionRequestId::generate().into_string();
    let turn_attempt = active.turn_attempt;

    let retry_exclusion_until_unix_secs = bound
        .pending_provider_drain
        .and_then(|directive| directive.retry_exclusion_until_unix_secs);
    let exhausted_key = record_exhausted_bound_key(bound, retry_exclusion_until_unix_secs);
    let exhausted_key_id = exhausted_key.as_ref().map(|(key_id, _)| key_id.clone());

    let planning_parts = build_planning_parts(context, &turn_request_id);
    let now_unix_secs = current_unix_secs();
    let excluded_key_ids = bound.exhausted_exclusions.key_ids(now_unix_secs);
    let excluded_codex_account_ids = bound.exhausted_exclusions.codex_account_ids(now_unix_secs);
    let excluded_key_ids = (!excluded_key_ids.is_empty()).then_some(excluded_key_ids);
    let excluded_codex_account_ids =
        (!excluded_codex_account_ids.is_empty()).then_some(excluded_codex_account_ids);
    let planning = spawn_owned_responses_websocket_plan(
        state.clone(),
        planning_parts,
        turn_request_id.clone(),
        turn_control_decision.clone(),
        client_event.clone(),
        excluded_key_ids,
        excluded_codex_account_ids,
        turn_auth_snapshot,
    );
    let owned_plan = match await_owned_responses_websocket_plan(planning).await {
        Ok(Some(decision)) => decision,
        Ok(None) => {
            warn!(
                event_name = "responses_websocket_quota_retry_provider_unavailable",
                log_type = "ops",
                transport = WEBSOCKET_LOG_TRANSPORT,
                websocket = true,
                trace_id = %context.trace_id,
                exhausted_key_id = ?exhausted_key_id,
                "gateway could not find an alternate Responses WebSocket provider after quota exhaustion"
            );
            return QuotaRetryDisposition::Failed;
        }
        Err(error) => {
            warn!(
                event_name = "responses_websocket_quota_retry_planning_failed",
                log_type = "ops",
                transport = WEBSOCKET_LOG_TRANSPORT,
                websocket = true,
                trace_id = %context.trace_id,
                exhausted_key_id = ?exhausted_key_id,
                error = ?error,
                "gateway could not plan an alternate Responses WebSocket provider after quota exhaustion"
            );
            return QuotaRetryDisposition::Failed;
        }
    };
    let planned = owned_plan.planned;
    let planned_lease = owned_plan.lease;
    let planning_parts = owned_plan.planning_parts;
    let backend = resolve_native_responses_websocket_backend(planned.backend);
    let provider_observer = resolve_responses_provider_observer(planned.provider_observer);
    let bound_candidate = planned.bound_candidate;
    let credential_binding_fingerprint = planned.credential_binding_fingerprint;
    let normalization = planned.normalization;
    let decision = canonicalize_responses_websocket_decision(planned.execution);
    if exhausted_key_id.as_deref() == decision.key_id.as_deref() {
        planned_lease.release().await;
        warn!(
            event_name = "responses_websocket_quota_retry_selected_exhausted_key",
            log_type = "ops",
            transport = WEBSOCKET_LOG_TRANSPORT,
            websocket = true,
            trace_id = %context.trace_id,
            key_id = ?decision.key_id,
            "gateway rejected an alternate Responses WebSocket plan that reused the exhausted key"
        );
        return QuotaRetryDisposition::Failed;
    }
    let provider_event = match planned_response_create_event(&decision, &client_event).and_then(
        |event| {
            serde_json::from_str::<Value>(&event)
                .map_err(|_| "response_create_serialization_failed")
        },
    ) {
        Ok(event) => event,
        Err(code) => {
            planned_lease.release().await;
            warn!(
                event_name = "responses_websocket_quota_retry_normalization_failed",
                log_type = "ops",
                transport = WEBSOCKET_LOG_TRANSPORT,
                websocket = true,
                trace_id = %context.trace_id,
                error_code = code,
                "gateway could not rebuild a Responses response.create for transparent quota retry"
            );
            return QuotaRetryDisposition::Failed;
        }
    };
    let turn_decision = prepare_responses_websocket_turn_decision(
        &decision,
        turn_request_id,
        Some(&parent_request_id),
        true,
        &client_event,
        &provider_event,
        &context.trace_id,
        turn_index,
        &logical_turn_id,
        turn_attempt,
    );
    let turn_start = spawn_owned_responses_websocket_turn(
        state.clone(),
        planning_parts,
        turn_control_decision,
        turn_decision,
        client_event.clone(),
        planned_lease,
        bound.session_termination.clone(),
    );
    let mut turn = match await_owned_responses_websocket_turn(turn_start).await {
        Ok(turn) => turn,
        Err(error) => {
            warn!(
                event_name = "responses_websocket_quota_retry_reporting_unavailable",
                log_type = "ops",
                transport = WEBSOCKET_LOG_TRANSPORT,
                websocket = true,
                trace_id = %context.trace_id,
                error = ?error,
                "gateway could not start usage and audit tracking for transparent quota retry"
            );
            return QuotaRetryDisposition::Failed;
        }
    };
    let mut replacement = match bind_responses_upstream(
        &decision,
        bound_candidate,
        credential_binding_fingerprint,
        normalization,
        &client_event,
        backend,
        provider_observer,
        || turn.admission_is_healthy(),
    )
    .await
    {
        Ok(connection) => connection,
        Err(ResponsesUpstreamBindError::ExecutionReservationLost) => {
            queue_turn_finalization(
                bound,
                state,
                disarm_owned_turn(turn),
                ResponsesWebSocketTurnOutcome::execution_reservation_lost(),
            );
            return QuotaRetryDisposition::ExecutionReservationLost;
        }
        Err(ResponsesUpstreamBindError::Upstream(code)) => {
            queue_turn_finalization(
                bound,
                state,
                disarm_owned_turn(turn),
                ResponsesWebSocketTurnOutcome::upstream_connect_failed(code),
            );
            warn!(
                event_name = "responses_websocket_quota_retry_rebind_failed",
                log_type = "ops",
                transport = WEBSOCKET_LOG_TRANSPORT,
                websocket = true,
                trace_id = %context.trace_id,
                error_code = code,
                "gateway could not bind an alternate Responses WebSocket provider after quota exhaustion"
            );
            return QuotaRetryDisposition::Failed;
        }
    };

    turn.mark_upstream_request_sent();
    turn.set_provider_response_headers(replacement.upstream_response_headers.clone());
    bound
        .backend_session
        .replace_from(&mut replacement.backend_session);
    let previous_key_id = bound.decision_template.key_id.clone();
    bound.backend = replacement.backend;
    bound.public_codec = replacement.public_codec;
    bound.provider_observer = replacement.provider_observer;
    bound.client_model = replacement.client_model;
    bound.provider_model = replacement.provider_model;
    bound.decision_template = replacement.decision_template;
    bound.bound_candidate = replacement.bound_candidate;
    bound.body_normalization = replacement.body_normalization;
    bound.binding_identity = replacement.binding_identity;
    // 同一个 logical turn 的下一个 attempt 就位。状态不符时把 attempt 交回
    // drop guard 结算并让调用方走「透明重试失败」分支，不静默丢弃一条已经写了
    // pending usage 行、占着 candidate 和 pool key lease 的 attempt。
    if let Err(orphan) = bound.turn_state.resume(turn) {
        drop(orphan);
        return QuotaRetryDisposition::Failed;
    }
    // A successful transparent retry occurs before any event from the failed
    // attempt is exposed publicly. Discard the speculative protocol progress
    // made while classifying that provider frame so the replacement must start
    // with its own response.created event.
    bound.public_event_state.reset();
    bound.public_event_sequence.reset();
    bound.upstream_response_headers = replacement.upstream_response_headers;
    bound.pending_provider_drain = None;
    debug!(
        event_name = "responses_websocket_quota_retry_rebound",
        log_type = "event",
        transport = WEBSOCKET_LOG_TRANSPORT,
        websocket = true,
        trace_id = %context.trace_id,
        turn_index,
        logical_turn_id = %logical_turn_id,
        turn_attempt,
        previous_key_id = ?previous_key_id,
        key_id = ?bound.decision_template.key_id,
        "gateway transparently rebound a Responses WebSocket turn after quota exhaustion"
    );
    QuotaRetryDisposition::Retried
}

pub(super) fn active_continuation_can_retry_from_full_input(
    bound: &BoundResponsesConnection,
) -> bool {
    bound.turn_state.logical().is_some_and(|active| {
        response_create_has_previous_response_id(&active.client_event)
            && active.retry_unsafe_reason.is_none()
    })
}

pub(super) fn is_usage_limit_error_event(event: &Value) -> bool {
    let is_error = |value: &Value| {
        value.get("type").and_then(Value::as_str) == Some("error")
            && value.pointer("/error/type").and_then(Value::as_str) == Some("usage_limit_reached")
    };
    is_error(event)
        || event
            .get("chunks")
            .and_then(Value::as_array)
            .is_some_and(|chunks| chunks.iter().any(is_error))
}

pub(super) fn should_request_full_continuation_retry(
    bound: &BoundResponsesConnection,
    retry_current_turn: bool,
    upstream_event: Option<&Value>,
) -> bool {
    retry_current_turn
        && active_continuation_can_retry_from_full_input(bound)
        && upstream_event.is_some_and(is_usage_limit_error_event)
}

pub(super) async fn send_previous_response_not_found(
    client_socket: &mut WebSocket,
    sequence: &super::state::ResponsesPublicEventSequence,
) {
    send_responses_websocket_error(
        client_socket,
        sequence,
        400,
        "invalid_request_error",
        "previous_response_not_found",
        PREVIOUS_RESPONSE_NOT_FOUND_MESSAGE,
        Some("previous_response_id"),
    )
    .await;
}

pub(super) fn observe_active_response_rebind_safety(
    bound: &mut BoundResponsesConnection,
    event: &Value,
) {
    let ResponsesWebSocketRebindSafety::Unsafe { reason } = bound
        .provider_observer
        .rebind_safety_for_upstream_event(event)
    else {
        return;
    };
    if let Some(active) = bound.turn_state.logical_mut() {
        active.mark_retry_unsafe(reason);
    }
}

pub(super) fn provider_frame_rebind_safety(
    frame: &ParsedResponsesWebSocketFrame<'_>,
    provider_observer: &dyn ResponsesProviderObserver,
) -> ResponsesWebSocketRebindSafety {
    frame
        .protocol_events()
        .into_iter()
        .map(|event| provider_observer.rebind_safety_for_upstream_event(event))
        .find(|safety| matches!(safety, ResponsesWebSocketRebindSafety::Unsafe { .. }))
        .unwrap_or(ResponsesWebSocketRebindSafety::Safe)
}

pub(super) fn mark_active_response_retry_unsafe(
    bound: &mut BoundResponsesConnection,
    reason: &'static str,
) {
    if let Some(active) = bound.turn_state.logical_mut() {
        active.mark_retry_unsafe(reason);
    }
}

#[cfg(test)]
mod tests {
    use super::super::adapter::{
        resolve_responses_provider_observer, ResponsesWebSocketRebindSafety,
    };
    use super::provider_frame_rebind_safety;
    use crate::handlers::proxy::websocket::responses::frame::ParsedResponsesWebSocketFrame;
    use crate::orchestration::ResponsesProviderObserverKind;

    #[test]
    fn quota_batch_with_any_public_response_event_is_not_transparently_replayable() {
        let raw = serde_json::json!({
            "type": "codex.response.metadata",
            "chunks": [
                {
                    "type": "response.created",
                    "response": {"id": "resp_side_effect_started"}
                },
                {
                    "type": "error",
                    "status_code": 429,
                    "error": {"type": "usage_limit_reached"}
                }
            ]
        })
        .to_string();
        let frame = ParsedResponsesWebSocketFrame::parse(&raw).expect("provider frame JSON");
        let observer = resolve_responses_provider_observer(ResponsesProviderObserverKind::Codex);

        assert_eq!(
            provider_frame_rebind_safety(&frame, observer),
            ResponsesWebSocketRebindSafety::Unsafe {
                reason: "standard_response_event"
            }
        );
    }
}
