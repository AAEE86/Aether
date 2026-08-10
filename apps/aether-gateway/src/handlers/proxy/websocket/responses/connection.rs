//! Connection-level Responses WebSocket FSM.

use axum::extract::ws::{Message as AxumWsMessage, WebSocket};
use futures_util::StreamExt;
use serde_json::Value;
use tokio::time::Instant as TokioInstant;

use super::adapter::ResponsesPublicWireError;
use super::backend::{
    ResponsesBackendFailure, ResponsesBackendInput, ResponsesBackendProtocolViolation,
};
use super::client::{forward_client_message, provider_drain_ready, RelayDisposition};
use super::frame::ParsedResponsesWebSocketFrame;
use super::lifecycle::{
    await_pending_provider_observation, finalize_active_turn, queue_turn_finalization,
    settle_turn_finalization, ActiveProviderAttempt, PreviousAttemptSettled,
};
use super::quota::{
    active_continuation_can_retry_from_full_input, detach_exhausted_upstream,
    is_usage_limit_error_event, mark_active_response_retry_unsafe,
    observe_active_response_rebind_safety, provider_frame_rebind_safety,
    retry_active_turn_after_quota_exhaustion, send_previous_response_not_found,
    should_request_full_continuation_retry, QuotaRetryDisposition,
};
use super::relay_policy::{
    classify_quota_relay, fatal_relay_policy, FatalRelaySignal, QuotaRelayAction, QuotaRelayFacts,
};
use super::request::response_create_previous_response_id;
use super::settlement::settle_signal_for_client_delivery_failure;
use super::state::{
    evict_referenced_public_response_id, BoundResponsesConnection,
    ResponsesPublicEventSequenceReservation,
};
use super::turn::{
    ResponsesProviderAttempt, ResponsesWebSocketTurnObservation, ResponsesWebSocketTurnOutcome,
};
use super::upstream::close_bound_upstream;
use crate::handlers::proxy::websocket::ingress::WebSocketRequestContext;
use crate::handlers::proxy::websocket::session::{
    wait_for_optional_deadline, CLOSE_INTERNAL_ERROR, CLOSE_TRY_AGAIN, RELAY_WRITE_TIMEOUT,
    WEBSOCKET_LOG_TRANSPORT,
};
use crate::handlers::proxy::websocket::transport::{
    close_client_socket, feed_client_message_until, flush_client_messages_until,
    send_gateway_error_with_status, send_responses_websocket_error,
};
use crate::AppState;

const LOG_TARGET: &str = "aether_gateway::handlers::proxy::responses_ws";

/// 写客户端 socket 失败时记录的投递失败原因。刻意不说「客户端在终态前断开」：
/// 供应商的终态可能已经到达，只是最后一跳没送出去。
const CLIENT_DELIVERY_FAILED_REASON: &str =
    "gateway could not relay the provider event to the client";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum UpstreamCloseAction {
    DetachIdleUpstream,
    ForwardQuotaAndDetach,
    FailActiveTurn,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum PhysicalBindingLossAction {
    DetachIdleUpstream,
    FailActiveTurn,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ProviderControlWriteAction {
    Continue,
    DetachIdleUpstream,
    FailActiveTurn,
}

fn physical_binding_loss_action(response_in_flight: bool) -> PhysicalBindingLossAction {
    if response_in_flight {
        PhysicalBindingLossAction::FailActiveTurn
    } else {
        PhysicalBindingLossAction::DetachIdleUpstream
    }
}

fn provider_control_write_action(
    write_succeeded: bool,
    response_in_flight: bool,
) -> ProviderControlWriteAction {
    if write_succeeded {
        return ProviderControlWriteAction::Continue;
    }
    match physical_binding_loss_action(response_in_flight) {
        PhysicalBindingLossAction::DetachIdleUpstream => {
            ProviderControlWriteAction::DetachIdleUpstream
        }
        PhysicalBindingLossAction::FailActiveTurn => ProviderControlWriteAction::FailActiveTurn,
    }
}

fn upstream_close_action(
    pending_provider_drain: Option<super::adapter::ResponsesWebSocketDrainDirective>,
    response_in_flight: bool,
) -> UpstreamCloseAction {
    if matches!(
        physical_binding_loss_action(response_in_flight),
        PhysicalBindingLossAction::DetachIdleUpstream
    ) {
        return UpstreamCloseAction::DetachIdleUpstream;
    }
    match classify_upstream_closed_quota_relay(pending_provider_drain, response_in_flight) {
        QuotaRelayAction::ForwardQuotaAndDetach => UpstreamCloseAction::ForwardQuotaAndDetach,
        QuotaRelayAction::None => UpstreamCloseAction::FailActiveTurn,
        QuotaRelayAction::AttemptTransparentRetry
        | QuotaRelayAction::RequestFullContinuationRetry => {
            unreachable!("a provider Close cannot trigger an event-driven quota retry")
        }
    }
}

async fn detach_idle_upstream_after_binding_loss(
    bound: &mut BoundResponsesConnection,
    trace_id: &str,
) {
    debug_assert!(!bound.turn_state.response_in_flight());
    if let Some(directive) = bound.pending_provider_drain {
        detach_exhausted_upstream(bound, directive, trace_id).await;
    } else {
        bound.backend_session.detach();
    }
}

fn commit_public_event_delivery<E>(
    delivery: Result<(), E>,
    reservation: ResponsesPublicEventSequenceReservation,
) -> Result<(), E> {
    delivery?;
    reservation.commit();
    Ok(())
}

fn active_previous_response_id(bound: &BoundResponsesConnection) -> Option<&str> {
    bound
        .turn_state
        .logical()
        .and_then(|logical| response_create_previous_response_id(&logical.client_event).ok())
        .flatten()
}

fn evict_active_continuation(bound: &mut BoundResponsesConnection) {
    let previous_response_id = active_previous_response_id(bound).map(str::to_string);
    evict_referenced_public_response_id(
        &mut bound.latest_public_response_id,
        previous_response_id.as_deref(),
    );
}

async fn fail_active_turn(
    bound: &mut BoundResponsesConnection,
    state: &AppState,
    outcome: ResponsesWebSocketTurnOutcome,
) {
    // Read ownership before finalization moves the logical turn to Idle.  This
    // is deliberately conditional: an independent failed turn has no
    // previous_response_id and therefore preserves the latest successful chain.
    evict_active_continuation(bound);
    finalize_active_turn(bound, state, outcome).await;
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum ContinuationCacheUpdate {
    Commit(String),
    EvictReferenced,
}

fn continuation_cache_update_for_event(event: &Value) -> Option<ContinuationCacheUpdate> {
    match event.get("type").and_then(Value::as_str) {
        Some("response.completed") | Some("response.incomplete") => event
            .pointer("/response/id")
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|response_id| !response_id.is_empty())
            .map(|response_id| ContinuationCacheUpdate::Commit(response_id.to_string())),
        Some("response.failed") | Some("response.cancelled") | Some("error") => {
            Some(ContinuationCacheUpdate::EvictReferenced)
        }
        _ => None,
    }
}

fn apply_continuation_cache_update(
    bound: &mut BoundResponsesConnection,
    update: Option<ContinuationCacheUpdate>,
) {
    match update {
        Some(ContinuationCacheUpdate::Commit(response_id)) => {
            bound.latest_public_response_id = Some(response_id);
        }
        Some(ContinuationCacheUpdate::EvictReferenced) => evict_active_continuation(bound),
        None => {}
    }
}

fn classify_upstream_closed_quota_relay(
    pending_provider_drain: Option<super::adapter::ResponsesWebSocketDrainDirective>,
    response_in_flight: bool,
) -> QuotaRelayAction {
    classify_quota_relay(QuotaRelayFacts {
        drain_ready: provider_drain_ready(pending_provider_drain, response_in_flight, None, true),
        retry_current_turn: pending_provider_drain
            .is_some_and(|directive| directive.retry_current_turn),
        transparent_retry_failed: false,
        usage_limit_error: false,
        continuation_retry_eligible: false,
        upstream_closed: true,
    })
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

pub(super) async fn relay_bound_connection(
    client_socket: &mut WebSocket,
    bound: &mut BoundResponsesConnection,
    state: &AppState,
    context: &WebSocketRequestContext,
    connection_deadline: TokioInstant,
) {
    let mut admission_health = tokio::time::interval_at(
        TokioInstant::now() + std::time::Duration::from_secs(1),
        std::time::Duration::from_secs(1),
    );
    admission_health.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
    loop {
        let active_turn_deadline = bound.turn_state.attempt().map(|turn| turn.deadline());
        tokio::select! {
            _ = admission_health.tick() => {
                let reservation_lost = bound
                    .turn_state
                    .attempt()
                    .is_some_and(|turn| !turn.admission_is_healthy());
                if !reservation_lost {
                    continue;
                }
                warn!(
                    event_name = "responses_websocket_execution_reservation_lost",
                    log_type = "ops",
                    transport = WEBSOCKET_LOG_TRANSPORT,
                    websocket = true,
                    trace_id = %context.trace_id,
                    "Responses WebSocket execution reservation renewal was lost"
                );
                terminate_after_execution_reservation_lost(client_socket, bound, state).await;
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
                fail_active_turn(bound, state, turn_deadline.phase.outcome()).await;
                if !claim_public_teardown(bound) {
                    break;
                }
                send_gateway_error_with_status(
                    client_socket,
                    &bound.public_event_sequence,
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
            client_message = client_socket.next() => {
                let Some(client_message) = client_message else {
                    let _ = claim_public_teardown(bound);
                    finalize_active_turn(
                        bound,
                        state,
                        ResponsesWebSocketTurnOutcome::client_disconnected(),
                    ).await;
                    close_bound_upstream(bound).await;
                    break;
                };
                let Ok(client_message) = client_message else {
                    let _ = claim_public_teardown(bound);
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
                        let _ = claim_public_teardown(bound);
                        finalize_active_turn(
                            bound,
                            state,
                            ResponsesWebSocketTurnOutcome::client_disconnected(),
                        ).await;
                        break;
                    }
                    RelayDisposition::ExecutionReservationLost => {
                        warn!(
                            event_name = "responses_websocket_execution_reservation_lost",
                            log_type = "ops",
                            transport = WEBSOCKET_LOG_TRANSPORT,
                            websocket = true,
                            trace_id = %context.trace_id,
                            "Responses WebSocket execution reservation was lost before provider send"
                        );
                        terminate_after_execution_reservation_lost(
                            client_socket,
                            bound,
                            state,
                        )
                        .await;
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
                        fail_active_turn(
                            bound,
                            state,
                            ResponsesWebSocketTurnOutcome::upstream_send_failed(),
                        ).await;
                        if !claim_public_teardown(bound) {
                            break;
                        }
                        send_gateway_error_with_status(
                            client_socket,
                            &bound.public_event_sequence,
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
            backend_input = bound.backend_session.receive() => {
                let provider_event = match backend_input {
                    ResponsesBackendInput::Event(event) => event,
                    ResponsesBackendInput::Closed => {
                        // Close codes/reasons belong to the provider transport
                        // and may contain account identifiers. EOF and an
                        // explicit provider Close have the same public meaning.
                        let response_in_flight = bound.turn_state.response_in_flight();
                        match upstream_close_action(
                            bound.pending_provider_drain,
                            response_in_flight,
                        ) {
                            UpstreamCloseAction::DetachIdleUpstream => {
                                // The preceding turn already emitted its public
                                // terminal. A provider Close/EOF now only ends the
                                // physical binding; a later independent turn can
                                // plan and bind another upstream on this client WS.
                                // Preserve any exhausted-key exclusion discovered
                                // while draining, but do not emit a second public
                                // terminal for the completed turn.
                                detach_idle_upstream_after_binding_loss(
                                    bound,
                                    &context.trace_id,
                                )
                                .await;
                                continue;
                            }
                            UpstreamCloseAction::ForwardQuotaAndDetach => {
                                let directive = bound
                                    .pending_provider_drain
                                    .expect("provider drain state should be present");
                                fail_active_turn(
                                    bound,
                                    state,
                                    ResponsesWebSocketTurnOutcome::provider_quota_exhausted(),
                                )
                                .await;
                                send_gateway_error_with_status(
                                    client_socket,
                                    &bound.public_event_sequence,
                                    429,
                                    directive.error_code,
                                    "Provider connection closed after reporting exhausted quota; send a new response.create to select another Provider connection",
                                )
                                .await;
                                detach_exhausted_upstream(bound, directive, &context.trace_id)
                                    .await;
                                continue;
                            }
                            UpstreamCloseAction::FailActiveTurn => {}
                        }
                        if claim_public_teardown(bound) {
                            terminate_after_upstream_closed(client_socket, bound, state).await;
                        }
                        break;
                    }
                    ResponsesBackendInput::Failed(ResponsesBackendFailure::Receive) => {
                        if matches!(
                            physical_binding_loss_action(
                                bound.turn_state.response_in_flight()
                            ),
                            PhysicalBindingLossAction::DetachIdleUpstream
                        ) {
                            debug!(
                                event_name = "responses_websocket_idle_upstream_receive_failed",
                                log_type = "event",
                                transport = WEBSOCKET_LOG_TRANSPORT,
                                websocket = true,
                                trace_id = %context.trace_id,
                                "gateway detached an idle Responses WebSocket upstream after a receive failure"
                            );
                            detach_idle_upstream_after_binding_loss(bound, &context.trace_id).await;
                            continue;
                        }
                        warn!(
                            event_name = "responses_websocket_upstream_receive_failed",
                            log_type = "ops",
                            transport = WEBSOCKET_LOG_TRANSPORT,
                            websocket = true,
                            trace_id = %context.trace_id,
                            "Upstream WebSocket receive failed"
                        );
                        fail_active_turn(
                            bound,
                            state,
                            ResponsesWebSocketTurnOutcome::upstream_receive_failed(),
                        ).await;
                        if !claim_public_teardown(bound) {
                            break;
                        }
                        send_gateway_error_with_status(
                            client_socket,
                            &bound.public_event_sequence,
                            502,
                            "responses_websocket_receive_failed",
                            "Provider connection closed unexpectedly",
                        ).await;
                        bound.backend_session.detach();
                        close_client_socket(client_socket, CLOSE_INTERNAL_ERROR, "upstream_receive_failed").await;
                        break;
                    }
                    ResponsesBackendInput::Failed(ResponsesBackendFailure::ControlWrite) => {
                        match provider_control_write_action(
                            false,
                            bound.turn_state.response_in_flight(),
                        ) {
                            ProviderControlWriteAction::Continue => continue,
                            ProviderControlWriteAction::DetachIdleUpstream => {
                                debug!(
                                    event_name = "responses_websocket_idle_provider_pong_failed",
                                    log_type = "event",
                                    transport = WEBSOCKET_LOG_TRANSPORT,
                                    websocket = true,
                                    trace_id = %context.trace_id,
                                    "gateway detached an idle Responses WebSocket upstream after its Pong write failed"
                                );
                                detach_idle_upstream_after_binding_loss(
                                    bound,
                                    &context.trace_id,
                                )
                                .await;
                                continue;
                            }
                            ProviderControlWriteAction::FailActiveTurn => {}
                        }
                        warn!(
                            event_name = "responses_websocket_provider_pong_failed",
                            log_type = "ops",
                            transport = WEBSOCKET_LOG_TRANSPORT,
                            websocket = true,
                            trace_id = %context.trace_id,
                            "gateway could not answer a provider WebSocket ping"
                        );
                        fail_active_turn(
                            bound,
                            state,
                            ResponsesWebSocketTurnOutcome::upstream_send_failed(),
                        )
                        .await;
                        if !claim_public_teardown(bound) {
                            break;
                        }
                        let policy = fatal_relay_policy(FatalRelaySignal::UpstreamClosed);
                        send_responses_websocket_error(
                            client_socket,
                            &bound.public_event_sequence,
                            policy.status_code,
                            "server_error",
                            policy.error_code,
                            policy.client_message,
                            None,
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
                    ResponsesBackendInput::ProtocolViolation(
                        ResponsesBackendProtocolViolation::BinaryFrame,
                    ) => {
                        // Binary is outside the public Responses protocol. Drop
                        // the payload at this boundary and report only a stable
                        // gateway error; provider bytes must never reach public
                        // logging, accounting captures, or the client socket.
                        mark_active_response_retry_unsafe(bound, "upstream_binary_frame");
                        let policy = fatal_relay_policy(FatalRelaySignal::InvalidUpstreamText);
                        warn!(
                            event_name = "responses_websocket_provider_binary_frame",
                            log_type = "ops",
                            transport = WEBSOCKET_LOG_TRANSPORT,
                            websocket = true,
                            trace_id = %context.trace_id,
                            "provider returned a binary frame on a Responses WebSocket"
                        );
                        fail_active_turn(
                            bound,
                            state,
                            ResponsesWebSocketTurnOutcome::provider_protocol_error(),
                        )
                        .await;
                        if !claim_public_teardown(bound) {
                            break;
                        }
                        send_responses_websocket_error(
                            client_socket,
                            &bound.public_event_sequence,
                            policy.status_code,
                            "server_error",
                            policy.error_code,
                            policy.client_message,
                            None,
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
                    ResponsesBackendInput::ProtocolViolation(
                        ResponsesBackendProtocolViolation::InvalidEventText(text),
                    ) => {
                        mark_active_response_retry_unsafe(bound, "invalid_upstream_event");
                        let observation = bound
                            .turn_state
                            .attempt_mut()
                            .and_then(|turn| turn.observe_invalid_upstream_text(text.as_str()));
                        if matches!(
                            observation,
                            Some(ResponsesWebSocketTurnObservation::Started)
                                | Some(ResponsesWebSocketTurnObservation::Terminal(_))
                        ) {
                            if let Some(turn) = bound.turn_state.attempt_mut() {
                                turn.mark_stream_started(state).await;
                            }
                        }
                        let policy = fatal_relay_policy(FatalRelaySignal::InvalidUpstreamText);
                        fail_active_turn(
                            bound,
                            state,
                            match observation {
                                Some(ResponsesWebSocketTurnObservation::Terminal(outcome)) => {
                                    outcome
                                }
                                _ => ResponsesWebSocketTurnOutcome::upstream_receive_failed(),
                            },
                        )
                        .await;
                        if !claim_public_teardown(bound) {
                            break;
                        }
                        send_responses_websocket_error(
                            client_socket,
                            &bound.public_event_sequence,
                            policy.status_code,
                            "server_error",
                            policy.error_code,
                            policy.client_message,
                            None,
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
                };
                let (provider_text, provider_event) = provider_event.into_parts();
                let parsed_upstream_frame = ParsedResponsesWebSocketFrame::from_event(
                    provider_text.as_str(),
                    provider_event,
                );
                let parsed_upstream_event = parsed_upstream_frame.event();
                let mut decoded_public_events = Some(
                    bound.public_codec.public_events(parsed_upstream_frame.event()),
                );
                let mut private_provider_error_status = None;
                {
                    debug!(
                        event_name = "responses_websocket_upstream_event",
                        log_type = "event",
                        transport = WEBSOCKET_LOG_TRANSPORT,
                        websocket = true,
                        trace_id = %context.trace_id,
                        event_type = %parsed_upstream_frame.event_type_for_log(),
                        frame_bytes = provider_text.len(),
                        chunked = parsed_upstream_frame.is_chunked(),
                        active_turn = bound.turn_state.response_in_flight(),
                        "gateway received Responses WebSocket event"
                    );
                }
                {
                    // Public response events only become replay-visible after
                    // their corresponding client write succeeds. Unknown
                    // provider-private events remain fail-closed immediately.
                    if let super::adapter::ResponsesWebSocketRebindSafety::Unsafe { reason } =
                        provider_frame_rebind_safety(&parsed_upstream_frame, bound.provider_observer)
                    {
                        mark_active_response_retry_unsafe(bound, reason);
                    }
                    if bound.provider_observer.observes_upstream_events() {
                        let provider_observer = bound.provider_observer;
                        if let Some(observation) = provider_observer.observe_upstream_event(parsed_upstream_event) {
                            let directive = observation.drain;
                            let private_error = observation.private_error;
                            await_pending_provider_observation(bound).await;
                            let state_for_observation = state.clone();
                            let trace_id = context.trace_id.clone();
                            let report_context = bound.decision_template.report_context.clone();
                            bound.pending_provider_observation = Some(tokio::spawn(async move {
                                provider_observer
                                    .persist_upstream_observation(
                                        &state_for_observation,
                                        &trace_id,
                                        report_context.as_ref(),
                                        observation,
                                    )
                                    .await;
                            }));
                            if let Some(directive) = directive {
                                if bound.pending_provider_drain.is_none() {
                                    bound.pending_provider_drain = Some(directive);
                                }
                                // A definitive quota signal must be visible to
                                // the next planner before a transparent retry.
                                await_pending_provider_observation(bound).await;
                            }
                            if let Some(private_error) = private_error {
                                private_provider_error_status = Some(private_error.status_code);
                                let events = vec![private_error.public_event()];
                                decoded_public_events = Some(Ok(events));
                            }
                        }
                    }
                }
                decoded_public_events = match decoded_public_events {
                    Some(Ok(mut events)) => {
                        let client_event = bound
                            .turn_state
                            .logical()
                            .map(|logical| &logical.client_event);
                        let sanitized = bound
                            .provider_observer
                            .sanitize_public_events(client_event, &mut events)
                            .and_then(|()| bound.public_event_state.accept_events(&events));
                        Some(sanitized.map(|()| events))
                    }
                    decoded => decoded,
                };
                let public_protocol_error = decoded_public_events
                    .as_ref()
                    .and_then(|result| result.as_ref().err().copied());
                let provider_observer = bound.provider_observer;
                let observation = match public_protocol_error {
                    None => bound.turn_state.attempt_mut().and_then(|turn| {
                        match private_provider_error_status {
                            Some(status_code) => turn.observe_private_provider_error(
                                &parsed_upstream_frame,
                                provider_observer,
                                status_code,
                            ),
                            None => turn.observe_upstream_frame(
                                &parsed_upstream_frame,
                                provider_observer,
                            ),
                        }
                    }),
                    Some(_) => {
                        if let Some(turn) = bound.turn_state.attempt_mut() {
                            turn.observe_invalid_upstream_protocol(
                                provider_text.as_str(),
                                "provider returned an invalid Responses WebSocket event sequence",
                            )
                        } else {
                            None
                        }
                    }
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
                let public_events = decoded_public_events
                    .as_ref()
                    .and_then(|result| result.as_ref().ok())
                    .map(Vec::as_slice)
                    .unwrap_or_default();
                if !public_events.is_empty() && !bound.turn_state.response_in_flight() {
                    let policy = fatal_relay_policy(FatalRelaySignal::InvalidUpstreamText);
                    warn!(
                        event_name = "responses_websocket_provider_event_while_idle",
                        log_type = "ops",
                        transport = WEBSOCKET_LOG_TRANSPORT,
                        websocket = true,
                        trace_id = %context.trace_id,
                        public_event_count = public_events.len(),
                        "provider emitted a public Responses event without an active turn"
                    );
                    if !claim_public_teardown(bound) {
                        break;
                    }
                    send_responses_websocket_error(
                        client_socket,
                        &bound.public_event_sequence,
                        policy.status_code,
                        "server_error",
                        policy.error_code,
                        policy.client_message,
                        None,
                    )
                    .await;
                    close_bound_upstream(bound).await;
                    close_client_socket(client_socket, policy.close_code, policy.close_reason).await;
                    break;
                }
                if public_protocol_error.is_some() {
                    let policy = fatal_relay_policy(FatalRelaySignal::InvalidUpstreamText);
                    warn!(
                        event_name = "responses_websocket_provider_protocol_error",
                        log_type = "ops",
                        transport = WEBSOCKET_LOG_TRANSPORT,
                        websocket = true,
                        trace_id = %context.trace_id,
                        protocol_error = public_protocol_error
                            .map(|error| error.as_str())
                            .unwrap_or("invalid_public_batch"),
                        "provider returned an invalid public Responses WebSocket batch"
                    );
                    fail_active_turn(
                        bound,
                        state,
                        terminal_outcome
                            .unwrap_or_else(ResponsesWebSocketTurnOutcome::provider_protocol_error),
                    )
                    .await;
                    if !claim_public_teardown(bound) {
                        break;
                    }
                    send_responses_websocket_error(
                        client_socket,
                        &bound.public_event_sequence,
                        policy.status_code,
                        "server_error",
                        policy.error_code,
                        policy.client_message,
                        None,
                    )
                    .await;
                    close_bound_upstream(bound).await;
                    close_client_socket(client_socket, policy.close_code, policy.close_reason).await;
                    break;
                }
                let drain_for_provider = provider_drain_ready(
                    bound.pending_provider_drain,
                    bound.turn_state.response_in_flight(),
                    observation,
                    false,
                );
                let quota_facts = QuotaRelayFacts {
                    drain_ready: drain_for_provider,
                    retry_current_turn: bound
                        .pending_provider_drain
                        .is_some_and(|directive| directive.retry_current_turn),
                    transparent_retry_failed: false,
                    usage_limit_error: is_usage_limit_error_event(parsed_upstream_event),
                    continuation_retry_eligible: active_continuation_can_retry_from_full_input(bound),
                    upstream_closed: false,
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
                        Some(turn) => settle_turn_finalization(
                            bound,
                            state,
                            turn,
                            terminal_outcome
                                .unwrap_or_else(ResponsesWebSocketTurnOutcome::upstream_closed),
                        )
                        .await,
                        None => PreviousAttemptSettled::nothing_to_settle(),
                    };
                    match retry_active_turn_after_quota_exhaustion(bound, state, context, settled)
                        .await
                    {
                        QuotaRetryDisposition::Retried => continue,
                        QuotaRetryDisposition::ExecutionReservationLost => {
                            warn!(
                                event_name = "responses_websocket_execution_reservation_lost",
                                log_type = "ops",
                                transport = WEBSOCKET_LOG_TRANSPORT,
                                websocket = true,
                                trace_id = %context.trace_id,
                                "Responses WebSocket execution reservation was lost before quota retry send"
                            );
                            terminate_after_execution_reservation_lost(
                                client_socket,
                                bound,
                                state,
                            )
                            .await;
                            break;
                        }
                        QuotaRetryDisposition::Failed => {}
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
                        .pending_provider_drain
                        .expect("provider drain state should be present");
                    debug!(
                        event_name = "responses_websocket_continuation_retry_required",
                        log_type = "event",
                        transport = WEBSOCKET_LOG_TRANSPORT,
                        websocket = true,
                        trace_id = %context.trace_id,
                        error_code = "previous_response_not_found",
                        "gateway will ask the client to retry the continuation with complete input"
                    );
                    evict_active_continuation(bound);
                    if let Some(turn) = bound.turn_state.end().map(ActiveProviderAttempt::disarm) {
                        queue_turn_finalization(
                            bound,
                            state,
                            turn,
                            terminal_outcome.unwrap_or_else(
                                ResponsesWebSocketTurnOutcome::upstream_closed,
                            ),
                        );
                    }
                    send_previous_response_not_found(
                        client_socket,
                        &bound.public_event_sequence,
                    )
                    .await;
                    detach_exhausted_upstream(bound, directive, &context.trace_id).await;
                    continue;
                }
                if matches!(quota_relay_action, QuotaRelayAction::ForwardQuotaAndDetach) {
                    let directive = bound
                        .pending_provider_drain
                        .expect("provider drain state should be present");
                    fail_active_turn(
                        bound,
                        state,
                        terminal_outcome
                            .unwrap_or_else(ResponsesWebSocketTurnOutcome::provider_quota_exhausted),
                    )
                    .await;
                    send_gateway_error_with_status(
                        client_socket,
                        &bound.public_event_sequence,
                        429,
                        directive.error_code,
                        "Provider connection closed after reporting exhausted quota; send a new response.create to select another Provider connection",
                    )
                    .await;
                    detach_exhausted_upstream(bound, directive, &context.trace_id).await;
                    continue;
                }
                // Provider observation and turn accounting above consume the complete raw
                // frame. The public codec is deliberately the last boundary: it removes
                // provider-only events and unwraps private batch envelopes into ordered,
                // standard Responses events. Build owned payloads before awaiting socket IO
                // so no borrow of the connection spans a send.
                let public_events = decoded_public_events
                    .unwrap_or_else(|| Ok(Vec::new()))
                    .expect("invalid public batches are rejected before relay");
                let turn_deadline = bound
                    .turn_state
                    .attempt()
                    .map(|turn| TokioInstant::from_std(turn.deadline().deadline));
                let batch_deadline = TokioInstant::now()
                    .checked_add(RELAY_WRITE_TIMEOUT)
                    .unwrap_or(connection_deadline)
                    .min(connection_deadline);
                let batch_deadline = turn_deadline
                    .map(|turn_deadline| batch_deadline.min(turn_deadline))
                    .unwrap_or(batch_deadline);
                let mut delivery_failed = None;
                let mut invalid_public_event = false;
                let mut continuation_cache_update = None;
                for mut public_event in public_events {
                    // Reserve and serialize exactly one event at a time. A
                    // batch must not consume numbers for frames that have not
                    // reached the client yet.
                    let Ok(reservation) = bound.public_event_sequence.stamp(&mut public_event)
                    else {
                        invalid_public_event = true;
                        break;
                    };
                    let Some(client_text) = bound
                        .redaction_restorer
                        .restore_provider_frame_text(&public_event)
                        .or_else(|| serde_json::to_string(&public_event).ok())
                    else {
                        invalid_public_event = true;
                        break;
                    };
                    let delivery = feed_client_message_until(
                        client_socket,
                        AxumWsMessage::Text(client_text.into()),
                        batch_deadline,
                    )
                    .await;
                    if let Err(error) = commit_public_event_delivery(
                        delivery,
                        reservation,
                    ) {
                        delivery_failed = Some(error);
                        break;
                    }
                    if let Some(update) = continuation_cache_update_for_event(&public_event) {
                        continuation_cache_update = Some(update);
                    }
                    observe_active_response_rebind_safety(bound, &public_event);
                    if let Some(turn) = bound.turn_state.attempt_mut() {
                        // Capture exactly what the public protocol exposed, still in its
                        // redacted form. Restoration only affects the wire copy above.
                        turn.capture_client_frame(&public_event);
                    }
                }
                if delivery_failed.is_none() && !invalid_public_event {
                    if let Err(error) =
                        flush_client_messages_until(client_socket, batch_deadline).await
                    {
                        delivery_failed = Some(error);
                    }
                }
                if invalid_public_event {
                    let policy = fatal_relay_policy(FatalRelaySignal::InvalidUpstreamText);
                    fail_active_turn(
                        bound,
                        state,
                        terminal_outcome
                            .unwrap_or_else(ResponsesWebSocketTurnOutcome::upstream_receive_failed),
                    )
                    .await;
                    if !claim_public_teardown(bound) {
                        break;
                    }
                    send_responses_websocket_error(
                        client_socket,
                        &bound.public_event_sequence,
                        policy.status_code,
                        "server_error",
                        policy.error_code,
                        policy.client_message,
                        None,
                    )
                    .await;
                    close_bound_upstream(bound).await;
                    close_client_socket(client_socket, policy.close_code, policy.close_reason).await;
                    break;
                }
                if let Some(error) = delivery_failed {
                    let _ = claim_public_teardown(bound);
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
                    fail_active_turn(
                        bound,
                        state,
                        settle_signal_for_client_delivery_failure(terminal_outcome),
                    )
                    .await;
                    close_bound_upstream(bound).await;
                    break;
                }

                // A WebSocket sink write is not the delivery boundary until
                // the whole provider batch has flushed.  Commit/evict the
                // connection-local continuation cache only after that point.
                apply_continuation_cache_update(bound, continuation_cache_update);

                if let Some(outcome) = terminal_outcome {
                    finalize_active_turn(bound, state, outcome).await;
                }
                if drain_for_provider {
                    let directive = bound
                        .pending_provider_drain
                        .expect("provider drain state should be present");
                    detach_exhausted_upstream(bound, directive, &context.trace_id).await;
                    continue;
                }
            }
        }
    }
}

fn claim_public_teardown(bound: &BoundResponsesConnection) -> bool {
    bound.public_teardown.try_claim()
}

async fn terminate_after_execution_reservation_lost(
    client_socket: &mut WebSocket,
    bound: &mut BoundResponsesConnection,
    state: &AppState,
) {
    let policy = fatal_relay_policy(FatalRelaySignal::ExecutionReservationLost);
    fail_active_turn(
        bound,
        state,
        ResponsesWebSocketTurnOutcome::execution_reservation_lost(),
    )
    .await;
    if !claim_public_teardown(bound) {
        return;
    }
    send_responses_websocket_error(
        client_socket,
        &bound.public_event_sequence,
        policy.status_code,
        "server_error",
        policy.error_code,
        policy.client_message,
        None,
    )
    .await;
    close_bound_upstream(bound).await;
    close_client_socket(client_socket, policy.close_code, policy.close_reason).await;
}

async fn terminate_after_upstream_closed(
    client_socket: &mut WebSocket,
    bound: &mut BoundResponsesConnection,
    state: &AppState,
) {
    let policy = fatal_relay_policy(FatalRelaySignal::UpstreamClosed);
    fail_active_turn(
        bound,
        state,
        ResponsesWebSocketTurnOutcome::upstream_closed(),
    )
    .await;
    bound.backend_session.detach();
    send_responses_websocket_error(
        client_socket,
        &bound.public_event_sequence,
        policy.status_code,
        "server_error",
        policy.error_code,
        policy.client_message,
        None,
    )
    .await;
    close_client_socket(client_socket, policy.close_code, policy.close_reason).await;
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::super::adapter::ResponsesWebSocketDrainDirective;
    use super::{
        classify_upstream_closed_quota_relay, commit_public_event_delivery,
        continuation_cache_update_for_event, physical_binding_loss_action,
        provider_control_write_action, upstream_close_action, ContinuationCacheUpdate,
        PhysicalBindingLossAction, ProviderControlWriteAction, QuotaRelayAction,
        UpstreamCloseAction,
    };

    fn exhausted_provider_drain() -> ResponsesWebSocketDrainDirective {
        ResponsesWebSocketDrainDirective {
            error_code: "provider_quota_exhausted",
            retry_current_turn: true,
            retry_exclusion_until_unix_secs: Some(1_800_000_000),
        }
    }

    #[test]
    fn sink_acceptance_only_commits_the_public_sequence() {
        let sequence = super::super::state::ResponsesPublicEventSequence::default();
        let mut event = json!({
            "type": "response.created",
            "response": {"id": "resp_current"}
        });

        let failed_reservation = sequence.stamp(&mut event).expect("event should stamp");
        assert_eq!(
            commit_public_event_delivery(Err("sink rejected event"), failed_reservation),
            Err("sink rejected event")
        );
        assert_eq!(sequence.reserve().sequence_number(), 0);

        let delivered_reservation = sequence.stamp(&mut event).expect("event should stamp");
        commit_public_event_delivery(Ok::<(), &str>(()), delivered_reservation)
            .expect("accepted event should commit");
        assert_eq!(sequence.reserve().sequence_number(), 1);
    }

    #[test]
    fn only_chainable_terminals_propose_a_committed_response_id() {
        let completed = json!({
            "type": "response.completed",
            "response": {"id": "resp_current"}
        });
        let incomplete = json!({
            "type": "response.incomplete",
            "response": {"id": "resp_limited"}
        });
        assert_eq!(
            continuation_cache_update_for_event(&completed),
            Some(ContinuationCacheUpdate::Commit("resp_current".to_string()))
        );
        assert_eq!(
            continuation_cache_update_for_event(&incomplete),
            Some(ContinuationCacheUpdate::Commit("resp_limited".to_string()))
        );
        assert_eq!(
            continuation_cache_update_for_event(&json!({
                "type": "response.created",
                "response": {"id": "resp_pending"}
            })),
            None
        );
    }

    #[test]
    fn failed_terminals_propose_referenced_id_eviction() {
        for event_type in ["response.failed", "response.cancelled", "error"] {
            assert_eq!(
                continuation_cache_update_for_event(&json!({"type": event_type})),
                Some(ContinuationCacheUpdate::EvictReferenced)
            );
        }
    }

    #[test]
    fn backend_close_has_one_transport_neutral_relay_classification() {
        assert_eq!(
            upstream_close_action(None, true),
            UpstreamCloseAction::FailActiveTurn
        );
    }

    #[test]
    fn provider_close_payload_is_not_part_of_the_public_fsm() {
        assert_eq!(
            upstream_close_action(None, false),
            UpstreamCloseAction::DetachIdleUpstream
        );
    }

    #[test]
    fn exhausted_metadata_then_provider_close_uses_quota_detach() {
        assert_eq!(
            classify_upstream_closed_quota_relay(Some(exhausted_provider_drain()), true),
            QuotaRelayAction::ForwardQuotaAndDetach
        );
    }

    #[test]
    fn exhausted_metadata_then_provider_eof_uses_quota_detach() {
        assert_eq!(
            classify_upstream_closed_quota_relay(Some(exhausted_provider_drain()), true),
            QuotaRelayAction::ForwardQuotaAndDetach
        );
    }

    #[test]
    fn ordinary_provider_close_remains_a_fatal_upstream_close() {
        assert_eq!(
            classify_upstream_closed_quota_relay(None, true),
            QuotaRelayAction::None
        );
    }

    #[test]
    fn provider_close_after_a_completed_turn_only_detaches_the_idle_upstream() {
        assert_eq!(
            upstream_close_action(None, false),
            UpstreamCloseAction::DetachIdleUpstream
        );
        assert_eq!(
            upstream_close_action(Some(exhausted_provider_drain()), false),
            UpstreamCloseAction::DetachIdleUpstream
        );
        assert_eq!(
            upstream_close_action(Some(exhausted_provider_drain()), true),
            UpstreamCloseAction::ForwardQuotaAndDetach
        );
        assert_eq!(
            upstream_close_action(None, true),
            UpstreamCloseAction::FailActiveTurn
        );
    }

    #[test]
    fn provider_receive_error_after_a_completed_turn_only_detaches_the_idle_upstream() {
        assert_eq!(
            physical_binding_loss_action(false),
            PhysicalBindingLossAction::DetachIdleUpstream
        );
        assert_eq!(
            physical_binding_loss_action(true),
            PhysicalBindingLossAction::FailActiveTurn
        );
    }

    #[test]
    fn provider_control_write_failure_after_a_completed_turn_only_detaches_the_idle_upstream() {
        assert_eq!(
            provider_control_write_action(false, false),
            ProviderControlWriteAction::DetachIdleUpstream
        );
        assert_eq!(
            provider_control_write_action(false, true),
            ProviderControlWriteAction::FailActiveTurn
        );
        assert_eq!(
            provider_control_write_action(true, false),
            ProviderControlWriteAction::Continue
        );
    }

    #[test]
    fn provider_binary_is_a_public_502_and_1011_protocol_failure() {
        let policy = super::fatal_relay_policy(super::FatalRelaySignal::InvalidUpstreamText);
        assert_eq!(policy.status_code, 502);
        assert_eq!(policy.close_code, 1011);
        assert!(!policy.client_message.contains("private-provider-binary"));
    }
}
