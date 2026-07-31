//! Connection-level Responses WebSocket FSM.

use std::time::Duration;

use axum::extract::ws::WebSocket;
use futures_util::{SinkExt, StreamExt};
use serde_json::Value;
use tokio::time::sleep;
use wreq::ws::message::Message as WreqWsMessage;

use super::client::{adapter_drain_ready, forward_client_message, RelayDisposition};
use super::frame::ParsedResponsesWebSocketFrame;
use super::lifecycle::{
    await_pending_adapter_observation, finalize_active_turn, queue_turn_finalization,
    ActiveResponsesWebSocketTurn,
};
use super::quota::{
    active_continuation_can_retry_from_full_input, detach_exhausted_upstream,
    is_usage_limit_error_event, mark_active_response_retry_unsafe,
    observe_active_response_rebind_safety, retry_active_turn_after_quota_exhaustion,
    send_previous_response_not_found, should_request_full_continuation_retry,
};
use super::relay_policy::{
    classify_quota_relay, classify_upstream_frame, fatal_relay_policy, FatalRelaySignal,
    QuotaRelayAction, QuotaRelayFacts, UpstreamFrameAction, UpstreamFrameKind,
};
use super::state::BoundResponsesConnection;
use super::turn::{
    ResponsesWebSocketTurn, ResponsesWebSocketTurnObservation, ResponsesWebSocketTurnOutcome,
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
        let active_turn_deadline = bound.active_turn.as_ref().map(|turn| turn.deadline());
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
                bound.active_response_create = None;
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
                bound.active_response_create = None;
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
                bound.active_response_create = None;
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
                    bound.active_response_create = None;
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
                    bound.active_response_create = None;
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
                        bound.active_response_create = None;
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
                    bound.active_response_create = None;
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
                    bound.active_response_create = None;
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
                        active_turn = bound.active_turn.is_some(),
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
                                .active_turn
                                .as_mut()
                                .and_then(|turn| turn.observe_upstream_frame(frame, adapter)),
                            None => {
                                if let Some(turn) = bound.active_turn.as_mut() {
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
                update_response_in_flight(bound, parsed_upstream_frame.as_ref());
                if matches!(
                    observation,
                    Some(ResponsesWebSocketTurnObservation::Started)
                        | Some(ResponsesWebSocketTurnObservation::Terminal(_))
                ) {
                    if let Some(turn) = bound.active_turn.as_mut() {
                        turn.mark_stream_started(state).await;
                    }
                }
                let terminal_outcome = match observation {
                    Some(ResponsesWebSocketTurnObservation::Terminal(outcome)) => Some(outcome),
                    _ => None,
                };
                if terminal_outcome.is_some() {
                    bound.response_in_flight = false;
                }
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
                    bound.active_response_create = None;
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
                    bound.response_in_flight,
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
                    let mut retry_turn = bound.active_turn.take().map(ActiveResponsesWebSocketTurn::disarm);
                    if let Some(turn) = retry_turn.as_mut() {
                        turn.release_admission().await;
                    }
                    if retry_active_turn_after_quota_exhaustion(bound, state, context).await {
                        if let Some(turn) = retry_turn {
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
                        continue;
                    }
                    bound.active_turn = retry_turn.map(|turn| ActiveResponsesWebSocketTurn::new(state, turn));
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
                    let mut turn = bound.active_turn.take().map(ActiveResponsesWebSocketTurn::disarm);
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
                    bound.active_response_create = None;
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
                    bound.active_response_create = None;
                    detach_exhausted_upstream(bound, directive, &context.trace_id).await;
                    continue;
                }
                if let Err(error) = send_client_message(
                    client_socket,
                    upstream_message_to_client(upstream_message.clone()),
                ).await {
                    warn!(
                        event_name = "responses_websocket_client_send_failed",
                        log_type = "ops",
                        transport = WEBSOCKET_LOG_TRANSPORT,
                        websocket = true,
                        trace_id = %context.trace_id,
                        error_code = error.as_str(),
                        "gateway could not relay a provider event to the client"
                    );
                    finalize_active_turn(
                        bound,
                        state,
                        ResponsesWebSocketTurnOutcome::client_disconnected(),
                    ).await;
                    bound.active_response_create = None;
                    close_bound_upstream(bound).await;
                    break;
                }
                if let (Some(turn), Some(frame)) =
                    (bound.active_turn.as_mut(), parsed_upstream_frame.as_ref())
                {
                    turn.capture_client_frame(frame.event());
                }
                if let Some(outcome) = terminal_outcome {
                    finalize_active_turn(bound, state, outcome).await;
                    bound.active_response_create = None;
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
                    bound.active_response_create = None;
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

fn update_response_in_flight(
    bound: &mut BoundResponsesConnection,
    frame: Option<&ParsedResponsesWebSocketFrame<'_>>,
) {
    let Some(frame) = frame else {
        return;
    };
    let frame_kind = if frame.is_terminal() {
        UpstreamFrameKind::Terminal
    } else if frame.is_started() {
        UpstreamFrameKind::Started
    } else {
        UpstreamFrameKind::Other
    };
    match classify_upstream_frame(frame_kind) {
        UpstreamFrameAction::Continue if frame.is_started() => {
            bound.response_in_flight = true;
        }
        UpstreamFrameAction::FinalizeTurn => {
            bound.response_in_flight = false;
        }
        UpstreamFrameAction::Continue | UpstreamFrameAction::FinalizeAndClose => {}
    }
}
