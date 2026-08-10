use std::collections::BTreeMap;
use std::future::Future;
use std::sync::{
    atomic::{AtomicBool, AtomicU8, Ordering},
    Arc,
};

use aether_usage_runtime::{build_usage_event_data_seed, UsageEvent, UsageEventType};
use axum::body::Body;
use axum::http::Response;
use serde_json::{json, Value};

use crate::ai_serving::{build_core_error_body_for_client_format, LocalCoreSyncErrorKind};
use crate::api::response::{attach_control_metadata_headers, build_client_response_from_parts};
use crate::control::GatewayControlDecision;
use crate::request_diagnostics::attach_current_request_diagnostics_and_candidate_timing_to_report_context;
use crate::{AppState, GatewayError};

const TRANSPORT_ERROR_CLIENT_MESSAGE: &str =
    "Upstream transport failed before an HTTP response was received";

const STREAM_CANDIDATE_CANCELLATION_NONE: u8 = 0;
const STREAM_CANDIDATE_CANCELLATION_WATCHDOG_TIMEOUT: u8 = 1;

pub(crate) const STREAM_CANDIDATE_WATCHDOG_TIMEOUT_ERROR_TYPE: &str =
    "local_stream_candidate_watchdog_timeout";
pub(crate) const STREAM_CANDIDATE_WATCHDOG_TIMEOUT_MESSAGE: &str = "Stream first byte timeout";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum StreamCandidateCancellationCause {
    WatchdogTimeout,
}

#[derive(Debug, Default)]
pub(crate) struct StreamCandidateWatchdogProgress {
    attempt_guard_armed: AtomicBool,
    terminal_started: AtomicBool,
    terminal_owner_claimed: AtomicBool,
    terminal_completed: AtomicBool,
    cancellation_cause: AtomicU8,
    terminal_completion: tokio::sync::Notify,
}

tokio::task_local! {
    static STREAM_CANDIDATE_WATCHDOG_PROGRESS: Arc<StreamCandidateWatchdogProgress>;
}

impl StreamCandidateWatchdogProgress {
    pub(crate) fn shared() -> Arc<Self> {
        Arc::new(Self::default())
    }

    pub(crate) fn terminal_started(&self) -> bool {
        self.terminal_started.load(Ordering::Acquire)
    }

    pub(crate) fn mark_attempt_guard_armed(&self) {
        self.attempt_guard_armed.store(true, Ordering::Release);
    }

    pub(crate) fn attempt_guard_armed(&self) -> bool {
        self.attempt_guard_armed.load(Ordering::Acquire)
    }

    pub(crate) fn mark_watchdog_timeout(&self) {
        self.cancellation_cause.store(
            STREAM_CANDIDATE_CANCELLATION_WATCHDOG_TIMEOUT,
            Ordering::Release,
        );
    }

    pub(crate) fn cancellation_cause(&self) -> Option<StreamCandidateCancellationCause> {
        match self.cancellation_cause.load(Ordering::Acquire) {
            STREAM_CANDIDATE_CANCELLATION_WATCHDOG_TIMEOUT => {
                Some(StreamCandidateCancellationCause::WatchdogTimeout)
            }
            STREAM_CANDIDATE_CANCELLATION_NONE => None,
            _ => None,
        }
    }

    /// Claims the one cancellation finalizer for this attempt.
    ///
    /// Normal terminal paths mark `terminal_started` before their first
    /// cancellable persistence await. A dropped execution can only claim the
    /// terminal when no such owner exists, which prevents the watchdog and
    /// the attempt guard from writing competing terminal usage records.
    pub(crate) fn try_claim_terminal(&self) -> bool {
        if self
            .terminal_started
            .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
            .is_err()
        {
            return false;
        }
        self.terminal_owner_claimed.store(true, Ordering::Release);
        true
    }

    pub(crate) fn terminal_owner_claimed(&self) -> bool {
        self.terminal_owner_claimed.load(Ordering::Acquire)
    }

    pub(crate) fn mark_terminal_completed(&self) {
        self.terminal_completed.store(true, Ordering::Release);
        self.terminal_completion.notify_waiters();
    }

    pub(crate) async fn wait_for_terminal_completion(&self) {
        while !self.terminal_completed.load(Ordering::Acquire) {
            let notified = self.terminal_completion.notified();
            if self.terminal_completed.load(Ordering::Acquire) {
                return;
            }
            notified.await;
        }
    }

    pub(crate) async fn scope<F>(self: Arc<Self>, future: F) -> F::Output
    where
        F: Future,
    {
        STREAM_CANDIDATE_WATCHDOG_PROGRESS.scope(self, future).await
    }
}

pub(crate) fn current_stream_candidate_watchdog_progress(
) -> Option<Arc<StreamCandidateWatchdogProgress>> {
    STREAM_CANDIDATE_WATCHDOG_PROGRESS.try_with(Arc::clone).ok()
}

pub(crate) fn mark_stream_candidate_watchdog_terminal_started() {
    let _ = STREAM_CANDIDATE_WATCHDOG_PROGRESS.try_with(|progress| {
        progress.terminal_started.store(true, Ordering::Release);
    });
}

#[allow(clippy::too_many_arguments)]
pub(crate) async fn build_transport_error_stop_response(
    state: &AppState,
    plan: &aether_contracts::ExecutionPlan,
    report_context: Option<&Value>,
    trace_id: &str,
    decision: &GatewayControlDecision,
    client_status_code: u16,
    error_type: &str,
    error_message: &str,
    elapsed_ms: u64,
) -> Result<Response<Body>, GatewayError> {
    let client_body = transport_error_client_body(plan);

    if state.usage_runtime.is_enabled() {
        let report_context_with_diagnostics =
            attach_current_request_diagnostics_and_candidate_timing_to_report_context(
                report_context,
                Some(elapsed_ms),
                None,
            );
        let mut usage_data = build_usage_event_data_seed(
            plan,
            report_context_with_diagnostics.as_ref().or(report_context),
        );
        usage_data.status_code = Some(client_status_code);
        usage_data.error_message = Some(error_message.to_string());
        usage_data.error_category = Some("server_error".to_string());
        usage_data.response_time_ms = Some(elapsed_ms);
        usage_data.response_headers = None;
        usage_data.response_body = None;
        usage_data.client_response_headers = Some(json!({"content-type": "application/json"}));
        usage_data.client_response_body = Some(client_body);
        let mut request_metadata = match usage_data.request_metadata.take() {
            Some(Value::Object(object)) => object,
            Some(other) => serde_json::Map::from_iter([("seed".to_string(), other)]),
            None => serde_json::Map::new(),
        };
        request_metadata.insert("transport_error".to_string(), Value::Bool(true));
        request_metadata.insert(
            "transport_error_type".to_string(),
            Value::String(error_type.to_string()),
        );
        usage_data.request_metadata = Some(Value::Object(request_metadata));
        let state = state.clone();
        let request_id = plan.request_id.clone();
        let task = tokio::spawn(async move {
            state
                .usage_runtime
                .record_terminal_event_direct(
                    state.usage_lifecycle_data_state().as_ref(),
                    UsageEvent::new(UsageEventType::Failed, request_id, usage_data),
                )
                .await;
        });
        mark_stream_candidate_watchdog_terminal_started();
        if let Err(err) = task.await {
            tracing::warn!(
                event_name = "transport_error_terminal_handoff_failed",
                log_type = "ops",
                error = %err,
                "gateway transport error terminal handoff task failed"
            );
        }
    } else {
        mark_stream_candidate_watchdog_terminal_started();
    }

    build_transport_error_response(plan, trace_id, decision, client_status_code)
}

fn transport_error_client_body(plan: &aether_contracts::ExecutionPlan) -> Value {
    build_core_error_body_for_client_format(
        &plan.client_api_format,
        TRANSPORT_ERROR_CLIENT_MESSAGE,
        Some("upstream_transport_error"),
        LocalCoreSyncErrorKind::ServerError,
    )
    .unwrap_or_else(|| {
        json!({
            "error": {
                "type": "server_error",
                "message": TRANSPORT_ERROR_CLIENT_MESSAGE,
                "code": "upstream_transport_error",
            }
        })
    })
}

pub(crate) fn build_transport_error_response(
    plan: &aether_contracts::ExecutionPlan,
    trace_id: &str,
    decision: &GatewayControlDecision,
    client_status_code: u16,
) -> Result<Response<Body>, GatewayError> {
    let body_bytes = serde_json::to_vec(&transport_error_client_body(plan))
        .map_err(|err| GatewayError::Internal(err.to_string()))?;
    let headers = BTreeMap::from([
        ("content-type".to_string(), "application/json".to_string()),
        ("content-length".to_string(), body_bytes.len().to_string()),
    ]);
    attach_control_metadata_headers(
        build_client_response_from_parts(
            client_status_code,
            &headers,
            Body::from(body_bytes),
            trace_id,
            Some(decision),
        )?,
        Some(plan.request_id.as_str()),
        plan.candidate_id.as_deref(),
    )
}
