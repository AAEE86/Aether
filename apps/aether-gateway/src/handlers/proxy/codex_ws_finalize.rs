//! Per-turn lifecycle accounting for the Codex Responses WebSocket bridge.
//!
//! Every `response.create` remains a separate billable and auditable request,
//! including turns that cause the bridge to re-plan a changed model. This
//! module turns the connection-local JSON events back into the existing
//! Responses stream report surface without exposing the socket protocol to the
//! normal HTTP/SSE execution runtime.

use std::collections::BTreeMap;
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
    build_terminal_usage_context_seed, DEFAULT_USAGE_RESPONSE_BODY_CAPTURE_LIMIT_BYTES,
};
use base64::Engine as _;
use serde_json::{json, Map, Value};
use tracing::warn;

use crate::ai_serving::api::StreamingStandardTerminalObserver;
use crate::ai_serving::{build_openai_responses_stream_plan_from_decision, AiExecutionDecision};
use crate::clock::current_unix_ms;
use crate::execution_runtime::attach_provider_response_headers_to_report_context;
use crate::request_candidate_runtime::{
    ensure_execution_request_candidate_slot, record_local_request_candidate_status,
};
use crate::usage::{submit_stream_report, GatewayStreamReportRequest};
use crate::{AppState, GatewayError};

const WEBSOCKET_CONNECTION_TRACE_REPORT_CONTEXT_FIELD: &str = "websocket_connection_trace_id";
const WEBSOCKET_TURN_INDEX_REPORT_CONTEXT_FIELD: &str = "websocket_turn_index";
const DEFAULT_WEBSOCKET_FIRST_EVENT_TIMEOUT_MS: u64 = 30_000;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum CodexWebSocketTurnObservation {
    Started,
    Terminal(CodexWebSocketTurnOutcome),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum CodexWebSocketTurnTimeoutPhase {
    AwaitingFirstEvent,
    AwaitingTerminal,
}

impl CodexWebSocketTurnTimeoutPhase {
    pub(super) const fn error_code(self) -> &'static str {
        match self {
            Self::AwaitingFirstEvent => "codex_websocket_first_event_timeout",
            Self::AwaitingTerminal => "codex_websocket_turn_timeout",
        }
    }

    pub(super) const fn client_message(self) -> &'static str {
        match self {
            Self::AwaitingFirstEvent => {
                "Codex provider did not emit a response event before the configured timeout"
            }
            Self::AwaitingTerminal => {
                "Codex provider did not finish the response before the configured timeout"
            }
        }
    }

    pub(super) const fn outcome(self) -> CodexWebSocketTurnOutcome {
        match self {
            Self::AwaitingFirstEvent => CodexWebSocketTurnOutcome::first_event_timeout(),
            Self::AwaitingTerminal => CodexWebSocketTurnOutcome::terminal_timeout(),
        }
    }
}

#[derive(Debug, Clone, Copy)]
pub(super) struct CodexWebSocketTurnDeadline {
    pub(super) phase: CodexWebSocketTurnTimeoutPhase,
    pub(super) deadline: Instant,
    pub(super) timeout: Duration,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum CodexWebSocketTurnOutcome {
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

impl CodexWebSocketTurnOutcome {
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

    pub(super) const fn upstream_closed() -> Self {
        Self::Failure {
            status_code: 502,
            reason: "Codex upstream WebSocket closed before provider terminal event",
        }
    }

    pub(super) const fn upstream_receive_failed() -> Self {
        Self::Failure {
            status_code: 502,
            reason: "Codex upstream WebSocket receive failed before provider terminal event",
        }
    }

    pub(super) const fn upstream_send_failed() -> Self {
        Self::Failure {
            status_code: 502,
            reason: "gateway could not forward response.create to the Codex upstream",
        }
    }

    pub(super) const fn upstream_connect_failed(reason: &'static str) -> Self {
        Self::Failure {
            status_code: 502,
            reason,
        }
    }

    pub(super) const fn first_event_timeout() -> Self {
        Self::Failure {
            status_code: 504,
            reason: "Codex upstream WebSocket did not emit a response event before timeout",
        }
    }

    pub(super) const fn terminal_timeout() -> Self {
        Self::Failure {
            status_code: 504,
            reason: "Codex upstream WebSocket did not finish the response before timeout",
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
}

pub(super) struct CodexWebSocketTurn {
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
    upstream_bytes: u64,
    first_event_elapsed_ms: Option<u64>,
    first_event_timeout: Duration,
    terminal_timeout: Duration,
}

pub(super) fn prepare_codex_websocket_turn_decision(
    template: &AiExecutionDecision,
    request_id: String,
    reuse_selected_candidate: bool,
    client_event: &Value,
    provider_event: &Value,
    connection_trace_id: &str,
    turn_index: u64,
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
        connection_trace_id,
        turn_index,
    ));
    decision
}

pub(super) async fn begin_codex_websocket_turn(
    state: &AppState,
    parts: &http::request::Parts,
    decision: AiExecutionDecision,
    client_event: &Value,
) -> Result<CodexWebSocketTurn, GatewayError> {
    let attempt =
        build_openai_responses_stream_plan_from_decision(parts, client_event, decision, false)?
            .ok_or_else(|| {
                GatewayError::Internal(
                    "Codex WebSocket request could not build a usage/audit stream plan".to_string(),
                )
            })?;
    let mut plan = attempt.plan;
    let (first_event_timeout, terminal_timeout) =
        resolve_codex_websocket_turn_timeouts(plan.timeouts.as_ref());
    let report_kind = attempt.report_kind.ok_or_else(|| {
        GatewayError::Internal(
            "Codex WebSocket request is missing an execution report kind".to_string(),
        )
    })?;
    let mut report_context = attempt.report_context;
    ensure_execution_request_candidate_slot(state, &mut plan, &mut report_context).await;

    let lifecycle_seed = build_lifecycle_usage_seed(&plan, report_context.as_ref());
    let usage_data = state.data.as_ref().clone();
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

    Ok(CodexWebSocketTurn {
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
        upstream_bytes: 0,
        first_event_elapsed_ms: None,
        first_event_timeout,
        terminal_timeout,
    })
}

impl CodexWebSocketTurn {
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

    pub(super) fn deadline(&self) -> CodexWebSocketTurnDeadline {
        let (phase, timeout) = if self.first_event_elapsed_ms.is_some() {
            (
                CodexWebSocketTurnTimeoutPhase::AwaitingTerminal,
                self.terminal_timeout,
            )
        } else {
            (
                CodexWebSocketTurnTimeoutPhase::AwaitingFirstEvent,
                self.first_event_timeout.min(self.terminal_timeout),
            )
        };
        CodexWebSocketTurnDeadline {
            phase,
            deadline: self.started_at + timeout,
            timeout,
        }
    }

    pub(super) fn observe_upstream_text(
        &mut self,
        text: &str,
    ) -> Option<CodexWebSocketTurnObservation> {
        self.upstream_bytes = self.upstream_bytes.saturating_add(text.len() as u64);
        if self.first_event_elapsed_ms.is_none() {
            self.first_event_elapsed_ms = Some(elapsed_ms(self.started_at));
        }

        let event = match serde_json::from_str::<Value>(text) {
            Ok(event) => event,
            Err(_) => {
                self.capture_sse_event(&json!({
                    "type": "error",
                    "error": {
                        "type": "gateway_protocol_error",
                        "message": "Codex upstream WebSocket event was not valid JSON"
                    }
                }));
                self.observer
                    .disable_with_error("Codex upstream WebSocket event was not valid JSON");
                return None;
            }
        };

        self.capture_sse_event(&event);
        let fallback_context = json!({
            "provider_api_format": "openai:responses",
            "client_api_format": "openai:responses",
        });
        let report_context = self.report_context.as_ref().unwrap_or(&fallback_context);
        if let Err(error) = self
            .observer
            .push_line(report_context, websocket_event_as_sse_line(&event))
        {
            self.observer.disable_with_error(error.to_string());
        }

        let event_type = event
            .get("type")
            .and_then(Value::as_str)
            .unwrap_or_default();
        if let Some(outcome) = provider_terminal_outcome(event_type, &event) {
            return Some(CodexWebSocketTurnObservation::Terminal(outcome));
        }
        if matches!(
            event_type,
            "response.created" | "response.in_progress" | "response.queued"
        ) {
            return Some(CodexWebSocketTurnObservation::Started);
        }
        None
    }

    pub(super) async fn mark_stream_started(&mut self, state: &AppState) {
        if self.stream_started {
            return;
        }
        self.stream_started = true;
        let lifecycle_seed = build_lifecycle_usage_seed(&self.plan, self.report_context.as_ref());
        let telemetry = self.telemetry();
        state.usage_runtime.record_stream_started(
            state.data.as_ref(),
            &lifecycle_seed,
            200,
            Some(&telemetry),
        );
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
        )
        .await;
    }

    async fn finalize(mut self, state: &AppState, outcome: CodexWebSocketTurnOutcome) {
        let summary = self.finish_summary(outcome);
        let cancelled = outcome.cancelled();
        let status_code = outcome.status_code();
        let missing_terminal = !cancelled && !summary.observed_finish;
        let failed = !cancelled
            && (status_code >= 400 || summary.parser_error.is_some() || missing_terminal);
        let telemetry = Some(self.telemetry());
        let (body_base64, body_state) =
            encode_stream_capture(&self.provider_capture, self.provider_capture_truncated);
        let payload = GatewayStreamReportRequest {
            trace_id: self.trace_id.clone(),
            report_kind: self.report_kind,
            report_context: self.report_context,
            status_code,
            headers: self.provider_headers,
            provider_body_base64: body_base64.clone(),
            provider_body_state: body_state,
            client_body_base64: body_base64,
            client_body_state: body_state,
            terminal_summary: Some(summary.clone()),
            telemetry,
        };

        let context_seed =
            build_terminal_usage_context_seed(&self.plan, payload.report_context.as_ref());
        let payload_seed = build_stream_terminal_usage_payload_seed(&payload);
        state.usage_runtime.record_stream_terminal(
            state.data.as_ref(),
            context_seed,
            payload_seed,
            cancelled,
        );

        let (error_type, error_message) = if cancelled {
            (
                Some("websocket_cancelled".to_string()),
                Some(outcome_reason(outcome).to_string()),
            )
        } else if missing_terminal {
            (
                Some("stream_missing_terminal_event".to_string()),
                Some(summary.parser_error.clone().unwrap_or_else(|| {
                    "Codex upstream WebSocket ended before a provider terminal event".to_string()
                })),
            )
        } else if failed {
            (
                Some("stream_terminal_error".to_string()),
                summary
                    .parser_error
                    .clone()
                    .or_else(|| Some(outcome_reason(outcome).to_string())),
            )
        } else {
            (None, None)
        };
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
        )
        .await;

        // The normal execution runtime does not submit a stream report after a
        // downstream disconnect either. The terminal usage record above still
        // captures cancellation without applying provider-success side effects.
        if !cancelled {
            if let Err(error) = submit_stream_report(state, payload).await {
                warn!(
                    event_name = "codex_websocket_execution_report_submit_failed",
                    log_type = "ops",
                    transport = "websocket",
                    websocket = true,
                    trace_id = %self.trace_id,
                    error = ?error,
                    "gateway failed to submit Codex WebSocket terminal report"
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
        outcome: CodexWebSocketTurnOutcome,
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
            summary.parser_error =
                Some("Codex upstream WebSocket ended before a provider terminal event".to_string());
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

pub(super) fn spawn_codex_websocket_turn_finalization(
    state: AppState,
    turn: CodexWebSocketTurn,
    outcome: CodexWebSocketTurnOutcome,
) {
    tokio::spawn(async move {
        turn.finalize(&state, outcome).await;
    });
}

fn prepare_websocket_report_context(
    report_context: Option<Value>,
    request_id: &str,
    reuse_selected_candidate: bool,
    client_event: &Value,
    connection_trace_id: &str,
    turn_index: u64,
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
        object.remove("candidate_id");
    }
    if !object
        .get("original_request_body")
        .is_some_and(Value::is_null)
    {
        object.insert("original_request_body".to_string(), client_event.clone());
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
        WEBSOCKET_TRANSPORT_METADATA_KEY.to_string(),
        Value::String("responses".to_string()),
    );
    Value::Object(object)
}

fn provider_terminal_outcome(event_type: &str, event: &Value) -> Option<CodexWebSocketTurnOutcome> {
    match event_type {
        "response.completed" | "response.incomplete" => {
            Some(CodexWebSocketTurnOutcome::ProviderTerminal {
                status_code: websocket_event_status_code(event, 200),
                cancelled: false,
            })
        }
        "response.cancelled" => Some(CodexWebSocketTurnOutcome::ProviderTerminal {
            status_code: 499,
            cancelled: true,
        }),
        "response.failed" => Some(CodexWebSocketTurnOutcome::ProviderTerminal {
            status_code: websocket_event_status_code(event, 200),
            cancelled: false,
        }),
        "error" => Some(CodexWebSocketTurnOutcome::ProviderTerminal {
            status_code: websocket_event_status_code(event, 502),
            cancelled: false,
        }),
        _ => None,
    }
}

fn websocket_event_status_code(event: &Value, default: u16) -> u16 {
    event
        .get("status")
        .or_else(|| {
            event
                .get("response")
                .and_then(|response| response.get("status_code"))
        })
        .and_then(Value::as_u64)
        .and_then(|value| u16::try_from(value).ok())
        .filter(|value| *value > 0)
        .unwrap_or(default)
}

fn resolve_codex_websocket_turn_timeouts(
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

fn outcome_reason(outcome: CodexWebSocketTurnOutcome) -> String {
    match outcome {
        CodexWebSocketTurnOutcome::ProviderTerminal {
            cancelled: true, ..
        } => "Codex provider cancelled the response".to_string(),
        CodexWebSocketTurnOutcome::ProviderTerminal {
            cancelled: false, ..
        } => "Codex provider returned a terminal response event".to_string(),
        CodexWebSocketTurnOutcome::Cancelled { reason }
        | CodexWebSocketTurnOutcome::Failure { reason, .. } => reason.to_string(),
    }
}

fn websocket_event_as_sse_line(event: &Value) -> Vec<u8> {
    let payload = serde_json::to_string(event).unwrap_or_else(|_| {
        json!({
            "type": "error",
            "error": {
                "type": "gateway_protocol_error",
                "message": "Codex upstream WebSocket event could not be serialized"
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

#[cfg(test)]
mod tests {
    use std::time::{Duration, Instant};

    use aether_contracts::ExecutionTimeouts;
    use serde_json::json;

    use super::{
        prepare_websocket_report_context, provider_terminal_outcome,
        resolve_codex_websocket_turn_timeouts, websocket_event_as_sse_line,
        CodexWebSocketTurnDeadline, CodexWebSocketTurnOutcome, CodexWebSocketTurnTimeoutPhase,
    };

    #[test]
    fn followup_context_uses_a_fresh_request_and_candidate() {
        let context = prepare_websocket_report_context(
            Some(
                json!({"request_id":"connection","candidate_id":"candidate","original_request_body":{"model":"public"}}),
            ),
            "turn-2",
            false,
            &json!({"type":"response.create","model":"public"}),
            "connection",
            2,
        );
        assert_eq!(context["request_id"], "turn-2");
        assert!(context.get("candidate_id").is_none());
        assert_eq!(context["original_request_body"]["type"], "response.create");
        assert_eq!(context["websocket_mode"], true);
        assert_eq!(context["websocket_transport"], "responses");
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
            "connection",
            2,
        );

        assert_eq!(context["request_id"], "turn-2");
        assert_eq!(context["candidate_id"], "terra-candidate");
        assert_eq!(context["original_request_body"]["model"], "gpt-5.6-terra");
        assert_eq!(context["websocket_mode"], true);
    }

    #[test]
    fn completed_event_is_captured_as_a_responses_sse_terminal_event() {
        let event = json!({"type":"response.completed","response":{"usage":{"input_tokens":3,"output_tokens":5,"total_tokens":8}}});
        let outcome = provider_terminal_outcome("response.completed", &event);
        assert_eq!(
            outcome,
            Some(CodexWebSocketTurnOutcome::ProviderTerminal {
                status_code: 200,
                cancelled: false
            })
        );
        let capture = String::from_utf8(websocket_event_as_sse_line(&event))
            .expect("capture should be UTF-8");
        assert_eq!(capture, format!("data: {event}\n\n"));
    }

    #[test]
    fn turn_timeouts_reuse_provider_first_byte_and_request_deadlines() {
        let (first_event, terminal) =
            resolve_codex_websocket_turn_timeouts(Some(&ExecutionTimeouts {
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
        let deadline = CodexWebSocketTurnDeadline {
            phase: CodexWebSocketTurnTimeoutPhase::AwaitingFirstEvent,
            deadline: started_at + first_event.min(terminal),
            timeout: first_event.min(terminal),
        };

        assert_eq!(
            deadline.phase,
            CodexWebSocketTurnTimeoutPhase::AwaitingFirstEvent
        );
        assert_eq!(deadline.timeout, Duration::from_secs(10));
        assert_eq!(deadline.deadline, started_at + Duration::from_secs(10));
    }
}
