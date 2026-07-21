//! Provider-specific hooks for the standard Responses WebSocket session.

use async_trait::async_trait;
use serde_json::Value;

use super::adapters::CODEX_RESPONSES_WEBSOCKET_ADAPTER;
use crate::handlers::proxy::websocket::transport::UpstreamWebSocketErrorCodes;
use crate::orchestration::ResponsesWebSocketAdapter;
use crate::AppState;

#[derive(Debug, Clone, Copy)]
pub(super) struct ResponsesWebSocketDrainDirective {
    pub(super) error_code: &'static str,
    pub(super) client_message: &'static str,
    pub(super) close_reason: &'static str,
}

/// Boundary between the standard Responses protocol engine and provider
/// behavior. Adapters receive already-planned provider requests; they never
/// own public WebSocket parsing, turn accounting, or model scheduling.
#[async_trait]
pub(super) trait ResponsesWebSocketProtocolAdapter: Send + Sync {
    fn kind(&self) -> ResponsesWebSocketAdapter;

    fn upstream_errors(&self) -> UpstreamWebSocketErrorCodes;

    /// Adds provider-specific metadata to an otherwise standard Responses
    /// stream report. The event payload is never rewritten for the client.
    fn decorate_turn_report_context(&self, report_context: &mut Option<Value>, event: &Value);

    /// Whether this adapter needs the shared session to parse each upstream
    /// text event before normal turn accounting runs.
    fn observes_upstream_events(&self) -> bool;

    /// Lets an adapter react to provider-only events. Returning a directive
    /// asks the shared session to drain after the active standard response.
    async fn observe_upstream_event(
        &self,
        state: &AppState,
        trace_id: &str,
        report_context: Option<&Value>,
        event: &Value,
    ) -> Option<ResponsesWebSocketDrainDirective>;
}

pub(super) fn resolve_responses_websocket_adapter(
    kind: ResponsesWebSocketAdapter,
) -> &'static dyn ResponsesWebSocketProtocolAdapter {
    match kind {
        ResponsesWebSocketAdapter::Standard => &STANDARD_RESPONSES_WEBSOCKET_ADAPTER,
        ResponsesWebSocketAdapter::Codex => &CODEX_RESPONSES_WEBSOCKET_ADAPTER,
    }
}

struct StandardResponsesWebSocketAdapter;

const STANDARD_UPSTREAM_WEBSOCKET_ERRORS: UpstreamWebSocketErrorCodes =
    UpstreamWebSocketErrorCodes {
        upstream_url_missing: "responses_upstream_url_missing",
        upstream_url_invalid: "responses_upstream_url_invalid",
        headers_invalid: "responses_websocket_headers_invalid",
        client_build_failed: "responses_websocket_client_build_failed",
        proxy_invalid: "responses_websocket_proxy_invalid",
        tunnel_proxy_unsupported: "responses_websocket_tunnel_proxy_unsupported",
        handshake_failed: "responses_websocket_handshake_failed",
        upgrade_rejected: "responses_websocket_upgrade_rejected",
        upgrade_failed: "responses_websocket_upgrade_failed",
    };

static STANDARD_RESPONSES_WEBSOCKET_ADAPTER: StandardResponsesWebSocketAdapter =
    StandardResponsesWebSocketAdapter;

#[async_trait]
impl ResponsesWebSocketProtocolAdapter for StandardResponsesWebSocketAdapter {
    fn kind(&self) -> ResponsesWebSocketAdapter {
        ResponsesWebSocketAdapter::Standard
    }

    fn upstream_errors(&self) -> UpstreamWebSocketErrorCodes {
        STANDARD_UPSTREAM_WEBSOCKET_ERRORS
    }

    fn decorate_turn_report_context(&self, _report_context: &mut Option<Value>, _event: &Value) {}

    fn observes_upstream_events(&self) -> bool {
        false
    }

    async fn observe_upstream_event(
        &self,
        _state: &AppState,
        _trace_id: &str,
        _report_context: Option<&Value>,
        _event: &Value,
    ) -> Option<ResponsesWebSocketDrainDirective> {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::{resolve_responses_websocket_adapter, ResponsesWebSocketProtocolAdapter};
    use crate::orchestration::ResponsesWebSocketAdapter;

    #[test]
    fn standard_adapter_has_no_codex_extensions() {
        let adapter = resolve_responses_websocket_adapter(ResponsesWebSocketAdapter::Standard);

        assert_eq!(adapter.kind(), ResponsesWebSocketAdapter::Standard);
        assert!(!adapter.observes_upstream_events());
        assert_eq!(
            adapter.upstream_errors().handshake_failed,
            "responses_websocket_handshake_failed"
        );
    }
}
