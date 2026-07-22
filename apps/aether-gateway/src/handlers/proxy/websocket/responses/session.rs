//! Standard OpenAI Responses WebSocket session engine.
//!
//! An incoming client socket is authenticated at Upgrade time. Its first
//! `response.create` selects a provider through the normal Responses planner.
//! Later turns reuse that upstream while the requested model remains eligible
//! on the selected key. A model change is planned again and keeps the current
//! upstream when the planner resolves to the same target; otherwise the bridge
//! transparently replaces the upstream between responses.

use axum::body::Bytes;
use axum::extract::ws::{Message as AxumWsMessage, WebSocket};
use axum::http::header::{AUTHORIZATION, CONNECTION, CONTENT_TYPE, UPGRADE};
use axum::http::Method;
use futures_util::{SinkExt, StreamExt};
use serde_json::Value;
use std::collections::{BTreeMap, BTreeSet};
use tokio::time::{sleep, timeout};
use uuid::Uuid;
use wreq::ws::message::Message as WreqWsMessage;

use super::adapter::{
    resolve_responses_websocket_adapter, ResponsesWebSocketDrainDirective,
    ResponsesWebSocketProtocolAdapter,
};
use super::turn::{
    begin_responses_websocket_turn, prepare_responses_websocket_turn_decision,
    spawn_responses_websocket_turn_finalization, ResponsesWebSocketTurn,
    ResponsesWebSocketTurnDeadline, ResponsesWebSocketTurnObservation,
    ResponsesWebSocketTurnOutcome,
};

use crate::ai_serving::{maybe_build_responses_websocket_decision, AiExecutionDecision};
use crate::control::{request_model_local_rejection, GatewayControlDecision};
use crate::handlers::proxy::websocket::ingress::{
    WebSocketConnectionLog, WebSocketConnectionLogSpec, WebSocketRequestContext,
};
use crate::handlers::proxy::websocket::session::{
    wait_for_optional_deadline, CLOSE_INTERNAL_ERROR, CLOSE_POLICY_VIOLATION, CLOSE_TRY_AGAIN,
    RESPONSES_WEBSOCKET_SESSION_LIMITS, WEBSOCKET_LOG_TRANSPORT,
};
use crate::handlers::proxy::websocket::transport::{
    client_close_to_upstream, close_client_socket, connect_upstream_websocket, send_gateway_error,
    upstream_message_to_client,
};
use crate::headers::request_origin_from_headers_and_remote_addr;
use crate::rate_limit::FrontdoorUserRpmOutcome;
use crate::AppState;

const RESPONSES_WEBSOCKET_LOG_TARGET: &str = "aether_gateway::handlers::proxy::responses_ws";

const RESPONSES_CONNECTION_LOG_SPEC: WebSocketConnectionLogSpec = WebSocketConnectionLogSpec {
    opened_event_name: "responses_websocket_connection_opened",
    closed_event_name: "responses_websocket_connection_closed",
    opened_message: "gateway accepted Responses WebSocket connection",
    closed_message: "gateway closed Responses WebSocket connection",
    execution_path: "responses_websocket_bridge",
    provider_type: "responses",
};

macro_rules! debug {
    ($($arg:tt)*) => {
        tracing::debug!(target: RESPONSES_WEBSOCKET_LOG_TARGET, $($arg)*)
    };
}

macro_rules! warn {
    ($($arg:tt)*) => {
        tracing::warn!(target: RESPONSES_WEBSOCKET_LOG_TARGET, $($arg)*)
    };
}

type ResponsesWebSocketRequestContext = WebSocketRequestContext;

struct BoundResponsesConnection {
    upstream: Option<wreq::ws::WebSocket>,
    adapter: &'static dyn ResponsesWebSocketProtocolAdapter,
    client_model: String,
    provider_model: String,
    response_in_flight: bool,
    decision_template: AiExecutionDecision,
    active_turn: Option<ResponsesWebSocketTurn>,
    active_response_create: Option<ActiveResponsesWebSocketRequest>,
    next_turn_index: u64,
    upstream_response_headers: BTreeMap<String, String>,
    pending_adapter_drain: Option<ResponsesWebSocketDrainDirective>,
    exhausted_key_ids: BTreeSet<String>,
}

#[derive(Debug, Clone)]
struct ActiveResponsesWebSocketRequest {
    client_event: Value,
    turn_index: u64,
    retry_attempted: bool,
    standard_response_started: bool,
}

impl ActiveResponsesWebSocketRequest {
    fn new(client_event: Value, turn_index: u64) -> Self {
        Self {
            client_event,
            turn_index,
            retry_attempted: false,
            standard_response_started: false,
        }
    }

    fn can_retry_after_quota_exhaustion(&self) -> bool {
        !self.retry_attempted
            && !self.standard_response_started
            && !response_create_has_previous_response_id(&self.client_event)
    }
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

pub(super) async fn run_responses_websocket(
    mut client_socket: WebSocket,
    state: AppState,
    context: ResponsesWebSocketRequestContext,
) {
    let connection_log = WebSocketConnectionLog::new(&context, RESPONSES_CONNECTION_LOG_SPEC);
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
                event_name = "responses_websocket_rate_limit_check_failed",
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
                event_name = "responses_websocket_model_access_check_failed",
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

    let planned = match maybe_build_responses_websocket_decision(
        &state,
        &planning_parts,
        &context.trace_id,
        &context.decision,
        &first_event,
        None,
    )
    .await
    {
        Ok(Some(decision)) => decision,
        Ok(None) => {
            send_gateway_error(
                &mut client_socket,
                "responses_provider_unavailable",
                "No eligible WebSocket-enabled Responses provider is available",
            )
            .await;
            close_client_socket(
                &mut client_socket,
                CLOSE_TRY_AGAIN,
                "responses_provider_unavailable",
            )
            .await;
            return;
        }
        Err(_) => {
            warn!(
                event_name = "responses_websocket_planning_failed",
                log_type = "ops",
                transport = WEBSOCKET_LOG_TRANSPORT,
                websocket = true,
                trace_id = %context.trace_id,
                "gateway failed to plan Responses WebSocket provider request"
            );
            send_gateway_error(
                &mut client_socket,
                "responses_provider_unavailable",
                "Gateway could not prepare a Provider connection",
            )
            .await;
            close_client_socket(
                &mut client_socket,
                CLOSE_INTERNAL_ERROR,
                "responses_planning_failed",
            )
            .await;
            return;
        }
    };

    let adapter = resolve_responses_websocket_adapter(planned.adapter);
    let decision = planned.execution;
    let first_provider_event = match planned_response_create_event(&decision, &first_event)
        .and_then(|event| {
            serde_json::from_str::<Value>(&event).map_err(|_| "responses_websocket_request_invalid")
        }) {
        Ok(event) => event,
        Err(code) => {
            warn!(
                event_name = "responses_websocket_initial_event_normalization_failed",
                log_type = "ops",
                transport = WEBSOCKET_LOG_TRANSPORT,
                websocket = true,
                trace_id = %context.trace_id,
                error_code = code,
                "gateway could not normalize the initial Responses WebSocket event"
            );
            send_gateway_error(
                &mut client_socket,
                code,
                "Gateway could not prepare the Responses response.create event",
            )
            .await;
            close_client_socket(&mut client_socket, CLOSE_POLICY_VIOLATION, code).await;
            return;
        }
    };
    let first_turn_decision = prepare_responses_websocket_turn_decision(
        &decision,
        context.trace_id.clone(),
        true,
        &first_event,
        &first_provider_event,
        &context.trace_id,
        1,
    );
    let mut first_turn = match begin_responses_websocket_turn(
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
                event_name = "responses_websocket_turn_lifecycle_start_failed",
                log_type = "ops",
                transport = WEBSOCKET_LOG_TRANSPORT,
                websocket = true,
                trace_id = %context.trace_id,
                error = ?error,
                "gateway could not start Responses WebSocket usage/audit lifecycle"
            );
            send_gateway_error(
                &mut client_socket,
                "responses_websocket_reporting_unavailable",
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

    let mut bound = match bind_responses_upstream(&decision, &first_event, adapter).await {
        Ok(connection) => connection,
        Err(code) => {
            spawn_responses_websocket_turn_finalization(
                state.clone(),
                first_turn,
                ResponsesWebSocketTurnOutcome::upstream_connect_failed(code),
            );
            warn!(
                event_name = "responses_websocket_upstream_connect_failed",
                log_type = "ops",
                transport = WEBSOCKET_LOG_TRANSPORT,
                websocket = true,
                trace_id = %context.trace_id,
                error_code = code,
                "gateway failed to establish Responses WebSocket upstream"
            );
            send_gateway_error(
                &mut client_socket,
                code,
                "Gateway could not establish the Provider connection",
            )
            .await;
            close_client_socket(&mut client_socket, CLOSE_TRY_AGAIN, code).await;
            return;
        }
    };
    first_turn.mark_upstream_request_sent();
    first_turn.set_provider_response_headers(bound.upstream_response_headers.clone());
    bound.active_turn = Some(first_turn);
    bound.active_response_create = Some(ActiveResponsesWebSocketRequest::new(first_event, 1));

    relay_bound_connection(&mut client_socket, &mut bound, &state, &context).await;
}

async fn receive_initial_response_create(
    client_socket: &mut WebSocket,
) -> Result<(String, Value), InitialMessageError> {
    loop {
        let message = timeout(
            RESPONSES_WEBSOCKET_SESSION_LIMITS.initial_message_timeout,
            client_socket.next(),
        )
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

fn build_planning_parts(context: &ResponsesWebSocketRequestContext) -> http::request::Parts {
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

async fn bind_responses_upstream(
    decision: &AiExecutionDecision,
    initial_event: &Value,
    adapter: &'static dyn ResponsesWebSocketProtocolAdapter,
) -> Result<BoundResponsesConnection, &'static str> {
    let mut upstream = connect_upstream_websocket(
        decision,
        RESPONSES_WEBSOCKET_SESSION_LIMITS,
        adapter.upstream_errors(),
    )
    .await?;
    let first_event = planned_response_create_event(decision, initial_event)?;
    upstream
        .socket
        .send(WreqWsMessage::text(first_event))
        .await
        .map_err(|_| "responses_websocket_initial_send_failed")?;

    let client_model = initial_event
        .get("model")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .ok_or("responses_websocket_model_missing")?
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
        .ok_or("responses_websocket_mapped_model_missing")?
        .to_string();

    Ok(BoundResponsesConnection {
        upstream: Some(upstream.socket),
        adapter,
        client_model,
        provider_model,
        response_in_flight: true,
        decision_template: decision.clone(),
        active_turn: None,
        active_response_create: None,
        next_turn_index: 2,
        upstream_response_headers: upstream.response_headers,
        pending_adapter_drain: None,
        exhausted_key_ids: BTreeSet::new(),
    })
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
        .ok_or("responses_websocket_request_invalid")?;
    object.insert(
        "type".to_string(),
        Value::String("response.create".to_string()),
    );
    object.remove("stream");
    object.remove("background");
    serde_json::to_string(&event).map_err(|_| "responses_websocket_request_invalid")
}

async fn relay_bound_connection(
    client_socket: &mut WebSocket,
    bound: &mut BoundResponsesConnection,
    state: &AppState,
    context: &ResponsesWebSocketRequestContext,
) {
    let connection_deadline = sleep(RESPONSES_WEBSOCKET_SESSION_LIMITS.max_connection_duration);
    tokio::pin!(connection_deadline);

    loop {
        let active_turn_deadline = bound
            .active_turn
            .as_ref()
            .map(ResponsesWebSocketTurn::deadline);
        let upstream_available = bound.upstream.is_some();
        tokio::select! {
            _ = &mut connection_deadline => {
                finalize_active_turn(
                    bound,
                    state,
                    ResponsesWebSocketTurnOutcome::connection_limit_reached(),
                );
                send_gateway_error(
                    client_socket,
                    "websocket_connection_limit_reached",
                    "WebSocket connection duration limit reached; reconnect to continue",
                ).await;
                bound.active_response_create = None;
                close_bound_upstream(bound).await;
                close_client_socket(client_socket, CLOSE_TRY_AGAIN, "connection_limit_reached").await;
                break;
            }
            _ = wait_for_optional_deadline(active_turn_deadline.map(|deadline| deadline.deadline)) => {
                let Some(turn_deadline) = active_turn_deadline else {
                    continue;
                };
                warn!(
                    event_name = "responses_websocket_turn_timeout",
                    log_type = "ops",
                    transport = WEBSOCKET_LOG_TRANSPORT,
                    websocket = true,
                    trace_id = %context.trace_id,
                    timeout_phase = ?turn_deadline.phase,
                    timeout_ms = turn_deadline.timeout.as_millis() as u64,
                    "Responses WebSocket response did not reach its configured deadline"
                );
                finalize_active_turn(bound, state, turn_deadline.phase.outcome());
                send_gateway_error(
                    client_socket,
                    turn_deadline.phase.error_code(),
                    turn_deadline.phase.client_message(),
                ).await;
                bound.active_response_create = None;
                close_bound_upstream(bound).await;
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
                        ResponsesWebSocketTurnOutcome::client_disconnected(),
                    );
                    bound.active_response_create = None;
                    close_bound_upstream(bound).await;
                    break;
                };
                let Ok(client_message) = client_message else {
                    warn!(
                        event_name = "responses_websocket_client_receive_failed",
                        log_type = "ops",
                        transport = WEBSOCKET_LOG_TRANSPORT,
                        websocket = true,
                        trace_id = %context.trace_id,
                        "client WebSocket receive failed"
                    );
                    finalize_active_turn(
                        bound,
                        state,
                        ResponsesWebSocketTurnOutcome::client_disconnected(),
                    );
                    bound.active_response_create = None;
                    close_bound_upstream(bound).await;
                    break;
                };
                match forward_client_message(client_message, bound, client_socket, state, context).await {
                    RelayDisposition::Continue => {}
                    RelayDisposition::Close => {
                        finalize_active_turn(
                            bound,
                            state,
                            ResponsesWebSocketTurnOutcome::client_disconnected(),
                        );
                        break;
                    }
                    RelayDisposition::UpstreamError(code) => {
                        warn!(
                            event_name = "responses_websocket_upstream_send_failed",
                            log_type = "ops",
                            transport = WEBSOCKET_LOG_TRANSPORT,
                            websocket = true,
                            trace_id = %context.trace_id,
                            error_code = code,
                            "Upstream WebSocket send failed"
                        );
                        finalize_active_turn(
                            bound,
                            state,
                            ResponsesWebSocketTurnOutcome::upstream_send_failed(),
                        );
                        send_gateway_error(
                            client_socket,
                            code,
                            "Gateway could not forward the WebSocket event upstream",
                        ).await;
                        bound.active_response_create = None;
                        close_bound_upstream(bound).await;
                        close_client_socket(client_socket, CLOSE_INTERNAL_ERROR, code).await;
                        break;
                    }
                }
            }
            upstream_message = bound.upstream.as_mut().expect("upstream should be present while selected").recv(), if upstream_available => {
                let Some(upstream_message) = upstream_message else {
                    finalize_active_turn(
                        bound,
                        state,
                        ResponsesWebSocketTurnOutcome::upstream_closed(),
                    );
                    bound.active_response_create = None;
                    bound.upstream = None;
                    close_client_socket(client_socket, 1000, "upstream_closed").await;
                    break;
                };
                let Ok(upstream_message) = upstream_message else {
                    warn!(
                        event_name = "responses_websocket_upstream_receive_failed",
                        log_type = "ops",
                        transport = WEBSOCKET_LOG_TRANSPORT,
                        websocket = true,
                        trace_id = %context.trace_id,
                        "Upstream WebSocket receive failed"
                    );
                    finalize_active_turn(
                        bound,
                        state,
                        ResponsesWebSocketTurnOutcome::upstream_receive_failed(),
                    );
                    send_gateway_error(
                        client_socket,
                        "responses_websocket_receive_failed",
                        "Provider connection closed unexpectedly",
                    ).await;
                    bound.active_response_create = None;
                    bound.upstream = None;
                    close_client_socket(client_socket, CLOSE_INTERNAL_ERROR, "upstream_receive_failed").await;
                    break;
                };
                if let WreqWsMessage::Text(text) = &upstream_message {
                    debug!(
                        event_name = "responses_websocket_upstream_event",
                        log_type = "event",
                        transport = WEBSOCKET_LOG_TRANSPORT,
                        websocket = true,
                        trace_id = %context.trace_id,
                        event_type = %websocket_event_type_for_log(text.as_str()),
                        frame_bytes = text.len(),
                        active_turn = bound.active_turn.is_some(),
                        "gateway received Responses WebSocket event"
                    );
                }
                if let WreqWsMessage::Text(text) = &upstream_message {
                    if bound.pending_adapter_drain.is_none()
                        && bound.adapter.observes_upstream_events()
                    {
                        if let Ok(event) = serde_json::from_str::<Value>(text.as_str()) {
                            let adapter = bound.adapter;
                            let report_context = bound.decision_template.report_context.clone();
                            if let Some(directive) = adapter
                                .observe_upstream_event(
                                    state,
                                    &context.trace_id,
                                    report_context.as_ref(),
                                    &event,
                                )
                                .await
                            {
                                bound.pending_adapter_drain = Some(directive);
                            }
                        }
                    }
                }
                let observation = match &upstream_message {
                    WreqWsMessage::Text(text) => {
                        let adapter = bound.adapter;
                        bound
                            .active_turn
                            .as_mut()
                            .and_then(|turn| turn.observe_upstream_text(text.as_str(), adapter))
                    }
                    _ => None,
                };
                update_response_in_flight(bound, &upstream_message);
                if matches!(
                    observation,
                    Some(ResponsesWebSocketTurnObservation::Started)
                        | Some(ResponsesWebSocketTurnObservation::Terminal(_))
                ) {
                    if let Some(turn) = bound.active_turn.as_mut() {
                        turn.mark_stream_started(state).await;
                    }
                }
                if let Some(ResponsesWebSocketTurnObservation::Terminal(outcome)) = observation {
                    finalize_active_turn(bound, state, outcome);
                }
                let is_close = matches!(upstream_message, WreqWsMessage::Close(_));
                if is_close {
                    finalize_active_turn(
                        bound,
                        state,
                        ResponsesWebSocketTurnOutcome::upstream_closed(),
                    );
                }
                let drain_for_adapter = adapter_drain_ready(
                    bound.pending_adapter_drain,
                    bound.response_in_flight,
                    observation,
                    is_close,
                );
                let retry_current_turn = drain_for_adapter
                    && bound
                        .pending_adapter_drain
                        .is_some_and(|directive| directive.retry_current_turn);
                if retry_current_turn
                    && retry_active_turn_after_quota_exhaustion(bound, state, context).await
                {
                    continue;
                }
                if is_close && drain_for_adapter {
                    let directive = bound
                        .pending_adapter_drain
                        .expect("adapter drain state should be present");
                    send_gateway_error(
                        client_socket,
                        directive.error_code,
                        "Provider connection closed after reporting exhausted quota; send a new response.create to select another Provider connection",
                    )
                    .await;
                    bound.active_response_create = None;
                    detach_exhausted_upstream(bound, directive, &context.trace_id).await;
                    continue;
                }
                mark_active_response_started(bound, &upstream_message);
                if client_socket
                    .send(upstream_message_to_client(upstream_message))
                    .await
                    .is_err()
                {
                    finalize_active_turn(
                        bound,
                        state,
                        ResponsesWebSocketTurnOutcome::client_disconnected(),
                    );
                    bound.active_response_create = None;
                    close_bound_upstream(bound).await;
                    break;
                }
                if drain_for_adapter {
                    let directive = bound
                        .pending_adapter_drain
                        .expect("adapter drain state should be present");
                    bound.active_response_create = None;
                    detach_exhausted_upstream(bound, directive, &context.trace_id).await;
                    continue;
                }
                if matches!(observation, Some(ResponsesWebSocketTurnObservation::Terminal(_))) {
                    bound.active_response_create = None;
                }
                if is_close {
                    bound.upstream = None;
                    break;
                }
            }
        }
    }
}

fn finalize_active_turn(
    bound: &mut BoundResponsesConnection,
    state: &AppState,
    outcome: ResponsesWebSocketTurnOutcome,
) {
    if let Some(turn) = bound.active_turn.take() {
        spawn_responses_websocket_turn_finalization(state.clone(), turn, outcome);
    }
}

async fn close_bound_upstream(bound: &mut BoundResponsesConnection) {
    if let Some(mut upstream) = bound.upstream.take() {
        let _ = upstream.send(WreqWsMessage::Close(None)).await;
    }
}

async fn detach_exhausted_upstream(
    bound: &mut BoundResponsesConnection,
    directive: ResponsesWebSocketDrainDirective,
    trace_id: &str,
) {
    if let Some(key_id) = bound
        .decision_template
        .key_id
        .as_deref()
        .map(str::trim)
        .filter(|key_id| !key_id.is_empty())
    {
        bound.exhausted_key_ids.insert(key_id.to_string());
    }
    close_bound_upstream(bound).await;
    bound.response_in_flight = false;
    bound.pending_adapter_drain = None;
    debug!(
        event_name = "responses_websocket_upstream_detached",
        log_type = "event",
        transport = WEBSOCKET_LOG_TRANSPORT,
        websocket = true,
        trace_id = %trace_id,
        reason = directive.error_code,
        exhausted_key_count = bound.exhausted_key_ids.len(),
        "gateway detached an exhausted Responses WebSocket upstream while preserving the client socket"
    );
}

async fn retry_active_turn_after_quota_exhaustion(
    bound: &mut BoundResponsesConnection,
    state: &AppState,
    context: &ResponsesWebSocketRequestContext,
) -> bool {
    let Some((client_event, turn_index)) =
        bound.active_response_create.as_mut().and_then(|active| {
            active.can_retry_after_quota_exhaustion().then(|| {
                active.retry_attempted = true;
                (active.client_event.clone(), active.turn_index)
            })
        })
    else {
        return false;
    };

    let exhausted_key_id = bound
        .decision_template
        .key_id
        .as_deref()
        .map(str::trim)
        .filter(|key_id| !key_id.is_empty())
        .map(str::to_string);
    if let Some(key_id) = exhausted_key_id.as_ref() {
        bound.exhausted_key_ids.insert(key_id.clone());
    }

    let planning_parts = build_planning_parts(context);
    let turn_request_id = Uuid::new_v4().to_string();
    let planned = match maybe_build_responses_websocket_decision(
        state,
        &planning_parts,
        &turn_request_id,
        &context.decision,
        &client_event,
        Some(&bound.exhausted_key_ids),
    )
    .await
    {
        Ok(Some(decision)) => decision,
        Ok(None) => {
            warn!(
                event_name = "responses_websocket_quota_retry_provider_unavailable",
                log_type = "ops",
                transport = WEBSOCKET_LOG_TRANSPORT,
                websocket = true,
                trace_id = %context.trace_id,
                exhausted_key_id = ?exhausted_key_id,
                "gateway could not find an alternate Responses WebSocket provider after quota exhaustion"
            );
            return false;
        }
        Err(error) => {
            warn!(
                event_name = "responses_websocket_quota_retry_planning_failed",
                log_type = "ops",
                transport = WEBSOCKET_LOG_TRANSPORT,
                websocket = true,
                trace_id = %context.trace_id,
                exhausted_key_id = ?exhausted_key_id,
                error = ?error,
                "gateway could not plan an alternate Responses WebSocket provider after quota exhaustion"
            );
            return false;
        }
    };
    let adapter = resolve_responses_websocket_adapter(planned.adapter);
    let decision = planned.execution;
    if exhausted_key_id.as_deref() == decision.key_id.as_deref() {
        warn!(
            event_name = "responses_websocket_quota_retry_selected_exhausted_key",
            log_type = "ops",
            transport = WEBSOCKET_LOG_TRANSPORT,
            websocket = true,
            trace_id = %context.trace_id,
            key_id = ?decision.key_id,
            "gateway rejected an alternate Responses WebSocket plan that reused the exhausted key"
        );
        return false;
    }
    let provider_event = match planned_response_create_event(&decision, &client_event).and_then(
        |event| {
            serde_json::from_str::<Value>(&event)
                .map_err(|_| "response_create_serialization_failed")
        },
    ) {
        Ok(event) => event,
        Err(code) => {
            warn!(
                event_name = "responses_websocket_quota_retry_normalization_failed",
                log_type = "ops",
                transport = WEBSOCKET_LOG_TRANSPORT,
                websocket = true,
                trace_id = %context.trace_id,
                error_code = code,
                "gateway could not rebuild a Responses response.create for transparent quota retry"
            );
            return false;
        }
    };
    let turn_decision = prepare_responses_websocket_turn_decision(
        &decision,
        turn_request_id,
        true,
        &client_event,
        &provider_event,
        &context.trace_id,
        turn_index,
    );
    let mut turn =
        match begin_responses_websocket_turn(state, &planning_parts, turn_decision, &client_event)
            .await
        {
            Ok(turn) => turn,
            Err(error) => {
                warn!(
                    event_name = "responses_websocket_quota_retry_reporting_unavailable",
                    log_type = "ops",
                    transport = WEBSOCKET_LOG_TRANSPORT,
                    websocket = true,
                    trace_id = %context.trace_id,
                    error = ?error,
                    "gateway could not start usage and audit tracking for transparent quota retry"
                );
                return false;
            }
        };
    let mut replacement = match bind_responses_upstream(&decision, &client_event, adapter).await {
        Ok(connection) => connection,
        Err(code) => {
            spawn_responses_websocket_turn_finalization(
                state.clone(),
                turn,
                ResponsesWebSocketTurnOutcome::upstream_connect_failed(code),
            );
            warn!(
                event_name = "responses_websocket_quota_retry_rebind_failed",
                log_type = "ops",
                transport = WEBSOCKET_LOG_TRANSPORT,
                websocket = true,
                trace_id = %context.trace_id,
                error_code = code,
                "gateway could not bind an alternate Responses WebSocket provider after quota exhaustion"
            );
            return false;
        }
    };

    turn.mark_upstream_request_sent();
    turn.set_provider_response_headers(replacement.upstream_response_headers.clone());
    let replacement_upstream = replacement
        .upstream
        .take()
        .expect("newly bound Responses upstream should be present");
    if let Some(mut previous_upstream) = bound.upstream.replace(replacement_upstream) {
        let _ = previous_upstream.send(WreqWsMessage::Close(None)).await;
    }
    let previous_key_id = bound.decision_template.key_id.clone();
    bound.adapter = replacement.adapter;
    bound.client_model = replacement.client_model;
    bound.provider_model = replacement.provider_model;
    bound.response_in_flight = true;
    bound.decision_template = replacement.decision_template;
    bound.active_turn = Some(turn);
    bound.upstream_response_headers = replacement.upstream_response_headers;
    bound.pending_adapter_drain = None;
    debug!(
        event_name = "responses_websocket_quota_retry_rebound",
        log_type = "event",
        transport = WEBSOCKET_LOG_TRANSPORT,
        websocket = true,
        trace_id = %context.trace_id,
        turn_index,
        previous_key_id = ?previous_key_id,
        key_id = ?bound.decision_template.key_id,
        "gateway transparently rebound a Responses WebSocket turn after quota exhaustion"
    );
    true
}

fn response_create_has_previous_response_id(event: &Value) -> bool {
    event
        .get("previous_response_id")
        .is_some_and(|value| !value.is_null())
}

fn mark_active_response_started(bound: &mut BoundResponsesConnection, message: &WreqWsMessage) {
    let WreqWsMessage::Text(text) = message else {
        return;
    };
    let is_standard_response_event = serde_json::from_str::<Value>(text.as_str())
        .ok()
        .and_then(|event| {
            event
                .get("type")
                .and_then(Value::as_str)
                .map(|event_type| event_type.starts_with("response."))
        })
        .unwrap_or(false);
    if is_standard_response_event {
        if let Some(active) = bound.active_response_create.as_mut() {
            active.standard_response_started = true;
        }
    }
}

enum RelayDisposition {
    Continue,
    Close,
    UpstreamError(&'static str),
}

fn adapter_drain_ready(
    pending_adapter_drain: Option<ResponsesWebSocketDrainDirective>,
    response_in_flight: bool,
    observation: Option<ResponsesWebSocketTurnObservation>,
    upstream_closed: bool,
) -> bool {
    pending_adapter_drain.is_some()
        && (upstream_closed
            || !response_in_flight
            || matches!(
                observation,
                Some(ResponsesWebSocketTurnObservation::Terminal(_))
            ))
}

async fn forward_client_message(
    client_message: AxumWsMessage,
    bound: &mut BoundResponsesConnection,
    client_socket: &mut WebSocket,
    state: &AppState,
    context: &ResponsesWebSocketRequestContext,
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
                let Some(upstream) = bound.upstream.as_mut() else {
                    send_gateway_error(
                        client_socket,
                        "responses_websocket_upstream_rebind_required",
                        "Send a new response.create to select another Provider connection",
                    )
                    .await;
                    return RelayDisposition::Continue;
                };
                return upstream
                    .send(WreqWsMessage::text(text))
                    .await
                    .map(|_| RelayDisposition::Continue)
                    .unwrap_or(RelayDisposition::UpstreamError(
                        "responses_websocket_send_failed",
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
            if bound.upstream.is_none() {
                if response_create_has_previous_response_id(&client_event) {
                    send_gateway_error(
                        client_socket,
                        "responses_websocket_continuation_unavailable",
                        "The previous response belongs to an exhausted Provider account; send a new request with complete input",
                    )
                    .await;
                    return RelayDisposition::Continue;
                }
                let mut client_event = client_event;
                let requested_model = match response_create_model_or_current(
                    &mut client_event,
                    &bound.client_model,
                ) {
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
                event_name = "responses_websocket_response_create_forwarding",
                log_type = "event",
                transport = WEBSOCKET_LOG_TRANSPORT,
                websocket = true,
                trace_id = %context.trace_id,
                turn_index,
                client_model = %bound.client_model,
                provider_model = %bound.provider_model,
                model_replanned = false,
                has_previous_response_id = response_create_has_previous_response_id(&client_event),
                "gateway is forwarding a Responses response.create"
            );
            let turn_decision = prepare_responses_websocket_turn_decision(
                &bound.decision_template,
                Uuid::new_v4().to_string(),
                false,
                &client_event,
                &provider_event,
                &context.trace_id,
                turn_index,
            );
            let planning_parts = build_planning_parts(context);
            let mut turn = match begin_responses_websocket_turn(
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
                        event_name = "responses_websocket_followup_turn_lifecycle_start_failed",
                        log_type = "ops",
                        transport = WEBSOCKET_LOG_TRANSPORT,
                        websocket = true,
                        trace_id = %context.trace_id,
                        error = ?error,
                        "gateway could not start Responses WebSocket follow-up usage/audit lifecycle"
                    );
                    send_gateway_error(
                        client_socket,
                        "responses_websocket_reporting_unavailable",
                        "Gateway could not start usage and audit tracking for this response",
                    )
                    .await;
                    return RelayDisposition::Continue;
                }
            };
            turn.set_provider_response_headers(bound.upstream_response_headers.clone());
            bound.active_turn = Some(turn);
            bound.active_response_create = Some(ActiveResponsesWebSocketRequest::new(
                client_event.clone(),
                turn_index,
            ));
            bound.next_turn_index = bound.next_turn_index.saturating_add(1);
            bound.response_in_flight = true;

            let Some(upstream) = bound.upstream.as_mut() else {
                return RelayDisposition::UpstreamError("responses_websocket_send_failed");
            };
            match upstream.send(WreqWsMessage::text(outbound)).await {
                Ok(()) => {
                    if let Some(turn) = bound.active_turn.as_mut() {
                        turn.mark_upstream_request_sent();
                    }
                    RelayDisposition::Continue
                }
                Err(_) => RelayDisposition::UpstreamError("responses_websocket_send_failed"),
            }
        }
        AxumWsMessage::Binary(data) => match bound.upstream.as_mut() {
            Some(upstream) => upstream
                .send(WreqWsMessage::Binary(data))
                .await
                .map(|_| RelayDisposition::Continue)
                .unwrap_or(RelayDisposition::UpstreamError(
                    "responses_websocket_send_failed",
                )),
            None => {
                send_gateway_error(
                    client_socket,
                    "responses_websocket_upstream_rebind_required",
                    "Send a new response.create to select another Provider connection",
                )
                .await;
                RelayDisposition::Continue
            }
        },
        AxumWsMessage::Ping(data) => match bound.upstream.as_mut() {
            Some(upstream) => upstream
                .send(WreqWsMessage::Ping(data))
                .await
                .map(|_| RelayDisposition::Continue)
                .unwrap_or(RelayDisposition::UpstreamError(
                    "responses_websocket_send_failed",
                )),
            None => client_socket
                .send(AxumWsMessage::Pong(data))
                .await
                .map(|_| RelayDisposition::Continue)
                .unwrap_or(RelayDisposition::Close),
        },
        AxumWsMessage::Pong(data) => match bound.upstream.as_mut() {
            Some(upstream) => upstream
                .send(WreqWsMessage::Pong(data))
                .await
                .map(|_| RelayDisposition::Continue)
                .unwrap_or(RelayDisposition::UpstreamError(
                    "responses_websocket_send_failed",
                )),
            None => RelayDisposition::Continue,
        },
        AxumWsMessage::Close(frame) => {
            if let Some(upstream) = bound.upstream.as_mut() {
                let _ = upstream.send(client_close_to_upstream(frame)).await;
            }
            RelayDisposition::Close
        }
    }
}

async fn forward_replanned_response_create(
    bound: &mut BoundResponsesConnection,
    client_socket: &mut WebSocket,
    state: &AppState,
    context: &ResponsesWebSocketRequestContext,
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
                event_name = "responses_websocket_followup_model_access_check_failed",
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
    let excluded_key_ids =
        (!bound.exhausted_key_ids.is_empty()).then_some(&bound.exhausted_key_ids);
    let planned = match maybe_build_responses_websocket_decision(
        state,
        &planning_parts,
        &turn_request_id,
        &context.decision,
        &client_event,
        excluded_key_ids,
    )
    .await
    {
        Ok(Some(decision)) => decision,
        Ok(None) => {
            send_gateway_error(
                client_socket,
                "responses_provider_unavailable",
                "No eligible WebSocket-enabled Responses provider is available for the requested model",
            )
            .await;
            return RelayDisposition::Continue;
        }
        Err(error) => {
            warn!(
                event_name = "responses_websocket_followup_model_planning_failed",
                log_type = "ops",
                transport = WEBSOCKET_LOG_TRANSPORT,
                websocket = true,
                trace_id = %context.trace_id,
                requested_model = %requested_model,
                error = ?error,
                "gateway failed to re-plan Responses WebSocket follow-up model"
            );
            send_gateway_error(
                client_socket,
                "responses_provider_unavailable",
                "Gateway could not prepare the requested model",
            )
            .await;
            return RelayDisposition::Continue;
        }
    };
    let adapter = resolve_responses_websocket_adapter(planned.adapter);
    let decision = planned.execution;
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
                    "Gateway could not prepare the requested model",
                )
                .await;
                return RelayDisposition::Continue;
            }
        };
    let turn_index = bound.next_turn_index;
    let turn_decision = prepare_responses_websocket_turn_decision(
        &decision,
        turn_request_id,
        true,
        &client_event,
        &provider_event,
        &context.trace_id,
        turn_index,
    );
    let mut turn =
        match begin_responses_websocket_turn(state, &planning_parts, turn_decision, &client_event)
            .await
        {
            Ok(turn) => turn,
            Err(error) => {
                warn!(
                    event_name = "responses_websocket_replanned_turn_lifecycle_start_failed",
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
                    "responses_websocket_reporting_unavailable",
                    "Gateway could not start usage and audit tracking for this response",
                )
                .await;
                return RelayDisposition::Continue;
            }
        };

    if decision_reuses_bound_upstream(bound, adapter, &decision) {
        let outbound = match serde_json::to_string(&provider_event) {
            Ok(outbound) => outbound,
            Err(_) => {
                spawn_responses_websocket_turn_finalization(
                    state.clone(),
                    turn,
                    ResponsesWebSocketTurnOutcome::upstream_send_failed(),
                );
                send_gateway_error(
                    client_socket,
                    "response_create_serialization_failed",
                    "Gateway could not prepare the requested model",
                )
                .await;
                return RelayDisposition::Continue;
            }
        };
        let Some(upstream) = bound.upstream.as_mut() else {
            spawn_responses_websocket_turn_finalization(
                state.clone(),
                turn,
                ResponsesWebSocketTurnOutcome::upstream_send_failed(),
            );
            return RelayDisposition::UpstreamError("responses_websocket_send_failed");
        };
        if upstream.send(WreqWsMessage::text(outbound)).await.is_err() {
            spawn_responses_websocket_turn_finalization(
                state.clone(),
                turn,
                ResponsesWebSocketTurnOutcome::upstream_send_failed(),
            );
            return RelayDisposition::UpstreamError("responses_websocket_send_failed");
        }

        turn.mark_upstream_request_sent();
        turn.set_provider_response_headers(bound.upstream_response_headers.clone());
        let provider_model =
            provider_model_from_decision(&decision).unwrap_or_else(|| bound.provider_model.clone());
        let previous_client_model = std::mem::replace(&mut bound.client_model, requested_model);
        let previous_provider_model = std::mem::replace(&mut bound.provider_model, provider_model);
        bound.decision_template = decision;
        bound.active_turn = Some(turn);
        bound.active_response_create = Some(ActiveResponsesWebSocketRequest::new(
            client_event.clone(),
            turn_index,
        ));
        bound.next_turn_index = bound.next_turn_index.saturating_add(1);
        bound.response_in_flight = true;
        debug!(
            event_name = "responses_websocket_followup_model_replanned",
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
            "gateway re-planned a Responses WebSocket model on the existing upstream"
        );
        return RelayDisposition::Continue;
    }

    let mut replacement = match bind_responses_upstream(&decision, &client_event, adapter).await {
        Ok(connection) => connection,
        Err(code) => {
            spawn_responses_websocket_turn_finalization(
                state.clone(),
                turn,
                ResponsesWebSocketTurnOutcome::upstream_connect_failed(code),
            );
            warn!(
                event_name = "responses_websocket_followup_model_rebind_failed",
                log_type = "ops",
                transport = WEBSOCKET_LOG_TRANSPORT,
                websocket = true,
                trace_id = %context.trace_id,
                requested_model = %requested_model,
                error_code = code,
                "gateway failed to rebind Responses WebSocket follow-up model"
            );
            send_gateway_error(
                client_socket,
                code,
                "Gateway could not establish the requested model",
            )
            .await;
            return RelayDisposition::Continue;
        }
    };

    turn.mark_upstream_request_sent();
    turn.set_provider_response_headers(replacement.upstream_response_headers.clone());
    let previous_client_model = bound.client_model.clone();
    let previous_provider_model = bound.provider_model.clone();
    let replacement_upstream = replacement
        .upstream
        .take()
        .expect("newly bound Responses upstream should be present");
    if let Some(mut previous_upstream) = bound.upstream.replace(replacement_upstream) {
        let _ = previous_upstream.send(WreqWsMessage::Close(None)).await;
    }
    bound.adapter = replacement.adapter;
    bound.client_model = replacement.client_model;
    bound.provider_model = replacement.provider_model;
    bound.response_in_flight = true;
    bound.decision_template = replacement.decision_template;
    bound.active_turn = Some(turn);
    bound.active_response_create = Some(ActiveResponsesWebSocketRequest::new(
        client_event,
        turn_index,
    ));
    bound.next_turn_index = bound.next_turn_index.saturating_add(1);
    bound.upstream_response_headers = replacement.upstream_response_headers;
    bound.pending_adapter_drain = replacement.pending_adapter_drain;
    debug!(
        event_name = "responses_websocket_followup_model_rebound",
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
        "gateway rebound Responses WebSocket for a follow-up model"
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

fn response_create_model_or_current(
    event: &mut Value,
    current_client_model: &str,
) -> Result<String, &'static str> {
    let Some(object) = event.as_object_mut() else {
        return Err("invalid_response_create");
    };
    let Some(model) = object.get("model") else {
        object.insert(
            "model".to_string(),
            Value::String(current_client_model.to_string()),
        );
        return Ok(current_client_model.to_string());
    };
    let Some(model) = model
        .as_str()
        .map(str::trim)
        .filter(|model| !model.is_empty())
    else {
        return Err("invalid_response_create_model");
    };
    Ok(model.to_string())
}

fn decision_reuses_bound_upstream(
    bound: &BoundResponsesConnection,
    adapter: &'static dyn ResponsesWebSocketProtocolAdapter,
    decision: &AiExecutionDecision,
) -> bool {
    bound.upstream.is_some()
        && bound.adapter.kind() == adapter.kind()
        && decisions_reuse_upstream(&bound.decision_template, decision)
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

fn update_response_in_flight(bound: &mut BoundResponsesConnection, message: &WreqWsMessage) {
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

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;
    use std::sync::Arc;
    use std::time::{Duration, Instant};

    use super::super::adapter::{
        resolve_responses_websocket_adapter, ResponsesWebSocketDrainDirective,
    };
    use super::super::turn::{
        ResponsesWebSocketTurnDeadline, ResponsesWebSocketTurnObservation,
        ResponsesWebSocketTurnOutcome, ResponsesWebSocketTurnTimeoutPhase,
    };
    use super::{
        adapter_drain_ready, bind_responses_upstream, changed_followup_response_create_model,
        decisions_reuse_upstream, normalize_followup_response_create,
        planned_response_create_event, response_create_model_or_current,
        websocket_event_type_for_log, ActiveResponsesWebSocketRequest,
    };
    use crate::ai_serving::AiExecutionDecision;
    use crate::handlers::proxy::websocket::session::wait_for_optional_deadline;
    use crate::handlers::proxy::websocket::transport::{
        websocket_handshake_headers, websocket_timeouts, websocket_upstream_url,
    };
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
    fn adapter_drain_waits_for_an_active_turn_terminal_event() {
        let directive = Some(ResponsesWebSocketDrainDirective {
            error_code: "adapter_draining",
            retry_current_turn: false,
        });
        assert!(!adapter_drain_ready(directive, true, None, false));
        assert!(adapter_drain_ready(
            directive,
            true,
            Some(ResponsesWebSocketTurnObservation::Terminal(
                ResponsesWebSocketTurnOutcome::upstream_closed()
            )),
            false,
        ));
        assert!(!adapter_drain_ready(None, false, None, false));
        assert!(adapter_drain_ready(directive, false, None, false));
        assert!(adapter_drain_ready(directive, true, None, true));
    }

    #[test]
    fn maps_http_responses_url_to_websocket_url_without_losing_path_or_query() {
        let url = websocket_upstream_url(
            "https://example.test/v1/responses?x=1",
            "responses_upstream_url_invalid",
        )
        .expect("URL should convert");
        assert_eq!(url.as_str(), "wss://example.test/v1/responses?x=1");
    }

    #[test]
    fn rejects_embedded_upstream_credentials() {
        assert!(websocket_upstream_url(
            "https://token@example.test/responses",
            "responses_upstream_url_invalid",
        )
        .is_err());
    }

    #[test]
    fn strips_http_entity_headers_from_websocket_handshake() {
        let headers = websocket_handshake_headers(
            &BTreeMap::from([
                (
                    "authorization".to_string(),
                    "Bearer provider-token".to_string(),
                ),
                ("chatgpt-account-id".to_string(), "account-id".to_string()),
                ("content-type".to_string(), "application/json".to_string()),
            ]),
            "responses_websocket_headers_invalid",
        )
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
    fn detached_followup_inherits_the_current_public_model() {
        let mut event = json!({
            "type": "response.create",
            "input": "start over",
        });

        assert_eq!(
            response_create_model_or_current(&mut event, "gpt-5.6-sol"),
            Ok("gpt-5.6-sol".to_string())
        );
        assert_eq!(event["model"], "gpt-5.6-sol");
    }

    #[test]
    fn quota_retry_is_limited_to_an_unstarted_stateless_turn() {
        let mut request = ActiveResponsesWebSocketRequest::new(
            json!({"type": "response.create", "model": "gpt-5.6-sol"}),
            2,
        );
        assert!(request.can_retry_after_quota_exhaustion());

        request.standard_response_started = true;
        assert!(!request.can_retry_after_quota_exhaustion());

        request.standard_response_started = false;
        request.retry_attempted = true;
        assert!(!request.can_retry_after_quota_exhaustion());

        let continuation = ActiveResponsesWebSocketRequest::new(
            json!({
                "type": "response.create",
                "model": "gpt-5.6-sol",
                "previous_response_id": "resp_previous",
            }),
            2,
        );
        assert!(!continuation.can_retry_after_quota_exhaustion());
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
        let deadline = ResponsesWebSocketTurnDeadline {
            phase: ResponsesWebSocketTurnTimeoutPhase::AwaitingFirstEvent,
            deadline: Instant::now() - Duration::from_millis(1),
            timeout: Duration::from_secs(1),
        };

        tokio::time::timeout(
            Duration::from_millis(50),
            wait_for_optional_deadline(Some(deadline.deadline)),
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

        let mut bound = bind_responses_upstream(
            &decision,
            &json!({
                "type": "response.create",
                "model": "public-model",
                "input": "hello",
            }),
            resolve_responses_websocket_adapter(
                crate::orchestration::ResponsesWebSocketAdapter::Standard,
            ),
        )
        .await
        .expect("upstream binding should succeed");
        let observed = tokio::time::timeout(Duration::from_secs(2), observed)
            .await
            .expect("mock should observe first event")
            .expect("mock event channel should remain open");
        let response = tokio::time::timeout(
            Duration::from_secs(2),
            bound
                .upstream
                .as_mut()
                .expect("bound upstream should be present")
                .recv(),
        )
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
            .route("/v1/responses", get(mock_websocket))
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
            format!("http://{address}/v1/responses"),
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
            provider_type: Some("custom".to_string()),
            provider_id: None,
            endpoint_id: None,
            key_id: None,
            upstream_base_url: None,
            upstream_url: Some("https://example.test/v1/responses".to_string()),
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
