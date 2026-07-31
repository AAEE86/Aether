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
    spawn_responses_websocket_turn_finalization, ResponsesProviderAttempt,
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
pub(super) struct ActiveProviderAttempt {
    turn: Option<ResponsesProviderAttempt>,
    state: AppState,
}

impl ActiveProviderAttempt {
    pub(super) fn new(state: &AppState, turn: ResponsesProviderAttempt) -> Self {
        Self {
            turn: Some(turn),
            state: state.clone(),
        }
    }

    /// Hands the turn back to a caller that will finalize it explicitly.
    pub(super) fn disarm(mut self) -> ResponsesProviderAttempt {
        self.turn
            .take()
            .expect("an armed active turn always holds its turn")
    }
}

impl std::ops::Deref for ActiveProviderAttempt {
    type Target = ResponsesProviderAttempt;

    fn deref(&self) -> &Self::Target {
        self.turn
            .as_ref()
            .expect("an armed active turn always holds its turn")
    }
}

impl std::ops::DerefMut for ActiveProviderAttempt {
    fn deref_mut(&mut self) -> &mut Self::Target {
        self.turn
            .as_mut()
            .expect("an armed active turn always holds its turn")
    }
}

impl Drop for ActiveProviderAttempt {
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
    turn: ResponsesProviderAttempt,
    outcome: ResponsesWebSocketTurnOutcome,
) {
    await_pending_adapter_observation(bound).await;
    await_pending_turn_finalization(bound).await;
    bound.pending_turn_finalization =
        Some(spawn_responses_websocket_turn_finalization(state.clone(), turn, outcome).await);
}

/// 「上一个 attempt 已经结算完毕」的凭证。
///
/// 只能由本模块颁发，且只有在结算真正落地之后。规划下一个 attempt 的入口
/// ([`super::quota::retry_active_turn_after_quota_exhaustion`]) 要求这个参数，
/// 于是「先结算、再规划」成为签名的一部分，而不是一句注释——顺序写反连编译都
/// 过不了。
pub(super) struct PreviousAttemptSettled(());

impl PreviousAttemptSettled {
    /// 没有 attempt 要结算（连接此刻不在 `Responding`）。
    pub(super) const fn nothing_to_settle() -> Self {
        Self(())
    }
}

/// 结算一个 attempt 并等它落地。
///
/// 与 [`queue_turn_finalization`] 的区别只在于「等」：后者把 handle 挂在连接上
/// 让 relay loop 继续跑，适用于结算之后不再需要读取共享状态的出口；这个用在
/// 必须先看到结算结果才能继续的路径上——典型的就是透明重试，它紧接着要按
/// health / adaptive / pool 状态规划下一个 attempt。
pub(super) async fn settle_turn_finalization(
    bound: &mut BoundResponsesConnection,
    state: &AppState,
    turn: ResponsesProviderAttempt,
    outcome: ResponsesWebSocketTurnOutcome,
) -> PreviousAttemptSettled {
    queue_turn_finalization(bound, state, turn, outcome).await;
    await_pending_turn_finalization(bound).await;
    PreviousAttemptSettled(())
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
    turn: ResponsesProviderAttempt,
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

#[cfg(test)]
mod tests {
    use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
    use std::sync::Arc;
    use std::time::Duration;

    use super::await_turn_finalization_handle;

    /// C6 依赖的性质：结算是「等到落地」而不是「排进队列」。
    ///
    /// 透明重试在这之后立刻按 health / adaptive / pool 状态规划下一个 attempt，
    /// 所以结算任务必须已经跑完——只把 handle 挂起来是不够的。
    #[tokio::test]
    async fn awaiting_a_finalization_handle_runs_the_settlement_to_completion() {
        let settled = Arc::new(AtomicBool::new(false));
        let flag = Arc::clone(&settled);
        let handle = tokio::spawn(async move {
            tokio::time::sleep(Duration::from_millis(60)).await;
            flag.store(true, Ordering::SeqCst);
        });

        assert!(
            !settled.load(Ordering::SeqCst),
            "the settlement has not finished yet"
        );
        await_turn_finalization_handle(handle).await;
        assert!(
            settled.load(Ordering::SeqCst),
            "the settlement must be complete before the caller proceeds"
        );
    }

    /// 顺序型：结算的每一步都要排在规划之前。
    ///
    /// 用计数器替身重放透明重试的两步——旧 attempt 结算完成写入 1，规划开始时
    /// 读到的必须已经是 1。旧实现在这里先规划、再把结算排进队列，规划读到的是 0。
    #[tokio::test]
    async fn transparent_retry_replans_only_after_the_previous_attempt_is_settled() {
        let steps = Arc::new(AtomicUsize::new(0));

        // 第一步：结算旧 attempt（等到落地）。
        let recorder = Arc::clone(&steps);
        let settlement = tokio::spawn(async move {
            tokio::time::sleep(Duration::from_millis(40)).await;
            recorder.store(1, Ordering::SeqCst);
        });
        await_turn_finalization_handle(settlement).await;

        // 第二步：规划下一个 attempt，它读到的状态必须是结算之后的。
        let observed_at_planning = steps.load(Ordering::SeqCst);
        assert_eq!(
            observed_at_planning, 1,
            "planning must observe the state projected by the settled attempt"
        );
    }

    /// 结算任务失败（panic / cancel）也必须让调用方继续，不能把 relay loop 卡死。
    #[tokio::test]
    async fn a_failed_finalization_task_still_releases_the_caller() {
        let handle = tokio::spawn(async { panic!("settlement task exploded") });
        await_turn_finalization_handle(handle).await;
    }
}
