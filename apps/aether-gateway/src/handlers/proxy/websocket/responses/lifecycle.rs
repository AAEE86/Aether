//! Turn finalization and terminal error mapping for a Responses WebSocket.
//!
//! A connection can outlive a turn, so persistence and adapter observation
//! handles are joined in order before the next turn is planned.

use std::time::Duration;

use axum::extract::ws::WebSocket;
use tokio::task::JoinHandle;
use tokio::time::timeout;

use super::state::BoundResponsesConnection;
use super::turn::{
    spawn_responses_websocket_turn_finalization, ResponsesWebSocketTurn,
    ResponsesWebSocketTurnOutcome,
};
use crate::handlers::proxy::websocket::session::{
    CLOSE_INTERNAL_ERROR, CLOSE_POLICY_VIOLATION, CLOSE_TRY_AGAIN, WEBSOCKET_LOG_TRANSPORT,
};
use crate::handlers::proxy::websocket::transport::send_responses_websocket_error;
use crate::{AppState, GatewayError};

const RESPONSES_WEBSOCKET_ADAPTER_OBSERVATION_TIMEOUT: Duration = Duration::from_secs(5);
const LOG_TARGET: &str = "aether_gateway::handlers::proxy::responses_ws";

macro_rules! warn {
    ($($arg:tt)*) => {
        tracing::warn!(target: LOG_TARGET, $($arg)*)
    };
}

pub(super) async fn finalize_active_turn(
    bound: &mut BoundResponsesConnection,
    state: &AppState,
    outcome: ResponsesWebSocketTurnOutcome,
) {
    if let Some(turn) = bound.active_turn.take() {
        queue_turn_finalization(bound, state, turn, outcome).await;
    }
}

pub(super) async fn queue_turn_finalization(
    bound: &mut BoundResponsesConnection,
    state: &AppState,
    turn: ResponsesWebSocketTurn,
    outcome: ResponsesWebSocketTurnOutcome,
) {
    await_pending_adapter_observation(bound).await;
    await_pending_turn_finalization(bound).await;
    bound.pending_turn_finalization =
        Some(spawn_responses_websocket_turn_finalization(state.clone(), turn, outcome).await);
}

pub(super) async fn await_pending_adapter_observation(bound: &mut BoundResponsesConnection) {
    if let Some(mut handle) = bound.pending_adapter_observation.take() {
        match timeout(RESPONSES_WEBSOCKET_ADAPTER_OBSERVATION_TIMEOUT, &mut handle).await {
            Ok(Err(error)) => {
                warn!(
                    event_name = "responses_websocket_adapter_observation_join_failed",
                    log_type = "ops",
                    transport = WEBSOCKET_LOG_TRANSPORT,
                    websocket = true,
                    error = ?error,
                    "gateway Responses WebSocket adapter observation task failed"
                );
            }
            Ok(Ok(())) => {}
            Err(_) => {
                handle.abort();
                let _ = handle.await;
                warn!(
                    event_name = "responses_websocket_adapter_observation_timeout",
                    log_type = "ops",
                    transport = WEBSOCKET_LOG_TRANSPORT,
                    websocket = true,
                    timeout_ms = RESPONSES_WEBSOCKET_ADAPTER_OBSERVATION_TIMEOUT.as_millis() as u64,
                    "gateway stopped waiting for a Responses WebSocket adapter observation"
                );
            }
        }
    }
}

pub(super) async fn finalize_unbound_turn(
    state: AppState,
    turn: ResponsesWebSocketTurn,
    outcome: ResponsesWebSocketTurnOutcome,
) -> JoinHandle<()> {
    spawn_responses_websocket_turn_finalization(state, turn, outcome).await
}

pub(super) async fn await_turn_finalization_handle(handle: JoinHandle<()>) {
    // Do not abort terminal persistence here. Each I/O stage inside the turn
    // finalizer is independently bounded, and aborting the owner would skip
    // pool-lease cleanup and leave usage/candidate state non-terminal.
    match handle.await {
        Ok(()) => {}
        Err(error) => {
            warn!(
                event_name = "responses_websocket_turn_finalization_join_failed",
                log_type = "ops",
                transport = WEBSOCKET_LOG_TRANSPORT,
                websocket = true,
                error = ?error,
                "gateway Responses WebSocket turn finalizer task failed"
            );
        }
    }
}

pub(super) async fn await_pending_turn_finalization(bound: &mut BoundResponsesConnection) {
    if let Some(handle) = bound.pending_turn_finalization.take() {
        await_turn_finalization_handle(handle).await;
    }
}

pub(super) async fn send_responses_websocket_turn_start_error(
    client_socket: &mut WebSocket,
    error: &GatewayError,
) {
    match error {
        GatewayError::Client { status, message } => {
            let (error_type, code) = if status.as_u16() == 429 {
                ("rate_limit_error", "gateway_request_capacity_exceeded")
            } else {
                ("invalid_request_error", "gateway_request_not_allowed")
            };
            send_responses_websocket_error(
                client_socket,
                status.as_u16(),
                error_type,
                code,
                message,
            )
            .await;
        }
        GatewayError::AdmissionTimeout { .. } => {
            send_responses_websocket_error(
                client_socket,
                503,
                "server_error",
                "gateway_admission_timeout",
                "Gateway capacity is busy; retry this response",
            )
            .await;
        }
        GatewayError::LocalExecutionPlanningTimeout { .. } => {
            send_responses_websocket_error(
                client_socket,
                504,
                "server_error",
                "gateway_planning_timeout",
                "Gateway planning timed out; retry this response",
            )
            .await;
        }
        _ => {
            send_responses_websocket_error(
                client_socket,
                500,
                "server_error",
                "responses_websocket_turn_start_failed",
                "Gateway could not start this response",
            )
            .await;
        }
    }
}

pub(super) fn responses_websocket_turn_start_close(error: &GatewayError) -> (u16, &'static str) {
    match error {
        GatewayError::Client { .. } => (CLOSE_POLICY_VIOLATION, "request_not_allowed"),
        GatewayError::AdmissionTimeout { .. }
        | GatewayError::LocalExecutionPlanningTimeout { .. } => (CLOSE_TRY_AGAIN, "gateway_busy"),
        _ => (CLOSE_INTERNAL_ERROR, "turn_start_failed"),
    }
}
