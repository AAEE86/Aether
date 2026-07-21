//! Codex-only bridge for the Responses WebSocket mode.
//!
//! An incoming client socket is authenticated at Upgrade time. Its first
//! `response.create` selects a Codex key through the normal Responses planner.
//! Later turns reuse that upstream while the requested model remains eligible
//! on the selected key. A model change is planned again and keeps the current
//! upstream when the planner resolves to the same target; otherwise the bridge
//! transparently replaces the upstream between responses.

use std::collections::BTreeMap;
use std::net::SocketAddr;
use std::time::{Duration, Instant};

use axum::body::{Body, Bytes};
use axum::extract::ws::{
    CloseFrame as AxumCloseFrame, Message as AxumWsMessage, WebSocket, WebSocketUpgrade,
};
use axum::extract::{ConnectInfo, State};
use axum::http::header::{
    ACCEPT, ACCEPT_ENCODING, AUTHORIZATION, CONNECTION, CONTENT_ENCODING, CONTENT_LENGTH,
    CONTENT_TYPE, HOST, TRANSFER_ENCODING, UPGRADE,
};
use axum::http::{HeaderMap, Method, Response, StatusCode, Uri};
use futures_util::{SinkExt, StreamExt};
use serde_json::{json, Value};
use tokio::time::{sleep, timeout};
use tracing::{debug, info, warn};
use url::Url;
use uuid::Uuid;
use wreq::ws::message::{CloseFrame as WreqCloseFrame, Message as WreqWsMessage};

use super::codex_ws_finalize::{
    begin_codex_websocket_turn, prepare_codex_websocket_turn_decision,
    spawn_codex_websocket_turn_finalization, CodexWebSocketTurn, CodexWebSocketTurnDeadline,
    CodexWebSocketTurnObservation, CodexWebSocketTurnOutcome,
};

use crate::ai_serving::{maybe_build_codex_responses_websocket_decision, AiExecutionDecision};
use crate::api::response::{
    build_local_auth_rejection_response, build_local_http_error_response,
    build_local_overloaded_response,
};
use crate::control::{
    request_model_local_rejection, trusted_auth_local_rejection, GatewayControlDecision,
    GatewayLocalAuthRejection,
};
use crate::execution_runtime::transport::{
    build_browser_wreq_client, build_request_headers, ExecutionTransportControls,
};
use crate::handlers::shared::ip_rules_allow;
use crate::headers::{
    effective_client_ip, extract_or_generate_trace_id, request_origin_from_headers_and_remote_addr,
};
use crate::orchestration::sync_codex_websocket_quota_metadata;
use crate::rate_limit::FrontdoorUserRpmOutcome;
use crate::router::RequestAdmissionError;
use crate::{AppState, GatewayError};

const MAX_FRAME_SIZE: usize = 16 << 20;
const MAX_MESSAGE_SIZE: usize = 16 << 20;
const INITIAL_MESSAGE_TIMEOUT: Duration = Duration::from_secs(60);
const MAX_CONNECTION_DURATION: Duration = Duration::from_secs(60 * 60);
const CLOSE_POLICY_VIOLATION: u16 = 1008;
const CLOSE_INTERNAL_ERROR: u16 = 1011;
const CLOSE_TRY_AGAIN: u16 = 1013;
const WEBSOCKET_LOG_TRANSPORT: &str = "websocket";

#[derive(Clone)]
struct CodexWebSocketRequestContext {
    trace_id: String,
    headers: HeaderMap,
    uri: Uri,
    remote_addr: SocketAddr,
    decision: GatewayControlDecision,
    rpm_bypassed: bool,
}

struct CodexWebSocketConnectionLog {
    trace_id: String,
    remote_addr: SocketAddr,
    path: String,
    route_class: String,
    user_id: String,
    api_key_id: String,
    started_at: Instant,
}

impl CodexWebSocketConnectionLog {
    fn new(context: &CodexWebSocketRequestContext) -> Self {
        let auth_context = context.decision.auth_context.as_ref();
        Self {
            trace_id: context.trace_id.clone(),
            remote_addr: context.remote_addr,
            path: context.uri.path().to_string(),
            route_class: context
                .decision
                .route_class
                .as_deref()
                .unwrap_or("ai_public")
                .to_string(),
            user_id: auth_context
                .map(|auth_context| auth_context.user_id.clone())
                .unwrap_or_else(|| "-".to_string()),
            api_key_id: auth_context
                .map(|auth_context| auth_context.api_key_id.clone())
                .unwrap_or_else(|| "-".to_string()),
            started_at: Instant::now(),
        }
    }

    fn log_opened(&self) {
        info!(
            event_name = "codex_websocket_connection_opened",
            log_type = "access",
            transport = WEBSOCKET_LOG_TRANSPORT,
            websocket = true,
            status = "upgraded",
            status_code = 101u16,
            trace_id = %self.trace_id,
            remote_addr = %self.remote_addr,
            method = "GET",
            path = %self.path,
            user_id = %self.user_id,
            api_key_id = %self.api_key_id,
            route_class = %self.route_class,
            execution_path = "codex_websocket_bridge",
            provider_type = "codex",
            "gateway accepted Codex Responses WebSocket connection"
        );
    }
}

impl Drop for CodexWebSocketConnectionLog {
    fn drop(&mut self) {
        info!(
            event_name = "codex_websocket_connection_closed",
            log_type = "access",
            transport = WEBSOCKET_LOG_TRANSPORT,
            websocket = true,
            status = "closed",
            status_code = 101u16,
            trace_id = %self.trace_id,
            remote_addr = %self.remote_addr,
            method = "GET",
            path = %self.path,
            user_id = %self.user_id,
            api_key_id = %self.api_key_id,
            route_class = %self.route_class,
            execution_path = "codex_websocket_bridge",
            provider_type = "codex",
            elapsed_ms = self.started_at.elapsed().as_millis() as u64,
            "gateway closed Codex Responses WebSocket connection"
        );
    }
}

struct BoundCodexConnection {
    upstream: wreq::ws::WebSocket,
    client_model: String,
    provider_model: String,
    response_in_flight: bool,
    decision_template: AiExecutionDecision,
    active_turn: Option<CodexWebSocketTurn>,
    next_turn_index: u64,
    upstream_response_headers: BTreeMap<String, String>,
    account_quota_exhausted: bool,
}

#[derive(Debug, Clone, Copy)]
enum InitialMessageError {
    TimedOut,
    ClientClosed,
    ClientRead,
    UnsupportedFrame,
    InvalidJson,
    MissingResponseCreate,
    MissingModel,
}

impl InitialMessageError {
    const fn code(self) -> &'static str {
        match self {
            Self::TimedOut => "initial_response_create_timeout",
            Self::ClientClosed => "client_closed",
            Self::ClientRead => "client_read_failed",
            Self::UnsupportedFrame => "initial_response_create_must_be_text",
            Self::InvalidJson => "invalid_response_create",
            Self::MissingResponseCreate => "expected_response_create",
            Self::MissingModel => "response_create_model_required",
        }
    }

    const fn close_code(self) -> u16 {
        match self {
            Self::TimedOut => CLOSE_TRY_AGAIN,
            Self::ClientClosed => 1000,
            Self::ClientRead | Self::UnsupportedFrame | Self::InvalidJson => CLOSE_POLICY_VIOLATION,
            Self::MissingResponseCreate | Self::MissingModel => CLOSE_POLICY_VIOLATION,
        }
    }
}

pub(crate) async fn codex_responses_websocket(
    State(state): State<AppState>,
    ConnectInfo(remote_addr): ConnectInfo<SocketAddr>,
    ws: WebSocketUpgrade,
    headers: HeaderMap,
    uri: Uri,
) -> Result<Response<Body>, GatewayError> {
    let trace_id = extract_or_generate_trace_id(&headers);
    let client_ip = effective_client_ip(&headers, &remote_addr);
    if state.admin_security_ip_blacklisted(client_ip).await? {
        return build_local_http_error_response(
            &trace_id,
            None,
            StatusCode::FORBIDDEN,
            "当前 IP 已被禁止访问",
        );
    }

    let request_context = crate::control::resolve_public_request_context(
        &state,
        &Method::GET,
        &uri,
        &headers,
        &trace_id,
    )
    .await?;
    let Some(decision) = request_context.control_decision else {
        return build_local_http_error_response(
            &trace_id,
            None,
            StatusCode::NOT_FOUND,
            "WebSocket route is unavailable",
        );
    };
    if let Some(rejection) = trusted_auth_local_rejection(Some(&decision), &headers) {
        return build_local_auth_rejection_response(&trace_id, Some(&decision), &rejection);
    }
    let Some(auth_context) = decision.auth_context.as_ref() else {
        return build_local_auth_rejection_response(
            &trace_id,
            Some(&decision),
            &GatewayLocalAuthRejection::InvalidApiKey,
        );
    };
    if !auth_context.access_allowed {
        return build_local_auth_rejection_response(
            &trace_id,
            Some(&decision),
            &GatewayLocalAuthRejection::InvalidApiKey,
        );
    }
    if !ip_rules_allow(auth_context.ip_rules.as_deref(), client_ip) {
        return build_local_auth_rejection_response(
            &trace_id,
            Some(&decision),
            &GatewayLocalAuthRejection::IpNotAllowed {
                remote_ip: client_ip.to_string(),
            },
        );
    }

    let ip_whitelisted = match state.admin_security_ip_whitelisted(client_ip).await {
        Ok(value) => value,
        Err(error) => {
            warn!(
                event_name = "codex_websocket_ip_whitelist_check_failed",
                log_type = "ops",
                transport = WEBSOCKET_LOG_TRANSPORT,
                websocket = true,
                trace_id = %trace_id,
                client_ip = %client_ip,
                error = ?error,
                "gateway continued with WebSocket rate limiting after IP whitelist check error"
            );
            false
        }
    };
    let request_permit = match state.try_acquire_request_permit().await {
        Ok(permit) => permit,
        Err(RequestAdmissionError::Local(aether_runtime::ConcurrencyError::Saturated {
            gate,
            limit,
        }))
        | Err(RequestAdmissionError::Distributed(
            aether_runtime_state::RuntimeSemaphoreError::Saturated { gate, limit },
        ))
        | Err(RequestAdmissionError::Distributed(
            aether_runtime_state::RuntimeSemaphoreError::Unavailable { gate, limit, .. },
        )) => {
            return build_local_overloaded_response(&trace_id, Some(&decision), gate, limit);
        }
        Err(RequestAdmissionError::Local(aether_runtime::ConcurrencyError::Closed { gate })) => {
            return Err(GatewayError::Internal(format!(
                "gateway request concurrency gate {gate} is closed"
            )));
        }
        Err(RequestAdmissionError::Distributed(
            aether_runtime_state::RuntimeSemaphoreError::InvalidConfiguration(message),
        )) => return Err(GatewayError::Internal(message)),
    };

    let context = CodexWebSocketRequestContext {
        trace_id,
        headers,
        uri,
        remote_addr,
        decision,
        rpm_bypassed: ip_whitelisted,
    };
    Ok(ws
        .max_frame_size(MAX_FRAME_SIZE)
        .max_message_size(MAX_MESSAGE_SIZE)
        .on_upgrade(move |socket| async move {
            // The permit intentionally covers the lifetime of this active socket.
            let _request_permit = request_permit;
            run_codex_responses_websocket(socket, state, context).await;
        }))
}

async fn run_codex_responses_websocket(
    mut client_socket: WebSocket,
    state: AppState,
    context: CodexWebSocketRequestContext,
) {
    let connection_log = CodexWebSocketConnectionLog::new(&context);
    connection_log.log_opened();

    let (first_text, first_event) = match receive_initial_response_create(&mut client_socket).await
    {
        Ok(value) => value,
        Err(error) => {
            if !matches!(error, InitialMessageError::ClientClosed) {
                send_gateway_error(
                    &mut client_socket,
                    error.code(),
                    "WebSocket must start with a valid response.create event",
                )
                .await;
                close_client_socket(
                    &mut client_socket,
                    error.close_code(),
                    "invalid_initial_event",
                )
                .await;
            }
            return;
        }
    };

    let planning_parts = build_planning_parts(&context);
    match consume_response_create_rate_limit(&state, &context.decision, context.rpm_bypassed).await
    {
        Ok(true) => {}
        Ok(false) => {
            send_gateway_error(
                &mut client_socket,
                "rate_limit_exceeded",
                "Too many response.create events; retry later",
            )
            .await;
            close_client_socket(&mut client_socket, CLOSE_TRY_AGAIN, "rate_limit_exceeded").await;
            return;
        }
        Err(()) => {
            warn!(
                event_name = "codex_websocket_rate_limit_check_failed",
                log_type = "ops",
                transport = WEBSOCKET_LOG_TRANSPORT,
                websocket = true,
                trace_id = %context.trace_id,
                "gateway failed to consume WebSocket response rate limit"
            );
            send_gateway_error(
                &mut client_socket,
                "gateway_rate_limit_unavailable",
                "Gateway could not evaluate the response rate limit",
            )
            .await;
            close_client_socket(
                &mut client_socket,
                CLOSE_INTERNAL_ERROR,
                "rate_limit_unavailable",
            )
            .await;
            return;
        }
    }
    match request_model_local_rejection(
        &state,
        Some(&context.decision),
        &planning_parts.uri,
        &planning_parts.headers,
        &Bytes::from(first_text.into_bytes()),
    )
    .await
    {
        Ok(Some(_)) => {
            send_gateway_error(
                &mut client_socket,
                "model_not_allowed",
                "The requested model is not available to this API key",
            )
            .await;
            close_client_socket(
                &mut client_socket,
                CLOSE_POLICY_VIOLATION,
                "model_not_allowed",
            )
            .await;
            return;
        }
        Ok(None) => {}
        Err(_) => {
            warn!(
                event_name = "codex_websocket_model_access_check_failed",
                log_type = "ops",
                transport = WEBSOCKET_LOG_TRANSPORT,
                websocket = true,
                trace_id = %context.trace_id,
                "gateway failed to evaluate WebSocket model access policy"
            );
            send_gateway_error(
                &mut client_socket,
                "gateway_auth_unavailable",
                "Gateway could not evaluate request access",
            )
            .await;
            close_client_socket(
                &mut client_socket,
                CLOSE_INTERNAL_ERROR,
                "gateway_auth_unavailable",
            )
            .await;
            return;
        }
    }

    let decision = match maybe_build_codex_responses_websocket_decision(
        &state,
        &planning_parts,
        &context.trace_id,
        &context.decision,
        &first_event,
    )
    .await
    {
        Ok(Some(decision)) => decision,
        Ok(None) => {
            send_gateway_error(
                &mut client_socket,
                "codex_provider_unavailable",
                "No eligible WebSocket-enabled Codex Responses provider is available",
            )
            .await;
            close_client_socket(
                &mut client_socket,
                CLOSE_TRY_AGAIN,
                "codex_provider_unavailable",
            )
            .await;
            return;
        }
        Err(_) => {
            warn!(
                event_name = "codex_websocket_planning_failed",
                log_type = "ops",
                transport = WEBSOCKET_LOG_TRANSPORT,
                websocket = true,
                trace_id = %context.trace_id,
                "gateway failed to plan Codex WebSocket provider request"
            );
            send_gateway_error(
                &mut client_socket,
                "codex_provider_unavailable",
                "Gateway could not prepare a Codex provider connection",
            )
            .await;
            close_client_socket(
                &mut client_socket,
                CLOSE_INTERNAL_ERROR,
                "codex_planning_failed",
            )
            .await;
            return;
        }
    };

    let first_provider_event = match planned_response_create_event(&decision, &first_event)
        .and_then(|event| {
            serde_json::from_str::<Value>(&event).map_err(|_| "codex_websocket_request_invalid")
        }) {
        Ok(event) => event,
        Err(code) => {
            warn!(
                event_name = "codex_websocket_initial_event_normalization_failed",
                log_type = "ops",
                transport = WEBSOCKET_LOG_TRANSPORT,
                websocket = true,
                trace_id = %context.trace_id,
                error_code = code,
                "gateway could not normalize the initial Codex WebSocket event"
            );
            send_gateway_error(
                &mut client_socket,
                code,
                "Gateway could not prepare the Codex response.create event",
            )
            .await;
            close_client_socket(&mut client_socket, CLOSE_POLICY_VIOLATION, code).await;
            return;
        }
    };
    let first_turn_decision = prepare_codex_websocket_turn_decision(
        &decision,
        context.trace_id.clone(),
        true,
        &first_event,
        &first_provider_event,
        &context.trace_id,
        1,
    );
    let mut first_turn = match begin_codex_websocket_turn(
        &state,
        &planning_parts,
        first_turn_decision,
        &first_event,
    )
    .await
    {
        Ok(turn) => turn,
        Err(error) => {
            warn!(
                event_name = "codex_websocket_turn_lifecycle_start_failed",
                log_type = "ops",
                transport = WEBSOCKET_LOG_TRANSPORT,
                websocket = true,
                trace_id = %context.trace_id,
                error = ?error,
                "gateway could not start Codex WebSocket usage/audit lifecycle"
            );
            send_gateway_error(
                &mut client_socket,
                "codex_websocket_reporting_unavailable",
                "Gateway could not start usage and audit tracking for this response",
            )
            .await;
            close_client_socket(
                &mut client_socket,
                CLOSE_INTERNAL_ERROR,
                "reporting_unavailable",
            )
            .await;
            return;
        }
    };

    let mut bound = match bind_codex_upstream(&decision, &first_event).await {
        Ok(connection) => connection,
        Err(code) => {
            spawn_codex_websocket_turn_finalization(
                state.clone(),
                first_turn,
                CodexWebSocketTurnOutcome::upstream_connect_failed(code),
            );
            warn!(
                event_name = "codex_websocket_upstream_connect_failed",
                log_type = "ops",
                transport = WEBSOCKET_LOG_TRANSPORT,
                websocket = true,
                trace_id = %context.trace_id,
                error_code = code,
                "gateway failed to establish Codex WebSocket upstream"
            );
            send_gateway_error(
                &mut client_socket,
                code,
                "Gateway could not establish the Codex provider connection",
            )
            .await;
            close_client_socket(&mut client_socket, CLOSE_TRY_AGAIN, code).await;
            return;
        }
    };
    first_turn.mark_upstream_request_sent();
    first_turn.set_provider_response_headers(bound.upstream_response_headers.clone());
    bound.active_turn = Some(first_turn);

    relay_bound_connection(&mut client_socket, &mut bound, &state, &context).await;
}

async fn receive_initial_response_create(
    client_socket: &mut WebSocket,
) -> Result<(String, Value), InitialMessageError> {
    loop {
        let message = timeout(INITIAL_MESSAGE_TIMEOUT, client_socket.next())
            .await
            .map_err(|_| InitialMessageError::TimedOut)?;
        let Some(message) = message else {
            return Err(InitialMessageError::ClientClosed);
        };
        let message = message.map_err(|_| InitialMessageError::ClientRead)?;
        match message {
            AxumWsMessage::Ping(payload) => {
                client_socket
                    .send(AxumWsMessage::Pong(payload))
                    .await
                    .map_err(|_| InitialMessageError::ClientRead)?;
            }
            AxumWsMessage::Pong(_) => {}
            AxumWsMessage::Close(_) => return Err(InitialMessageError::ClientClosed),
            AxumWsMessage::Binary(_) => return Err(InitialMessageError::UnsupportedFrame),
            AxumWsMessage::Text(text) => {
                let text = text.to_string();
                let event: Value =
                    serde_json::from_str(&text).map_err(|_| InitialMessageError::InvalidJson)?;
                validate_initial_response_create(&event)?;
                return Ok((text, event));
            }
        }
    }
}

fn validate_initial_response_create(event: &Value) -> Result<(), InitialMessageError> {
    let object = event.as_object().ok_or(InitialMessageError::InvalidJson)?;
    if object.get("type").and_then(Value::as_str) != Some("response.create") {
        return Err(InitialMessageError::MissingResponseCreate);
    }
    if object
        .get("model")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .is_none()
    {
        return Err(InitialMessageError::MissingModel);
    }
    Ok(())
}

fn build_planning_parts(context: &CodexWebSocketRequestContext) -> http::request::Parts {
    let mut request = http::Request::builder()
        .method(Method::POST)
        .uri(context.uri.clone())
        .body(())
        .expect("a validated request URI should build planning request parts");
    let headers = request.headers_mut();
    *headers = context.headers.clone();
    headers.remove(AUTHORIZATION);
    headers.remove("x-api-key");
    headers.remove("api-key");
    headers.remove("x-goog-api-key");
    headers.remove(CONNECTION);
    headers.remove(UPGRADE);
    headers.remove("sec-websocket-key");
    headers.remove("sec-websocket-version");
    headers.remove("sec-websocket-protocol");
    headers.remove("sec-websocket-extensions");
    headers.insert(
        CONTENT_TYPE,
        http::HeaderValue::from_static("application/json"),
    );
    request
        .extensions_mut()
        .insert(request_origin_from_headers_and_remote_addr(
            &context.headers,
            &context.remote_addr,
        ));
    request.into_parts().0
}

async fn bind_codex_upstream(
    decision: &AiExecutionDecision,
    initial_event: &Value,
) -> Result<BoundCodexConnection, &'static str> {
    let upstream_url = decision
        .upstream_url
        .as_deref()
        .ok_or("codex_upstream_url_missing")?;
    let upstream_url = websocket_upstream_url(upstream_url)?;
    let headers = websocket_handshake_headers(&decision.provider_request_headers)?;
    let client = build_websocket_client(decision)?;
    let response = client
        .websocket(upstream_url.as_str())
        .headers(headers)
        .max_frame_size(MAX_FRAME_SIZE)
        .max_message_size(MAX_MESSAGE_SIZE)
        .send()
        .await
        .map_err(|_| "codex_websocket_handshake_failed")?;
    if response.status().as_u16() != 101 {
        return Err("codex_websocket_upgrade_rejected");
    }
    let upstream_response_headers = websocket_response_headers(response.headers());
    let mut upstream = response
        .into_websocket()
        .await
        .map_err(|_| "codex_websocket_upgrade_failed")?;
    let first_event = planned_response_create_event(decision, initial_event)?;
    upstream
        .send(WreqWsMessage::text(first_event))
        .await
        .map_err(|_| "codex_websocket_initial_send_failed")?;

    let client_model = initial_event
        .get("model")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .ok_or("codex_websocket_model_missing")?
        .to_string();
    let provider_model = decision
        .provider_request_body
        .as_ref()
        .and_then(|body| body.get("model"))
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .or_else(|| {
            decision
                .mapped_model
                .as_deref()
                .map(str::trim)
                .filter(|value| !value.is_empty())
        })
        .ok_or("codex_websocket_mapped_model_missing")?
        .to_string();

    Ok(BoundCodexConnection {
        upstream,
        client_model,
        provider_model,
        response_in_flight: true,
        decision_template: decision.clone(),
        active_turn: None,
        next_turn_index: 2,
        upstream_response_headers,
        account_quota_exhausted: false,
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

fn websocket_upstream_url(raw: &str) -> Result<Url, &'static str> {
    let mut url = Url::parse(raw).map_err(|_| "codex_upstream_url_invalid")?;
    if url.host_str().is_none() || !url.username().is_empty() || url.password().is_some() {
        return Err("codex_upstream_url_invalid");
    }
    let websocket_scheme = match url.scheme() {
        "https" => "wss",
        "http" => "ws",
        "wss" | "ws" => return Ok(url),
        _ => return Err("codex_upstream_url_invalid"),
    };
    url.set_scheme(websocket_scheme)
        .map_err(|_| "codex_upstream_url_invalid")?;
    Ok(url)
}

fn websocket_handshake_headers(
    provider_headers: &BTreeMap<String, String>,
) -> Result<HeaderMap, &'static str> {
    let mut headers = build_request_headers(provider_headers, None, false)
        .map_err(|_| "codex_websocket_headers_invalid")?;
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

fn build_websocket_client(decision: &AiExecutionDecision) -> Result<wreq::Client, &'static str> {
    // HTTP read and total timeouts are unsuitable for a long-lived Responses
    // socket. Keep only the configured connection timeout; the bridge owns the
    // 60-minute connection limit above.
    let timeouts = websocket_timeouts(decision);
    if let Some(profile) = decision.transport_profile.as_ref() {
        return build_browser_wreq_client(
            timeouts.as_ref(),
            decision.proxy.as_ref(),
            profile,
            ExecutionTransportControls::default(),
            false,
        )
        .map_err(|_| "codex_websocket_client_build_failed");
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
            let proxy = wreq::Proxy::all(proxy_url).map_err(|_| "codex_websocket_proxy_invalid")?;
            builder = builder.proxy(proxy);
        } else if proxy.node_id.is_some() || proxy.mode.as_deref() == Some("tunnel") {
            return Err("codex_websocket_tunnel_proxy_unsupported");
        }
    }
    builder
        .build()
        .map_err(|_| "codex_websocket_client_build_failed")
}

fn websocket_timeouts(
    decision: &AiExecutionDecision,
) -> Option<aether_contracts::ExecutionTimeouts> {
    let mut timeouts = decision.timeouts.clone()?;
    timeouts.read_ms = None;
    timeouts.first_byte_ms = None;
    timeouts.total_ms = None;
    Some(timeouts)
}

fn planned_response_create_event(
    decision: &AiExecutionDecision,
    fallback: &Value,
) -> Result<String, &'static str> {
    let mut event = decision
        .provider_request_body
        .clone()
        .unwrap_or_else(|| fallback.clone());
    let object = event
        .as_object_mut()
        .ok_or("codex_websocket_request_invalid")?;
    object.insert(
        "type".to_string(),
        Value::String("response.create".to_string()),
    );
    object.remove("stream");
    object.remove("background");
    serde_json::to_string(&event).map_err(|_| "codex_websocket_request_invalid")
}

async fn relay_bound_connection(
    client_socket: &mut WebSocket,
    bound: &mut BoundCodexConnection,
    state: &AppState,
    context: &CodexWebSocketRequestContext,
) {
    let connection_deadline = sleep(MAX_CONNECTION_DURATION);
    tokio::pin!(connection_deadline);

    loop {
        let active_turn_deadline = bound.active_turn.as_ref().map(CodexWebSocketTurn::deadline);
        tokio::select! {
            _ = &mut connection_deadline => {
                finalize_active_turn(
                    bound,
                    state,
                    CodexWebSocketTurnOutcome::connection_limit_reached(),
                );
                send_gateway_error(
                    client_socket,
                    "websocket_connection_limit_reached",
                    "WebSocket connection duration limit reached; reconnect to continue",
                ).await;
                let _ = bound.upstream.send(WreqWsMessage::Close(None)).await;
                close_client_socket(client_socket, CLOSE_TRY_AGAIN, "connection_limit_reached").await;
                break;
            }
            _ = wait_for_active_turn_deadline(active_turn_deadline) => {
                let Some(turn_deadline) = active_turn_deadline else {
                    continue;
                };
                warn!(
                    event_name = "codex_websocket_turn_timeout",
                    log_type = "ops",
                    transport = WEBSOCKET_LOG_TRANSPORT,
                    websocket = true,
                    trace_id = %context.trace_id,
                    timeout_phase = ?turn_deadline.phase,
                    timeout_ms = turn_deadline.timeout.as_millis() as u64,
                    "Codex WebSocket response did not reach its configured deadline"
                );
                finalize_active_turn(bound, state, turn_deadline.phase.outcome());
                send_gateway_error(
                    client_socket,
                    turn_deadline.phase.error_code(),
                    turn_deadline.phase.client_message(),
                ).await;
                let _ = bound.upstream.send(WreqWsMessage::Close(None)).await;
                close_client_socket(
                    client_socket,
                    CLOSE_TRY_AGAIN,
                    turn_deadline.phase.error_code(),
                ).await;
                break;
            }
            client_message = client_socket.next() => {
                let Some(client_message) = client_message else {
                    finalize_active_turn(
                        bound,
                        state,
                        CodexWebSocketTurnOutcome::client_disconnected(),
                    );
                    let _ = bound.upstream.send(WreqWsMessage::Close(None)).await;
                    break;
                };
                let Ok(client_message) = client_message else {
                    warn!(
                        event_name = "codex_websocket_client_receive_failed",
                        log_type = "ops",
                        transport = WEBSOCKET_LOG_TRANSPORT,
                        websocket = true,
                        trace_id = %context.trace_id,
                        "client WebSocket receive failed"
                    );
                    finalize_active_turn(
                        bound,
                        state,
                        CodexWebSocketTurnOutcome::client_disconnected(),
                    );
                    let _ = bound.upstream.send(WreqWsMessage::Close(None)).await;
                    break;
                };
                match forward_client_message(client_message, bound, client_socket, state, context).await {
                    RelayDisposition::Continue => {}
                    RelayDisposition::Close => {
                        finalize_active_turn(
                            bound,
                            state,
                            CodexWebSocketTurnOutcome::client_disconnected(),
                        );
                        break;
                    }
                    RelayDisposition::UpstreamError(code) => {
                        warn!(
                            event_name = "codex_websocket_upstream_send_failed",
                            log_type = "ops",
                            transport = WEBSOCKET_LOG_TRANSPORT,
                            websocket = true,
                            trace_id = %context.trace_id,
                            error_code = code,
                            "Codex upstream WebSocket send failed"
                        );
                        finalize_active_turn(
                            bound,
                            state,
                            CodexWebSocketTurnOutcome::upstream_send_failed(),
                        );
                        send_gateway_error(
                            client_socket,
                            code,
                            "Gateway could not forward the WebSocket event upstream",
                        ).await;
                        close_client_socket(client_socket, CLOSE_INTERNAL_ERROR, code).await;
                        break;
                    }
                }
            }
            upstream_message = bound.upstream.recv() => {
                let Some(upstream_message) = upstream_message else {
                    finalize_active_turn(
                        bound,
                        state,
                        CodexWebSocketTurnOutcome::upstream_closed(),
                    );
                    close_client_socket(client_socket, 1000, "upstream_closed").await;
                    break;
                };
                let Ok(upstream_message) = upstream_message else {
                    warn!(
                        event_name = "codex_websocket_upstream_receive_failed",
                        log_type = "ops",
                        transport = WEBSOCKET_LOG_TRANSPORT,
                        websocket = true,
                        trace_id = %context.trace_id,
                        "Codex upstream WebSocket receive failed"
                    );
                    finalize_active_turn(
                        bound,
                        state,
                        CodexWebSocketTurnOutcome::upstream_receive_failed(),
                    );
                    send_gateway_error(
                        client_socket,
                        "codex_websocket_receive_failed",
                        "Codex provider connection closed unexpectedly",
                    ).await;
                    close_client_socket(client_socket, CLOSE_INTERNAL_ERROR, "upstream_receive_failed").await;
                    break;
                };
                if let WreqWsMessage::Text(text) = &upstream_message {
                    debug!(
                        event_name = "codex_websocket_upstream_event",
                        log_type = "event",
                        transport = WEBSOCKET_LOG_TRANSPORT,
                        websocket = true,
                        trace_id = %context.trace_id,
                        event_type = %websocket_event_type_for_log(text.as_str()),
                        frame_bytes = text.len(),
                        active_turn = bound.active_turn.is_some(),
                        "gateway received Codex WebSocket event"
                    );
                }
                if let Some(rate_limits) = codex_websocket_rate_limits_from_message(
                    &upstream_message,
                    crate::clock::current_unix_secs(),
                ) {
                    if aether_admin::provider::quota::codex_rate_limit_metadata_exhausted(&rate_limits)
                        && !bound.account_quota_exhausted
                    {
                        bound.account_quota_exhausted = true;
                        if let Err(error) = sync_codex_websocket_quota_metadata(
                            state,
                            bound.decision_template.report_context.as_ref(),
                            rate_limits,
                        )
                        .await
                        {
                            warn!(
                                event_name = "codex_websocket_quota_exhausted_sync_failed",
                                log_type = "ops",
                                transport = WEBSOCKET_LOG_TRANSPORT,
                                websocket = true,
                                trace_id = %context.trace_id,
                                error = ?error,
                                "gateway failed to persist an exhausted Codex WebSocket account before draining the connection"
                            );
                        }
                        info!(
                            event_name = "codex_websocket_account_quota_exhausted",
                            log_type = "event",
                            transport = WEBSOCKET_LOG_TRANSPORT,
                            websocket = true,
                            trace_id = %context.trace_id,
                            "gateway will drain the Codex WebSocket after the active response"
                        );
                    }
                }
                let observation = match &upstream_message {
                    WreqWsMessage::Text(text) => bound
                        .active_turn
                        .as_mut()
                        .and_then(|turn| turn.observe_upstream_text(text.as_str())),
                    _ => None,
                };
                update_response_in_flight(bound, &upstream_message);
                if matches!(
                    observation,
                    Some(CodexWebSocketTurnObservation::Started)
                        | Some(CodexWebSocketTurnObservation::Terminal(_))
                ) {
                    if let Some(turn) = bound.active_turn.as_mut() {
                        turn.mark_stream_started(state).await;
                    }
                }
                if let Some(CodexWebSocketTurnObservation::Terminal(outcome)) = observation {
                    finalize_active_turn(bound, state, outcome);
                }
                let drain_for_quota = quota_drain_ready(
                    bound.account_quota_exhausted,
                    bound.response_in_flight,
                    observation,
                );
                let is_close = matches!(upstream_message, WreqWsMessage::Close(_));
                if is_close {
                    finalize_active_turn(
                        bound,
                        state,
                        CodexWebSocketTurnOutcome::upstream_closed(),
                    );
                }
                if client_socket.send(wreq_message_to_axum(upstream_message)).await.is_err() {
                    finalize_active_turn(
                        bound,
                        state,
                        CodexWebSocketTurnOutcome::client_disconnected(),
                    );
                    let _ = bound.upstream.send(WreqWsMessage::Close(None)).await;
                    break;
                }
                if drain_for_quota {
                    send_gateway_error(
                        client_socket,
                        "codex_account_quota_exhausted",
                        "The bound Codex account quota is exhausted; reconnect to select another account",
                    )
                    .await;
                    let _ = bound.upstream.send(WreqWsMessage::Close(None)).await;
                    close_client_socket(
                        client_socket,
                        CLOSE_TRY_AGAIN,
                        "account_quota_exhausted",
                    )
                    .await;
                    break;
                }
                if is_close {
                    break;
                }
            }
        }
    }
}

fn codex_websocket_rate_limits_from_message(
    message: &WreqWsMessage,
    updated_at_unix_secs: u64,
) -> Option<Value> {
    let WreqWsMessage::Text(text) = message else {
        return None;
    };
    let event = serde_json::from_str::<Value>(text.as_str()).ok()?;
    aether_admin::provider::quota::parse_codex_websocket_rate_limits_response(
        &event,
        updated_at_unix_secs,
    )
}

async fn wait_for_active_turn_deadline(deadline: Option<CodexWebSocketTurnDeadline>) {
    match deadline {
        Some(deadline) => {
            tokio::time::sleep_until(tokio::time::Instant::from_std(deadline.deadline)).await
        }
        None => std::future::pending::<()>().await,
    }
}

fn finalize_active_turn(
    bound: &mut BoundCodexConnection,
    state: &AppState,
    outcome: CodexWebSocketTurnOutcome,
) {
    if let Some(turn) = bound.active_turn.take() {
        spawn_codex_websocket_turn_finalization(state.clone(), turn, outcome);
    }
}

enum RelayDisposition {
    Continue,
    Close,
    UpstreamError(&'static str),
}

fn quota_drain_ready(
    account_quota_exhausted: bool,
    response_in_flight: bool,
    observation: Option<CodexWebSocketTurnObservation>,
) -> bool {
    account_quota_exhausted
        && (!response_in_flight
            || matches!(
                observation,
                Some(CodexWebSocketTurnObservation::Terminal(_))
            ))
}

async fn forward_client_message(
    client_message: AxumWsMessage,
    bound: &mut BoundCodexConnection,
    client_socket: &mut WebSocket,
    state: &AppState,
    context: &CodexWebSocketRequestContext,
) -> RelayDisposition {
    match client_message {
        AxumWsMessage::Text(text) => {
            let text = text.to_string();
            let client_event = serde_json::from_str::<Value>(&text).ok();
            let is_response_create = client_event
                .as_ref()
                .and_then(|event| event.get("type"))
                .and_then(Value::as_str)
                == Some("response.create");
            if !is_response_create {
                return bound
                    .upstream
                    .send(WreqWsMessage::text(text))
                    .await
                    .map(|_| RelayDisposition::Continue)
                    .unwrap_or(RelayDisposition::UpstreamError(
                        "codex_websocket_send_failed",
                    ));
            }

            if bound.response_in_flight {
                send_gateway_error(
                    client_socket,
                    "response_already_in_progress",
                    "This connection runs one response at a time",
                )
                .await;
                return RelayDisposition::Continue;
            }

            match consume_response_create_rate_limit(state, &context.decision, context.rpm_bypassed)
                .await
            {
                Ok(true) => {}
                Ok(false) => {
                    send_gateway_error(
                        client_socket,
                        "rate_limit_exceeded",
                        "Too many response.create events; retry later",
                    )
                    .await;
                    return RelayDisposition::Continue;
                }
                Err(()) => {
                    send_gateway_error(
                        client_socket,
                        "gateway_rate_limit_unavailable",
                        "Gateway could not evaluate the response rate limit",
                    )
                    .await;
                    close_client_socket(
                        client_socket,
                        CLOSE_INTERNAL_ERROR,
                        "rate_limit_unavailable",
                    )
                    .await;
                    return RelayDisposition::Close;
                }
            }

            let Some(client_event) = client_event else {
                send_gateway_error(
                    client_socket,
                    "invalid_response_create",
                    "response.create must be valid JSON",
                )
                .await;
                return RelayDisposition::Continue;
            };
            let changed_model =
                match changed_followup_response_create_model(&client_event, &bound.client_model) {
                    Ok(model) => model,
                    Err(code) => {
                        send_gateway_error(
                            client_socket,
                            code,
                            "response.create.model must be a non-empty string",
                        )
                        .await;
                        return RelayDisposition::Continue;
                    }
                };
            if let Some(requested_model) = changed_model {
                return forward_replanned_response_create(
                    bound,
                    client_socket,
                    state,
                    context,
                    client_event,
                    requested_model,
                )
                .await;
            }

            let outbound =
                match normalize_followup_response_create(&client_event, &bound.provider_model) {
                    Ok(value) => value,
                    Err(code) => {
                        send_gateway_error(
                            client_socket,
                            code,
                            "Gateway could not prepare the response.create event",
                        )
                        .await;
                        return RelayDisposition::Continue;
                    }
                };
            let provider_event = match serde_json::from_str::<Value>(&outbound) {
                Ok(event) => event,
                Err(_) => {
                    send_gateway_error(
                        client_socket,
                        "response_create_serialization_failed",
                        "Gateway could not prepare the response.create event",
                    )
                    .await;
                    return RelayDisposition::Continue;
                }
            };
            let turn_index = bound.next_turn_index;
            debug!(
                event_name = "codex_websocket_response_create_forwarding",
                log_type = "event",
                transport = WEBSOCKET_LOG_TRANSPORT,
                websocket = true,
                trace_id = %context.trace_id,
                turn_index,
                client_model = %bound.client_model,
                provider_model = %bound.provider_model,
                model_replanned = false,
                has_previous_response_id = client_event
                    .get("previous_response_id")
                    .is_some_and(|value| !value.is_null()),
                "gateway is forwarding a Codex response.create"
            );
            let turn_decision = prepare_codex_websocket_turn_decision(
                &bound.decision_template,
                Uuid::new_v4().to_string(),
                false,
                &client_event,
                &provider_event,
                &context.trace_id,
                turn_index,
            );
            let planning_parts = build_planning_parts(context);
            let mut turn = match begin_codex_websocket_turn(
                state,
                &planning_parts,
                turn_decision,
                &client_event,
            )
            .await
            {
                Ok(turn) => turn,
                Err(error) => {
                    warn!(
                        event_name = "codex_websocket_followup_turn_lifecycle_start_failed",
                        log_type = "ops",
                        transport = WEBSOCKET_LOG_TRANSPORT,
                        websocket = true,
                        trace_id = %context.trace_id,
                        error = ?error,
                        "gateway could not start Codex WebSocket follow-up usage/audit lifecycle"
                    );
                    send_gateway_error(
                        client_socket,
                        "codex_websocket_reporting_unavailable",
                        "Gateway could not start usage and audit tracking for this response",
                    )
                    .await;
                    return RelayDisposition::Continue;
                }
            };
            turn.set_provider_response_headers(bound.upstream_response_headers.clone());
            bound.active_turn = Some(turn);
            bound.next_turn_index = bound.next_turn_index.saturating_add(1);
            bound.response_in_flight = true;

            match bound.upstream.send(WreqWsMessage::text(outbound)).await {
                Ok(()) => {
                    if let Some(turn) = bound.active_turn.as_mut() {
                        turn.mark_upstream_request_sent();
                    }
                    RelayDisposition::Continue
                }
                Err(_) => RelayDisposition::UpstreamError("codex_websocket_send_failed"),
            }
        }
        AxumWsMessage::Binary(data) => bound
            .upstream
            .send(WreqWsMessage::Binary(data))
            .await
            .map(|_| RelayDisposition::Continue)
            .unwrap_or(RelayDisposition::UpstreamError(
                "codex_websocket_send_failed",
            )),
        AxumWsMessage::Ping(data) => bound
            .upstream
            .send(WreqWsMessage::Ping(data))
            .await
            .map(|_| RelayDisposition::Continue)
            .unwrap_or(RelayDisposition::UpstreamError(
                "codex_websocket_send_failed",
            )),
        AxumWsMessage::Pong(data) => bound
            .upstream
            .send(WreqWsMessage::Pong(data))
            .await
            .map(|_| RelayDisposition::Continue)
            .unwrap_or(RelayDisposition::UpstreamError(
                "codex_websocket_send_failed",
            )),
        AxumWsMessage::Close(frame) => {
            let _ = bound.upstream.send(axum_close_to_wreq(frame)).await;
            RelayDisposition::Close
        }
    }
}

async fn forward_replanned_response_create(
    bound: &mut BoundCodexConnection,
    client_socket: &mut WebSocket,
    state: &AppState,
    context: &CodexWebSocketRequestContext,
    client_event: Value,
    requested_model: String,
) -> RelayDisposition {
    let planning_parts = build_planning_parts(context);
    let client_event_text = match serde_json::to_vec(&client_event) {
        Ok(value) => Bytes::from(value),
        Err(_) => {
            send_gateway_error(
                client_socket,
                "invalid_response_create",
                "response.create must be valid JSON",
            )
            .await;
            return RelayDisposition::Continue;
        }
    };
    match request_model_local_rejection(
        state,
        Some(&context.decision),
        &planning_parts.uri,
        &planning_parts.headers,
        &client_event_text,
    )
    .await
    {
        Ok(Some(_)) => {
            send_gateway_error(
                client_socket,
                "model_not_allowed",
                "The requested model is not available to this API key",
            )
            .await;
            return RelayDisposition::Continue;
        }
        Ok(None) => {}
        Err(error) => {
            warn!(
                event_name = "codex_websocket_followup_model_access_check_failed",
                log_type = "ops",
                transport = WEBSOCKET_LOG_TRANSPORT,
                websocket = true,
                trace_id = %context.trace_id,
                requested_model = %requested_model,
                error = ?error,
                "gateway failed to evaluate follow-up WebSocket model access policy"
            );
            send_gateway_error(
                client_socket,
                "gateway_auth_unavailable",
                "Gateway could not evaluate request access",
            )
            .await;
            close_client_socket(
                client_socket,
                CLOSE_INTERNAL_ERROR,
                "gateway_auth_unavailable",
            )
            .await;
            return RelayDisposition::Close;
        }
    }

    let turn_request_id = Uuid::new_v4().to_string();
    let decision = match maybe_build_codex_responses_websocket_decision(
        state,
        &planning_parts,
        &turn_request_id,
        &context.decision,
        &client_event,
    )
    .await
    {
        Ok(Some(decision)) => decision,
        Ok(None) => {
            send_gateway_error(
                client_socket,
                "codex_provider_unavailable",
                "No eligible WebSocket-enabled Codex Responses provider is available for the requested model",
            )
            .await;
            return RelayDisposition::Continue;
        }
        Err(error) => {
            warn!(
                event_name = "codex_websocket_followup_model_planning_failed",
                log_type = "ops",
                transport = WEBSOCKET_LOG_TRANSPORT,
                websocket = true,
                trace_id = %context.trace_id,
                requested_model = %requested_model,
                error = ?error,
                "gateway failed to re-plan Codex WebSocket follow-up model"
            );
            send_gateway_error(
                client_socket,
                "codex_provider_unavailable",
                "Gateway could not prepare the requested Codex model",
            )
            .await;
            return RelayDisposition::Continue;
        }
    };
    let provider_event =
        match planned_response_create_event(&decision, &client_event).and_then(|event| {
            serde_json::from_str::<Value>(&event)
                .map_err(|_| "response_create_serialization_failed")
        }) {
            Ok(event) => event,
            Err(code) => {
                send_gateway_error(
                    client_socket,
                    code,
                    "Gateway could not prepare the requested Codex model",
                )
                .await;
                return RelayDisposition::Continue;
            }
        };
    let turn_index = bound.next_turn_index;
    let turn_decision = prepare_codex_websocket_turn_decision(
        &decision,
        turn_request_id,
        true,
        &client_event,
        &provider_event,
        &context.trace_id,
        turn_index,
    );
    let mut turn = match begin_codex_websocket_turn(
        state,
        &planning_parts,
        turn_decision,
        &client_event,
    )
    .await
    {
        Ok(turn) => turn,
        Err(error) => {
            warn!(
                event_name = "codex_websocket_replanned_turn_lifecycle_start_failed",
                log_type = "ops",
                transport = WEBSOCKET_LOG_TRANSPORT,
                websocket = true,
                trace_id = %context.trace_id,
                requested_model = %requested_model,
                error = ?error,
                "gateway could not start re-planned WebSocket usage/audit lifecycle"
            );
            send_gateway_error(
                client_socket,
                "codex_websocket_reporting_unavailable",
                "Gateway could not start usage and audit tracking for this response",
            )
            .await;
            return RelayDisposition::Continue;
        }
    };

    if decision_reuses_bound_upstream(bound, &decision) {
        let outbound = match serde_json::to_string(&provider_event) {
            Ok(outbound) => outbound,
            Err(_) => {
                spawn_codex_websocket_turn_finalization(
                    state.clone(),
                    turn,
                    CodexWebSocketTurnOutcome::upstream_send_failed(),
                );
                send_gateway_error(
                    client_socket,
                    "response_create_serialization_failed",
                    "Gateway could not prepare the requested Codex model",
                )
                .await;
                return RelayDisposition::Continue;
            }
        };
        if bound
            .upstream
            .send(WreqWsMessage::text(outbound))
            .await
            .is_err()
        {
            spawn_codex_websocket_turn_finalization(
                state.clone(),
                turn,
                CodexWebSocketTurnOutcome::upstream_send_failed(),
            );
            return RelayDisposition::UpstreamError("codex_websocket_send_failed");
        }

        turn.mark_upstream_request_sent();
        turn.set_provider_response_headers(bound.upstream_response_headers.clone());
        let provider_model =
            provider_model_from_decision(&decision).unwrap_or_else(|| bound.provider_model.clone());
        let previous_client_model = std::mem::replace(&mut bound.client_model, requested_model);
        let previous_provider_model = std::mem::replace(&mut bound.provider_model, provider_model);
        bound.decision_template = decision;
        bound.active_turn = Some(turn);
        bound.next_turn_index = bound.next_turn_index.saturating_add(1);
        bound.response_in_flight = true;
        debug!(
            event_name = "codex_websocket_followup_model_replanned",
            log_type = "event",
            transport = WEBSOCKET_LOG_TRANSPORT,
            websocket = true,
            trace_id = %context.trace_id,
            turn_index,
            previous_client_model = %previous_client_model,
            client_model = %bound.client_model,
            previous_provider_model = %previous_provider_model,
            provider_model = %bound.provider_model,
            upstream_rebound = false,
            model_replanned = true,
            "gateway re-planned a Codex WebSocket model on the existing upstream"
        );
        return RelayDisposition::Continue;
    }

    let mut replacement = match bind_codex_upstream(&decision, &client_event).await {
        Ok(connection) => connection,
        Err(code) => {
            spawn_codex_websocket_turn_finalization(
                state.clone(),
                turn,
                CodexWebSocketTurnOutcome::upstream_connect_failed(code),
            );
            warn!(
                event_name = "codex_websocket_followup_model_rebind_failed",
                log_type = "ops",
                transport = WEBSOCKET_LOG_TRANSPORT,
                websocket = true,
                trace_id = %context.trace_id,
                requested_model = %requested_model,
                error_code = code,
                "gateway failed to rebind Codex WebSocket follow-up model"
            );
            send_gateway_error(
                client_socket,
                code,
                "Gateway could not establish the requested Codex model",
            )
            .await;
            return RelayDisposition::Continue;
        }
    };

    turn.mark_upstream_request_sent();
    turn.set_provider_response_headers(replacement.upstream_response_headers.clone());
    let previous_client_model = bound.client_model.clone();
    let previous_provider_model = bound.provider_model.clone();
    let _ = bound.upstream.send(WreqWsMessage::Close(None)).await;
    std::mem::swap(&mut bound.upstream, &mut replacement.upstream);
    bound.client_model = replacement.client_model;
    bound.provider_model = replacement.provider_model;
    bound.response_in_flight = replacement.response_in_flight;
    bound.decision_template = replacement.decision_template;
    bound.active_turn = Some(turn);
    bound.next_turn_index = bound.next_turn_index.saturating_add(1);
    bound.upstream_response_headers = replacement.upstream_response_headers;
    debug!(
        event_name = "codex_websocket_followup_model_rebound",
        log_type = "event",
        transport = WEBSOCKET_LOG_TRANSPORT,
        websocket = true,
        trace_id = %context.trace_id,
        turn_index,
        previous_client_model = %previous_client_model,
        requested_model = %requested_model,
        previous_provider_model = %previous_provider_model,
        provider_model = %bound.provider_model,
        upstream_rebound = true,
        model_replanned = true,
        "gateway rebound Codex WebSocket for a follow-up model"
    );
    RelayDisposition::Continue
}

async fn consume_response_create_rate_limit(
    state: &AppState,
    decision: &GatewayControlDecision,
    rpm_bypassed: bool,
) -> Result<bool, ()> {
    if rpm_bypassed {
        return Ok(true);
    }
    match state
        .frontdoor_user_rpm()
        .check_and_consume(state, Some(decision))
        .await
        .map_err(|_| ())?
    {
        FrontdoorUserRpmOutcome::Rejected(_) => Ok(false),
        FrontdoorUserRpmOutcome::Allowed | FrontdoorUserRpmOutcome::NotApplicable => Ok(true),
    }
}

fn changed_followup_response_create_model(
    event: &Value,
    current_client_model: &str,
) -> Result<Option<String>, &'static str> {
    let Some(object) = event.as_object() else {
        return Err("invalid_response_create");
    };
    let Some(model) = object.get("model") else {
        return Ok(None);
    };
    let Some(model) = model
        .as_str()
        .map(str::trim)
        .filter(|model| !model.is_empty())
    else {
        return Err("invalid_response_create_model");
    };
    if model.eq_ignore_ascii_case(current_client_model) {
        Ok(None)
    } else {
        Ok(Some(model.to_string()))
    }
}

fn decision_reuses_bound_upstream(
    bound: &BoundCodexConnection,
    decision: &AiExecutionDecision,
) -> bool {
    decisions_reuse_upstream(&bound.decision_template, decision)
}

fn decisions_reuse_upstream(current: &AiExecutionDecision, decision: &AiExecutionDecision) -> bool {
    current.provider_id == decision.provider_id
        && current.endpoint_id == decision.endpoint_id
        && current.key_id == decision.key_id
        && current.upstream_url == decision.upstream_url
        && current.provider_request_headers == decision.provider_request_headers
}

fn provider_model_from_decision(decision: &AiExecutionDecision) -> Option<String> {
    decision
        .provider_request_body
        .as_ref()
        .and_then(|body| body.get("model"))
        .and_then(Value::as_str)
        .or(decision.mapped_model.as_deref())
        .map(str::trim)
        .filter(|model| !model.is_empty())
        .map(str::to_string)
}

fn normalize_followup_response_create(
    event: &Value,
    provider_model: &str,
) -> Result<String, &'static str> {
    let mut event = event.clone();
    let Some(object) = event.as_object_mut() else {
        return Err("invalid_response_create");
    };
    if object.get("type").and_then(Value::as_str) != Some("response.create") {
        return Err("invalid_response_create");
    }
    object.insert(
        "model".to_string(),
        Value::String(provider_model.to_string()),
    );
    object.remove("stream");
    object.remove("background");
    serde_json::to_string(&event).map_err(|_| "response_create_serialization_failed")
}

fn update_response_in_flight(bound: &mut BoundCodexConnection, message: &WreqWsMessage) {
    let WreqWsMessage::Text(text) = message else {
        return;
    };
    let Some(event_type) = serde_json::from_str::<Value>(text.as_str())
        .ok()
        .and_then(|event| {
            event
                .get("type")
                .and_then(Value::as_str)
                .map(str::to_string)
        })
    else {
        return;
    };
    match event_type.as_str() {
        "response.created" | "response.in_progress" | "response.queued" => {
            bound.response_in_flight = true;
        }
        "response.completed"
        | "response.failed"
        | "response.incomplete"
        | "response.cancelled"
        | "error" => bound.response_in_flight = false,
        _ => {}
    }
}

fn websocket_event_type_for_log(text: &str) -> String {
    serde_json::from_str::<Value>(text)
        .ok()
        .and_then(|event| {
            event
                .get("type")
                .and_then(Value::as_str)
                .map(safe_websocket_event_label)
        })
        .unwrap_or_else(|| "invalid_json".to_string())
}

fn safe_websocket_event_label(value: &str) -> String {
    let value = value.trim();
    if value.is_empty()
        || value.len() > 80
        || !value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'_' | b'-'))
    {
        return "unknown".to_string();
    }
    value.to_string()
}

fn wreq_message_to_axum(message: WreqWsMessage) -> AxumWsMessage {
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

fn axum_close_to_wreq(frame: Option<AxumCloseFrame>) -> WreqWsMessage {
    WreqWsMessage::Close(frame.map(|frame| WreqCloseFrame {
        code: frame.code.into(),
        reason: frame.reason.to_string().into(),
    }))
}

async fn send_gateway_error(client_socket: &mut WebSocket, code: &str, message: &str) {
    let event = json!({
        "type": "error",
        "error": {
            "type": "gateway_error",
            "code": code,
            "message": message,
        },
    });
    let _ = client_socket
        .send(AxumWsMessage::Text(event.to_string().into()))
        .await;
}

async fn close_client_socket(client_socket: &mut WebSocket, code: u16, reason: &str) {
    let _ = client_socket
        .send(AxumWsMessage::Close(Some(AxumCloseFrame {
            code,
            reason: reason.to_string().into(),
        })))
        .await;
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;
    use std::sync::Arc;
    use std::time::{Duration, Instant};

    use axum::extract::ws::{Message, WebSocket, WebSocketUpgrade};
    use axum::extract::State;
    use axum::http::header::{AUTHORIZATION, CONTENT_TYPE};
    use axum::http::HeaderMap;
    use axum::response::IntoResponse;
    use axum::routing::get;
    use axum::Router;
    use futures_util::{SinkExt, StreamExt};
    use serde_json::json;
    use tokio::sync::{oneshot, Mutex};
    use wreq::ws::message::Message as WreqWsMessage;

    use super::super::codex_ws_finalize::{
        CodexWebSocketTurnObservation, CodexWebSocketTurnOutcome, CodexWebSocketTurnTimeoutPhase,
    };
    use super::{
        bind_codex_upstream, changed_followup_response_create_model,
        codex_websocket_rate_limits_from_message, decisions_reuse_upstream,
        normalize_followup_response_create, planned_response_create_event,
        wait_for_active_turn_deadline, websocket_event_type_for_log, websocket_handshake_headers,
        websocket_timeouts, websocket_upstream_url, CodexWebSocketTurnDeadline,
    };
    use crate::ai_serving::AiExecutionDecision;

    #[derive(Default)]
    struct MockState {
        observed: Mutex<Option<oneshot::Sender<ObservedInitialEvent>>>,
    }

    struct ObservedInitialEvent {
        authorization_present: bool,
        account_header_present: bool,
        event: serde_json::Value,
    }

    #[test]
    fn websocket_message_exposes_exhausted_codex_rate_limits() {
        let message = WreqWsMessage::text(
            json!({
                "chunks": [{
                    "type": "codex.rate_limits",
                    "rate_limits": {
                        "allowed": false,
                        "limit_reached": true
                    }
                }]
            })
            .to_string(),
        );
        let parsed = codex_websocket_rate_limits_from_message(&message, 1_787_000_000)
            .expect("rate-limit message should parse");

        assert!(aether_admin::provider::quota::codex_rate_limit_metadata_exhausted(&parsed));
        assert_eq!(parsed.get("updated_at"), Some(&json!(1_787_000_000u64)));
    }

    #[test]
    fn quota_drain_waits_for_an_active_turn_terminal_event() {
        assert!(!super::quota_drain_ready(true, true, None));
        assert!(super::quota_drain_ready(
            true,
            true,
            Some(CodexWebSocketTurnObservation::Terminal(
                CodexWebSocketTurnOutcome::upstream_closed()
            ))
        ));
        assert!(!super::quota_drain_ready(false, false, None));
        assert!(super::quota_drain_ready(true, false, None));
    }

    #[test]
    fn maps_http_codex_url_to_websocket_url_without_losing_path_or_query() {
        let url = websocket_upstream_url("https://example.test/backend-api/codex/responses?x=1")
            .expect("URL should convert");
        assert_eq!(
            url.as_str(),
            "wss://example.test/backend-api/codex/responses?x=1"
        );
    }

    #[test]
    fn rejects_embedded_upstream_credentials() {
        assert!(websocket_upstream_url("https://token@example.test/responses").is_err());
    }

    #[test]
    fn strips_http_entity_headers_from_websocket_handshake() {
        let headers = websocket_handshake_headers(&BTreeMap::from([
            (
                "authorization".to_string(),
                "Bearer provider-token".to_string(),
            ),
            ("chatgpt-account-id".to_string(), "account-id".to_string()),
            ("content-type".to_string(), "application/json".to_string()),
        ]))
        .expect("headers should build");
        assert!(headers.contains_key(AUTHORIZATION));
        assert!(!headers.contains_key(CONTENT_TYPE));
    }

    #[test]
    fn planned_event_uses_mapped_model_and_removes_http_stream_fields() {
        let mut decision = sample_decision();
        decision.provider_request_body = Some(json!({
            "model": "provider-model",
            "input": "hello",
            "stream": true,
            "background": true,
        }));
        let event = planned_response_create_event(
            &decision,
            &json!({"type": "response.create", "model": "public-model"}),
        )
        .expect("event should serialize");
        let event: serde_json::Value = serde_json::from_str(&event).expect("event JSON");
        assert_eq!(event["type"], "response.create");
        assert_eq!(event["model"], "provider-model");
        assert!(event.get("stream").is_none());
        assert!(event.get("background").is_none());
    }

    #[test]
    fn followup_rewrites_the_provider_model_and_removes_http_stream_fields() {
        let event = json!({
            "type": "response.create",
            "model": "public-model",
            "stream": true,
            "background": true,
        });
        let normalized = normalize_followup_response_create(&event, "provider-model")
            .expect("response.create should be normalized");
        let event: serde_json::Value = serde_json::from_str(&normalized).expect("event JSON");
        assert_eq!(event["model"], "provider-model");
        assert!(event.get("stream").is_none());
        assert!(event.get("background").is_none());
    }

    #[test]
    fn followup_model_change_requires_per_turn_replanning() {
        let prewarm = json!({
            "type": "response.create",
            "model": "gpt-5.6-sol",
            "generate": false,
        });
        let turn = json!({
            "type": "response.create",
            "model": "gpt-5.6-terra",
            "input": [{"role": "user", "content": "hello"}],
        });

        assert_eq!(
            changed_followup_response_create_model(&prewarm, "gpt-5.6-sol"),
            Ok(None)
        );
        assert_eq!(
            changed_followup_response_create_model(&turn, "gpt-5.6-sol"),
            Ok(Some("gpt-5.6-terra".to_string()))
        );
    }

    #[test]
    fn followup_without_a_model_reuses_the_current_connection_model() {
        let event = json!({
            "type": "response.create",
            "input": "continue",
        });

        assert_eq!(
            changed_followup_response_create_model(&event, "gpt-5.6-sol"),
            Ok(None)
        );
    }

    #[test]
    fn replanned_model_reuses_only_the_same_upstream_target() {
        let mut current = sample_decision();
        current.provider_id = Some("provider-1".to_string());
        current.endpoint_id = Some("endpoint-1".to_string());
        current.key_id = Some("key-1".to_string());
        current.provider_request_headers =
            BTreeMap::from([("authorization".to_string(), "Bearer token-1".to_string())]);
        let mut replanned = current.clone();
        replanned.provider_request_body = Some(json!({"model": "gpt-5.6-terra"}));

        assert!(decisions_reuse_upstream(&current, &replanned));

        replanned.key_id = Some("key-2".to_string());
        replanned
            .provider_request_headers
            .insert("authorization".to_string(), "Bearer token-2".to_string());
        assert!(!decisions_reuse_upstream(&current, &replanned));
    }

    #[test]
    fn websocket_transport_keeps_only_the_connect_timeout() {
        let mut decision = sample_decision();
        decision.timeouts = Some(aether_contracts::ExecutionTimeouts {
            connect_ms: Some(123),
            read_ms: Some(456),
            first_byte_ms: Some(789),
            total_ms: Some(1_000),
            ..aether_contracts::ExecutionTimeouts::default()
        });

        let timeouts = websocket_timeouts(&decision).expect("timeouts should be retained");
        assert_eq!(timeouts.connect_ms, Some(123));
        assert_eq!(timeouts.read_ms, None);
        assert_eq!(timeouts.first_byte_ms, None);
        assert_eq!(timeouts.total_ms, None);
    }

    #[test]
    fn upstream_event_log_label_never_uses_untrusted_text() {
        assert_eq!(
            websocket_event_type_for_log(r#"{"type":"response.in_progress"}"#),
            "response.in_progress"
        );
        assert_eq!(
            websocket_event_type_for_log(r#"{"type":"not safe / contains spaces"}"#),
            "unknown"
        );
        assert_eq!(websocket_event_type_for_log("not-json"), "invalid_json");
    }

    #[tokio::test]
    async fn expired_turn_deadline_returns_without_waiting_for_socket_io() {
        let deadline = CodexWebSocketTurnDeadline {
            phase: CodexWebSocketTurnTimeoutPhase::AwaitingFirstEvent,
            deadline: Instant::now() - Duration::from_millis(1),
            timeout: Duration::from_secs(1),
        };

        tokio::time::timeout(
            Duration::from_millis(50),
            wait_for_active_turn_deadline(Some(deadline)),
        )
        .await
        .expect("expired deadline should resolve immediately");
    }

    #[tokio::test]
    async fn upstream_binding_uses_provider_headers_and_rewrites_the_first_event() {
        let (upstream_url, observed, server) = spawn_mock_server().await;
        let mut decision = sample_decision();
        decision.upstream_url = Some(upstream_url);
        decision.provider_request_headers = BTreeMap::from([
            (
                "authorization".to_string(),
                "Bearer provider-token".to_string(),
            ),
            ("chatgpt-account-id".to_string(), "account-id".to_string()),
            ("content-type".to_string(), "application/json".to_string()),
        ]);
        decision.provider_request_body = Some(json!({
            "model": "provider-model",
            "input": "hello",
            "stream": true,
            "background": true,
        }));

        let mut bound = bind_codex_upstream(
            &decision,
            &json!({
                "type": "response.create",
                "model": "public-model",
                "input": "hello",
            }),
        )
        .await
        .expect("upstream binding should succeed");
        let observed = tokio::time::timeout(Duration::from_secs(2), observed)
            .await
            .expect("mock should observe first event")
            .expect("mock event channel should remain open");
        let response = tokio::time::timeout(Duration::from_secs(2), bound.upstream.recv())
            .await
            .expect("mock should send a response event")
            .expect("upstream should remain open")
            .expect("upstream response should be valid");
        server.abort();

        assert!(observed.authorization_present);
        assert!(observed.account_header_present);
        assert_eq!(observed.event["type"], "response.create");
        assert_eq!(observed.event["model"], "provider-model");
        assert!(observed.event.get("stream").is_none());
        assert!(observed.event.get("background").is_none());
        assert!(matches!(response, wreq::ws::message::Message::Text(_)));
    }

    async fn spawn_mock_server() -> (
        String,
        oneshot::Receiver<ObservedInitialEvent>,
        tokio::task::JoinHandle<()>,
    ) {
        let (observed_tx, observed_rx) = oneshot::channel();
        let state = Arc::new(MockState {
            observed: Mutex::new(Some(observed_tx)),
        });
        let app = Router::new()
            .route("/backend-api/codex/responses", get(mock_websocket))
            .with_state(state);
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("mock listener should bind");
        let address = listener
            .local_addr()
            .expect("mock listener should expose address");
        let server = tokio::spawn(async move {
            axum::serve(listener, app)
                .await
                .expect("mock server should run");
        });
        (
            format!("http://{address}/backend-api/codex/responses"),
            observed_rx,
            server,
        )
    }

    async fn mock_websocket(
        ws: WebSocketUpgrade,
        State(state): State<Arc<MockState>>,
        headers: HeaderMap,
    ) -> impl IntoResponse {
        let authorization_present = headers
            .get("authorization")
            .and_then(|value| value.to_str().ok())
            .is_some_and(|value| value.starts_with("Bearer "));
        let account_header_present = headers.contains_key("chatgpt-account-id");
        ws.on_upgrade(move |socket| async move {
            serve_mock_socket(socket, state, authorization_present, account_header_present).await;
        })
    }

    async fn serve_mock_socket(
        socket: WebSocket,
        state: Arc<MockState>,
        authorization_present: bool,
        account_header_present: bool,
    ) {
        let (mut sender, mut receiver) = socket.split();
        let message = receiver
            .next()
            .await
            .expect("client should send the initial event")
            .expect("initial event should be valid");
        let Message::Text(text) = message else {
            panic!("expected a text response.create event");
        };
        let event = serde_json::from_str(text.as_str()).expect("event should be JSON");
        let _ = sender
            .send(Message::Text(
                json!({"type": "response.created", "response": {"id": "resp-test"}})
                    .to_string()
                    .into(),
            ))
            .await;
        if let Some(observed) = state.observed.lock().await.take() {
            let _ = observed.send(ObservedInitialEvent {
                authorization_present,
                account_header_present,
                event,
            });
        }
    }

    fn sample_decision() -> AiExecutionDecision {
        AiExecutionDecision {
            action: "local".to_string(),
            decision_kind: None,
            execution_strategy: None,
            conversion_mode: None,
            request_id: None,
            candidate_id: None,
            provider_name: None,
            provider_type: Some("codex".to_string()),
            provider_id: None,
            endpoint_id: None,
            key_id: None,
            upstream_base_url: None,
            upstream_url: Some("https://example.test/backend-api/codex/responses".to_string()),
            provider_request_method: None,
            auth_header: None,
            auth_value: None,
            provider_api_format: Some("openai:responses".to_string()),
            client_api_format: Some("openai:responses".to_string()),
            provider_contract: None,
            client_contract: None,
            model_name: None,
            mapped_model: Some("provider-model".to_string()),
            prompt_cache_key: None,
            extra_headers: BTreeMap::new(),
            provider_request_headers: BTreeMap::new(),
            provider_request_body: None,
            provider_request_body_base64: None,
            content_type: None,
            content_encoding: None,
            request_gzip: None,
            proxy: None,
            transport_profile: None,
            timeouts: None,
            upstream_is_stream: true,
            report_kind: None,
            report_context: None,
            auth_context: None,
        }
    }
}
