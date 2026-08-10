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
use futures_util::{SinkExt, TryFutureExt};
use serde_json::json;
use url::Url;
use wreq::ws::message::{CloseFrame as WreqCloseFrame, Message as WreqWsMessage};

use crate::ai_serving::AiExecutionDecision;
use crate::execution_runtime::transport::{
    build_browser_wreq_client, build_request_headers, ExecutionTransportControls,
};
use crate::handlers::proxy::websocket::responses::ResponsesPublicEventSequence;
use crate::handlers::proxy::websocket::session::{
    WebSocketSessionLimits, RELAY_WRITE_TIMEOUT, TEARDOWN_WRITE_TIMEOUT,
};
use aether_contracts::{
    ResolvedTransportProfile, TRANSPORT_BACKEND_BROWSER_WREQ, TRANSPORT_BACKEND_REQWEST_RUSTLS,
};

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
    match websocket_client_backend(decision.transport_profile.as_ref()) {
        Ok(WebSocketClientBackend::BrowserWreq) => {
            return build_browser_wreq_client(
                timeouts.as_ref(),
                decision.proxy.as_ref(),
                decision
                    .transport_profile
                    .as_ref()
                    .expect("browser backend requires a transport profile"),
                ExecutionTransportControls::default(),
                false,
            )
            .map_err(|_| errors.client_build_failed);
        }
        Ok(WebSocketClientBackend::ReqwestRustlsCompatible) => {}
        Err(()) => return Err(errors.client_build_failed),
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

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum WebSocketClientBackend {
    ReqwestRustlsCompatible,
    BrowserWreq,
}

fn websocket_client_backend(
    profile: Option<&ResolvedTransportProfile>,
) -> Result<WebSocketClientBackend, ()> {
    let Some(profile) = profile else {
        return Ok(WebSocketClientBackend::ReqwestRustlsCompatible);
    };
    let backend = profile.backend.trim();
    if backend.eq_ignore_ascii_case(TRANSPORT_BACKEND_REQWEST_RUSTLS) {
        return Ok(WebSocketClientBackend::ReqwestRustlsCompatible);
    }
    if backend.eq_ignore_ascii_case(TRANSPORT_BACKEND_BROWSER_WREQ) {
        return Ok(WebSocketClientBackend::BrowserWreq);
    }
    Err(())
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

/// Why a frame did not reach its peer.  A timeout is reported separately from
/// a socket error because the two describe different peers: one has gone away,
/// the other is still connected but has stopped reading.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum WebSocketWriteError {
    Failed,
    TimedOut,
}

impl WebSocketWriteError {
    pub(crate) const fn as_str(self) -> &'static str {
        match self {
            Self::Failed => "write_failed",
            Self::TimedOut => "write_timeout",
        }
    }
}

/// Relays one frame to the client under [`RELAY_WRITE_TIMEOUT`].
pub(crate) async fn send_client_message(
    client_socket: &mut WebSocket,
    message: AxumWsMessage,
) -> Result<(), WebSocketWriteError> {
    bounded_send(
        RELAY_WRITE_TIMEOUT,
        client_socket.send(message).map_err(|_| ()),
    )
    .await
}

/// Relays one frame without extending an already established batch, turn, or
/// connection deadline. A deadline at or before `now` fails immediately.
pub(crate) async fn send_client_message_until(
    client_socket: &mut WebSocket,
    message: AxumWsMessage,
    deadline: tokio::time::Instant,
) -> Result<(), WebSocketWriteError> {
    bounded_send_until(deadline, client_socket.send(message).map_err(|_| ())).await
}

/// Accepts one public frame into the WebSocket sink without flushing it.
/// Callers commit protocol-visible state immediately after this returns: once
/// `start_send` has accepted the frame, a later cancellation may still flush
/// it through the next writer and must not reuse its sequence number.
pub(crate) async fn feed_client_message_until(
    client_socket: &mut WebSocket,
    message: AxumWsMessage,
    deadline: tokio::time::Instant,
) -> Result<(), WebSocketWriteError> {
    bounded_send_until(deadline, client_socket.feed(message).map_err(|_| ())).await
}

pub(crate) async fn flush_client_messages_until(
    client_socket: &mut WebSocket,
    deadline: tokio::time::Instant,
) -> Result<(), WebSocketWriteError> {
    bounded_send_until(deadline, client_socket.flush().map_err(|_| ())).await
}

async fn bounded_send_until<F>(
    deadline: tokio::time::Instant,
    write: F,
) -> Result<(), WebSocketWriteError>
where
    F: std::future::Future<Output = Result<(), ()>>,
{
    match tokio::time::timeout_at(deadline, write).await {
        Ok(Ok(())) => Ok(()),
        Ok(Err(())) => Err(WebSocketWriteError::Failed),
        Err(_) => Err(WebSocketWriteError::TimedOut),
    }
}

/// Sends one frame to the upstream under [`RELAY_WRITE_TIMEOUT`].
pub(crate) async fn send_upstream_message(
    upstream: &mut wreq::ws::WebSocket,
    message: WreqWsMessage,
) -> Result<(), WebSocketWriteError> {
    bounded_send(RELAY_WRITE_TIMEOUT, upstream.send(message).map_err(|_| ())).await
}

/// Best-effort teardown write.  The caller is already ending the session, so
/// the outcome only matters for keeping the wait bounded.
async fn send_teardown_message<F>(write: F)
where
    F: std::future::Future<Output = Result<(), ()>>,
{
    let _ = bounded_send(TEARDOWN_WRITE_TIMEOUT, write).await;
}

async fn bounded_send<F>(budget: Duration, write: F) -> Result<(), WebSocketWriteError>
where
    F: std::future::Future<Output = Result<(), ()>>,
{
    match tokio::time::timeout(budget, write).await {
        Ok(Ok(())) => Ok(()),
        Ok(Err(())) => Err(WebSocketWriteError::Failed),
        Err(_) => Err(WebSocketWriteError::TimedOut),
    }
}

/// Sends a WebSocket Close frame upstream without waiting on an unresponsive
/// provider.  The socket is dropped by the caller either way.
pub(crate) async fn close_upstream_socket(
    upstream: &mut wreq::ws::WebSocket,
    frame: Option<WreqCloseFrame>,
) {
    send_teardown_message(upstream.send(WreqWsMessage::Close(frame)).map_err(|_| ())).await;
}

/// Builds a Responses WebSocket error event in the shape understood by the
/// official client implementations.  The status is part of the event body,
/// not the WebSocket handshake, because the connection is already upgraded.
pub(crate) fn responses_websocket_error_event(
    status: u16,
    error_type: &str,
    code: &str,
    message: &str,
    param: Option<&str>,
    sequence_number: u64,
) -> serde_json::Value {
    json!({
        "type": "error",
        // The Responses server-event schema is discriminated by `type` and
        // requires these top-level fields. Keep them even for the WebSocket
        // guide's response-style error envelope so typed SDKs can consume the
        // same event without losing the documented status/error details.
        "code": code,
        "message": message,
        "param": param,
        "sequence_number": sequence_number,
        "status": status,
        "error": {
            "type": error_type,
            "code": code,
            "message": message,
            "param": param,
        },
    })
}

pub(crate) async fn send_responses_websocket_error(
    client_socket: &mut WebSocket,
    sequence: &ResponsesPublicEventSequence,
    status: u16,
    error_type: &str,
    code: &str,
    message: &str,
    param: Option<&str>,
) {
    let _ = send_responses_websocket_error_with(
        client_socket,
        sequence,
        status,
        error_type,
        code,
        message,
        param,
    )
    .await;
}

async fn send_responses_websocket_error_with<S>(
    client_socket: &mut S,
    sequence: &ResponsesPublicEventSequence,
    status: u16,
    error_type: &str,
    code: &str,
    message: &str,
    param: Option<&str>,
) -> Result<(), WebSocketWriteError>
where
    S: futures_util::Sink<AxumWsMessage> + Unpin,
{
    // The sequence becomes durable at the sink-acceptance boundary, before
    // flush. `SinkExt::send` combines both phases and is cancellation-ambiguous:
    // a frame can already be buffered even though the future has not returned.
    let reservation = sequence.reserve();
    let event = responses_websocket_error_event(
        status,
        error_type,
        code,
        message,
        param,
        reservation.sequence_number(),
    );
    bounded_send(
        TEARDOWN_WRITE_TIMEOUT,
        client_socket
            .feed(AxumWsMessage::Text(event.to_string().into()))
            .map_err(|_| ()),
    )
    .await?;
    reservation.commit();
    bounded_send(
        TEARDOWN_WRITE_TIMEOUT,
        client_socket.flush().map_err(|_| ()),
    )
    .await
}

pub(crate) async fn send_gateway_error(
    client_socket: &mut WebSocket,
    sequence: &ResponsesPublicEventSequence,
    code: &str,
    message: &str,
) {
    send_gateway_error_with_status(client_socket, sequence, 400, code, message).await;
}

pub(crate) async fn send_gateway_error_with_status(
    client_socket: &mut WebSocket,
    sequence: &ResponsesPublicEventSequence,
    status: u16,
    code: &str,
    message: &str,
) {
    send_responses_websocket_error(
        client_socket,
        sequence,
        status,
        "gateway_error",
        code,
        message,
        None,
    )
    .await;
}

pub(crate) async fn close_client_socket(client_socket: &mut WebSocket, code: u16, reason: &str) {
    send_teardown_message(
        client_socket
            .send(AxumWsMessage::Close(Some(AxumCloseFrame {
                code,
                reason: reason.to_string().into(),
            })))
            .map_err(|_| ()),
    )
    .await;
}

#[cfg(test)]
mod tests {
    use std::pin::Pin;
    use std::task::{Context, Poll};

    use axum::extract::ws::Message as AxumWsMessage;
    use futures_util::Sink;

    use super::{
        bounded_send, bounded_send_until, responses_websocket_error_event,
        send_responses_websocket_error_with, websocket_client_backend, websocket_upstream_url,
        WebSocketClientBackend, WebSocketWriteError, RELAY_WRITE_TIMEOUT, TEARDOWN_WRITE_TIMEOUT,
    };
    use crate::handlers::proxy::websocket::responses::ResponsesPublicEventSequence;
    use aether_contracts::{
        ResolvedTransportProfile, TRANSPORT_BACKEND_BROWSER_WREQ, TRANSPORT_BACKEND_REQWEST_RUSTLS,
    };
    use std::time::Duration;

    enum TestSink {
        Accept,
        FailReady,
        PendingReady,
        FailFlush,
    }

    impl Sink<AxumWsMessage> for TestSink {
        type Error = ();

        fn poll_ready(
            self: Pin<&mut Self>,
            _cx: &mut Context<'_>,
        ) -> Poll<Result<(), Self::Error>> {
            match self.as_ref().get_ref() {
                Self::FailReady => Poll::Ready(Err(())),
                Self::PendingReady => Poll::Pending,
                Self::Accept | Self::FailFlush => Poll::Ready(Ok(())),
            }
        }

        fn start_send(self: Pin<&mut Self>, _item: AxumWsMessage) -> Result<(), Self::Error> {
            Ok(())
        }

        fn poll_flush(
            self: Pin<&mut Self>,
            _cx: &mut Context<'_>,
        ) -> Poll<Result<(), Self::Error>> {
            if matches!(self.as_ref().get_ref(), Self::FailFlush) {
                Poll::Ready(Err(()))
            } else {
                Poll::Ready(Ok(()))
            }
        }

        fn poll_close(
            self: Pin<&mut Self>,
            _cx: &mut Context<'_>,
        ) -> Poll<Result<(), Self::Error>> {
            Poll::Ready(Ok(()))
        }
    }

    #[tokio::test]
    async fn a_peer_that_never_drains_its_window_times_out_instead_of_pinning_the_relay() {
        let stalled = std::future::pending::<Result<(), ()>>();

        let outcome = bounded_send(Duration::from_millis(1), stalled).await;

        assert_eq!(outcome, Err(WebSocketWriteError::TimedOut));
    }

    #[tokio::test]
    async fn a_socket_error_is_reported_separately_from_a_stalled_peer() {
        let outcome = bounded_send(RELAY_WRITE_TIMEOUT, std::future::ready(Err(()))).await;

        assert_eq!(outcome, Err(WebSocketWriteError::Failed));
        assert_eq!(WebSocketWriteError::Failed.as_str(), "write_failed");
        assert_eq!(WebSocketWriteError::TimedOut.as_str(), "write_timeout");
    }

    #[tokio::test]
    async fn a_write_that_completes_within_its_budget_succeeds() {
        let outcome = bounded_send(RELAY_WRITE_TIMEOUT, std::future::ready(Ok::<(), ()>(()))).await;

        assert_eq!(outcome, Ok(()));
    }

    #[tokio::test]
    async fn client_write_respects_an_existing_absolute_deadline() {
        let result = bounded_send_until(
            tokio::time::Instant::now() - Duration::from_millis(1),
            std::future::pending::<Result<(), ()>>(),
        )
        .await;

        assert_eq!(result, Err(WebSocketWriteError::TimedOut));
    }

    #[test]
    fn teardown_writes_are_given_a_shorter_budget_than_relayed_frames() {
        assert!(TEARDOWN_WRITE_TIMEOUT < RELAY_WRITE_TIMEOUT);
    }

    #[test]
    fn builds_a_client_compatible_responses_error_event() {
        let event = responses_websocket_error_event(
            400,
            "invalid_request_error",
            "previous_response_not_found",
            "Previous response was not found.",
            Some("previous_response_id"),
            7,
        );

        assert_eq!(event["type"], "error");
        assert_eq!(event["code"], "previous_response_not_found");
        assert_eq!(event["message"], "Previous response was not found.");
        assert_eq!(event["param"], "previous_response_id");
        assert_eq!(event["sequence_number"], 7);
        assert_eq!(event["status"], 400);
        assert_eq!(event["error"]["type"], "invalid_request_error");
        assert_eq!(event["error"]["code"], "previous_response_not_found");
        assert_eq!(
            event["error"]["message"],
            "Previous response was not found."
        );
        assert_eq!(event["error"]["param"], "previous_response_id");
    }

    #[tokio::test]
    async fn responses_error_sequence_commits_only_after_a_successful_write() {
        let sequence = ResponsesPublicEventSequence::default();
        let mut failed_sink = TestSink::FailReady;
        let failed = send_responses_websocket_error_with(
            &mut failed_sink,
            &sequence,
            502,
            "server_error",
            "responses_websocket_upstream_closed",
            "Provider connection closed unexpectedly",
            None,
        )
        .await;

        assert_eq!(failed, Err(WebSocketWriteError::Failed));
        assert_eq!(sequence.reserve().sequence_number(), 0);

        let mut successful_sink = TestSink::Accept;
        let succeeded = send_responses_websocket_error_with(
            &mut successful_sink,
            &sequence,
            502,
            "server_error",
            "responses_websocket_upstream_closed",
            "Provider connection closed unexpectedly",
            None,
        )
        .await;

        assert_eq!(succeeded, Ok(()));
        assert_eq!(sequence.reserve().sequence_number(), 1);
    }

    #[tokio::test]
    async fn accepted_error_sequence_survives_a_flush_failure() {
        let sequence = ResponsesPublicEventSequence::default();
        let mut sink = TestSink::FailFlush;

        let result = send_responses_websocket_error_with(
            &mut sink,
            &sequence,
            502,
            "server_error",
            "responses_websocket_upstream_closed",
            "Provider connection closed unexpectedly",
            None,
        )
        .await;

        assert_eq!(result, Err(WebSocketWriteError::Failed));
        assert_eq!(sequence.reserve().sequence_number(), 1);
    }

    #[tokio::test]
    async fn cancelling_a_responses_error_write_releases_its_sequence_number() {
        let sequence = ResponsesPublicEventSequence::default();
        let mut sink = TestSink::PendingReady;
        let result = tokio::time::timeout(
            Duration::from_millis(1),
            send_responses_websocket_error_with(
                &mut sink,
                &sequence,
                502,
                "server_error",
                "responses_websocket_upstream_closed",
                "Provider connection closed unexpectedly",
                None,
            ),
        )
        .await;

        assert!(result.is_err());
        assert_eq!(sequence.reserve().sequence_number(), 0);
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

    #[test]
    fn dispatches_websocket_transport_profiles_by_backend() {
        assert_eq!(
            websocket_client_backend(None),
            Ok(WebSocketClientBackend::ReqwestRustlsCompatible)
        );

        let reqwest = ResolvedTransportProfile {
            profile_id: "ordinary-profile".to_string(),
            backend: TRANSPORT_BACKEND_REQWEST_RUSTLS.to_string(),
            ..Default::default()
        };
        assert_eq!(
            websocket_client_backend(Some(&reqwest)),
            Ok(WebSocketClientBackend::ReqwestRustlsCompatible)
        );

        let browser = ResolvedTransportProfile {
            profile_id: "chrome136".to_string(),
            backend: TRANSPORT_BACKEND_BROWSER_WREQ.to_string(),
            ..Default::default()
        };
        assert_eq!(
            websocket_client_backend(Some(&browser)),
            Ok(WebSocketClientBackend::BrowserWreq)
        );

        let unsupported = ResolvedTransportProfile {
            backend: "hyper_rustls".to_string(),
            ..Default::default()
        };
        assert_eq!(websocket_client_backend(Some(&unsupported)), Err(()));
    }
}
