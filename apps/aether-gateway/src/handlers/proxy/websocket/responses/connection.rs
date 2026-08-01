//! Connection-level Responses WebSocket FSM.

use std::time::Duration;

use axum::extract::ws::{Message as AxumWsMessage, WebSocket};
use futures_util::{SinkExt, StreamExt};
use serde_json::Value;
use tokio::time::sleep;
use wreq::ws::message::Message as WreqWsMessage;

use super::client::{adapter_drain_ready, forward_client_message, RelayDisposition};
use super::frame::ParsedResponsesWebSocketFrame;
use super::lifecycle::{
    await_pending_adapter_observation, finalize_active_turn, queue_turn_finalization,
    settle_turn_finalization, ActiveProviderAttempt, PreviousAttemptSettled,
};
use super::quota::{
    active_continuation_can_retry_from_full_input, detach_exhausted_upstream,
    is_usage_limit_error_event, mark_active_response_retry_unsafe,
    observe_active_response_rebind_safety, retry_active_turn_after_quota_exhaustion,
    send_previous_response_not_found, should_request_full_continuation_retry,
};
use super::relay_policy::{
    classify_quota_relay, fatal_relay_policy, FatalRelaySignal, QuotaRelayAction, QuotaRelayFacts,
};
use super::settlement::settle_signal_for_client_delivery_failure;
use super::state::BoundResponsesConnection;
use super::turn::{
    ResponsesProviderAttempt, ResponsesWebSocketTurnObservation, ResponsesWebSocketTurnOutcome,
};
use super::upstream::{close_bound_upstream, receive_optional_upstream};
use crate::handlers::proxy::websocket::ingress::WebSocketRequestContext;
use crate::handlers::proxy::websocket::session::{
    wait_for_optional_deadline, CLOSE_INTERNAL_ERROR, CLOSE_TRY_AGAIN,
    RESPONSES_WEBSOCKET_SESSION_LIMITS, WEBSOCKET_LOG_TRANSPORT,
};
use crate::handlers::proxy::websocket::transport::{
    close_client_socket, send_client_message, send_gateway_error_with_status,
    send_responses_websocket_error, upstream_message_to_client,
};
use crate::AppState;

const LOG_TARGET: &str = "aether_gateway::handlers::proxy::responses_ws";

/// 写客户端 socket 失败时记录的投递失败原因。刻意不说「客户端在终态前断开」：
/// 供应商的终态可能已经到达，只是最后一跳没送出去。
const CLIENT_DELIVERY_FAILED_REASON: &str =
    "gateway could not relay the provider event to the client";

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

pub(super) async fn relay_bound_connection(
    client_socket: &mut WebSocket,
    bound: &mut BoundResponsesConnection,
    state: &AppState,
    context: &WebSocketRequestContext,
    connection_permit: Option<aether_runtime::AdmissionPermit>,
) {
    let connection_deadline = sleep(RESPONSES_WEBSOCKET_SESSION_LIMITS.max_connection_duration);
    tokio::pin!(connection_deadline);

    loop {
        let active_turn_deadline = bound.turn_state.attempt().map(|turn| turn.deadline());
        tokio::select! {
            _ = &mut connection_deadline => {
                finalize_active_turn(
                    bound,
                    state,
                    ResponsesWebSocketTurnOutcome::connection_limit_reached(),
                ).await;
                send_gateway_error_with_status(
                    client_socket,
                    503,
                    "websocket_connection_limit_reached",
                    "WebSocket connection duration limit reached; reconnect to continue",
                ).await;
                close_bound_upstream(bound).await;
                close_client_socket(client_socket, CLOSE_TRY_AGAIN, "connection_limit_reached").await;
                break;
            }
            _ = wait_for_optional_deadline(active_turn_deadline.map(|deadline| deadline.deadline)) => {
                let Some(turn_deadline) = active_turn_deadline else {
                    continue;
                };
                warn!(
                    event_name = "responses_websocket_turn_timeout",
                    log_type = "ops",
                    transport = WEBSOCKET_LOG_TRANSPORT,
                    websocket = true,
                    trace_id = %context.trace_id,
                    timeout_phase = ?turn_deadline.phase,
                    timeout_ms = turn_deadline.timeout.as_millis() as u64,
                    "Responses WebSocket response did not reach its configured deadline"
                );
                finalize_active_turn(bound, state, turn_deadline.phase.outcome()).await;
                send_gateway_error_with_status(
                    client_socket,
                    504,
                    turn_deadline.phase.error_code(),
                    turn_deadline.phase.client_message(),
                ).await;
                close_bound_upstream(bound).await;
                close_client_socket(
                    client_socket,
                    CLOSE_TRY_AGAIN,
                    turn_deadline.phase.error_code(),
                ).await;
                break;
            }
            _ = wait_for_connection_permit_loss(connection_permit.as_ref()) => {
                let policy = fatal_relay_policy(FatalRelaySignal::ConnectionAdmissionLost);
                warn!(
                    event_name = "responses_websocket_connection_admission_lost",
                    log_type = "ops",
                    transport = WEBSOCKET_LOG_TRANSPORT,
                    websocket = true,
                    trace_id = %context.trace_id,
                    "gateway closed Responses WebSocket after its connection admission became unhealthy"
                );
                finalize_active_turn(
                    bound,
                    state,
                    ResponsesWebSocketTurnOutcome::connection_admission_lost(),
                ).await;
                close_bound_upstream(bound).await;
                send_gateway_error_with_status(
                    client_socket,
                    policy.status_code,
                    policy.error_code,
                    policy.client_message,
                ).await;
                close_client_socket(client_socket, policy.close_code, policy.close_reason).await;
                break;
            }
            client_message = client_socket.next() => {
                let Some(client_message) = client_message else {
                    finalize_active_turn(
                        bound,
                        state,
                        ResponsesWebSocketTurnOutcome::client_disconnected(),
                    ).await;
                    close_bound_upstream(bound).await;
                    break;
                };
                let Ok(client_message) = client_message else {
                    warn!(
                        event_name = "responses_websocket_client_receive_failed",
                        log_type = "ops",
                        transport = WEBSOCKET_LOG_TRANSPORT,
                        websocket = true,
                        trace_id = %context.trace_id,
                        "client WebSocket receive failed"
                    );
                    finalize_active_turn(
                        bound,
                        state,
                        ResponsesWebSocketTurnOutcome::client_disconnected(),
                    ).await;
                    close_bound_upstream(bound).await;
                    break;
                };
                match forward_client_message(client_message, bound, client_socket, state, context).await {
                    RelayDisposition::Continue => {}
                    RelayDisposition::Close => {
                        finalize_active_turn(
                            bound,
                            state,
                            ResponsesWebSocketTurnOutcome::client_disconnected(),
                        ).await;
                        break;
                    }
                    RelayDisposition::UpstreamError(code) => {
                        warn!(
                            event_name = "responses_websocket_upstream_send_failed",
                            log_type = "ops",
                            transport = WEBSOCKET_LOG_TRANSPORT,
                            websocket = true,
                            trace_id = %context.trace_id,
                            error_code = code,
                            "Upstream WebSocket send failed"
                        );
                        finalize_active_turn(
                            bound,
                            state,
                            ResponsesWebSocketTurnOutcome::upstream_send_failed(),
                        ).await;
                        send_gateway_error_with_status(
                            client_socket,
                            502,
                            code,
                            "Gateway could not forward the WebSocket event upstream",
                        ).await;
                        close_bound_upstream(bound).await;
                        close_client_socket(client_socket, CLOSE_INTERNAL_ERROR, code).await;
                        break;
                    }
                }
            }
            upstream_message = receive_optional_upstream(&mut bound.upstream) => {
                let Some(upstream_message) = upstream_message else {
                    finalize_active_turn(
                        bound,
                        state,
                        ResponsesWebSocketTurnOutcome::upstream_closed(),
                    ).await;
                    bound.upstream = None;
                    close_client_socket(client_socket, 1000, "upstream_closed").await;
                    break;
                };
                let Ok(upstream_message) = upstream_message else {
                    warn!(
                        event_name = "responses_websocket_upstream_receive_failed",
                        log_type = "ops",
                        transport = WEBSOCKET_LOG_TRANSPORT,
                        websocket = true,
                        trace_id = %context.trace_id,
                        "Upstream WebSocket receive failed"
                    );
                    finalize_active_turn(
                        bound,
                        state,
                        ResponsesWebSocketTurnOutcome::upstream_receive_failed(),
                    ).await;
                    send_gateway_error_with_status(
                        client_socket,
                        502,
                        "responses_websocket_receive_failed",
                        "Provider connection closed unexpectedly",
                    ).await;
                    bound.upstream = None;
                    close_client_socket(client_socket, CLOSE_INTERNAL_ERROR, "upstream_receive_failed").await;
                    break;
                };
                let parsed_upstream_frame = match &upstream_message {
                    WreqWsMessage::Text(text) => {
                        ParsedResponsesWebSocketFrame::parse(text.as_str()).ok()
                    }
                    _ => None,
                };
                let parsed_upstream_event = parsed_upstream_frame
                    .as_ref()
                    .map(ParsedResponsesWebSocketFrame::event);
                if let WreqWsMessage::Text(text) = &upstream_message {
                    debug!(
                        event_name = "responses_websocket_upstream_event",
                        log_type = "event",
                        transport = WEBSOCKET_LOG_TRANSPORT,
                        websocket = true,
                        trace_id = %context.trace_id,
                        event_type = %parsed_upstream_frame
                            .as_ref()
                            .map(ParsedResponsesWebSocketFrame::event_type_for_log)
                            .unwrap_or_else(|| "invalid_json".to_string()),
                        frame_bytes = text.len(),
                        chunked = parsed_upstream_frame
                            .as_ref()
                            .is_some_and(ParsedResponsesWebSocketFrame::is_chunked),
                        active_turn = bound.turn_state.response_in_flight(),
                        "gateway received Responses WebSocket event"
                    );
                }
                if matches!(&upstream_message, WreqWsMessage::Binary(_)) {
                    mark_active_response_retry_unsafe(bound, "upstream_binary_frame");
                } else if matches!(&upstream_message, WreqWsMessage::Text(_))
                    && parsed_upstream_event.is_none()
                {
                    mark_active_response_retry_unsafe(bound, "invalid_upstream_event");
                }
                if let Some(event) = parsed_upstream_event {
                    observe_active_response_rebind_safety(bound, event);
                    if bound.pending_adapter_drain.is_none()
                        && bound.adapter.observes_upstream_events()
                    {
                        let adapter = bound.adapter;
                        if let Some(observation) = adapter.observe_upstream_event(event) {
                            let directive = observation.drain;
                            await_pending_adapter_observation(bound).await;
                            let state_for_observation = state.clone();
                            let trace_id = context.trace_id.clone();
                            let report_context = bound.decision_template.report_context.clone();
                            bound.pending_adapter_observation = Some(tokio::spawn(async move {
                                adapter
                                    .persist_upstream_observation(
                                        &state_for_observation,
                                        &trace_id,
                                        report_context.as_ref(),
                                        observation,
                                    )
                                    .await;
                            }));
                            if let Some(directive) = directive {
                                bound.pending_adapter_drain = Some(directive);
                                // A definitive quota signal must be visible to
                                // the next planner before a transparent retry.
                                await_pending_adapter_observation(bound).await;
                            }
                        }
                    }
                }
                let observation = match &upstream_message {
                    WreqWsMessage::Text(text) => {
                        let adapter = bound.adapter;
                        match parsed_upstream_frame.as_ref() {
                            Some(frame) => bound
                                .turn_state
                                .attempt_mut()
                                .and_then(|turn| turn.observe_upstream_frame(frame, adapter)),
                            None => {
                                if let Some(turn) = bound.turn_state.attempt_mut() {
                                    turn.observe_invalid_upstream_text(text.as_str())
                                }
                                else {
                                    None
                                }
                            }
                        }
                    }
                    _ => None,
                };
                if matches!(
                    observation,
                    Some(ResponsesWebSocketTurnObservation::Started)
                        | Some(ResponsesWebSocketTurnObservation::Terminal(_))
                ) {
                    if let Some(turn) = bound.turn_state.attempt_mut() {
                        turn.mark_stream_started(state).await;
                    }
                }
                let terminal_outcome = match observation {
                    Some(ResponsesWebSocketTurnObservation::Terminal(outcome)) => Some(outcome),
                    _ => None,
                };
                if matches!(&upstream_message, WreqWsMessage::Text(_))
                    && parsed_upstream_frame.is_none()
                {
                    let policy = fatal_relay_policy(FatalRelaySignal::InvalidUpstreamText);
                    finalize_active_turn(
                        bound,
                        state,
                        terminal_outcome.unwrap_or_else(
                            ResponsesWebSocketTurnOutcome::upstream_receive_failed,
                        ),
                    )
                    .await;
                    send_responses_websocket_error(
                        client_socket,
                        policy.status_code,
                        "server_error",
                        policy.error_code,
                        policy.client_message,
                    )
                    .await;
                    close_bound_upstream(bound).await;
                    close_client_socket(
                        client_socket,
                        policy.close_code,
                        policy.close_reason,
                    )
                    .await;
                    break;
                }
                let is_close = matches!(upstream_message, WreqWsMessage::Close(_));
                let drain_for_adapter = adapter_drain_ready(
                    bound.pending_adapter_drain,
                    bound.turn_state.response_in_flight(),
                    observation,
                    is_close,
                );
                let quota_facts = QuotaRelayFacts {
                    drain_ready: drain_for_adapter,
                    retry_current_turn: bound
                        .pending_adapter_drain
                        .is_some_and(|directive| directive.retry_current_turn),
                    transparent_retry_failed: false,
                    usage_limit_error: parsed_upstream_event.is_some_and(is_usage_limit_error_event),
                    continuation_retry_eligible: active_continuation_can_retry_from_full_input(bound),
                    upstream_closed: is_close,
                };
                let mut quota_relay_action = classify_quota_relay(quota_facts);
                if matches!(quota_relay_action, QuotaRelayAction::AttemptTransparentRetry) {
                    // detach_attempt 保留 logical turn：重试是同一轮请求的下一个 attempt。
                    let retry_turn = bound
                        .turn_state
                        .detach_attempt()
                        .map(ActiveProviderAttempt::disarm);
                    // 先结算旧 attempt 并等它落地，再规划下一个 attempt。两个理由：
                    //
                    // 1. 规划要读 health / adaptive / pool 状态，而这些正是旧
                    //    attempt 结算时才投射的。普通的新 turn 早就在 client.rs 里
                    //    用 await_pending_turn_finalization 挡住了「基于陈旧状态
                    //    规划」，透明重试这条路径原先漏了这一步。
                    // 2. 旧 attempt 还占着自己的 pool key lease。不先释放，重试就
                    //    可能因为「这把 key 仍被占用」而挑不到本该可用的替代 key，
                    //    或者干脆判成无可用供应商。
                    let settled = match retry_turn {
                        Some(mut turn) => {
                            turn.release_admission().await;
                            settle_turn_finalization(
                                bound,
                                state,
                                turn,
                                terminal_outcome.unwrap_or_else(
                                    ResponsesWebSocketTurnOutcome::upstream_closed,
                                ),
                            )
                            .await
                        }
                        None => PreviousAttemptSettled::nothing_to_settle(),
                    };
                    if retry_active_turn_after_quota_exhaustion(bound, state, context, settled).await
                    {
                        continue;
                    }
                    // 重试失败。旧 attempt 已经结算，logical turn 仍停在
                    // Replanning，所以后面分支里的 end() / finalize_active_turn
                    // 只会清掉 logical turn 而不会交出 attempt——不存在重复结算。
                    quota_relay_action = classify_quota_relay(QuotaRelayFacts {
                        retry_current_turn: false,
                        transparent_retry_failed: true,
                        ..quota_facts
                    });
                }
                if matches!(
                    quota_relay_action,
                    QuotaRelayAction::RequestFullContinuationRetry
                ) {
                    let directive = bound
                        .pending_adapter_drain
                        .expect("adapter drain state should be present");
                    debug!(
                        event_name = "responses_websocket_continuation_retry_required",
                        log_type = "event",
                        transport = WEBSOCKET_LOG_TRANSPORT,
                        websocket = true,
                        trace_id = %context.trace_id,
                        error_code = "previous_response_not_found",
                        "gateway will ask the client to retry the continuation with complete input"
                    );
                    let mut turn = bound.turn_state.end().map(ActiveProviderAttempt::disarm);
                    if let Some(active_turn) = turn.as_mut() {
                        active_turn.release_admission().await;
                    }
                    send_previous_response_not_found(client_socket).await;
                    if let Some(turn) = turn {
                        queue_turn_finalization(
                            bound,
                            state,
                            turn,
                            terminal_outcome.unwrap_or_else(
                                ResponsesWebSocketTurnOutcome::upstream_closed,
                            ),
                        )
                        .await;
                    }
                    detach_exhausted_upstream(bound, directive, &context.trace_id).await;
                    continue;
                }
                if matches!(quota_relay_action, QuotaRelayAction::ForwardQuotaAndDetach) {
                    let directive = bound
                        .pending_adapter_drain
                        .expect("adapter drain state should be present");
                    finalize_active_turn(
                        bound,
                        state,
                        terminal_outcome
                            .unwrap_or_else(ResponsesWebSocketTurnOutcome::provider_quota_exhausted),
                    )
                    .await;
                    send_gateway_error_with_status(
                        client_socket,
                        429,
                        directive.error_code,
                        "Provider connection closed after reporting exhausted quota; send a new response.create to select another Provider connection",
                    )
                    .await;
                    detach_exhausted_upstream(bound, directive, &context.trace_id).await;
                    continue;
                }
                // 响应侧还原：HTTP 在把响应体交给客户端之前会把占位符换回真实值
                // （`privacy::restore_sync_response_body` /
                // `privacy::StreamingResponseRestorer`），这里是 WS 的同一个位置
                // ——最后一跳之前，并且在 `capture_client_frame` 之前，所以审计与
                // 终态观测继续消费脱敏态的事件。没有命中还原时保持上游原字节。
                let restored_client_frame = parsed_upstream_frame
                    .as_ref()
                    .map(ParsedResponsesWebSocketFrame::event)
                    .and_then(|event| {
                        bound.redaction_restorer.restore_provider_frame_text(event)
                    });
                let client_frame = match restored_client_frame {
                    Some(restored) => AxumWsMessage::Text(restored.into()),
                    None => upstream_message_to_client(upstream_message.clone()),
                };
                if let Err(error) = send_client_message(client_socket, client_frame).await {
                    warn!(
                        event_name = "responses_websocket_client_send_failed",
                        log_type = "ops",
                        transport = WEBSOCKET_LOG_TRANSPORT,
                        websocket = true,
                        trace_id = %context.trace_id,
                        error_code = error.as_str(),
                        provider_terminal_reached = terminal_outcome.is_some(),
                        "gateway could not relay a provider event to the client"
                    );
                    // 投递失败是独立事实，不能覆盖已经到达的 provider 终态：
                    // 供应商已经完成推理并消耗 token，账单按它的终态计。
                    bound
                        .turn_state
                        .record_client_delivery_aborted(CLIENT_DELIVERY_FAILED_REASON);
                    finalize_active_turn(
                        bound,
                        state,
                        settle_signal_for_client_delivery_failure(terminal_outcome),
                    ).await;
                    close_bound_upstream(bound).await;
                    break;
                }
                if let (Some(turn), Some(frame)) =
                    (bound.turn_state.attempt_mut(), parsed_upstream_frame.as_ref())
                {
                    turn.capture_client_frame(frame.event());
                }
                if let Some(outcome) = terminal_outcome {
                    finalize_active_turn(bound, state, outcome).await;
                } else if is_close {
                    finalize_active_turn(
                        bound,
                        state,
                        ResponsesWebSocketTurnOutcome::upstream_closed(),
                    )
                    .await;
                }
                if drain_for_adapter {
                    let directive = bound
                        .pending_adapter_drain
                        .expect("adapter drain state should be present");
                    detach_exhausted_upstream(bound, directive, &context.trace_id).await;
                    continue;
                }
                if is_close {
                    bound.upstream = None;
                    break;
                }
            }
        }
    }
}

async fn wait_for_connection_permit_loss(permit: Option<&aether_runtime::AdmissionPermit>) {
    let Some(permit) = permit else {
        std::future::pending::<()>().await;
        return;
    };
    let mut health = tokio::time::interval(Duration::from_secs(1));
    health.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
    loop {
        health.tick().await;
        if !permit.is_healthy() {
            return;
        }
    }
}
