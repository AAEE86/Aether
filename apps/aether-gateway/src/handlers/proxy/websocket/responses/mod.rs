//! OpenAI Responses WebSocket protocol entry point and adapters.
//!
//! The public route is protocol-oriented rather than provider-oriented.  The
//! first adapter is Codex because it is the only validated upstream capability
//! today; future compatible Responses adapters can be dispatched here without
//! changing the public route or the shared WebSocket admission layer.

mod codex;
mod codex_turn;

use std::net::SocketAddr;

use axum::body::Body;
use axum::extract::ws::WebSocketUpgrade;
use axum::extract::{ConnectInfo, State};
use axum::http::{HeaderMap, Response, Uri};

use crate::handlers::proxy::websocket::ingress::upgrade_authenticated_ai_websocket;
use crate::handlers::proxy::websocket::session::RESPONSES_WEBSOCKET_SESSION_LIMITS;
use crate::{AppState, GatewayError};

pub(crate) async fn responses_websocket(
    State(state): State<AppState>,
    ConnectInfo(remote_addr): ConnectInfo<SocketAddr>,
    ws: WebSocketUpgrade,
    headers: HeaderMap,
    uri: Uri,
) -> Result<Response<Body>, GatewayError> {
    upgrade_authenticated_ai_websocket(
        state,
        remote_addr,
        ws,
        headers,
        uri,
        RESPONSES_WEBSOCKET_SESSION_LIMITS,
        codex::CODEX_RESPONSES_INGRESS_SPEC,
        codex::run_codex_responses_websocket,
    )
    .await
}
