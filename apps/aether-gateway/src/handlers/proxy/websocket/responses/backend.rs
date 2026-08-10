//! Provider execution sessions behind the public Responses WebSocket FSM.
//!
//! The public connection loop deals only in JSON provider events and stable
//! session outcomes. Native WebSocket frames, control traffic, and handshake
//! details terminate here so another backend can produce the same inputs
//! without pretending to be a `wreq` socket.

use std::collections::BTreeMap;
use std::time::Duration;

use async_trait::async_trait;
use serde_json::Value;
use wreq::ws::message::Message as WreqWsMessage;

use crate::ai_serving::AiExecutionDecision;
use crate::handlers::proxy::websocket::session::{
    RESPONSES_WEBSOCKET_SESSION_LIMITS, TEARDOWN_WRITE_TIMEOUT,
};
use crate::handlers::proxy::websocket::transport::{
    close_upstream_socket, connect_upstream_websocket, send_upstream_message,
    UpstreamWebSocketErrorCodes,
};
use crate::orchestration::ResponsesWebSocketBackendKind;

#[derive(Debug)]
pub(super) struct ResponsesProviderEvent {
    raw_text: String,
    event: Value,
}

impl ResponsesProviderEvent {
    fn from_text(raw_text: String) -> Result<Self, String> {
        let event = serde_json::from_str(&raw_text).map_err(|_| raw_text.clone())?;
        Ok(Self { raw_text, event })
    }

    pub(super) fn into_parts(self) -> (String, Value) {
        (self.raw_text, self.event)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum ResponsesBackendFailure {
    Receive,
    ControlWrite,
}

#[derive(Debug, PartialEq, Eq)]
pub(super) enum ResponsesBackendProtocolViolation {
    InvalidEventText(String),
    BinaryFrame,
}

#[derive(Debug)]
pub(super) enum ResponsesBackendInput {
    Event(ResponsesProviderEvent),
    Closed,
    Failed(ResponsesBackendFailure),
    ProtocolViolation(ResponsesBackendProtocolViolation),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum ResponsesBackendSendError {
    Unavailable,
    Failed,
}

/// One provider-side execution session. Its concrete transport and framing
/// are private; the public FSM sends canonical `response.create` values and
/// receives canonical provider JSON events.
#[async_trait]
pub(super) trait ResponsesBackendSession: Send {
    async fn send_response_create(
        &mut self,
        event: &Value,
    ) -> Result<(), ResponsesBackendSendError>;

    async fn receive(&mut self) -> ResponsesBackendInput;

    async fn close(&mut self);
}

/// Connection-state slot for a transport-neutral provider session.
///
/// A missing session intentionally waits forever in `receive`: the public
/// client remains usable for a later independent `response.create` that can
/// plan and install a replacement backend.
#[derive(Default)]
pub(super) struct ResponsesBackendSessionHandle {
    session: Option<Box<dyn ResponsesBackendSession>>,
}

impl ResponsesBackendSessionHandle {
    pub(super) fn new(session: Box<dyn ResponsesBackendSession>) -> Self {
        Self {
            session: Some(session),
        }
    }

    pub(super) fn is_bound(&self) -> bool {
        self.session.is_some()
    }

    pub(super) async fn send_response_create(
        &mut self,
        event: &Value,
    ) -> Result<(), ResponsesBackendSendError> {
        match self.session.as_mut() {
            Some(session) => session.send_response_create(event).await,
            None => Err(ResponsesBackendSendError::Unavailable),
        }
    }

    pub(super) async fn receive(&mut self) -> ResponsesBackendInput {
        match self.session.as_mut() {
            Some(session) => session.receive().await,
            None => std::future::pending().await,
        }
    }

    pub(super) async fn close(&mut self) {
        if let Some(mut session) = self.session.take() {
            session.close().await;
        }
    }

    pub(super) fn detach(&mut self) {
        self.session = None;
    }

    /// Installs a replacement session without making its receive path wait for
    /// teardown of the previous provider. The replacement may already have a
    /// response queued after its initial `response.create`; synchronously
    /// awaiting the old Close write here would consume the new turn's
    /// first-event deadline before the relay can read that response.
    pub(super) fn replace_from(&mut self, replacement: &mut Self) {
        self.replace_from_with_close_timeout(replacement, TEARDOWN_WRITE_TIMEOUT);
    }

    fn replace_from_with_close_timeout(&mut self, replacement: &mut Self, close_timeout: Duration) {
        let next = replacement
            .session
            .take()
            .expect("newly opened Responses backend session should be present");
        if let Some(mut previous) = self.session.replace(next) {
            if let Ok(handle) = tokio::runtime::Handle::try_current() {
                handle.spawn(async move {
                    let _ = tokio::time::timeout(close_timeout, previous.close()).await;
                });
            }
        }
    }
}

pub(super) struct OpenedResponsesBackendSession {
    pub(super) session: ResponsesBackendSessionHandle,
    pub(super) response_headers: BTreeMap<String, String>,
}

/// Physical native backend. Opening and operating the provider session are
/// both backend responsibilities; observer and public-wire policy remain
/// separate axes.
#[async_trait]
pub(super) trait NativeResponsesWebSocketBackend: Send + Sync {
    fn kind(&self) -> ResponsesWebSocketBackendKind;

    fn upstream_errors(&self) -> UpstreamWebSocketErrorCodes;

    async fn open_session(
        &self,
        decision: &AiExecutionDecision,
    ) -> Result<OpenedResponsesBackendSession, &'static str>;
}

struct NativeResponsesWebSocket;

const NATIVE_RESPONSES_WEBSOCKET_ERRORS: UpstreamWebSocketErrorCodes =
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

static NATIVE_RESPONSES_WEBSOCKET_BACKEND: NativeResponsesWebSocket = NativeResponsesWebSocket;

#[async_trait]
impl NativeResponsesWebSocketBackend for NativeResponsesWebSocket {
    fn kind(&self) -> ResponsesWebSocketBackendKind {
        ResponsesWebSocketBackendKind::NativeResponsesWebSocket
    }

    fn upstream_errors(&self) -> UpstreamWebSocketErrorCodes {
        NATIVE_RESPONSES_WEBSOCKET_ERRORS
    }

    async fn open_session(
        &self,
        decision: &AiExecutionDecision,
    ) -> Result<OpenedResponsesBackendSession, &'static str> {
        let upstream = connect_upstream_websocket(
            decision,
            RESPONSES_WEBSOCKET_SESSION_LIMITS,
            self.upstream_errors(),
        )
        .await?;
        Ok(OpenedResponsesBackendSession {
            session: ResponsesBackendSessionHandle::new(Box::new(
                NativeResponsesWebSocketSession {
                    socket: Some(upstream.socket),
                    pending_pong: None,
                },
            )),
            response_headers: upstream.response_headers,
        })
    }
}

struct NativeResponsesWebSocketSession {
    socket: Option<wreq::ws::WebSocket>,
    // Keep a consumed Ping durable across cancellation of `receive()`. The
    // next call retries the bounded Pong write before reading another frame.
    pending_pong: Option<axum::body::Bytes>,
}

#[async_trait]
impl ResponsesBackendSession for NativeResponsesWebSocketSession {
    async fn send_response_create(
        &mut self,
        event: &Value,
    ) -> Result<(), ResponsesBackendSendError> {
        let outbound =
            serde_json::to_string(event).map_err(|_| ResponsesBackendSendError::Failed)?;
        let socket = self
            .socket
            .as_mut()
            .ok_or(ResponsesBackendSendError::Unavailable)?;
        send_upstream_message(socket, WreqWsMessage::text(outbound))
            .await
            .map_err(|_| ResponsesBackendSendError::Failed)
    }

    async fn receive(&mut self) -> ResponsesBackendInput {
        loop {
            if let Some(payload) = self.pending_pong.clone() {
                let Some(socket) = self.socket.as_mut() else {
                    return ResponsesBackendInput::Closed;
                };
                if send_upstream_message(socket, WreqWsMessage::Pong(payload))
                    .await
                    .is_err()
                {
                    self.socket = None;
                    return ResponsesBackendInput::Failed(ResponsesBackendFailure::ControlWrite);
                }
                self.pending_pong = None;
            }

            let Some(socket) = self.socket.as_mut() else {
                return ResponsesBackendInput::Closed;
            };
            let received = socket.recv().await;
            match received {
                None | Some(Ok(WreqWsMessage::Close(_))) => {
                    self.socket = None;
                    return ResponsesBackendInput::Closed;
                }
                Some(Err(_)) => {
                    self.socket = None;
                    return ResponsesBackendInput::Failed(ResponsesBackendFailure::Receive);
                }
                Some(Ok(WreqWsMessage::Text(text))) => {
                    let raw_text = text.to_string();
                    return match ResponsesProviderEvent::from_text(raw_text) {
                        Ok(event) => ResponsesBackendInput::Event(event),
                        Err(raw_text) => ResponsesBackendInput::ProtocolViolation(
                            ResponsesBackendProtocolViolation::InvalidEventText(raw_text),
                        ),
                    };
                }
                Some(Ok(WreqWsMessage::Binary(_))) => {
                    return ResponsesBackendInput::ProtocolViolation(
                        ResponsesBackendProtocolViolation::BinaryFrame,
                    );
                }
                Some(Ok(WreqWsMessage::Ping(payload))) => {
                    self.pending_pong = Some(payload);
                    tokio::task::yield_now().await;
                }
                Some(Ok(WreqWsMessage::Pong(_))) => {
                    tokio::task::yield_now().await;
                }
            }
        }
    }

    async fn close(&mut self) {
        self.pending_pong = None;
        if let Some(mut socket) = self.socket.take() {
            close_upstream_socket(&mut socket, None).await;
        }
    }
}

pub(super) fn resolve_native_responses_websocket_backend(
    kind: ResponsesWebSocketBackendKind,
) -> &'static dyn NativeResponsesWebSocketBackend {
    match kind {
        ResponsesWebSocketBackendKind::NativeResponsesWebSocket => {
            &NATIVE_RESPONSES_WEBSOCKET_BACKEND
        }
    }
}

#[cfg(test)]
mod tests {
    use std::collections::VecDeque;
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::sync::{Arc, Mutex};
    use std::time::Duration;

    use serde_json::{json, Value};
    use tokio::sync::oneshot;

    use super::{
        NativeResponsesWebSocketBackend, ResponsesBackendInput, ResponsesBackendSendError,
        ResponsesBackendSession, ResponsesBackendSessionHandle, ResponsesProviderEvent,
    };

    struct AlternativeSession {
        sent: Arc<Mutex<Vec<Value>>>,
        inputs: VecDeque<ResponsesBackendInput>,
        closed: Arc<Mutex<bool>>,
    }

    struct StalledCloseSession {
        close_started: Option<oneshot::Sender<()>>,
        close_release: Option<oneshot::Receiver<()>>,
        dropped: Arc<AtomicBool>,
    }

    #[async_trait::async_trait]
    impl ResponsesBackendSession for AlternativeSession {
        async fn send_response_create(
            &mut self,
            event: &Value,
        ) -> Result<(), ResponsesBackendSendError> {
            self.sent.lock().unwrap().push(event.clone());
            Ok(())
        }

        async fn receive(&mut self) -> ResponsesBackendInput {
            self.inputs
                .pop_front()
                .unwrap_or(ResponsesBackendInput::Closed)
        }

        async fn close(&mut self) {
            *self.closed.lock().unwrap() = true;
        }
    }

    #[async_trait::async_trait]
    impl ResponsesBackendSession for StalledCloseSession {
        async fn send_response_create(
            &mut self,
            _event: &Value,
        ) -> Result<(), ResponsesBackendSendError> {
            Ok(())
        }

        async fn receive(&mut self) -> ResponsesBackendInput {
            std::future::pending().await
        }

        async fn close(&mut self) {
            if let Some(started) = self.close_started.take() {
                let _ = started.send(());
            }
            if let Some(release) = self.close_release.take() {
                let _ = release.await;
            }
        }
    }

    impl Drop for StalledCloseSession {
        fn drop(&mut self) {
            self.dropped.store(true, Ordering::Release);
        }
    }

    #[tokio::test]
    async fn session_handle_accepts_an_alternative_non_websocket_session_shape() {
        let sent = Arc::new(Mutex::new(Vec::new()));
        let closed = Arc::new(Mutex::new(false));
        let input = ResponsesProviderEvent::from_text(
            json!({"type": "response.created", "response": {"id": "resp_fake"}}).to_string(),
        )
        .unwrap();
        let session = AlternativeSession {
            sent: Arc::clone(&sent),
            inputs: VecDeque::from([ResponsesBackendInput::Event(input)]),
            closed: Arc::clone(&closed),
        };
        let mut handle = ResponsesBackendSessionHandle::new(Box::new(session));
        let request = json!({"type": "response.create", "model": "fake-model"});

        handle.send_response_create(&request).await.unwrap();
        let ResponsesBackendInput::Event(event) = handle.receive().await else {
            panic!("alternative backend should yield a canonical provider event");
        };
        let (_, event) = event.into_parts();
        assert_eq!(event["response"]["id"], "resp_fake");
        assert_eq!(sent.lock().unwrap().as_slice(), &[request]);

        handle.close().await;
        assert!(!handle.is_bound());
        assert!(*closed.lock().unwrap());
    }

    #[tokio::test]
    async fn replacement_receives_within_a_short_first_event_budget_while_old_close_stalls() {
        let (close_started_tx, close_started_rx) = oneshot::channel();
        let (_close_release_tx, close_release_rx) = oneshot::channel();
        let old_session_dropped = Arc::new(AtomicBool::new(false));
        let mut current = ResponsesBackendSessionHandle::new(Box::new(StalledCloseSession {
            close_started: Some(close_started_tx),
            close_release: Some(close_release_rx),
            dropped: Arc::clone(&old_session_dropped),
        }));
        let response = ResponsesProviderEvent::from_text(
            json!({"type": "response.created", "response": {"id": "resp_replacement"}}).to_string(),
        )
        .unwrap();
        let mut replacement = ResponsesBackendSessionHandle::new(Box::new(AlternativeSession {
            sent: Arc::new(Mutex::new(Vec::new())),
            inputs: VecDeque::from([ResponsesBackendInput::Event(response)]),
            closed: Arc::new(Mutex::new(false)),
        }));

        current.replace_from_with_close_timeout(&mut replacement, Duration::from_millis(25));
        assert!(!replacement.is_bound());

        let input = tokio::time::timeout(Duration::from_millis(100), current.receive())
            .await
            .expect("old provider teardown must not consume the replacement first-event budget");
        let ResponsesBackendInput::Event(event) = input else {
            panic!("replacement should yield its already-queued response event");
        };
        let (_, event) = event.into_parts();
        assert_eq!(event["response"]["id"], "resp_replacement");

        tokio::time::timeout(Duration::from_secs(1), close_started_rx)
            .await
            .expect("old provider teardown should run in a background owner")
            .expect("old provider teardown should signal startup");
        tokio::time::timeout(Duration::from_secs(1), async {
            while !old_session_dropped.load(Ordering::Acquire) {
                tokio::task::yield_now().await;
            }
        })
        .await
        .expect("bounded background teardown should release the old session");
    }

    #[test]
    fn native_backend_contract_is_object_safe() {
        fn accept_backend(_backend: &dyn NativeResponsesWebSocketBackend) {}

        accept_backend(super::resolve_native_responses_websocket_backend(
            crate::orchestration::ResponsesWebSocketBackendKind::NativeResponsesWebSocket,
        ));
    }
}
