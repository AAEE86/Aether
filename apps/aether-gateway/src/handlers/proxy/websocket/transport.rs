//! Upstream WebSocket handshake and frame conversion utilities.
//!
//! These helpers intentionally do not parse messages.  A protocol adapter is
//! responsible for deciding when and what to send, while this module owns the
//! HTTP-to-WebSocket transport conversion and provider transport profile.

use std::collections::BTreeMap;
use std::time::Duration;

use axum::extract::ws::{CloseFrame as AxumCloseFrame, Message as AxumWsMessage, WebSocket};
use axum::http::header::{
    ACCEPT, ACCEPT_ENCODING, CONNECTION, CONTENT_ENCODING, CONTENT_LENGTH, CONTENT_TYPE, HOST,
    TRANSFER_ENCODING, UPGRADE,
};
use axum::http::HeaderMap;
use futures_util::SinkExt;
use serde_json::json;
use url::Url;
use wreq::ws::message::{CloseFrame as WreqCloseFrame, Message as WreqWsMessage};

use crate::ai_serving::AiExecutionDecision;
use crate::execution_runtime::transport::{
    build_browser_wreq_client, build_request_headers, ExecutionTransportControls,
};
use crate::handlers::proxy::websocket::session::WebSocketSessionLimits;

#[derive(Clone, Copy)]
pub(crate) struct UpstreamWebSocketErrorCodes {
    pub(crate) upstream_url_missing: &'static str,
    pub(crate) upstream_url_invalid: &'static str,
    pub(crate) headers_invalid: &'static str,
    pub(crate) client_build_failed: &'static str,
    pub(crate) proxy_invalid: &'static str,
    pub(crate) tunnel_proxy_unsupported: &'static str,
    pub(crate) handshake_failed: &'static str,
    pub(crate) upgrade_rejected: &'static str,
    pub(crate) upgrade_failed: &'static str,
}

pub(crate) struct UpstreamWebSocketConnection {
    pub(crate) socket: wreq::ws::WebSocket,
    pub(crate) response_headers: BTreeMap<String, String>,
}

pub(crate) async fn connect_upstream_websocket(
    decision: &AiExecutionDecision,
    limits: WebSocketSessionLimits,
    errors: UpstreamWebSocketErrorCodes,
) -> Result<UpstreamWebSocketConnection, &'static str> {
    let upstream_url = decision
        .upstream_url
        .as_deref()
        .ok_or(errors.upstream_url_missing)?;
    let upstream_url = websocket_upstream_url(upstream_url, errors.upstream_url_invalid)?;
    let headers =
        websocket_handshake_headers(&decision.provider_request_headers, errors.headers_invalid)?;
    let client = build_websocket_client(decision, errors)?;
    let response = client
        .websocket(upstream_url.as_str())
        .headers(headers)
        .max_frame_size(limits.max_frame_size)
        .max_message_size(limits.max_message_size)
        .send()
        .await
        .map_err(|_| errors.handshake_failed)?;
    if response.status().as_u16() != 101 {
        return Err(errors.upgrade_rejected);
    }
    let response_headers = websocket_response_headers(response.headers());
    let socket = response
        .into_websocket()
        .await
        .map_err(|_| errors.upgrade_failed)?;
    Ok(UpstreamWebSocketConnection {
        socket,
        response_headers,
    })
}

fn websocket_response_headers(headers: &HeaderMap) -> BTreeMap<String, String> {
    headers
        .iter()
        .filter_map(|(name, value)| {
            value
                .to_str()
                .ok()
                .map(|value| (name.as_str().to_string(), value.to_string()))
        })
        .collect()
}

pub(crate) fn websocket_upstream_url(
    raw: &str,
    invalid_code: &'static str,
) -> Result<Url, &'static str> {
    let mut url = Url::parse(raw).map_err(|_| invalid_code)?;
    if url.host_str().is_none() || !url.username().is_empty() || url.password().is_some() {
        return Err(invalid_code);
    }
    let websocket_scheme = match url.scheme() {
        "https" => "wss",
        "http" => "ws",
        "wss" | "ws" => return Ok(url),
        _ => return Err(invalid_code),
    };
    url.set_scheme(websocket_scheme).map_err(|_| invalid_code)?;
    Ok(url)
}

pub(crate) fn websocket_handshake_headers(
    provider_headers: &BTreeMap<String, String>,
    invalid_code: &'static str,
) -> Result<HeaderMap, &'static str> {
    let mut headers =
        build_request_headers(provider_headers, None, false).map_err(|_| invalid_code)?;
    for header in [
        ACCEPT,
        ACCEPT_ENCODING,
        CONNECTION,
        CONTENT_ENCODING,
        CONTENT_LENGTH,
        CONTENT_TYPE,
        HOST,
        TRANSFER_ENCODING,
        UPGRADE,
    ] {
        headers.remove(header);
    }
    Ok(headers)
}

fn build_websocket_client(
    decision: &AiExecutionDecision,
    errors: UpstreamWebSocketErrorCodes,
) -> Result<wreq::Client, &'static str> {
    let timeouts = websocket_timeouts(decision);
    if let Some(profile) = decision.transport_profile.as_ref() {
        return build_browser_wreq_client(
            timeouts.as_ref(),
            decision.proxy.as_ref(),
            profile,
            ExecutionTransportControls::default(),
            false,
        )
        .map_err(|_| errors.client_build_failed);
    }

    let mut builder = wreq::Client::builder();
    if let Some(connect_ms) = timeouts.as_ref().and_then(|timeouts| timeouts.connect_ms) {
        builder = builder.connect_timeout(Duration::from_millis(connect_ms));
    }
    if let Some(proxy) = decision
        .proxy
        .as_ref()
        .filter(|proxy| proxy.enabled != Some(false))
    {
        if let Some(proxy_url) = proxy
            .url
            .as_deref()
            .map(str::trim)
            .filter(|url| !url.is_empty())
        {
            let proxy = wreq::Proxy::all(proxy_url).map_err(|_| errors.proxy_invalid)?;
            builder = builder.proxy(proxy);
        } else if proxy.node_id.is_some() || proxy.mode.as_deref() == Some("tunnel") {
            return Err(errors.tunnel_proxy_unsupported);
        }
    }
    builder.build().map_err(|_| errors.client_build_failed)
}

pub(crate) fn websocket_timeouts(
    decision: &AiExecutionDecision,
) -> Option<aether_contracts::ExecutionTimeouts> {
    let mut timeouts = decision.timeouts.clone()?;
    timeouts.read_ms = None;
    timeouts.first_byte_ms = None;
    timeouts.total_ms = None;
    Some(timeouts)
}

pub(crate) fn upstream_message_to_client(message: WreqWsMessage) -> AxumWsMessage {
    match message {
        WreqWsMessage::Text(text) => AxumWsMessage::Text(text.to_string().into()),
        WreqWsMessage::Binary(data) => AxumWsMessage::Binary(data),
        WreqWsMessage::Ping(data) => AxumWsMessage::Ping(data),
        WreqWsMessage::Pong(data) => AxumWsMessage::Pong(data),
        WreqWsMessage::Close(frame) => AxumWsMessage::Close(frame.map(|frame| AxumCloseFrame {
            code: frame.code.into(),
            reason: frame.reason.to_string().into(),
        })),
    }
}

pub(crate) fn client_close_to_upstream(frame: Option<AxumCloseFrame>) -> WreqWsMessage {
    WreqWsMessage::Close(frame.map(|frame| WreqCloseFrame {
        code: frame.code.into(),
        reason: frame.reason.to_string().into(),
    }))
}

/// Builds a Responses WebSocket error event in the shape understood by the
/// official client implementations.  The status is part of the event body,
/// not the WebSocket handshake, because the connection is already upgraded.
pub(crate) fn responses_websocket_error_event(
    status: u16,
    error_type: &str,
    code: &str,
    message: &str,
) -> serde_json::Value {
    json!({
        "type": "error",
        "status": status,
        "error": {
            "type": error_type,
            "code": code,
            "message": message,
        },
    })
}

pub(crate) async fn send_responses_websocket_error(
    client_socket: &mut WebSocket,
    status: u16,
    error_type: &str,
    code: &str,
    message: &str,
) {
    let event = responses_websocket_error_event(status, error_type, code, message);
    let _ = client_socket
        .send(AxumWsMessage::Text(event.to_string().into()))
        .await;
}

pub(crate) async fn send_gateway_error(client_socket: &mut WebSocket, code: &str, message: &str) {
    send_responses_websocket_error(client_socket, 400, "gateway_error", code, message).await;
}

pub(crate) async fn close_client_socket(client_socket: &mut WebSocket, code: u16, reason: &str) {
    let _ = client_socket
        .send(AxumWsMessage::Close(Some(AxumCloseFrame {
            code,
            reason: reason.to_string().into(),
        })))
        .await;
}

#[cfg(test)]
mod tests {
    use super::{responses_websocket_error_event, websocket_upstream_url};

    #[test]
    fn builds_a_client_compatible_responses_error_event() {
        let event = responses_websocket_error_event(
            400,
            "invalid_request_error",
            "previous_response_not_found",
            "Previous response was not found.",
        );

        assert_eq!(event["type"], "error");
        assert_eq!(event["status"], 400);
        assert_eq!(event["error"]["type"], "invalid_request_error");
        assert_eq!(event["error"]["code"], "previous_response_not_found");
        assert_eq!(
            event["error"]["message"],
            "Previous response was not found."
        );
    }

    #[test]
    fn maps_http_url_to_websocket_url_without_losing_path_or_query() {
        let url = websocket_upstream_url(
            "https://example.test/backend-api/codex/responses?x=1",
            "invalid",
        )
        .expect("URL should be converted");
        assert_eq!(
            url.as_str(),
            "wss://example.test/backend-api/codex/responses?x=1"
        );
    }

    #[test]
    fn rejects_upstream_url_with_credentials() {
        assert!(websocket_upstream_url("https://token@example.test/responses", "invalid").is_err());
    }
}
