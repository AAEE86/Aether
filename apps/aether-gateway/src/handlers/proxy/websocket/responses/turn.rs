//! Per-turn lifecycle accounting for the standard Responses WebSocket bridge.
//!
//! Every `response.create` remains a separate billable and auditable request,
//! including turns that cause the bridge to re-plan a changed model. This
//! module turns the connection-local JSON events back into the existing
//! Responses stream report surface without exposing the socket protocol to the
//! normal HTTP/SSE execution runtime.

use std::collections::BTreeMap;
use std::future::Future;
use std::sync::Arc;
use std::time::{Duration, Instant};

use aether_contracts::{
    ExecutionPlan, ExecutionStreamTerminalSummary, ExecutionTelemetry, ExecutionTimeouts,
    MAX_EXECUTION_REQUEST_TIMEOUT_MS, MAX_EXECUTION_STREAM_FIRST_BYTE_TIMEOUT_MS,
};
use aether_data_contracts::repository::candidates::RequestCandidateStatus;
use aether_data_contracts::repository::usage::{
    UsageBodyCaptureState, WEBSOCKET_MODE_METADATA_KEY, WEBSOCKET_TRANSPORT_METADATA_KEY,
};
use aether_scheduler_core::SchedulerRequestCandidateStatusUpdate;
use aether_usage_runtime::{
    build_lifecycle_usage_seed, build_stream_terminal_usage_payload_seed,
    build_terminal_usage_context_seed, stream_report_represents_failure,
    DEFAULT_USAGE_RESPONSE_BODY_CAPTURE_LIMIT_BYTES,
};
use axum::http::StatusCode;
use base64::Engine as _;
use serde_json::{json, Map, Value};
use tracing::warn;

use super::adapter::ResponsesWebSocketProtocolAdapter;
use super::admission::ResponsesWebSocketTurnAdmission;
use super::frame::ParsedResponsesWebSocketFrame;
use crate::ai_serving::api::StreamingStandardTerminalObserver;
use crate::ai_serving::{build_openai_responses_stream_plan_from_decision, AiExecutionDecision};
use crate::clock::current_unix_ms;
use crate::control::{
    execution_plan_balance_capacity_rejection, refresh_execution_runtime_auth_context,
    request_model_local_rejection, GatewayControlDecision, GatewayLocalAuthRejection,
};
use crate::execution_runtime::attach_provider_response_headers_to_report_context;
use crate::orchestration::{
    apply_local_stream_failure_effects, apply_local_stream_success_effects,
    release_local_pool_key_lease, release_pool_key_lease_from_report_context,
    LocalExecutionEffectContext, LocalStreamFailureEffect,
};
use crate::request_candidate_runtime::{
    ensure_execution_request_candidate_slot, record_local_request_candidate_status,
};
use crate::usage::{submit_stream_report, GatewayStreamReportRequest};
use crate::{AppState, GatewayError};

const WEBSOCKET_CONNECTION_TRACE_REPORT_CONTEXT_FIELD: &str = "websocket_connection_trace_id";
const WEBSOCKET_TURN_INDEX_REPORT_CONTEXT_FIELD: &str = "websocket_turn_index";
const WEBSOCKET_LOGICAL_TURN_ID_REPORT_CONTEXT_FIELD: &str = "websocket_logical_turn_id";
const WEBSOCKET_TURN_ATTEMPT_REPORT_CONTEXT_FIELD: &str = "websocket_turn_attempt";
const DEFAULT_WEBSOCKET_FIRST_EVENT_TIMEOUT_MS: u64 = 30_000;
const RESPONSES_WEBSOCKET_LIFECYCLE_STAGE_TIMEOUT: Duration = Duration::from_secs(5);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum ResponsesWebSocketTurnObservation {
    Started,
    Terminal(ResponsesWebSocketTurnOutcome),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum ResponsesWebSocketTurnTimeoutPhase {
    AwaitingFirstEvent,
    AwaitingTerminal,
}

impl ResponsesWebSocketTurnTimeoutPhase {
    pub(super) const fn error_code(self) -> &'static str {
        match self {
            Self::AwaitingFirstEvent => "responses_websocket_first_event_timeout",
            Self::AwaitingTerminal => "responses_websocket_turn_timeout",
        }
    }

    pub(super) const fn client_message(self) -> &'static str {
        match self {
            Self::AwaitingFirstEvent => {
                "Provider did not emit a response event before the configured timeout"
            }
            Self::AwaitingTerminal => {
                "Provider did not finish the response before the configured timeout"
            }
        }
    }

    pub(super) const fn outcome(self) -> ResponsesWebSocketTurnOutcome {
        match self {
            Self::AwaitingFirstEvent => ResponsesWebSocketTurnOutcome::first_event_timeout(),
            Self::AwaitingTerminal => ResponsesWebSocketTurnOutcome::terminal_timeout(),
        }
    }
}

#[derive(Debug, Clone, Copy)]
pub(super) struct ResponsesWebSocketTurnDeadline {
    pub(super) phase: ResponsesWebSocketTurnTimeoutPhase,
    pub(super) deadline: Instant,
    pub(super) timeout: Duration,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum ResponsesWebSocketTurnOutcome {
    ProviderTerminal {
        status_code: u16,
        cancelled: bool,
    },
    Cancelled {
        reason: &'static str,
    },
    Failure {
        status_code: u16,
        reason: &'static str,
    },
}

impl ResponsesWebSocketTurnOutcome {
    pub(super) const fn client_disconnected() -> Self {
        Self::Cancelled {
            reason: "client disconnected before provider terminal event",
        }
    }

    pub(super) const fn connection_limit_reached() -> Self {
        Self::Cancelled {
            reason: "gateway WebSocket connection duration limit reached",
        }
    }

    pub(super) const fn connection_admission_lost() -> Self {
        Self::Cancelled {
            reason: "gateway WebSocket connection admission became unhealthy",
        }
    }

    pub(super) const fn upstream_closed() -> Self {
        Self::Failure {
            status_code: 502,
            reason: "upstream WebSocket closed before provider terminal event",
        }
    }

    pub(super) const fn upstream_receive_failed() -> Self {
        Self::Failure {
            status_code: 502,
            reason: "upstream WebSocket receive failed before provider terminal event",
        }
    }

    pub(super) const fn upstream_send_failed() -> Self {
        Self::Failure {
            status_code: 502,
            reason: "gateway could not forward response.create to the upstream",
        }
    }

    pub(super) const fn upstream_connect_failed(reason: &'static str) -> Self {
        Self::Failure {
            status_code: 502,
            reason,
        }
    }

    pub(super) const fn provider_quota_exhausted() -> Self {
        Self::Failure {
            status_code: 429,
            reason: "provider reported exhausted quota before closing the WebSocket",
        }
    }

    pub(super) const fn first_event_timeout() -> Self {
        Self::Failure {
            status_code: 504,
            reason: "upstream WebSocket did not emit a response event before timeout",
        }
    }

    pub(super) const fn terminal_timeout() -> Self {
        Self::Failure {
            status_code: 504,
            reason: "upstream WebSocket did not finish the response before timeout",
        }
    }

    pub(super) const fn relay_task_abandoned() -> Self {
        Self::Failure {
            status_code: 500,
            reason: "gateway relay task went away before the response finished",
        }
    }

    const fn status_code(self) -> u16 {
        match self {
            Self::ProviderTerminal { status_code, .. } | Self::Failure { status_code, .. } => {
                status_code
            }
            Self::Cancelled { .. } => 499,
        }
    }

    const fn cancelled(self) -> bool {
        matches!(
            self,
            Self::ProviderTerminal {
                cancelled: true,
                ..
            } | Self::Cancelled { .. }
        )
    }

    const fn forced_error(self) -> Option<&'static str> {
        match self {
            Self::Failure { reason, .. } => Some(reason),
            Self::ProviderTerminal { .. } | Self::Cancelled { .. } => None,
        }
    }

    const fn stream_timeout(self) -> bool {
        matches!(
            self,
            Self::Failure {
                status_code: 504,
                ..
            }
        )
    }
}

/// 一轮 turn 结束后要投射给供应商/密钥池的效果。
///
/// 每个分支都会释放 pool key lease：`ProviderFailure` 由 `PoolError` 释放，
/// `ProviderSuccess` 由 `PoolSuccessStream` 释放，其余情况直接释放。少一条
/// 分支就会把 lease 挂到 TTL 过期，等于短时间占死一把 key。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ResponsesWebSocketTurnEffect {
    /// 既不投射成功也不投射失败，只把 lease 还回去。
    ReleasePoolKeyLease,
    ProviderFailure,
    ProviderSuccess,
}

#[cfg(test)]
impl ResponsesWebSocketTurnEffect {
    /// 把「每个分支都必须释放 lease」这条不变量显式化，便于测试锁住
    /// 「没进任何分支导致 lease 泄漏」这类回归。
    const fn releases_pool_key_lease(self) -> bool {
        match self {
            Self::ReleasePoolKeyLease | Self::ProviderFailure | Self::ProviderSuccess => true,
        }
    }
}

/// 判定一轮 turn 结束后要投射的效果。
///
/// 关键分支是「记账层判成 failed，但这一轮没有投射供应商失败」：例如合法的
/// `response.incomplete`（写满 max_output_tokens）。共享 usage 判定目前仍会
/// 把这类终态记成失败，但供应商本身工作正常，既不该扣健康分，也不能因为落
/// 不到任何分支而漏掉 lease 释放。
const fn classify_responses_websocket_turn_effect(
    cancelled: bool,
    projects_provider_failure: bool,
    failed: bool,
) -> ResponsesWebSocketTurnEffect {
    if cancelled {
        ResponsesWebSocketTurnEffect::ReleasePoolKeyLease
    } else if projects_provider_failure {
        ResponsesWebSocketTurnEffect::ProviderFailure
    } else if failed {
        ResponsesWebSocketTurnEffect::ReleasePoolKeyLease
    } else {
        ResponsesWebSocketTurnEffect::ProviderSuccess
    }
}

pub(super) struct ResponsesWebSocketTurn {
    plan: ExecutionPlan,
    trace_id: String,
    report_kind: String,
    report_context: Option<Value>,
    started_at: Instant,
    candidate_started_at_unix_ms: u64,
    provider_headers: BTreeMap<String, String>,
    stream_started: bool,
    observer: StreamingStandardTerminalObserver,
    provider_capture: Vec<u8>,
    provider_capture_truncated: bool,
    client_capture: Vec<u8>,
    client_capture_truncated: bool,
    upstream_bytes: u64,
    first_event_elapsed_ms: Option<u64>,
    first_event_timeout: Duration,
    terminal_timeout: Duration,
    admission: Option<ResponsesWebSocketTurnAdmission>,
    terminal_error_body: Option<String>,
}

pub(super) fn prepare_responses_websocket_turn_decision(
    template: &AiExecutionDecision,
    request_id: String,
    reuse_selected_candidate: bool,
    client_event: &Value,
    provider_event: &Value,
    connection_trace_id: &str,
    turn_index: u64,
    logical_turn_id: &str,
    turn_attempt: u32,
) -> AiExecutionDecision {
    let mut decision = template.clone();
    decision.request_id = Some(request_id.clone());
    if !reuse_selected_candidate {
        decision.candidate_id = None;
    }
    decision.provider_request_body = Some(provider_event.clone());
    decision.provider_request_body_base64 = None;
    decision.report_context = Some(prepare_websocket_report_context(
        decision.report_context.take(),
        request_id.as_str(),
        reuse_selected_candidate,
        client_event,
        provider_event,
        connection_trace_id,
        turn_index,
        logical_turn_id,
        turn_attempt,
    ));
    decision
}

pub(super) async fn begin_responses_websocket_turn(
    state: &AppState,
    parts: &http::request::Parts,
    control_decision: &GatewayControlDecision,
    decision: AiExecutionDecision,
    client_event: &Value,
) -> Result<ResponsesWebSocketTurn, GatewayError> {
    let planned_report_context = decision.report_context.clone();
    let effective_control_decision =
        match refresh_websocket_turn_auth_context(state, control_decision, parts, client_event)
            .await
        {
            Ok(decision) => decision,
            Err(error) => {
                release_pool_key_lease_from_report_context(state, planned_report_context.as_ref())
                    .await;
                return Err(error);
            }
        };
    let attempt = match build_openai_responses_stream_plan_from_decision(
        parts,
        client_event,
        decision,
        false,
    ) {
        Ok(Some(attempt)) => attempt,
        Ok(None) => {
            release_pool_key_lease_from_report_context(state, planned_report_context.as_ref())
                .await;
            return Err(GatewayError::Internal(
                "Responses WebSocket request could not build a usage/audit stream plan".to_string(),
            ));
        }
        Err(error) => {
            release_pool_key_lease_from_report_context(state, planned_report_context.as_ref())
                .await;
            return Err(error);
        }
    };
    let mut plan = attempt.plan;
    let (first_event_timeout, terminal_timeout) =
        resolve_responses_websocket_turn_timeouts(plan.timeouts.as_ref());
    let report_kind = match attempt.report_kind {
        Some(report_kind) => report_kind,
        None => {
            release_local_pool_key_lease(
                state,
                LocalExecutionEffectContext {
                    plan: &plan,
                    report_context: attempt.report_context.as_ref(),
                },
            )
            .await;
            return Err(GatewayError::Internal(
                "Responses WebSocket request is missing an execution report kind".to_string(),
            ));
        }
    };
    let mut report_context = attempt.report_context;

    let balance_rejection = execution_plan_balance_capacity_rejection(
        state,
        &effective_control_decision,
        &plan,
        report_context.as_ref(),
    )
    .await;
    let balance_rejection = match balance_rejection {
        Ok(rejection) => rejection,
        Err(error) => {
            release_local_pool_key_lease(
                state,
                LocalExecutionEffectContext {
                    plan: &plan,
                    report_context: report_context.as_ref(),
                },
            )
            .await;
            return Err(error);
        }
    };
    if let Some(rejection) = balance_rejection {
        release_local_pool_key_lease(
            state,
            LocalExecutionEffectContext {
                plan: &plan,
                report_context: report_context.as_ref(),
            },
        )
        .await;
        return Err(websocket_auth_rejection_error(rejection));
    }

    ensure_execution_request_candidate_slot(state, &mut plan, &mut report_context).await;
    let admission = match ResponsesWebSocketTurnAdmission::acquire(
        state,
        &plan,
        plan.request_id.as_str(),
    )
    .await
    {
        Ok(admission) => admission,
        Err(error) => {
            release_local_pool_key_lease(
                state,
                LocalExecutionEffectContext {
                    plan: &plan,
                    report_context: report_context.as_ref(),
                },
            )
            .await;
            return Err(error);
        }
    };

    let lifecycle_seed = build_lifecycle_usage_seed(&plan, report_context.as_ref());
    // Keep WebSocket turns on the same lifecycle data path as HTTP streams.
    // `AppState` can dedicate an isolated background database pool to usage
    // writes; using the foreground state here bypasses that path and leaves
    // this transport with a different persistence lifecycle.
    let usage_data = state.usage_lifecycle_data_state().as_ref().clone();
    state
        .usage_runtime
        .record_pending_direct(&usage_data, lifecycle_seed)
        .await;

    let candidate_started_at_unix_ms = current_unix_ms();
    record_local_request_candidate_status(
        state,
        &plan,
        report_context.as_ref(),
        SchedulerRequestCandidateStatusUpdate {
            status: RequestCandidateStatus::Pending,
            status_code: None,
            error_type: None,
            error_message: None,
            latency_ms: None,
            started_at_unix_ms: Some(candidate_started_at_unix_ms),
            finished_at_unix_ms: None,
        },
    )
    .await;

    Ok(ResponsesWebSocketTurn {
        trace_id: plan.request_id.clone(),
        plan,
        report_kind,
        report_context,
        started_at: Instant::now(),
        candidate_started_at_unix_ms,
        provider_headers: BTreeMap::new(),
        stream_started: false,
        observer: StreamingStandardTerminalObserver::default(),
        provider_capture: Vec::new(),
        provider_capture_truncated: false,
        client_capture: Vec::new(),
        client_capture_truncated: false,
        upstream_bytes: 0,
        first_event_elapsed_ms: None,
        first_event_timeout,
        terminal_timeout,
        admission: Some(admission),
        terminal_error_body: None,
    })
}

async fn refresh_websocket_turn_auth_context(
    state: &AppState,
    control_decision: &GatewayControlDecision,
    parts: &http::request::Parts,
    client_event: &Value,
) -> Result<GatewayControlDecision, GatewayError> {
    let mut effective = control_decision.clone();
    if let Some(auth_context) = effective.auth_context.take() {
        let refreshed = refresh_execution_runtime_auth_context(
            state,
            auth_context,
            effective.auth_endpoint_signature.as_deref(),
        )
        .await?;
        effective.local_auth_rejection = refreshed.local_rejection.clone();
        effective.auth_context = Some(refreshed);
    }
    if let Some(rejection) = effective.local_auth_rejection.clone() {
        return Err(websocket_auth_rejection_error(rejection));
    }

    let body = serde_json::to_vec(client_event)
        .map(axum::body::Bytes::from)
        .map_err(|error| GatewayError::Internal(error.to_string()))?;
    if let Some(rejection) =
        request_model_local_rejection(state, Some(&effective), &parts.uri, &parts.headers, &body)
            .await?
    {
        return Err(websocket_auth_rejection_error(rejection));
    }
    Ok(effective)
}

fn websocket_auth_rejection_error(rejection: GatewayLocalAuthRejection) -> GatewayError {
    let (status, message) = match rejection {
        GatewayLocalAuthRejection::InvalidApiKey => {
            (StatusCode::UNAUTHORIZED, "The API key is invalid")
        }
        GatewayLocalAuthRejection::LockedApiKey => (
            StatusCode::FORBIDDEN,
            "The API key is locked and cannot be used",
        ),
        GatewayLocalAuthRejection::WalletUnavailable => {
            (StatusCode::FORBIDDEN, "The account wallet is unavailable")
        }
        GatewayLocalAuthRejection::BalanceDenied { remaining } => {
            let message = match remaining {
                Some(remaining) => format!("Insufficient balance (remaining: ${remaining:.2})"),
                None => "Insufficient balance".to_string(),
            };
            return GatewayError::Client {
                status: StatusCode::TOO_MANY_REQUESTS,
                message,
            };
        }
        GatewayLocalAuthRejection::ProviderNotAllowed { .. } => (
            StatusCode::FORBIDDEN,
            "The provider is not allowed for this API key",
        ),
        GatewayLocalAuthRejection::ApiFormatNotAllowed { .. } => (
            StatusCode::FORBIDDEN,
            "The API format is not allowed for this API key",
        ),
        GatewayLocalAuthRejection::ModelNotAllowed { .. } => (
            StatusCode::FORBIDDEN,
            "The requested model is not allowed for this API key",
        ),
        GatewayLocalAuthRejection::IpNotAllowed { .. } => (
            StatusCode::UNAUTHORIZED,
            "The current IP is not allowed for this API key",
        ),
    };
    GatewayError::Client {
        status,
        message: message.to_string(),
    }
}

impl ResponsesWebSocketTurn {
    /// Releases all per-turn capacity before terminal persistence starts.
    /// Provider-pool runtime tokens normally use an awaited removal. The
    /// bounded wait prevents a broken runtime backend from stalling the relay;
    /// the guard's `Drop` path remains the timeout fallback.
    pub(super) async fn release_admission(&mut self) {
        if let Some(admission) = self.admission.take() {
            let _ = await_websocket_lifecycle_stage(
                &self.trace_id,
                "turn_admission_release",
                admission.release(),
            )
            .await;
        }
    }

    pub(super) fn set_provider_response_headers(&mut self, headers: BTreeMap<String, String>) {
        self.report_context = attach_provider_response_headers_to_report_context(
            self.report_context.take(),
            &headers,
        );
        self.provider_headers = headers;
    }

    /// Starts the per-turn response deadlines only after the corresponding
    /// `response.create` has been accepted by the upstream socket writer.
    pub(super) fn mark_upstream_request_sent(&mut self) {
        self.started_at = Instant::now();
        self.first_event_elapsed_ms = None;
    }

    pub(super) fn deadline(&self) -> ResponsesWebSocketTurnDeadline {
        let (phase, timeout) = if self.first_event_elapsed_ms.is_some() {
            (
                ResponsesWebSocketTurnTimeoutPhase::AwaitingTerminal,
                self.terminal_timeout,
            )
        } else {
            (
                ResponsesWebSocketTurnTimeoutPhase::AwaitingFirstEvent,
                self.first_event_timeout.min(self.terminal_timeout),
            )
        };
        ResponsesWebSocketTurnDeadline {
            phase,
            deadline: self.started_at + timeout,
            timeout,
        }
    }

    pub(super) fn observe_upstream_frame(
        &mut self,
        frame: &ParsedResponsesWebSocketFrame<'_>,
        adapter: &dyn ResponsesWebSocketProtocolAdapter,
    ) -> Option<ResponsesWebSocketTurnObservation> {
        self.upstream_bytes = self
            .upstream_bytes
            .saturating_add(frame.raw_text().len() as u64);
        if self.first_event_elapsed_ms.is_none() {
            self.first_event_elapsed_ms = Some(elapsed_ms(self.started_at));
        }

        // A batched frame carries several events; the usage observer parses one
        // Responses event per SSE line, so the batch must be unwrapped or its
        // token usage is lost.
        let events = frame.protocol_events();
        for event in &events {
            self.capture_sse_event(event);
            adapter.decorate_turn_report_context(&mut self.report_context, event);
        }
        let fallback_context = json!({
            "provider_api_format": "openai:responses",
            "client_api_format": "openai:responses",
        });
        let report_context = self.report_context.as_ref().unwrap_or(&fallback_context);
        for event in &events {
            if let Err(error) = self
                .observer
                .push_line(report_context, websocket_event_as_sse_line(event))
            {
                self.observer.disable_with_error(error.to_string());
                break;
            }
        }

        let event_type = frame.event_type().unwrap_or_default();
        if matches!(event_type, "error" | "response.failed") {
            self.terminal_error_body = frame
                .terminal_event()
                .and_then(|event| serde_json::to_string(event).ok());
        }
        if let Some(outcome) = provider_terminal_outcome(frame) {
            return Some(ResponsesWebSocketTurnObservation::Terminal(outcome));
        }
        if frame.is_started() {
            return Some(ResponsesWebSocketTurnObservation::Started);
        }
        None
    }

    pub(super) fn observe_invalid_upstream_text(
        &mut self,
        text: &str,
    ) -> Option<ResponsesWebSocketTurnObservation> {
        self.upstream_bytes = self.upstream_bytes.saturating_add(text.len() as u64);
        if self.first_event_elapsed_ms.is_none() {
            self.first_event_elapsed_ms = Some(elapsed_ms(self.started_at));
        }
        self.capture_sse_event(&json!({
            "type": "error",
            "error": {
                "type": "gateway_protocol_error",
                "message": "upstream Responses WebSocket event was not valid JSON"
            }
        }));
        self.observer
            .disable_with_error("upstream Responses WebSocket event was not valid JSON");
        Some(ResponsesWebSocketTurnObservation::Terminal(
            ResponsesWebSocketTurnOutcome::Failure {
                status_code: 502,
                reason: "upstream Responses WebSocket event was not valid JSON",
            },
        ))
    }

    pub(super) fn capture_client_frame(&mut self, event: &Value) {
        append_capture(
            &mut self.client_capture,
            &websocket_event_as_sse_line(event),
            &mut self.client_capture_truncated,
        );
    }

    pub(super) async fn mark_stream_started(&mut self, state: &AppState) {
        if self.stream_started {
            return;
        }
        self.stream_started = true;
        let lifecycle_seed = build_lifecycle_usage_seed(&self.plan, self.report_context.as_ref());
        let telemetry = self.telemetry();
        state.usage_runtime.record_stream_started(
            state.usage_lifecycle_data_state().as_ref(),
            &lifecycle_seed,
            200,
            Some(&telemetry),
        );
        let trace_id = self.trace_id.clone();
        let _ = await_websocket_lifecycle_stage(
            &trace_id,
            "candidate_stream_started",
            record_local_request_candidate_status(
                state,
                &self.plan,
                self.report_context.as_ref(),
                SchedulerRequestCandidateStatusUpdate {
                    status: RequestCandidateStatus::Streaming,
                    status_code: Some(200),
                    error_type: None,
                    error_message: None,
                    latency_ms: None,
                    started_at_unix_ms: Some(self.candidate_started_at_unix_ms),
                    finished_at_unix_ms: None,
                },
            ),
        )
        .await;
    }

    async fn finalize(mut self, state: &AppState, outcome: ResponsesWebSocketTurnOutcome) {
        let summary = self.finish_summary(outcome);
        let cancelled = outcome.cancelled();
        let status_code = outcome.status_code();
        let missing_terminal = !cancelled && !summary.observed_finish;
        let terminal_error_body = self.terminal_error_body.take();
        let outcome_reason = outcome_reason(outcome);
        let telemetry = Some(self.telemetry());
        let (provider_body_base64, provider_body_state) =
            encode_stream_capture(&self.provider_capture, self.provider_capture_truncated);
        let (client_body_base64, client_body_state) =
            encode_stream_capture(&self.client_capture, self.client_capture_truncated);
        let payload = GatewayStreamReportRequest {
            trace_id: self.trace_id.clone(),
            report_kind: self.report_kind,
            report_context: self.report_context,
            status_code,
            headers: self.provider_headers,
            provider_body_base64,
            provider_body_state,
            client_body_base64,
            client_body_state,
            terminal_summary: Some(summary.clone()),
            telemetry,
        };
        let failed = !cancelled && stream_report_represents_failure(&payload);

        // Do not hold gateway/provider capacity while usage and audit writes
        // run. The turn has a complete terminal payload at this point.
        if let Some(admission) = self.admission.take() {
            let _ = await_websocket_lifecycle_stage(
                &self.trace_id,
                "turn_admission_release",
                admission.release(),
            )
            .await;
        }

        let context_seed =
            build_terminal_usage_context_seed(&self.plan, payload.report_context.as_ref());
        let payload_seed = build_stream_terminal_usage_payload_seed(&payload);
        // This write is the turn's billing record, so it must not be abandoned
        // when the usage runtime is slow: the row was created as Pending and
        // nothing else reconciles it.
        let usage_runtime = Arc::clone(&state.usage_runtime);
        let usage_data = Arc::clone(state.usage_lifecycle_data_state());
        await_detachable_lifecycle_stage(&self.trace_id, "usage_terminal", async move {
            usage_runtime
                .record_stream_terminal(usage_data.as_ref(), context_seed, payload_seed, cancelled)
                .await;
        })
        .await;

        let (error_type, error_message) = if cancelled {
            (
                Some("websocket_cancelled".to_string()),
                Some(outcome_reason.clone()),
            )
        } else if missing_terminal {
            (
                Some("stream_missing_terminal_event".to_string()),
                Some(summary.parser_error.clone().unwrap_or_else(|| {
                    "upstream Responses WebSocket ended before a provider terminal event"
                        .to_string()
                })),
            )
        } else if failed {
            (
                Some("stream_terminal_error".to_string()),
                summary
                    .parser_error
                    .clone()
                    .or_else(|| Some(outcome_reason.clone())),
            )
        } else {
            (None, None)
        };
        let _ = await_websocket_lifecycle_stage(
            &self.trace_id,
            "candidate_terminal",
            record_local_request_candidate_status(
                state,
                &self.plan,
                payload.report_context.as_ref(),
                SchedulerRequestCandidateStatusUpdate {
                    status: if cancelled {
                        RequestCandidateStatus::Cancelled
                    } else if failed {
                        RequestCandidateStatus::Failed
                    } else {
                        RequestCandidateStatus::Success
                    },
                    status_code: Some(status_code),
                    error_type,
                    error_message,
                    latency_ms: payload
                        .telemetry
                        .as_ref()
                        .and_then(|value| value.elapsed_ms),
                    started_at_unix_ms: Some(self.candidate_started_at_unix_ms),
                    finished_at_unix_ms: Some(current_unix_ms()),
                },
            ),
        )
        .await;

        // Health, adaptive, and pool feedback are secondary to the terminal
        // usage/candidate record. A slow dependency must not leave a turn in
        // Pending or Streaming indefinitely.
        let effect_context = LocalExecutionEffectContext {
            plan: &self.plan,
            report_context: payload.report_context.as_ref(),
        };
        let projects_provider_failure = !cancelled
            && (status_code >= 400
                || outcome.forced_error().is_some()
                || summary.parser_error.is_some()
                || missing_terminal);
        let provider_effect =
            classify_responses_websocket_turn_effect(cancelled, projects_provider_failure, failed);
        let effects_completed =
            await_websocket_lifecycle_stage(&self.trace_id, "provider_effects", async {
                match provider_effect {
                    ResponsesWebSocketTurnEffect::ReleasePoolKeyLease => {
                        release_local_pool_key_lease(state, effect_context).await;
                    }
                    ResponsesWebSocketTurnEffect::ProviderFailure => {
                        let response_text = terminal_error_body
                            .as_deref()
                            .or(summary.parser_error.as_deref())
                            .unwrap_or(outcome_reason.as_str());
                        let mut effect = LocalStreamFailureEffect::new(
                            status_code,
                            &payload.headers,
                            Some(response_text),
                        );
                        if outcome.stream_timeout() {
                            effect = effect.with_stream_timeout();
                        }
                        apply_local_stream_failure_effects(state, effect_context, effect).await;
                    }
                    ResponsesWebSocketTurnEffect::ProviderSuccess => {
                        apply_local_stream_success_effects(state, effect_context, &payload).await;
                    }
                }
            })
            .await
            .is_some();
        if !effects_completed {
            let _ = await_websocket_lifecycle_stage(
                &self.trace_id,
                "pool_lease_release_after_effect_timeout",
                release_local_pool_key_lease(state, effect_context),
            )
            .await;
        }

        // The normal execution runtime does not submit a stream report after a
        // downstream disconnect either. The terminal usage record above still
        // captures cancellation without applying provider-success side effects.
        if !cancelled {
            if let Some(Err(error)) = await_websocket_lifecycle_stage(
                &self.trace_id,
                "execution_report",
                submit_stream_report(state, payload),
            )
            .await
            {
                warn!(
                    event_name = "responses_websocket_execution_report_submit_failed",
                    log_type = "ops",
                    transport = "websocket",
                    websocket = true,
                    trace_id = %self.trace_id,
                    error = ?error,
                    "gateway failed to submit Responses WebSocket terminal report"
                );
            }
        }
    }

    fn capture_sse_event(&mut self, event: &Value) {
        append_capture(
            &mut self.provider_capture,
            &websocket_event_as_sse_line(event),
            &mut self.provider_capture_truncated,
        );
    }

    fn finish_summary(
        &mut self,
        outcome: ResponsesWebSocketTurnOutcome,
    ) -> ExecutionStreamTerminalSummary {
        let fallback_context = json!({
            "provider_api_format": "openai:responses",
            "client_api_format": "openai:responses",
        });
        let report_context = self.report_context.as_ref().unwrap_or(&fallback_context);
        let mut summary = match self.observer.finish(report_context) {
            Ok(Some(summary)) => summary,
            Ok(None) => ExecutionStreamTerminalSummary::default(),
            Err(error) => {
                self.observer.disable_with_error(error.to_string());
                self.observer.latest_summary().cloned().unwrap_or_default()
            }
        };
        if let Some(reason) = outcome.forced_error() {
            if summary.parser_error.is_none() {
                summary.parser_error = Some(reason.to_string());
            }
        }
        if outcome.cancelled() {
            summary.observed_finish = true;
            if summary.finish_reason.is_none() {
                summary.finish_reason = Some("cancelled".to_string());
            }
        } else if !summary.observed_finish && summary.parser_error.is_none() {
            summary.parser_error = Some(
                "upstream Responses WebSocket ended before a provider terminal event".to_string(),
            );
        }
        summary
    }

    fn telemetry(&self) -> ExecutionTelemetry {
        ExecutionTelemetry {
            ttfb_ms: self.first_event_elapsed_ms,
            elapsed_ms: Some(elapsed_ms(self.started_at)),
            upstream_bytes: Some(self.upstream_bytes),
        }
    }
}

impl ResponsesWebSocketTurn {
    /// Finalizes a turn whose owner is already gone, releasing admission first.
    ///
    /// The normal path releases admission before spawning the finalizer; a
    /// turn reclaimed from a lost relay task has to do both itself.
    pub(super) async fn finalize_detached(
        mut self,
        state: &AppState,
        outcome: ResponsesWebSocketTurnOutcome,
    ) {
        self.release_admission().await;
        self.finalize(state, outcome).await;
    }
}

pub(super) async fn spawn_responses_websocket_turn_finalization(
    state: AppState,
    mut turn: ResponsesWebSocketTurn,
    outcome: ResponsesWebSocketTurnOutcome,
) -> tokio::task::JoinHandle<()> {
    turn.release_admission().await;
    tokio::spawn(async move {
        turn.finalize(&state, outcome).await;
    })
}

fn prepare_websocket_report_context(
    report_context: Option<Value>,
    request_id: &str,
    reuse_selected_candidate: bool,
    client_event: &Value,
    provider_event: &Value,
    connection_trace_id: &str,
    turn_index: u64,
    logical_turn_id: &str,
    turn_attempt: u32,
) -> Value {
    let mut object = match report_context {
        Some(Value::Object(object)) => object,
        Some(other) => Map::from_iter([("seed".to_string(), other)]),
        None => Map::new(),
    };
    object.insert(
        "request_id".to_string(),
        Value::String(request_id.to_string()),
    );
    if !reuse_selected_candidate {
        for field in [
            "candidate_id",
            "candidate_index",
            "retry_index",
            "pool_key_index",
            "candidate_group_id",
            "pool_key_lease_key",
            "pool_key_lease_owner",
            "pool_key_lease_token",
            "pool_key_lease_fencing_token",
            "pool_key_lease_ttl_ms",
            "scheduler_affinity_epoch",
        ] {
            object.remove(field);
        }
    }
    object.insert("original_request_body".to_string(), client_event.clone());
    if let Some(model) = client_event
        .get("model")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|model| !model.is_empty())
    {
        object.insert("model".to_string(), Value::String(model.to_string()));
    }
    if let Some(mapped_model) = provider_event
        .get("model")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|model| !model.is_empty())
    {
        object.insert(
            "mapped_model".to_string(),
            Value::String(mapped_model.to_string()),
        );
    }
    object.insert(WEBSOCKET_MODE_METADATA_KEY.to_string(), Value::Bool(true));
    object.insert(
        WEBSOCKET_CONNECTION_TRACE_REPORT_CONTEXT_FIELD.to_string(),
        Value::String(connection_trace_id.to_string()),
    );
    object.insert(
        WEBSOCKET_TURN_INDEX_REPORT_CONTEXT_FIELD.to_string(),
        Value::Number(turn_index.into()),
    );
    object.insert(
        WEBSOCKET_LOGICAL_TURN_ID_REPORT_CONTEXT_FIELD.to_string(),
        Value::String(logical_turn_id.to_string()),
    );
    object.insert(
        WEBSOCKET_TURN_ATTEMPT_REPORT_CONTEXT_FIELD.to_string(),
        Value::Number(turn_attempt.into()),
    );
    object.insert(
        WEBSOCKET_TRANSPORT_METADATA_KEY.to_string(),
        Value::String("responses".to_string()),
    );
    Value::Object(object)
}

fn provider_terminal_outcome(
    frame: &ParsedResponsesWebSocketFrame<'_>,
) -> Option<ResponsesWebSocketTurnOutcome> {
    frame
        .terminal()
        .map(|terminal| ResponsesWebSocketTurnOutcome::ProviderTerminal {
            status_code: terminal.status_code,
            cancelled: terminal.cancelled,
        })
}

fn resolve_responses_websocket_turn_timeouts(
    timeouts: Option<&ExecutionTimeouts>,
) -> (Duration, Duration) {
    let first_event_timeout_ms = timeouts
        .and_then(|timeouts| timeouts.first_byte_ms)
        .filter(|value| *value > 0)
        .unwrap_or(DEFAULT_WEBSOCKET_FIRST_EVENT_TIMEOUT_MS)
        .min(MAX_EXECUTION_STREAM_FIRST_BYTE_TIMEOUT_MS);
    let terminal_timeout_ms = timeouts
        .and_then(|timeouts| timeouts.total_ms)
        .filter(|value| *value > 0)
        .unwrap_or(MAX_EXECUTION_REQUEST_TIMEOUT_MS)
        .min(MAX_EXECUTION_REQUEST_TIMEOUT_MS);
    (
        Duration::from_millis(first_event_timeout_ms),
        Duration::from_millis(terminal_timeout_ms),
    )
}

fn outcome_reason(outcome: ResponsesWebSocketTurnOutcome) -> String {
    match outcome {
        ResponsesWebSocketTurnOutcome::ProviderTerminal {
            cancelled: true, ..
        } => "provider cancelled the response".to_string(),
        ResponsesWebSocketTurnOutcome::ProviderTerminal {
            cancelled: false, ..
        } => "provider returned a terminal response event".to_string(),
        ResponsesWebSocketTurnOutcome::Cancelled { reason }
        | ResponsesWebSocketTurnOutcome::Failure { reason, .. } => reason.to_string(),
    }
}

fn websocket_event_as_sse_line(event: &Value) -> Vec<u8> {
    let payload = serde_json::to_string(event).unwrap_or_else(|_| {
        json!({
            "type": "error",
            "error": {
                "type": "gateway_protocol_error",
                "message": "upstream Responses WebSocket event could not be serialized"
            }
        })
        .to_string()
    });
    format!("data: {payload}\n\n").into_bytes()
}

fn append_capture(buffer: &mut Vec<u8>, bytes: &[u8], truncated: &mut bool) {
    if bytes.is_empty() || *truncated {
        return;
    }
    let max_bytes = DEFAULT_USAGE_RESPONSE_BODY_CAPTURE_LIMIT_BYTES;
    if buffer.len() >= max_bytes {
        *truncated = true;
        return;
    }
    let remaining = max_bytes - buffer.len();
    let copied = bytes.len().min(remaining);
    buffer.extend_from_slice(&bytes[..copied]);
    if copied < bytes.len() {
        *truncated = true;
    }
}

fn encode_stream_capture(
    bytes: &[u8],
    truncated: bool,
) -> (Option<String>, Option<UsageBodyCaptureState>) {
    let body = (!bytes.is_empty()).then(|| base64::engine::general_purpose::STANDARD.encode(bytes));
    let state = if truncated {
        UsageBodyCaptureState::Truncated
    } else if bytes.is_empty() {
        UsageBodyCaptureState::None
    } else {
        UsageBodyCaptureState::Inline
    };
    (body, Some(state))
}

fn elapsed_ms(started_at: Instant) -> u64 {
    started_at.elapsed().as_millis().min(u128::from(u64::MAX)) as u64
}

/// Runs a lifecycle write that must not be lost, while still bounding how long
/// the caller waits for it.
///
/// [`await_websocket_lifecycle_stage`] drops the future it is waiting on. That
/// is the right trade for secondary effects, but it would silently discard a
/// write the rest of the system depends on. Spawning first makes the deadline
/// bound only the wait: dropping the `JoinHandle` detaches the task, which runs
/// to completion in the background.
async fn await_detachable_lifecycle_stage<F>(trace_id: &str, stage: &'static str, write: F)
where
    F: Future<Output = ()> + Send + 'static,
{
    let _ = await_websocket_lifecycle_stage(trace_id, stage, tokio::spawn(write)).await;
}

async fn await_websocket_lifecycle_stage<T>(
    trace_id: &str,
    stage: &'static str,
    future: impl Future<Output = T>,
) -> Option<T> {
    match tokio::time::timeout(RESPONSES_WEBSOCKET_LIFECYCLE_STAGE_TIMEOUT, future).await {
        Ok(value) => Some(value),
        Err(_) => {
            warn!(
                event_name = "responses_websocket_lifecycle_stage_timeout",
                log_type = "ops",
                transport = "websocket",
                websocket = true,
                trace_id,
                stage,
                timeout_ms = RESPONSES_WEBSOCKET_LIFECYCLE_STAGE_TIMEOUT.as_millis() as u64,
                "gateway stopped waiting for a Responses WebSocket lifecycle stage"
            );
            None
        }
    }
}

#[cfg(test)]
mod tests {
    use std::time::{Duration, Instant};

    use aether_contracts::ExecutionTimeouts;
    use serde_json::json;

    use crate::ai_serving::api::StreamingStandardTerminalObserver;

    use super::super::frame::ParsedResponsesWebSocketFrame;
    use super::{
        classify_responses_websocket_turn_effect, prepare_websocket_report_context,
        provider_terminal_outcome, resolve_responses_websocket_turn_timeouts,
        websocket_event_as_sse_line, ResponsesWebSocketTurnDeadline, ResponsesWebSocketTurnEffect,
        ResponsesWebSocketTurnOutcome, ResponsesWebSocketTurnTimeoutPhase,
    };

    #[test]
    fn followup_context_uses_a_fresh_request_and_candidate() {
        let context = prepare_websocket_report_context(
            Some(json!({
                "request_id":"connection",
                "candidate_id":"candidate",
                "candidate_index": 0,
                "pool_key_lease_key": "lease",
                "original_request_body":{"model":"public"}
            })),
            "turn-2",
            false,
            &json!({"type":"response.create","model":"public"}),
            &json!({"type":"response.create","model":"provider-public"}),
            "connection",
            2,
            "logical-turn-2",
            1,
        );
        assert_eq!(context["request_id"], "turn-2");
        assert!(context.get("candidate_id").is_none());
        assert!(context.get("candidate_index").is_none());
        assert!(context.get("pool_key_lease_key").is_none());
        assert_eq!(context["original_request_body"]["type"], "response.create");
        assert_eq!(context["model"], "public");
        assert_eq!(context["mapped_model"], "provider-public");
        assert_eq!(context["websocket_mode"], true);
        assert_eq!(context["websocket_transport"], "responses");
        assert_eq!(context["websocket_logical_turn_id"], "logical-turn-2");
        assert_eq!(context["websocket_turn_attempt"], 1);
    }

    #[test]
    fn replanned_context_keeps_selected_candidate_and_records_the_new_client_model() {
        let context = prepare_websocket_report_context(
            Some(json!({
                "request_id": "prewarm",
                "candidate_id": "terra-candidate",
                "original_request_body": {"model": "gpt-5.6-sol", "generate": false}
            })),
            "turn-2",
            true,
            &json!({
                "type": "response.create",
                "model": "gpt-5.6-terra",
                "input": "hello"
            }),
            &json!({
                "type": "response.create",
                "model": "gpt-5.6-terra-provider",
                "input": "hello"
            }),
            "connection",
            2,
            "logical-turn-2",
            2,
        );

        assert_eq!(context["request_id"], "turn-2");
        assert_eq!(context["candidate_id"], "terra-candidate");
        assert_eq!(context["original_request_body"]["model"], "gpt-5.6-terra");
        assert_eq!(context["model"], "gpt-5.6-terra");
        assert_eq!(context["mapped_model"], "gpt-5.6-terra-provider");
        assert_eq!(context["websocket_mode"], true);
        assert_eq!(context["websocket_logical_turn_id"], "logical-turn-2");
        assert_eq!(context["websocket_turn_attempt"], 2);
    }

    #[test]
    fn completed_event_is_captured_as_a_responses_sse_terminal_event() {
        let event = json!({
            "type": "response.completed",
            "response": {
                "id": "resp_ws_usage_123",
                "model": "gpt-5.6",
                "usage": {
                    "input_tokens": 3,
                    "output_tokens": 5,
                    "total_tokens": 8
                }
            }
        });
        let raw = serde_json::to_string(&event).expect("event should serialize");
        let frame = ParsedResponsesWebSocketFrame::parse(&raw).expect("event should parse");
        let outcome = provider_terminal_outcome(&frame);
        assert_eq!(
            outcome,
            Some(ResponsesWebSocketTurnOutcome::ProviderTerminal {
                status_code: 200,
                cancelled: false
            })
        );
        let capture = String::from_utf8(websocket_event_as_sse_line(&event))
            .expect("capture should be UTF-8");
        assert_eq!(capture, format!("data: {event}\n\n"));

        let report_context = json!({
            "provider_api_format": "openai:responses",
            "client_api_format": "openai:responses",
        });
        let mut observer = StreamingStandardTerminalObserver::default();
        observer
            .push_line(&report_context, capture.into_bytes())
            .expect("WebSocket terminal event should be accepted by the usage observer");
        let summary = observer
            .finish(&report_context)
            .expect("WebSocket terminal observer should finish")
            .expect("WebSocket terminal observer should produce a summary");
        let usage = summary
            .standardized_usage
            .expect("response.completed usage must reach the terminal summary");
        assert_eq!(usage.input_tokens, 3);
        assert_eq!(usage.output_tokens, 5);
        assert_eq!(usage.dimensions.get("total_tokens"), Some(&json!(8)));
    }

    #[test]
    fn a_legitimate_incomplete_is_a_successful_provider_terminal_that_keeps_its_usage() {
        // 写满 max_output_tokens 的 incomplete 是合法终态：状态码不再是 502，
        // usage 观测器照样能看到 finish 和 token，记账层不该把它当解析失败。
        let event = json!({
            "type": "response.incomplete",
            "response": {
                "id": "resp_ws_incomplete_123",
                "model": "gpt-5.6",
                "status": "incomplete",
                "incomplete_details": {"reason": "max_output_tokens"},
                "output": [],
                "usage": {
                    "input_tokens": 4,
                    "output_tokens": 7,
                    "total_tokens": 11
                }
            }
        });
        let raw = serde_json::to_string(&event).expect("event should serialize");
        let frame = ParsedResponsesWebSocketFrame::parse(&raw).expect("event should parse");
        let outcome = provider_terminal_outcome(&frame);
        assert_eq!(
            outcome,
            Some(ResponsesWebSocketTurnOutcome::ProviderTerminal {
                status_code: 200,
                cancelled: false
            })
        );
        let outcome = outcome.expect("incomplete should end the turn");
        assert!(!outcome.cancelled());
        assert!(outcome.forced_error().is_none());
        assert!(!outcome.stream_timeout());

        let report_context = json!({
            "provider_api_format": "openai:responses",
            "client_api_format": "openai:responses",
        });
        let mut observer = StreamingStandardTerminalObserver::default();
        observer
            .push_line(&report_context, websocket_event_as_sse_line(&event))
            .expect("a legitimate incomplete must be accepted by the usage observer");
        let summary = observer
            .finish(&report_context)
            .expect("terminal observer should finish")
            .expect("terminal observer should produce a summary");
        assert!(summary.observed_finish);
        assert_eq!(summary.finish_reason.as_deref(), Some("length"));
        assert!(summary.parser_error.is_none());
        let usage = summary
            .standardized_usage
            .expect("incomplete usage must reach the terminal summary");
        assert_eq!(usage.input_tokens, 4);
        assert_eq!(usage.output_tokens, 7);

        // finalize() 用这些事实决定是否投射供应商失败：合法 incomplete 必须
        // 全部落在“非失败”一侧。
        let missing_terminal = !outcome.cancelled() && !summary.observed_finish;
        let projects_provider_failure = !outcome.cancelled()
            && (outcome.status_code() >= 400
                || outcome.forced_error().is_some()
                || summary.parser_error.is_some()
                || missing_terminal);
        assert!(!missing_terminal);
        assert!(!projects_provider_failure);
    }

    #[test]
    fn an_incomplete_without_a_legitimate_reason_still_projects_a_provider_failure() {
        let raw = r#"{"type":"response.incomplete","response":{"incomplete_details":{"reason":"error"}}}"#;
        let frame = ParsedResponsesWebSocketFrame::parse(raw).expect("event should parse");

        assert_eq!(
            provider_terminal_outcome(&frame),
            Some(ResponsesWebSocketTurnOutcome::ProviderTerminal {
                status_code: 502,
                cancelled: false
            })
        );
    }

    #[test]
    fn a_legitimate_incomplete_still_releases_the_pool_key_lease() {
        // 共享 usage 判定目前仍把 response.incomplete 记成终态失败，于是会出现
        // failed=true 而 projects_provider_failure=false 的组合。这种组合必须
        // 明确落到“只释放 lease”的分支，否则 lease 会挂到 TTL 过期。
        let effect = classify_responses_websocket_turn_effect(false, false, true);

        assert_eq!(effect, ResponsesWebSocketTurnEffect::ReleasePoolKeyLease);
        assert!(effect.releases_pool_key_lease());
    }

    #[test]
    fn every_turn_effect_releases_the_pool_key_lease() {
        for (cancelled, projects_provider_failure, failed, expected) in [
            (
                true,
                false,
                false,
                ResponsesWebSocketTurnEffect::ReleasePoolKeyLease,
            ),
            (
                true,
                true,
                true,
                ResponsesWebSocketTurnEffect::ReleasePoolKeyLease,
            ),
            (
                false,
                true,
                true,
                ResponsesWebSocketTurnEffect::ProviderFailure,
            ),
            (
                false,
                false,
                true,
                ResponsesWebSocketTurnEffect::ReleasePoolKeyLease,
            ),
            (
                false,
                false,
                false,
                ResponsesWebSocketTurnEffect::ProviderSuccess,
            ),
        ] {
            let effect = classify_responses_websocket_turn_effect(
                cancelled,
                projects_provider_failure,
                failed,
            );
            assert_eq!(
                effect, expected,
                "cancelled={cancelled} projects_provider_failure={projects_provider_failure} failed={failed}"
            );
            assert!(
                effect.releases_pool_key_lease(),
                "every effect branch must release the pool key lease"
            );
        }
    }

    #[test]
    fn error_event_uses_the_top_level_status_code() {
        let event = json!({
            "type": "error",
            "status_code": 429,
            "error": {"type": "usage_limit_reached"},
        });
        let raw = serde_json::to_string(&event).expect("event should serialize");
        let frame = ParsedResponsesWebSocketFrame::parse(&raw).expect("event should parse");
        assert_eq!(
            provider_terminal_outcome(&frame),
            Some(ResponsesWebSocketTurnOutcome::ProviderTerminal {
                status_code: 429,
                cancelled: false,
            })
        );
    }

    #[test]
    fn quota_close_fallback_preserves_the_client_visible_status() {
        let outcome = ResponsesWebSocketTurnOutcome::provider_quota_exhausted();
        assert_eq!(outcome.status_code(), 429);
        assert!(matches!(
            outcome,
            ResponsesWebSocketTurnOutcome::Failure {
                status_code: 429,
                ..
            }
        ));
    }

    #[test]
    fn an_abandoned_turn_is_recorded_as_a_gateway_failure_not_a_cancellation() {
        // A turn reclaimed by the Drop guard must not look like a client
        // cancellation: cancelled turns skip the stream report entirely, which
        // would defeat the point of reclaiming it.
        let outcome = ResponsesWebSocketTurnOutcome::relay_task_abandoned();

        assert_eq!(outcome.status_code(), 500);
        assert!(!outcome.cancelled());
        assert!(outcome.forced_error().is_some());
    }

    #[test]
    fn turn_timeouts_reuse_provider_first_byte_and_request_deadlines() {
        let (first_event, terminal) =
            resolve_responses_websocket_turn_timeouts(Some(&ExecutionTimeouts {
                first_byte_ms: Some(12_345),
                total_ms: Some(67_890),
                ..ExecutionTimeouts::default()
            }));

        assert_eq!(first_event, Duration::from_millis(12_345));
        assert_eq!(terminal, Duration::from_millis(67_890));
    }

    #[test]
    fn first_event_deadline_never_outlives_the_turn_deadline() {
        let started_at = Instant::now();
        let first_event = Duration::from_secs(30);
        let terminal = Duration::from_secs(10);
        let deadline = ResponsesWebSocketTurnDeadline {
            phase: ResponsesWebSocketTurnTimeoutPhase::AwaitingFirstEvent,
            deadline: started_at + first_event.min(terminal),
            timeout: first_event.min(terminal),
        };

        assert_eq!(
            deadline.phase,
            ResponsesWebSocketTurnTimeoutPhase::AwaitingFirstEvent
        );
        assert_eq!(deadline.timeout, Duration::from_secs(10));
        assert_eq!(deadline.deadline, started_at + Duration::from_secs(10));
    }
}
