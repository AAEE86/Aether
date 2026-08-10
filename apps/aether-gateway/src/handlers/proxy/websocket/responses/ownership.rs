//! Cancellation-safe ownership handoff for planned leases and turn startup.

use serde_json::Value;
use tokio::task::JoinHandle;

use super::lifecycle::{ActiveProviderAttempt, ResponsesSessionTerminationSignal};
use super::turn::{begin_responses_websocket_turn, ResponsesProviderAttempt};
use crate::ai_serving::{
    maybe_build_responses_websocket_decision,
    maybe_build_responses_websocket_decision_with_auth_snapshot, AiExecutionDecision,
    ResponsesWebSocketDecision,
};
use crate::control::GatewayControlDecision;
use crate::orchestration::release_pool_key_lease_from_report_context;
use crate::{AppState, GatewayError};

/// Owns a pool-key lease between planner selection and turn-lifecycle startup.
///
/// Normal startup disarms the guard after the lease has moved into the turn's
/// report context. Any early return, panic, or cancelled task schedules an
/// idempotent release instead of leaving the key unavailable until its TTL.
pub(super) struct PlannedPoolKeyLeaseGuard {
    state: AppState,
    report_context: Option<Value>,
}

/// Planner output whose selected pool-key lease remains guarded until turn
/// startup takes ownership of the report context.
pub(super) struct OwnedResponsesWebSocketDecision {
    pub(super) planned: ResponsesWebSocketDecision,
    pub(super) lease: PlannedPoolKeyLeaseGuard,
    pub(super) planning_parts: http::request::Parts,
}

/// A startup task is part of the connection lifetime. If its waiter is
/// cancelled, abort the task so locally owned admissions and lease guards are
/// dropped immediately instead of continuing after the public socket closes.
pub(super) struct AbortOnDropJoinHandle<T> {
    handle: Option<JoinHandle<T>>,
}

impl<T> AbortOnDropJoinHandle<T> {
    fn new(handle: JoinHandle<T>) -> Self {
        Self {
            handle: Some(handle),
        }
    }

    async fn join(mut self) -> Result<T, tokio::task::JoinError> {
        let result = self
            .handle
            .as_mut()
            .expect("an owned startup handle is joined at most once")
            .await;
        self.handle = None;
        result
    }
}

impl<T> Drop for AbortOnDropJoinHandle<T> {
    fn drop(&mut self) {
        if let Some(handle) = self.handle.take() {
            handle.abort();
        }
    }
}

/// Runs planning in a detached owner task. Dropping the returned handle never
/// cancels a planner after it may have acquired a pool-key lease; an unobserved
/// successful result drops its guard and schedules release.
pub(super) fn spawn_owned_responses_websocket_plan(
    state: AppState,
    parts: http::request::Parts,
    trace_id: String,
    control_decision: GatewayControlDecision,
    client_event: Value,
    excluded_key_ids: Option<std::collections::BTreeSet<String>>,
    excluded_codex_account_ids: Option<std::collections::BTreeSet<String>>,
    auth_snapshot: Option<crate::data::auth::GatewayAuthApiKeySnapshot>,
) -> JoinHandle<Result<Option<OwnedResponsesWebSocketDecision>, GatewayError>> {
    tokio::spawn(async move {
        let runtime_miss_key =
            crate::ai_serving::runtime_miss_diagnostic_key_from_parts(&parts, &trace_id)
                .to_string();
        let _runtime_miss_cleanup =
            crate::ai_serving::RuntimeMissDiagnosticCleanupGuard::new(&state, runtime_miss_key);
        let planned = match auth_snapshot.as_ref() {
            Some(auth_snapshot) => {
                maybe_build_responses_websocket_decision_with_auth_snapshot(
                    &state,
                    &parts,
                    &trace_id,
                    &control_decision,
                    &client_event,
                    excluded_key_ids.as_ref(),
                    excluded_codex_account_ids.as_ref(),
                    auth_snapshot,
                )
                .await
            }
            None => {
                maybe_build_responses_websocket_decision(
                    &state,
                    &parts,
                    &trace_id,
                    &control_decision,
                    &client_event,
                    excluded_key_ids.as_ref(),
                    excluded_codex_account_ids.as_ref(),
                )
                .await
            }
        };
        let planned = planned?;
        Ok(planned.map(|planned| {
            let lease =
                PlannedPoolKeyLeaseGuard::new(&state, planned.execution.report_context.as_ref());
            OwnedResponsesWebSocketDecision {
                planned,
                lease,
                planning_parts: parts,
            }
        }))
    })
}

pub(super) async fn await_owned_responses_websocket_plan(
    handle: JoinHandle<Result<Option<OwnedResponsesWebSocketDecision>, GatewayError>>,
) -> Result<Option<OwnedResponsesWebSocketDecision>, GatewayError> {
    handle.await.map_err(|error| {
        GatewayError::Internal(format!("Responses WebSocket planning task failed: {error}"))
    })?
}

impl PlannedPoolKeyLeaseGuard {
    pub(super) fn new(state: &AppState, report_context: Option<&Value>) -> Self {
        Self {
            state: state.clone(),
            report_context: report_context.cloned(),
        }
    }

    pub(super) async fn release(mut self) {
        release_pool_key_lease_from_report_context(&self.state, self.report_context.as_ref()).await;
        self.report_context = None;
    }

    fn disarm(&mut self) {
        self.report_context = None;
    }
}

impl Drop for PlannedPoolKeyLeaseGuard {
    fn drop(&mut self) {
        let Some(report_context) = self.report_context.take() else {
            return;
        };
        let state = self.state.clone();
        if let Ok(handle) = tokio::runtime::Handle::try_current() {
            handle.spawn(async move {
                release_pool_key_lease_from_report_context(&state, Some(&report_context)).await;
            });
        }
    }
}

/// Starts a turn in a detached owner task.
///
/// Dropping the returned `JoinHandle` does not cancel startup. If the caller's
/// connection deadline wins while the unbounded pending usage/candidate writes
/// are running, the task continues. Its successful output already owns an
/// [`ActiveProviderAttempt`], so dropping an unobserved output triggers the
/// normal detached finalizer.
pub(super) fn spawn_owned_responses_websocket_turn(
    state: AppState,
    parts: http::request::Parts,
    control_decision: GatewayControlDecision,
    decision: AiExecutionDecision,
    client_event: Value,
    mut planned_lease: PlannedPoolKeyLeaseGuard,
    session_termination: ResponsesSessionTerminationSignal,
) -> AbortOnDropJoinHandle<Result<ActiveProviderAttempt, GatewayError>> {
    AbortOnDropJoinHandle::new(tokio::spawn(async move {
        let turn = begin_responses_websocket_turn(
            &state,
            &parts,
            &control_decision,
            decision,
            &client_event,
        )
        .await;
        // Keep the outer lease guard armed until the active attempt owns the
        // report context. Candidate repository failures can return before
        // `begin_responses_websocket_turn` reaches its explicit release paths;
        // dropping this guard is the cancellation- and error-safe fallback.
        let startup = turn?;
        let (turn, candidate_guard) = startup.into_parts();
        let turn = ActiveProviderAttempt::new(&state, turn, session_termination);
        // The active attempt is now the sole owner of candidate terminal state
        // and the planned pool-key lease. Both handoffs are synchronous, so
        // cancellation cannot land between ownership transfer and disarm.
        candidate_guard.disarm();
        planned_lease.disarm();
        // The cancellation guard owns the complete attempt before this await.
        // Aborting startup now runs its Drop finalizer and releases admission,
        // candidate state, usage state, and the pool-key lease.
        turn.initialize_pending_candidate(&state).await;
        Ok(turn)
    }))
}

pub(super) async fn await_owned_responses_websocket_turn(
    handle: AbortOnDropJoinHandle<Result<ActiveProviderAttempt, GatewayError>>,
) -> Result<ActiveProviderAttempt, GatewayError> {
    handle.join().await.map_err(|error| {
        GatewayError::Internal(format!(
            "Responses WebSocket turn startup task failed: {error}"
        ))
    })?
}

/// Converts an owned cancellation guard back to the raw attempt only at an
/// explicit finalization handoff.
pub(super) fn disarm_owned_turn(turn: ActiveProviderAttempt) -> ResponsesProviderAttempt {
    turn.disarm()
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::Arc;
    use std::time::Duration;

    struct DropProbe(Arc<AtomicUsize>);

    impl Drop for DropProbe {
        fn drop(&mut self) {
            self.0.fetch_add(1, Ordering::SeqCst);
        }
    }

    #[tokio::test]
    async fn dropped_startup_handle_aborts_the_task_and_drops_owned_resources() {
        let gate = Arc::new(tokio::sync::Notify::new());
        let task_gate = Arc::clone(&gate);
        let started = Arc::new(tokio::sync::Notify::new());
        let task_started = Arc::clone(&started);
        let dropped = Arc::new(AtomicUsize::new(0));
        let task_dropped = Arc::clone(&dropped);
        let handle = super::AbortOnDropJoinHandle::new(tokio::spawn(async move {
            let probe = DropProbe(task_dropped);
            task_started.notify_one();
            task_gate.notified().await;
            probe
        }));

        started.notified().await;
        drop(handle);

        tokio::time::timeout(Duration::from_secs(1), async {
            while dropped.load(Ordering::SeqCst) == 0 {
                tokio::task::yield_now().await;
            }
        })
        .await
        .expect("aborted startup should drop its owned resources");
        assert_eq!(dropped.load(Ordering::SeqCst), 1);
        // No notification is required: the task was cancelled while waiting.
        drop(gate);
    }

    #[tokio::test]
    async fn cancelling_a_join_waiter_aborts_the_owned_startup_task() {
        let gate = Arc::new(tokio::sync::Notify::new());
        let task_gate = Arc::clone(&gate);
        let started = Arc::new(tokio::sync::Notify::new());
        let task_started = Arc::clone(&started);
        let dropped = Arc::new(AtomicUsize::new(0));
        let task_dropped = Arc::clone(&dropped);
        let owner = super::AbortOnDropJoinHandle::new(tokio::spawn(async move {
            let probe = DropProbe(task_dropped);
            task_started.notify_one();
            task_gate.notified().await;
            probe
        }));
        let waiter = tokio::spawn(async move { owner.join().await });

        started.notified().await;
        waiter.abort();
        let _ = waiter.await;

        tokio::time::timeout(Duration::from_secs(1), async {
            while dropped.load(Ordering::SeqCst) == 0 {
                tokio::task::yield_now().await;
            }
        })
        .await
        .expect("cancelling the join waiter should abort the startup task");
        assert_eq!(dropped.load(Ordering::SeqCst), 1);
        drop(gate);
    }
}
