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

/// Owns the in-flight turn so that losing the relay task still finalizes it.
///
/// Every ordinary exit path takes the turn out of here and finalizes it
/// explicitly. This guard only covers the paths that are not exit paths at all
/// — a panic in the relay loop, or the task being dropped — where the turn
/// would otherwise be discarded with its usage row left `Pending`, its
/// candidate row left `Streaming`, and its distributed pool key lease leaked
/// until the lease expires. Mirrors the HTTP path's `DirectPassthroughFinalizer`.
pub(super) struct ActiveResponsesWebSocketTurn {
    turn: Option<ResponsesWebSocketTurn>,
    state: AppState,
}

impl ActiveResponsesWebSocketTurn {
    pub(super) fn new(state: &AppState, turn: ResponsesWebSocketTurn) -> Self {
        Self {
            turn: Some(turn),
            state: state.clone(),
        }
    }

    /// Hands the turn back to a caller that will finalize it explicitly.
    pub(super) fn disarm(mut self) -> ResponsesWebSocketTurn {
        self.turn
            .take()
            .expect("an armed active turn always holds its turn")
    }
}

impl std::ops::Deref for ActiveResponsesWebSocketTurn {
    type Target = ResponsesWebSocketTurn;

    fn deref(&self) -> &Self::Target {
        self.turn
            .as_ref()
            .expect("an armed active turn always holds its turn")
    }
}

impl std::ops::DerefMut for ActiveResponsesWebSocketTurn {
    fn deref_mut(&mut self) -> &mut Self::Target {
        self.turn
            .as_mut()
            .expect("an armed active turn always holds its turn")
    }
}

impl Drop for ActiveResponsesWebSocketTurn {
    fn drop(&mut self) {
        let Some(turn) = self.turn.take() else {
            return;
        };
        let state = self.state.clone();
        // No runtime means the process is going down; the spawn could not
        // complete anyway.
        if let Ok(handle) = tokio::runtime::Handle::try_current() {
            warn!(
                event_name = "responses_websocket_turn_abandoned",
                log_type = "ops",
                transport = WEBSOCKET_LOG_TRANSPORT,
                websocket = true,
                "gateway finalized a Responses WebSocket turn whose relay task went away"
            );
            handle.spawn(async move {
                turn.finalize_detached(
                    &state,
                    ResponsesWebSocketTurnOutcome::relay_task_abandoned(),
                )
                .await;
            });
        }
    }
}

/// 结束当前 logical turn 并结算它的 attempt。
///
/// `end()` 同时清掉 logical turn 和 attempt，取代原来「take active_turn +
/// 在每个出口手写 `active_response_create = None`」的两步组合。
pub(super) async fn finalize_active_turn(
    bound: &mut BoundResponsesConnection,
    state: &AppState,
    outcome: ResponsesWebSocketTurnOutcome,
) {
    if let Some(turn) = bound.turn_state.end() {
        queue_turn_finalization(bound, state, turn.disarm(), outcome).await;
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
