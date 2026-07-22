//! Codex-specific extensions for the standard Responses WebSocket session.

use async_trait::async_trait;
use serde_json::{Map, Value};

use super::super::adapter::{ResponsesWebSocketDrainDirective, ResponsesWebSocketProtocolAdapter};
use crate::clock::current_unix_secs;
use crate::handlers::proxy::websocket::transport::UpstreamWebSocketErrorCodes;
use crate::orchestration::{sync_codex_websocket_quota_metadata, ResponsesWebSocketAdapter};
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

    async fn observe_upstream_event(
        &self,
        state: &AppState,
        trace_id: &str,
        report_context: Option<&Value>,
        event: &Value,
    ) -> Option<ResponsesWebSocketDrainDirective> {
        let rate_limits = parse_codex_rate_limits(event)?;
        let exhausted =
            aether_admin::provider::quota::codex_rate_limit_metadata_exhausted(&rate_limits);
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
        if !exhausted {
            return None;
        }
        tracing::info!(
            target: CODEX_WEBSOCKET_LOG_TARGET,
            event_name = "codex_websocket_account_quota_exhausted",
            log_type = "event",
            transport = "websocket",
            websocket = true,
            trace_id = %trace_id,
            "gateway will detach the exhausted Codex upstream after the active response"
        );
        Some(ResponsesWebSocketDrainDirective {
            error_code: "codex_account_quota_exhausted",
            retry_current_turn: true,
        })
    }
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

    use super::{CodexResponsesWebSocketAdapter, ResponsesWebSocketProtocolAdapter};

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
}
