//! Codex-specific extensions for the standard Responses WebSocket session.

use async_trait::async_trait;
use serde_json::{Map, Value};

use super::super::adapter::{
    is_standard_responses_event, ResponsesWebSocketAdapterObservation,
    ResponsesWebSocketDrainDirective, ResponsesWebSocketExclusionIdentity,
    ResponsesWebSocketProtocolAdapter, ResponsesWebSocketRebindSafety,
};
use crate::ai_serving::AiExecutionDecision;
use crate::clock::current_unix_secs;
use crate::handlers::proxy::websocket::transport::UpstreamWebSocketErrorCodes;
use crate::orchestration::{
    codex_account_id_from_headers, codex_quota_exhaustion_reset_at,
    sync_codex_websocket_quota_metadata, ResponsesWebSocketAdapter,
};
use crate::AppState;

const CODEX_WEBSOCKET_LOG_TARGET: &str = "aether_gateway::handlers::proxy::codex_ws";
const CODEX_WEBSOCKET_RATE_LIMITS_REPORT_CONTEXT_FIELD: &str = "codex_websocket_rate_limits";

const CODEX_UPSTREAM_WEBSOCKET_ERRORS: UpstreamWebSocketErrorCodes = UpstreamWebSocketErrorCodes {
    upstream_url_missing: "codex_upstream_url_missing",
    upstream_url_invalid: "codex_upstream_url_invalid",
    headers_invalid: "codex_websocket_headers_invalid",
    client_build_failed: "codex_websocket_client_build_failed",
    proxy_invalid: "codex_websocket_proxy_invalid",
    tunnel_proxy_unsupported: "codex_websocket_tunnel_proxy_unsupported",
    handshake_failed: "codex_websocket_handshake_failed",
    upgrade_rejected: "codex_websocket_upgrade_rejected",
    upgrade_failed: "codex_websocket_upgrade_failed",
};

pub(crate) static CODEX_RESPONSES_WEBSOCKET_ADAPTER: CodexResponsesWebSocketAdapter =
    CodexResponsesWebSocketAdapter;

pub(crate) struct CodexResponsesWebSocketAdapter;

#[async_trait]
impl ResponsesWebSocketProtocolAdapter for CodexResponsesWebSocketAdapter {
    fn kind(&self) -> ResponsesWebSocketAdapter {
        ResponsesWebSocketAdapter::Codex
    }

    fn upstream_errors(&self) -> UpstreamWebSocketErrorCodes {
        CODEX_UPSTREAM_WEBSOCKET_ERRORS
    }

    fn decorate_turn_report_context(&self, report_context: &mut Option<Value>, event: &Value) {
        let Some(rate_limits) = parse_codex_rate_limits(event) else {
            return;
        };
        let context = report_context.get_or_insert_with(|| Value::Object(Map::new()));
        let Some(context) = context.as_object_mut() else {
            return;
        };
        context.insert(
            CODEX_WEBSOCKET_RATE_LIMITS_REPORT_CONTEXT_FIELD.to_string(),
            rate_limits,
        );
    }

    fn observes_upstream_events(&self) -> bool {
        true
    }

    fn rebind_safety_for_upstream_event(&self, event: &Value) -> ResponsesWebSocketRebindSafety {
        if let Some(chunks) = event.get("chunks").and_then(Value::as_array) {
            if chunks.is_empty() {
                return ResponsesWebSocketRebindSafety::Unsafe {
                    reason: "unrecognized_upstream_event",
                };
            }
            return chunks
                .iter()
                .map(codex_direct_rebind_safety)
                .find(|safety| matches!(safety, ResponsesWebSocketRebindSafety::Unsafe { .. }))
                .unwrap_or(ResponsesWebSocketRebindSafety::Safe);
        }
        codex_direct_rebind_safety(event)
    }

    fn observe_upstream_event(
        &self,
        event: &Value,
    ) -> Option<ResponsesWebSocketAdapterObservation> {
        let rate_limits = parse_codex_rate_limits(event)?;
        let exhausted =
            aether_admin::provider::quota::codex_rate_limit_metadata_exhausted(&rate_limits);
        let retry_exclusion_until_unix_secs =
            codex_quota_exhaustion_reset_at(&rate_limits, current_unix_secs());
        Some(ResponsesWebSocketAdapterObservation {
            drain: exhausted.then_some(ResponsesWebSocketDrainDirective {
                error_code: "codex_account_quota_exhausted",
                retry_current_turn: true,
                retry_exclusion_until_unix_secs,
            }),
            quota_metadata: Some(rate_limits),
        })
    }

    fn exhaustion_exclusion_identity(
        &self,
        decision: &AiExecutionDecision,
    ) -> Option<ResponsesWebSocketExclusionIdentity> {
        Some(ResponsesWebSocketExclusionIdentity {
            account_id: codex_account_id_from_headers(&decision.provider_request_headers)
                .map(str::to_string),
        })
    }

    async fn persist_upstream_observation(
        &self,
        state: &AppState,
        trace_id: &str,
        report_context: Option<&Value>,
        observation: ResponsesWebSocketAdapterObservation,
    ) {
        let Some(rate_limits) = observation.quota_metadata else {
            return;
        };
        if let Err(error) =
            sync_codex_websocket_quota_metadata(state, report_context, rate_limits).await
        {
            tracing::warn!(
                target: CODEX_WEBSOCKET_LOG_TARGET,
                event_name = "codex_websocket_quota_sync_failed",
                log_type = "ops",
                transport = "websocket",
                websocket = true,
                trace_id = %trace_id,
                error = ?error,
                "gateway failed to persist Codex WebSocket quota metadata"
            );
        }
    }
}

fn codex_direct_rebind_safety(event: &Value) -> ResponsesWebSocketRebindSafety {
    let event_type = event
        .get("type")
        .and_then(Value::as_str)
        .unwrap_or_default();
    if matches!(event_type, "codex.rate_limits" | "codex.response.metadata") {
        // Codex emits these as pre-response advisory metadata. They do
        // not create a public `response.*` object, so a replacement
        // upstream can safely emit its own current snapshot.
        return ResponsesWebSocketRebindSafety::Safe;
    }
    if event_type == "error" && parse_codex_rate_limits(event).is_some() {
        // The quota error is withheld from the client when the shared
        // session successfully rebinds, therefore it remains replay-safe.
        return ResponsesWebSocketRebindSafety::Safe;
    }
    let reason = if is_standard_responses_event(event) {
        "standard_response_event"
    } else {
        "unrecognized_upstream_event"
    };
    ResponsesWebSocketRebindSafety::Unsafe { reason }
}

fn parse_codex_rate_limits(event: &Value) -> Option<Value> {
    aether_admin::provider::quota::parse_codex_websocket_rate_limits_response(
        event,
        current_unix_secs(),
    )
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::{
        CodexResponsesWebSocketAdapter, ResponsesWebSocketProtocolAdapter,
        ResponsesWebSocketRebindSafety,
    };

    #[test]
    fn codex_rate_limit_chunk_is_kept_for_the_terminal_report() {
        let adapter = CodexResponsesWebSocketAdapter;
        assert!(adapter.observes_upstream_events());
        let mut context = Some(json!({"key_id": "codex-key"}));
        adapter.decorate_turn_report_context(
            &mut context,
            &json!({
                "chunks": [{
                    "type": "codex.rate_limits",
                    "plan_type": "free",
                    "rate_limits": {
                        "allowed": true,
                        "limit_reached": false,
                        "primary": {
                            "used_percent": 91,
                            "window_minutes": 43200,
                            "reset_after_seconds": 2590791
                        }
                    }
                }]
            }),
        );

        assert_eq!(
            context.as_ref().and_then(
                |context| context.pointer("/codex_websocket_rate_limits/primary_used_percent")
            ),
            Some(&json!(91.0))
        );
    }

    #[test]
    fn usage_limit_error_is_kept_for_the_terminal_report() {
        let adapter = CodexResponsesWebSocketAdapter;
        let mut context = Some(json!({"key_id": "codex-key"}));
        adapter.decorate_turn_report_context(
            &mut context,
            &json!({
                "type": "error",
                "error": {
                    "type": "usage_limit_reached",
                    "plan_type": "free",
                    "resets_at": 1_787_274_385u64,
                },
                "status_code": 429,
                "headers": {
                    "X-Codex-Primary-Used-Percent": "100",
                    "X-Codex-Primary-Reset-At": "1787274385",
                },
            }),
        );

        assert_eq!(
            context
                .as_ref()
                .and_then(|context| context.pointer("/codex_websocket_rate_limits/allowed")),
            Some(&json!(false))
        );
        assert_eq!(
            context.as_ref().and_then(|context| {
                context.pointer("/codex_websocket_rate_limits/primary_used_percent")
            }),
            Some(&json!(100.0))
        );
    }

    #[test]
    fn only_known_codex_pre_response_metadata_is_safe_to_rebind() {
        let adapter = CodexResponsesWebSocketAdapter;

        assert_eq!(
            adapter.rebind_safety_for_upstream_event(&json!({
                "type": "codex.rate_limits",
                "rate_limits": {"allowed": true}
            })),
            ResponsesWebSocketRebindSafety::Safe
        );
        assert_eq!(
            adapter.rebind_safety_for_upstream_event(&json!({
                "type": "codex.response.metadata"
            })),
            ResponsesWebSocketRebindSafety::Safe
        );
        assert_eq!(
            adapter.rebind_safety_for_upstream_event(&json!({
                "chunks": [
                    {"type": "codex.rate_limits", "rate_limits": {"allowed": true}},
                    {"type": "codex.response.metadata"}
                ]
            })),
            ResponsesWebSocketRebindSafety::Safe
        );
        assert_eq!(
            adapter.rebind_safety_for_upstream_event(&json!({
                "type": "response.created"
            })),
            ResponsesWebSocketRebindSafety::Unsafe {
                reason: "standard_response_event"
            }
        );
        assert_eq!(
            adapter.rebind_safety_for_upstream_event(&json!({
                "type": "codex.unknown"
            })),
            ResponsesWebSocketRebindSafety::Unsafe {
                reason: "unrecognized_upstream_event"
            }
        );
    }
}
