//! Standard OpenAI Responses WebSocket session engine.
//!
//! An incoming client socket is authenticated at Upgrade time. Its first
//! `response.create` selects a provider through the normal Responses planner.
//! Later turns reuse that upstream while the requested model remains eligible
//! on the selected key. A model change is planned again and keeps the current
//! upstream when the planner resolves to the same target; an independent
//! request may replace it, but a continuation must stay on the original
//! connection and account.

use axum::extract::ws::{Message as AxumWsMessage, WebSocket};
use futures_util::{SinkExt, StreamExt};
use serde_json::Value;
use uuid::Uuid;

use super::adapter::resolve_responses_provider_observer;
use super::backend::resolve_native_responses_websocket_backend;
use super::client::consume_response_create_rate_limit;
use super::connection::relay_bound_connection;
use super::lifecycle::{
    await_pending_provider_observation, await_pending_turn_finalization,
    await_turn_finalization_handle, finalize_unbound_turn, responses_websocket_turn_start_close,
    send_responses_websocket_turn_start_error, ResponsesSessionTermination,
    ResponsesSessionTerminationSignal,
};
use super::ownership::{
    await_owned_responses_websocket_plan, await_owned_responses_websocket_turn, disarm_owned_turn,
    spawn_owned_responses_websocket_plan, spawn_owned_responses_websocket_turn,
};
use super::redaction::redact_responses_websocket_client_event;
use super::relay_policy::{fatal_relay_policy, FatalRelaySignal};
use super::request::{
    build_planning_parts, planned_response_create_event, response_create_previous_response_id,
    responses_public_request_codec, ResponsesPublicRequestError,
};
use super::state::{ResponsesPublicEventSequence, ResponsesPublicTeardownClaim};
use super::turn::{
    prepare_responses_websocket_turn_decision, refresh_responses_websocket_turn_auth,
    ResponsesWebSocketTurnOutcome,
};
use super::turn_state::LogicalTurn;
use super::upstream::{
    bind_responses_upstream, canonicalize_responses_websocket_decision, ResponsesUpstreamBindError,
};

use crate::control::request_model_local_rejection;
use crate::handlers::proxy::websocket::ingress::{
    WebSocketConnectionLog, WebSocketConnectionLogSpec, WebSocketRequestContext,
};
use crate::handlers::proxy::websocket::session::{
    CLOSE_INTERNAL_ERROR, CLOSE_POLICY_VIOLATION, CLOSE_TRY_AGAIN,
    RESPONSES_WEBSOCKET_SESSION_LIMITS, WEBSOCKET_LOG_TRANSPORT,
};
use crate::handlers::proxy::websocket::transport::{
    close_client_socket, send_gateway_error, send_gateway_error_with_status,
    send_responses_websocket_error,
};
use crate::headers::effective_client_ip;
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

macro_rules! warn {
    ($($arg:tt)*) => {
        tracing::warn!(target: RESPONSES_WEBSOCKET_LOG_TARGET, $($arg)*)
    };
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
    InvalidPreviousResponseId,
    PreviousResponseNotFound,
    PublicRequest(ResponsesPublicRequestError),
}

#[derive(Debug, Clone, Copy)]
enum BootstrapTermination {
    ConnectionLimitReached,
    ConnectionAdmissionLost,
}

impl BootstrapTermination {
    const fn outcome(self) -> ResponsesWebSocketTurnOutcome {
        match self {
            Self::ConnectionLimitReached => {
                ResponsesWebSocketTurnOutcome::connection_limit_reached()
            }
            Self::ConnectionAdmissionLost => {
                ResponsesWebSocketTurnOutcome::connection_admission_lost()
            }
        }
    }

    const fn session_termination(self) -> ResponsesSessionTermination {
        match self {
            Self::ConnectionLimitReached => ResponsesSessionTermination::ConnectionLimitReached,
            Self::ConnectionAdmissionLost => ResponsesSessionTermination::ConnectionAdmissionLost,
        }
    }
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
            Self::InvalidPreviousResponseId => "invalid_previous_response_id",
            Self::PreviousResponseNotFound => "previous_response_not_found",
            Self::PublicRequest(error) => error.code(),
        }
    }

    const fn close_code(self) -> u16 {
        match self {
            Self::TimedOut => CLOSE_TRY_AGAIN,
            Self::ClientClosed => 1000,
            Self::ClientRead | Self::UnsupportedFrame | Self::InvalidJson => CLOSE_POLICY_VIOLATION,
            Self::MissingResponseCreate
            | Self::MissingModel
            | Self::InvalidPreviousResponseId
            | Self::PreviousResponseNotFound
            | Self::PublicRequest(_) => CLOSE_POLICY_VIOLATION,
        }
    }
}

pub(super) async fn run_responses_websocket(
    mut client_socket: WebSocket,
    state: AppState,
    mut context: WebSocketRequestContext,
) {
    // This is the lifetime of the upgraded public session, including initial
    // authentication, planning, admission, and the provider handshake. Passing
    // the same absolute instant into the relay prevents bootstrap latency from
    // extending the advertised one-hour connection limit.
    let connection_deadline =
        tokio::time::Instant::now() + RESPONSES_WEBSOCKET_SESSION_LIMITS.max_connection_duration;
    let connection_permit = context.websocket_connection_permit.take();
    let connection_log = WebSocketConnectionLog::new(&context, RESPONSES_CONNECTION_LOG_SPEC);
    connection_log.log_opened();

    let session_termination = ResponsesSessionTerminationSignal::default();
    let public_event_sequence = ResponsesPublicEventSequence::default();
    let public_teardown = ResponsesPublicTeardownClaim::default();
    let termination = {
        let session = run_responses_websocket_session(
            &mut client_socket,
            state,
            context,
            connection_permit.as_ref(),
            connection_deadline,
            session_termination.clone(),
            public_event_sequence.clone(),
            public_teardown.clone(),
        );
        tokio::pin!(session);
        tokio::select! {
            biased;
            termination = wait_for_bootstrap_termination(connection_deadline, connection_permit.as_ref()) => {
                session_termination.terminate(termination.session_termination());
                if public_teardown.try_claim() {
                    Some(termination)
                } else {
                    session.await;
                    None
                }
            }
            termination = &mut session => {
                termination.filter(|_| public_teardown.try_claim())
            },
        }
    };

    // Dropping the session future above first releases its sockets and invokes
    // any active attempt/planning guards. The connection permit is then
    // released before best-effort public teardown writes.
    drop(connection_permit);
    if let Some(termination) = termination {
        close_terminated_bootstrap(&mut client_socket, &public_event_sequence, termination).await;
    }
}

async fn run_responses_websocket_session(
    client_socket: &mut WebSocket,
    state: AppState,
    mut context: WebSocketRequestContext,
    connection_permit: Option<&aether_runtime::AdmissionPermit>,
    connection_deadline: tokio::time::Instant,
    session_termination: ResponsesSessionTerminationSignal,
    public_event_sequence: ResponsesPublicEventSequence,
    public_teardown: ResponsesPublicTeardownClaim,
) -> Option<BootstrapTermination> {
    let initial_message = receive_initial_response_create(client_socket, connection_deadline).await;
    let (first_text, first_event) = match initial_message {
        Ok(value) => value,
        Err(error) => {
            if !matches!(error, InitialMessageError::ClientClosed) {
                if !public_teardown.try_claim() {
                    return None;
                }
                match error {
                    InitialMessageError::InvalidPreviousResponseId => {
                        send_responses_websocket_error(
                            client_socket,
                            &public_event_sequence,
                            400,
                            "invalid_request_error",
                            error.code(),
                            "response.create.previous_response_id must be a non-empty string or null",
                            Some("previous_response_id"),
                        )
                        .await;
                    }
                    InitialMessageError::PreviousResponseNotFound => {
                        send_responses_websocket_error(
                            client_socket,
                            &public_event_sequence,
                            400,
                            "invalid_request_error",
                            error.code(),
                            "Previous response was not found on this WebSocket connection",
                            Some("previous_response_id"),
                        )
                        .await;
                    }
                    InitialMessageError::PublicRequest(request_error) => {
                        send_responses_websocket_error(
                            client_socket,
                            &public_event_sequence,
                            400,
                            "invalid_request_error",
                            request_error.code(),
                            request_error.message(),
                            request_error.param(),
                        )
                        .await;
                    }
                    _ => {
                        send_gateway_error(
                            client_socket,
                            &public_event_sequence,
                            error.code(),
                            "WebSocket must start with a valid response.create event",
                        )
                        .await;
                    }
                }
                close_client_socket(client_socket, error.close_code(), "invalid_initial_event")
                    .await;
            }
            return None;
        }
    };

    if let Some(termination) = bootstrap_termination(connection_deadline, connection_permit) {
        return Some(termination);
    }

    let first_turn_request_id =
        crate::execution_identity::ExecutionRequestId::generate().into_string();
    let planning_parts = build_planning_parts(&context, &first_turn_request_id);
    let client_ip = effective_client_ip(&context.headers, &context.remote_addr);
    let turn_authorization =
        match refresh_responses_websocket_turn_auth(&state, &context.decision, client_ip).await {
            Ok(authorization) => authorization,
            Err(error) => {
                if !public_teardown.try_claim() {
                    return None;
                }
                send_responses_websocket_turn_start_error(
                    client_socket,
                    &public_event_sequence,
                    &error,
                )
                .await;
                let (close_code, close_reason) = responses_websocket_turn_start_close(&error);
                close_client_socket(client_socket, close_code, close_reason).await;
                return None;
            }
        };
    let turn_control_decision = turn_authorization.decision;
    let turn_auth_snapshot = turn_authorization.auth_snapshot;
    match consume_response_create_rate_limit(
        &state,
        &turn_control_decision,
        client_ip,
        &context.trace_id,
    )
    .await
    {
        Ok(true) => {}
        Ok(false) => {
            if !public_teardown.try_claim() {
                return None;
            }
            send_gateway_error_with_status(
                client_socket,
                &public_event_sequence,
                429,
                "rate_limit_exceeded",
                "Too many response.create events; retry later",
            )
            .await;
            close_client_socket(client_socket, CLOSE_TRY_AGAIN, "rate_limit_exceeded").await;
            return None;
        }
        Err(()) => {
            if !public_teardown.try_claim() {
                return None;
            }
            warn!(
                event_name = "responses_websocket_rate_limit_check_failed",
                log_type = "ops",
                transport = WEBSOCKET_LOG_TRANSPORT,
                websocket = true,
                trace_id = %context.trace_id,
                "gateway failed to consume WebSocket response rate limit"
            );
            send_gateway_error_with_status(
                client_socket,
                &public_event_sequence,
                503,
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
            return None;
        }
    }
    match request_model_local_rejection(
        &state,
        Some(&turn_control_decision),
        &planning_parts.uri,
        &planning_parts.headers,
        &axum::body::Bytes::from(first_text.into_bytes()),
    )
    .await
    {
        Ok(Some(_)) => {
            if !public_teardown.try_claim() {
                return None;
            }
            send_gateway_error(
                client_socket,
                &public_event_sequence,
                "model_not_allowed",
                "The requested model is not available to this API key",
            )
            .await;
            close_client_socket(client_socket, CLOSE_POLICY_VIOLATION, "model_not_allowed").await;
            return None;
        }
        Ok(None) => {}
        Err(_) => {
            if !public_teardown.try_claim() {
                return None;
            }
            warn!(
                event_name = "responses_websocket_model_access_check_failed",
                log_type = "ops",
                transport = WEBSOCKET_LOG_TRANSPORT,
                websocket = true,
                trace_id = %context.trace_id,
                "gateway failed to evaluate WebSocket model access policy"
            );
            send_gateway_error(
                client_socket,
                &public_event_sequence,
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
            return None;
        }
    }

    // 请求侧脱敏必须在规划之前完成，而且这一轮只在这里做一次：planner 会把这份
    // body 写进 upstream 请求体和审计 original_request_body，绑定上游的首条
    // response.create 也从它派生。脱敏失败时直接断开，绝不退回原文发上游。
    let redacted_first_event = redact_responses_websocket_client_event(
        &state,
        &planning_parts,
        &turn_control_decision,
        &first_event,
    )
    .await;
    // 首轮的 mask session 要活到响应帧还原，但连接此刻还没绑定，只能先接住，
    // 等 `bind_responses_upstream` 之后登记到连接上。
    let (first_event, first_turn_redaction_session) = match redacted_first_event {
        Ok(Some(redaction)) => (redaction.client_event, Some(redaction.session)),
        Ok(None) => (first_event, None),
        Err(error) => {
            if !public_teardown.try_claim() {
                return None;
            }
            warn!(
                event_name = "responses_websocket_redaction_failed",
                log_type = "ops",
                transport = WEBSOCKET_LOG_TRANSPORT,
                websocket = true,
                trace_id = %context.trace_id,
                error = ?error,
                "gateway could not apply chat PII redaction to the initial Responses WebSocket event"
            );
            send_gateway_error_with_status(
                client_socket,
                &public_event_sequence,
                500,
                "responses_websocket_redaction_unavailable",
                "Gateway could not apply the configured PII redaction",
            )
            .await;
            close_client_socket(
                client_socket,
                CLOSE_INTERNAL_ERROR,
                "responses_websocket_redaction_unavailable",
            )
            .await;
            return None;
        }
    };

    if let Some(termination) = bootstrap_termination(connection_deadline, connection_permit) {
        return Some(termination);
    }

    let planning = spawn_owned_responses_websocket_plan(
        state.clone(),
        planning_parts,
        context.trace_id.clone(),
        turn_control_decision.clone(),
        first_event.clone(),
        None,
        None,
        turn_auth_snapshot.clone(),
    );
    let owned_plan = match await_owned_responses_websocket_plan(planning).await {
        Ok(Some(decision)) => decision,
        Ok(None) => {
            if !public_teardown.try_claim() {
                return None;
            }
            send_gateway_error_with_status(
                client_socket,
                &public_event_sequence,
                503,
                "responses_provider_unavailable",
                "No eligible WebSocket-enabled Responses provider is available",
            )
            .await;
            close_client_socket(
                client_socket,
                CLOSE_TRY_AGAIN,
                "responses_provider_unavailable",
            )
            .await;
            return None;
        }
        Err(_) => {
            if !public_teardown.try_claim() {
                return None;
            }
            warn!(
                event_name = "responses_websocket_planning_failed",
                log_type = "ops",
                transport = WEBSOCKET_LOG_TRANSPORT,
                websocket = true,
                trace_id = %context.trace_id,
                "gateway failed to plan Responses WebSocket provider request"
            );
            send_gateway_error_with_status(
                client_socket,
                &public_event_sequence,
                503,
                "responses_provider_unavailable",
                "Gateway could not prepare a Provider connection",
            )
            .await;
            close_client_socket(
                client_socket,
                CLOSE_INTERNAL_ERROR,
                "responses_planning_failed",
            )
            .await;
            return None;
        }
    };

    let planned = owned_plan.planned;
    let planned_lease = owned_plan.lease;
    let planning_parts = owned_plan.planning_parts;
    let backend = resolve_native_responses_websocket_backend(planned.backend);
    let provider_observer = resolve_responses_provider_observer(planned.provider_observer);
    let bound_candidate = planned.bound_candidate;
    let credential_binding_fingerprint = planned.credential_binding_fingerprint;
    let normalization = planned.normalization;
    let decision = canonicalize_responses_websocket_decision(planned.execution);
    if let Some(termination) = bootstrap_termination(connection_deadline, connection_permit) {
        planned_lease.release().await;
        return Some(termination);
    }
    let first_provider_event = match planned_response_create_event(&decision, &first_event)
        .and_then(|event| {
            serde_json::from_str::<Value>(&event).map_err(|_| "responses_websocket_request_invalid")
        }) {
        Ok(event) => event,
        Err(code) => {
            planned_lease.release().await;
            if !public_teardown.try_claim() {
                return None;
            }
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
                client_socket,
                &public_event_sequence,
                code,
                "Gateway could not prepare the Responses response.create event",
            )
            .await;
            close_client_socket(client_socket, CLOSE_POLICY_VIOLATION, code).await;
            return None;
        }
    };
    let first_logical_turn_id = Uuid::new_v4().to_string();
    let first_turn_decision = prepare_responses_websocket_turn_decision(
        &decision,
        first_turn_request_id.clone(),
        None,
        true,
        &first_event,
        &first_provider_event,
        &context.trace_id,
        1,
        &first_logical_turn_id,
        1,
    );
    let first_turn_start = spawn_owned_responses_websocket_turn(
        state.clone(),
        planning_parts,
        turn_control_decision.clone(),
        first_turn_decision,
        first_event.clone(),
        planned_lease,
        session_termination.clone(),
    );
    let mut first_turn = match await_owned_responses_websocket_turn(first_turn_start).await {
        Ok(turn) => turn,
        Err(error) => {
            if !public_teardown.try_claim() {
                return None;
            }
            warn!(
                event_name = "responses_websocket_turn_lifecycle_start_failed",
                log_type = "ops",
                transport = WEBSOCKET_LOG_TRANSPORT,
                websocket = true,
                trace_id = %context.trace_id,
                error = ?error,
                "gateway could not start Responses WebSocket usage/audit lifecycle"
            );
            send_responses_websocket_turn_start_error(
                client_socket,
                &public_event_sequence,
                &error,
            )
            .await;
            let (close_code, close_reason) = responses_websocket_turn_start_close(&error);
            close_client_socket(client_socket, close_code, close_reason).await;
            return None;
        }
    };

    if let Some(termination) = bootstrap_termination(connection_deadline, connection_permit) {
        let finalizer = finalize_unbound_turn(
            state.clone(),
            disarm_owned_turn(first_turn),
            termination.outcome(),
        );
        await_turn_finalization_handle(finalizer).await;
        return Some(termination);
    }

    let mut bound = match bind_responses_upstream(
        &decision,
        bound_candidate,
        credential_binding_fingerprint,
        normalization,
        &first_event,
        backend,
        provider_observer,
        || first_turn.admission_is_healthy(),
    )
    .await
    {
        Ok(connection) => connection,
        Err(ResponsesUpstreamBindError::ExecutionReservationLost) => {
            let policy = fatal_relay_policy(FatalRelaySignal::ExecutionReservationLost);
            let finalizer = finalize_unbound_turn(
                state.clone(),
                disarm_owned_turn(first_turn),
                ResponsesWebSocketTurnOutcome::execution_reservation_lost(),
            );
            if public_teardown.try_claim() {
                send_responses_websocket_error(
                    client_socket,
                    &public_event_sequence,
                    policy.status_code,
                    "server_error",
                    policy.error_code,
                    policy.client_message,
                    None,
                )
                .await;
                close_client_socket(client_socket, policy.close_code, policy.close_reason).await;
            }
            await_turn_finalization_handle(finalizer).await;
            return None;
        }
        Err(ResponsesUpstreamBindError::Upstream(code)) => {
            let finalizer = finalize_unbound_turn(
                state.clone(),
                disarm_owned_turn(first_turn),
                ResponsesWebSocketTurnOutcome::upstream_connect_failed(code),
            );
            if !public_teardown.try_claim() {
                await_turn_finalization_handle(finalizer).await;
                return None;
            }
            warn!(
                event_name = "responses_websocket_upstream_connect_failed",
                log_type = "ops",
                transport = WEBSOCKET_LOG_TRANSPORT,
                websocket = true,
                trace_id = %context.trace_id,
                error_code = code,
                "gateway failed to establish Responses WebSocket upstream"
            );
            send_gateway_error_with_status(
                client_socket,
                &public_event_sequence,
                502,
                code,
                "Gateway could not establish the Provider connection",
            )
            .await;
            close_client_socket(client_socket, CLOSE_TRY_AGAIN, code).await;
            await_turn_finalization_handle(finalizer).await;
            return None;
        }
    };
    bound.session_termination = session_termination.clone();
    bound.public_event_sequence = public_event_sequence.clone();
    bound.public_teardown = public_teardown;
    first_turn.mark_upstream_request_sent();
    first_turn.set_provider_response_headers(bound.upstream_response_headers.clone());
    if let Some(termination) = bootstrap_termination(connection_deadline, connection_permit) {
        bound.backend_session.close().await;
        let finalizer = finalize_unbound_turn(
            state.clone(),
            disarm_owned_turn(first_turn),
            termination.outcome(),
        );
        await_turn_finalization_handle(finalizer).await;
        return Some(termination);
    }
    if let Some(session) = first_turn_redaction_session {
        bound.redaction_restorer.register(session);
    }
    bound.turn_state.begin(
        LogicalTurn::authorized(
            first_turn_request_id,
            first_event,
            turn_control_decision,
            turn_auth_snapshot,
            1,
            first_logical_turn_id,
        ),
        first_turn,
    );

    relay_bound_connection(
        client_socket,
        &mut bound,
        &state,
        &context,
        connection_deadline,
    )
    .await;
    await_pending_turn_finalization(&mut bound).await;
    await_pending_provider_observation(&mut bound).await;
    None
}

/// 等待客户端发送第一条 response.create 事件。
/// 使用绝对 deadline：从函数入口起计算一次截止时间，Ping/Pong 只会被正常回复，
/// 但不会重置计时器。防止客户端通过周期性 Ping 无限占用 connection permit。
async fn receive_initial_response_create(
    client_socket: &mut WebSocket,
    connection_deadline: tokio::time::Instant,
) -> Result<(String, Value), InitialMessageError> {
    let initial_message_deadline = std::cmp::min(
        connection_deadline,
        tokio::time::Instant::now() + RESPONSES_WEBSOCKET_SESSION_LIMITS.initial_message_timeout,
    );
    receive_initial_response_create_until(client_socket, initial_message_deadline).await
}

/// 核心循环：在绝对 deadline 内等待客户端发送 response.create。
/// 泛型约束允许测试注入 fake socket，驱动真实逻辑。
///
/// - `deadline_budget`：从调用时刻起的最长等待时间，全循环共享同一截止时刻。
/// - Ping 帧被回复 Pong 但不重置计时器。
/// - Pong / 非法帧 / Close 按协议处理。
async fn receive_initial_response_create_with_deadline<S>(
    socket: &mut S,
    deadline_budget: std::time::Duration,
) -> Result<(String, Value), InitialMessageError>
where
    S: futures_util::Stream<Item = Result<AxumWsMessage, axum::Error>>
        + futures_util::Sink<AxumWsMessage, Error = axum::Error>
        + Unpin,
{
    let deadline = tokio::time::Instant::now() + deadline_budget;
    receive_initial_response_create_until(socket, deadline).await
}

async fn receive_initial_response_create_until<S>(
    socket: &mut S,
    deadline: tokio::time::Instant,
) -> Result<(String, Value), InitialMessageError>
where
    S: futures_util::Stream<Item = Result<AxumWsMessage, axum::Error>>
        + futures_util::Sink<AxumWsMessage, Error = axum::Error>
        + Unpin,
{
    use futures_util::{SinkExt as _, StreamExt as _};

    // 绝对 deadline：入口计算一次，后续所有迭代共享，Ping/Pong 不会重启
    loop {
        let message = tokio::time::timeout_at(deadline, socket.next())
            .await
            .map_err(|_| InitialMessageError::TimedOut)?;
        let Some(message) = message else {
            return Err(InitialMessageError::ClientClosed);
        };
        let message = message.map_err(|_| InitialMessageError::ClientRead)?;
        match message {
            AxumWsMessage::Ping(payload) => {
                tokio::time::timeout_at(deadline, socket.send(AxumWsMessage::Pong(payload)))
                    .await
                    .map_err(|_| InitialMessageError::TimedOut)?
                    .map_err(|_| InitialMessageError::ClientRead)?;
            }
            AxumWsMessage::Pong(_) => {}
            AxumWsMessage::Close(_) => return Err(InitialMessageError::ClientClosed),
            AxumWsMessage::Binary(_) => return Err(InitialMessageError::UnsupportedFrame),
            AxumWsMessage::Text(text) => {
                return canonicalize_initial_response_create(text.as_str());
            }
        }
    }
}

fn canonicalize_initial_response_create(
    text: &str,
) -> Result<(String, Value), InitialMessageError> {
    let event: Value = serde_json::from_str(text).map_err(|_| InitialMessageError::InvalidJson)?;
    let event = responses_public_request_codec()
        .response_create(&event)
        .map_err(initial_public_request_error)?;
    validate_initial_response_create(&event)?;
    let text = serde_json::to_string(&event).map_err(|_| InitialMessageError::InvalidJson)?;
    Ok((text, event))
}

fn initial_public_request_error(error: ResponsesPublicRequestError) -> InitialMessageError {
    match error {
        ResponsesPublicRequestError::InvalidEventShape => InitialMessageError::InvalidJson,
        ResponsesPublicRequestError::UnsupportedEventType => {
            InitialMessageError::MissingResponseCreate
        }
        error => InitialMessageError::PublicRequest(error),
    }
}

fn bootstrap_termination(
    connection_deadline: tokio::time::Instant,
    connection_permit: Option<&aether_runtime::AdmissionPermit>,
) -> Option<BootstrapTermination> {
    if tokio::time::Instant::now() >= connection_deadline {
        return Some(BootstrapTermination::ConnectionLimitReached);
    }
    if connection_permit.is_some_and(|permit| !permit.is_healthy()) {
        return Some(BootstrapTermination::ConnectionAdmissionLost);
    }
    None
}

async fn wait_for_bootstrap_termination(
    connection_deadline: tokio::time::Instant,
    connection_permit: Option<&aether_runtime::AdmissionPermit>,
) -> BootstrapTermination {
    let mut health = tokio::time::interval(std::time::Duration::from_secs(1));
    health.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
    loop {
        tokio::select! {
            _ = tokio::time::sleep_until(connection_deadline) => {
                return BootstrapTermination::ConnectionLimitReached;
            }
            _ = health.tick(), if connection_permit.is_some() => {
                if connection_permit.is_some_and(|permit| !permit.is_healthy()) {
                    return BootstrapTermination::ConnectionAdmissionLost;
                }
            }
        }
    }
}

async fn close_terminated_bootstrap(
    client_socket: &mut WebSocket,
    sequence: &ResponsesPublicEventSequence,
    termination: BootstrapTermination,
) {
    let (error_code, message, close_reason) = match termination {
        BootstrapTermination::ConnectionLimitReached => (
            "websocket_connection_limit_reached",
            "Responses websocket connection limit reached (60 minutes). Create a new websocket connection to continue.",
            "connection_limit_reached",
        ),
        BootstrapTermination::ConnectionAdmissionLost => (
            "websocket_connection_admission_lost",
            "WebSocket connection capacity is no longer available; reconnect to continue",
            "connection_admission_lost",
        ),
    };
    match termination {
        BootstrapTermination::ConnectionLimitReached => {
            send_responses_websocket_error(
                client_socket,
                sequence,
                400,
                "invalid_request_error",
                error_code,
                message,
                None,
            )
            .await;
        }
        BootstrapTermination::ConnectionAdmissionLost => {
            send_gateway_error_with_status(client_socket, sequence, 503, error_code, message).await;
        }
    }
    close_client_socket(client_socket, CLOSE_TRY_AGAIN, close_reason).await;
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
    match response_create_previous_response_id(event) {
        Ok(Some(_)) => return Err(InitialMessageError::PreviousResponseNotFound),
        Ok(None) => {}
        Err(_) => return Err(InitialMessageError::InvalidPreviousResponseId),
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;
    use std::sync::Arc;
    use std::time::{Duration, Instant};

    use super::super::adapter::{
        resolve_responses_provider_observer, responses_public_wire_codec,
        ResponsesWebSocketDrainDirective,
    };
    use super::super::backend::resolve_native_responses_websocket_backend;
    use super::super::binding::UpstreamBindingIdentity;
    use super::super::client::provider_drain_ready;
    use super::super::quota::{
        active_continuation_can_retry_from_full_input, is_usage_limit_error_event,
        observe_active_response_rebind_safety, record_exhausted_bound_key,
        send_previous_response_not_found, should_request_full_continuation_retry,
    };
    use super::super::redaction::ResponsesWebSocketRedactionRestorer;
    use super::super::request::{
        changed_followup_response_create_model, continuation_requires_same_upstream,
        normalize_followup_response_create, planned_response_create_event,
        response_create_model_or_current,
    };
    use super::super::state::{
        BoundResponsesConnection, ExhaustedResponsesWebSocketExclusions,
        ResponsesPublicEventSequence,
    };
    use super::super::turn::{
        ResponsesWebSocketTurnDeadline, ResponsesWebSocketTurnObservation,
        ResponsesWebSocketTurnOutcome, ResponsesWebSocketTurnTimeoutPhase,
    };
    use super::super::turn_state::{LogicalTurn, ResponsesTurnState};
    use super::super::upstream::bind_responses_upstream;
    use crate::ai_serving::{AiExecutionDecision, ResponsesWebSocketBodyNormalization};
    use crate::handlers::proxy::websocket::session::{wait_for_optional_deadline, CLOSE_TRY_AGAIN};
    use crate::handlers::proxy::websocket::transport::{
        websocket_handshake_headers, websocket_timeouts, websocket_upstream_url,
    };
    use aether_scheduler_core::SchedulerMinimalCandidateSelectionCandidate;
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
        beta_header_present: bool,
        event: serde_json::Value,
    }

    #[test]
    fn provider_drain_waits_for_an_active_turn_terminal_event() {
        let directive = Some(ResponsesWebSocketDrainDirective {
            error_code: "adapter_draining",
            retry_current_turn: false,
            retry_exclusion_until_unix_secs: None,
        });
        assert!(!provider_drain_ready(directive, true, None, false));
        assert!(provider_drain_ready(
            directive,
            true,
            Some(ResponsesWebSocketTurnObservation::Terminal(
                ResponsesWebSocketTurnOutcome::upstream_closed()
            )),
            false,
        ));
        assert!(!provider_drain_ready(None, false, None, false));
        assert!(provider_drain_ready(directive, false, None, false));
        assert!(provider_drain_ready(directive, true, None, true));
    }

    #[test]
    fn exhausted_key_and_account_exclusions_expire_at_the_reported_reset_or_fallback() {
        let mut exclusions = ExhaustedResponsesWebSocketExclusions::default();

        assert_eq!(
            exclusions.exclude(
                "key-1".to_string(),
                Some("account-1".to_string()),
                Some(1_050),
                1_000,
            ),
            1_050
        );
        assert!(exclusions.key_ids(1_049).contains("key-1"));
        assert!(exclusions.codex_account_ids(1_049).contains("account-1"));
        assert!(!exclusions.key_ids(1_050).contains("key-1"));
        assert!(!exclusions.codex_account_ids(1_050).contains("account-1"));

        assert_eq!(
            exclusions.exclude("key-2".to_string(), None, None, 2_000),
            2_300
        );
        assert!(exclusions.key_ids(2_299).contains("key-2"));
        assert!(!exclusions.key_ids(2_300).contains("key-2"));

        assert_eq!(
            exclusions.exclude("key-3".to_string(), None, Some(3_100), 3_000),
            3_100
        );
        assert_eq!(
            exclusions.exclude("key-3".to_string(), None, Some(3_050), 3_001),
            3_100
        );
    }

    #[test]
    fn exhausted_codex_binding_excludes_the_account_before_retry_planning() {
        let mut bound = sample_bound_for_rebind_safety();
        bound.decision_template.provider_type = Some("codex".to_string());
        bound.decision_template.key_id = Some("key-codex".to_string());
        bound.decision_template.provider_request_headers.insert(
            "ChatGPT-Account-ID".to_string(),
            "account-codex".to_string(),
        );

        // The exclusion deadline is evaluated against the wall clock, so a
        // provider reset time only survives if it is still in the future.
        let reset_at = crate::clock::current_unix_secs() + 600;

        assert_eq!(
            record_exhausted_bound_key(&mut bound, Some(reset_at)),
            Some(("key-codex".to_string(), reset_at))
        );
        assert!(bound
            .exhausted_exclusions
            .codex_account_ids(reset_at - 1)
            .contains("account-codex"));
        assert!(!bound
            .exhausted_exclusions
            .codex_account_ids(reset_at)
            .contains("account-codex"));
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
            &json!({
                "type": "response.create",
                "model": "public-model",
                "previous_response_id": "resp-previous",
                "generate": false,
            }),
        )
        .expect("event should serialize");
        let event: serde_json::Value = serde_json::from_str(&event).expect("event JSON");
        assert_eq!(event["type"], "response.create");
        assert_eq!(event["model"], "provider-model");
        assert_eq!(event["previous_response_id"], "resp-previous");
        assert_eq!(event["generate"], false);
        assert!(event.get("stream").is_none());
        assert!(event.get("background").is_none());
    }

    #[test]
    fn continuation_requires_the_existing_upstream_connection_and_account() {
        let continuation = json!({
            "type": "response.create",
            "previous_response_id": "resp-previous",
        });

        assert!(!continuation_requires_same_upstream(&continuation, true));
        assert!(continuation_requires_same_upstream(&continuation, false));
        assert!(!continuation_requires_same_upstream(
            &json!({"type": "response.create"}),
            false,
        ));
    }

    #[test]
    fn initial_response_create_cannot_continue_a_response_from_another_connection() {
        assert!(matches!(
            super::validate_initial_response_create(&json!({
                "type": "response.create",
                "model": "public-model",
                "previous_response_id": "resp_from_another_connection"
            })),
            Err(super::InitialMessageError::PreviousResponseNotFound)
        ));
    }

    #[test]
    fn initial_response_create_is_canonical_before_model_access_and_planning() {
        let (canonical_text, canonical_event) = super::canonicalize_initial_response_create(
            r#"{
                "type":"response.create",
                "model":"public-model",
                "input":"hello",
                "generate":false,
                "future_responses_option":{"enabled":true}
            }"#,
        )
        .expect("ordinary unknown fields should not break schema-forward compatibility");
        let canonical_text: serde_json::Value =
            serde_json::from_str(&canonical_text).expect("canonical model-access body is JSON");

        for canonical in [&canonical_event, &canonical_text] {
            assert_eq!(canonical["type"], "response.create");
            assert_eq!(canonical["model"], "public-model");
            assert_eq!(canonical["input"], "hello");
            assert_eq!(canonical["generate"], false);
            assert_eq!(
                canonical["future_responses_option"],
                json!({"enabled": true})
            );
        }
    }

    #[test]
    fn initial_public_codec_rejects_private_fields_before_model_validation() {
        assert!(matches!(
            super::canonicalize_initial_response_create(
                r#"{"type":"response.create","client_metadata":{"thread_id":"private"}}"#
            ),
            Err(super::InitialMessageError::PublicRequest(
                super::ResponsesPublicRequestError::ProviderPrivateField {
                    field: "client_metadata"
                }
            ))
        ));
    }

    #[test]
    fn initial_public_codec_rejects_multi_agent_before_model_validation() {
        let error = super::canonicalize_initial_response_create(
            r#"{"type":"response.create","multi_agent":{"enabled":true}}"#,
        )
        .expect_err("stable WebSocket mode must reject multi-agent beta");
        assert!(matches!(
            error,
            super::InitialMessageError::PublicRequest(
                super::ResponsesPublicRequestError::UnsupportedWebSocketField {
                    field: "multi_agent"
                }
            )
        ));

        let error = super::canonicalize_initial_response_create(
            r#"{"type":"response.create","input":[{"type":"message","caller":{"type":"multi_agent"}}]}"#,
        )
        .expect_err("stable WebSocket mode must reject multi-agent callers");
        assert!(matches!(
            error,
            super::InitialMessageError::PublicRequest(
                super::ResponsesPublicRequestError::InvalidField { field: "input" }
            )
        ));
    }

    #[test]
    fn quota_error_can_request_a_full_retry_only_before_public_response_state() {
        let mut bound = sample_bound_for_rebind_safety();
        bound.turn_state = ResponsesTurnState::Replanning {
            logical: LogicalTurn::new(
                json!({
                    "type": "response.create",
                    "previous_response_id": "resp-previous",
                }),
                2,
                "logical-turn".to_string(),
            ),
        };

        assert!(active_continuation_can_retry_from_full_input(&bound));
        bound
            .turn_state
            .logical_mut()
            .expect("active request")
            .mark_retry_unsafe("standard_response_event");
        assert!(!active_continuation_can_retry_from_full_input(&bound));
    }

    #[test]
    fn only_an_actual_usage_limit_error_requests_full_retry() {
        assert!(is_usage_limit_error_event(&json!({
            "type": "error",
            "error": {"type": "usage_limit_reached"},
            "status_code": 429,
        })));
        assert!(!is_usage_limit_error_event(&json!({
            "type": "codex.rate_limits",
            "rate_limits": {"limit_reached": true},
        })));
        assert!(!is_usage_limit_error_event(&json!({
            "type": "response.completed",
            "response": {"id": "resp-completed"},
        })));
    }

    #[test]
    fn full_continuation_retry_does_not_consume_a_successful_terminal_event() {
        let mut bound = sample_bound_for_rebind_safety();
        bound.turn_state = ResponsesTurnState::Replanning {
            logical: LogicalTurn::new(
                json!({
                    "type": "response.create",
                    "previous_response_id": "resp-previous",
                }),
                2,
                "logical-turn".to_string(),
            ),
        };

        assert!(should_request_full_continuation_retry(
            &bound,
            true,
            Some(&json!({
                "type": "error",
                "error": {"type": "usage_limit_reached"},
            })),
        ));
        assert!(!should_request_full_continuation_retry(
            &bound,
            true,
            Some(&json!({
                "type": "response.completed",
                "response": {"id": "resp-completed"},
            })),
        ));
        assert!(!should_request_full_continuation_retry(
            &bound,
            false,
            Some(&json!({
                "type": "error",
                "error": {"type": "usage_limit_reached"},
            })),
        ));
    }

    #[test]
    fn followup_rewrites_the_provider_model_and_removes_http_stream_fields() {
        let event = json!({
            "type": "response.create",
            "model": "public-model",
            "stream": true,
            "background": true,
        });
        let normalized = normalize_followup_response_create(
            &event,
            "provider-model",
            &ResponsesWebSocketBodyNormalization::for_tests("provider-model"),
        )
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
    fn quota_retry_requires_an_explicitly_replay_safe_turn() {
        let mut request = LogicalTurn::new(
            json!({"type": "response.create", "model": "gpt-5.6-sol"}),
            2,
            "logical-turn".to_string(),
        );
        assert_eq!(request.quota_retry_block_reason(), None);

        request.mark_retry_unsafe("standard_response_event");
        assert_eq!(
            request.quota_retry_block_reason(),
            Some("standard_response_event")
        );

        let mut retried = LogicalTurn::new(
            json!({"type": "response.create", "model": "gpt-5.6-sol"}),
            2,
            "logical-turn".to_string(),
        );
        retried.retry_attempted = true;
        assert_eq!(
            retried.quota_retry_block_reason(),
            Some("quota_retry_already_attempted")
        );

        let mut client_control = LogicalTurn::new(
            json!({"type": "response.create", "model": "gpt-5.6-sol"}),
            2,
            "logical-turn".to_string(),
        );
        client_control.mark_retry_unsafe("client_control_event");
        assert_eq!(
            client_control.quota_retry_block_reason(),
            Some("client_control_event")
        );

        let continuation = LogicalTurn::new(
            json!({
                "type": "response.create",
                "model": "gpt-5.6-sol",
                "previous_response_id": "resp_previous",
            }),
            2,
            "logical-turn".to_string(),
        );
        assert_eq!(
            continuation.quota_retry_block_reason(),
            Some("previous_response_id")
        );
    }

    #[test]
    fn provider_observer_safety_contract_controls_transparent_rebind_eligibility() {
        let mut bound = sample_bound_for_rebind_safety();
        observe_active_response_rebind_safety(
            &mut bound,
            &json!({
                "type": "codex.rate_limits",
                "rate_limits": {"allowed": true}
            }),
        );
        assert_eq!(
            bound
                .turn_state
                .logical()
                .and_then(LogicalTurn::quota_retry_block_reason),
            None
        );

        observe_active_response_rebind_safety(&mut bound, &json!({"type": "response.created"}));
        assert_eq!(
            bound
                .turn_state
                .logical()
                .and_then(LogicalTurn::quota_retry_block_reason),
            Some("standard_response_event")
        );

        let mut unknown = sample_bound_for_rebind_safety();
        observe_active_response_rebind_safety(&mut unknown, &json!({"type": "codex.unknown"}));
        assert_eq!(
            unknown
                .turn_state
                .logical()
                .and_then(LogicalTurn::quota_retry_block_reason),
            Some("unrecognized_upstream_event")
        );
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
            (
                "OpEnAI-BeTa".to_string(),
                "responses_multi_agent=v1".to_string(),
            ),
        ]);
        decision.provider_request_body = Some(json!({
            "model": "provider-model",
            "input": "hello",
            "stream": true,
            "background": true,
        }));

        let mut bound = bind_responses_upstream(
            &decision,
            sample_bound_candidate("provider-model"),
            "credential-1".to_string(),
            ResponsesWebSocketBodyNormalization::for_tests("provider-model"),
            &json!({
                "type": "response.create",
                "model": "public-model",
                "input": "hello",
            }),
            resolve_native_responses_websocket_backend(
                crate::orchestration::ResponsesWebSocketBackendKind::NativeResponsesWebSocket,
            ),
            resolve_responses_provider_observer(
                crate::orchestration::ResponsesProviderObserverKind::Standard,
            ),
            || true,
        )
        .await
        .expect("upstream binding should succeed");
        let observed = tokio::time::timeout(Duration::from_secs(2), observed)
            .await
            .expect("mock should observe first event")
            .expect("mock event channel should remain open");
        let response =
            tokio::time::timeout(Duration::from_secs(2), bound.backend_session.receive())
                .await
                .expect("mock should send a response event");
        server.abort();

        assert!(observed.authorization_present);
        assert!(observed.account_header_present);
        assert!(!observed.beta_header_present);
        assert!(bound
            .decision_template
            .provider_request_headers
            .keys()
            .all(|name| !name.eq_ignore_ascii_case("openai-beta")));
        assert_eq!(observed.event["type"], "response.create");
        assert_eq!(observed.event["model"], "provider-model");
        assert!(observed.event.get("stream").is_none());
        assert!(observed.event.get("background").is_none());
        assert!(matches!(
            response,
            super::super::backend::ResponsesBackendInput::Event(_)
        ));
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

    async fn connect_test_websocket(
        app: Router,
        path: &str,
    ) -> (wreq::ws::WebSocket, tokio::task::JoinHandle<()>) {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("test WebSocket listener should bind");
        let address = listener
            .local_addr()
            .expect("test WebSocket listener should expose its address");
        let server = tokio::spawn(async move {
            axum::serve(listener, app)
                .await
                .expect("test WebSocket server should run");
        });
        let response = wreq::Client::builder()
            .build()
            .expect("test WebSocket client should build")
            .websocket(format!("ws://{address}{path}"))
            .send()
            .await
            .expect("test WebSocket handshake should succeed");
        assert_eq!(response.status().as_u16(), 101);
        let socket = response
            .into_websocket()
            .await
            .expect("test response should upgrade to WebSocket");
        (socket, server)
    }

    #[tokio::test]
    async fn outer_supervisor_cancels_a_stalled_inner_session_on_admission_loss() {
        use super::run_responses_websocket;
        use crate::control::GatewayControlDecision;
        use crate::handlers::proxy::websocket::ingress::WebSocketRequestContext;

        struct UnhealthyPermit;

        impl aether_runtime::AdmissionPermitHealth for UnhealthyPermit {
            fn is_healthy(&self) -> bool {
                false
            }
        }

        let state = crate::AppState::new().expect("test gateway state should build");
        let app = Router::new().route(
            "/v1/responses",
            get(move |ws: WebSocketUpgrade| {
                let state = state.clone();
                async move {
                    let context = WebSocketRequestContext {
                        trace_id: "trace-stalled-inner".to_string(),
                        headers: HeaderMap::new(),
                        uri: "/v1/responses".parse().expect("test URI should parse"),
                        remote_addr: "127.0.0.1:41000"
                            .parse()
                            .expect("test remote address should parse"),
                        decision: GatewayControlDecision::synthetic(
                            "/v1/responses",
                            Some("ai_public".to_string()),
                            Some("openai".to_string()),
                            Some("responses_websocket".to_string()),
                            Some("openai:responses".to_string()),
                        ),
                        websocket_connection_permit: aether_runtime::AdmissionPermit::from_parts(
                            None,
                            Some(UnhealthyPermit),
                        ),
                    };
                    ws.on_upgrade(move |socket| run_responses_websocket(socket, state, context))
                }
            }),
        );
        let (mut client, server) = connect_test_websocket(app, "/v1/responses").await;

        // Deliberately send no response.create: the inner session remains
        // blocked in its first socket read until the outer supervisor drops it.
        let error = tokio::time::timeout(Duration::from_secs(2), client.recv())
            .await
            .expect("outer supervisor should cancel the stalled inner session")
            .expect("gateway should send an admission error")
            .expect("admission error frame should be valid");
        let wreq::ws::message::Message::Text(error) = error else {
            panic!("expected a public error event before close: {error:?}");
        };
        let error: serde_json::Value =
            serde_json::from_str(error.as_str()).expect("error event should be JSON");
        assert_eq!(error["type"], "error");
        assert_eq!(error["code"], "websocket_connection_admission_lost");
        assert!(error["param"].is_null());
        assert_eq!(error["sequence_number"], 0);
        assert_eq!(error["status"], 503);
        assert_eq!(error["error"]["type"], "gateway_error");
        assert_eq!(
            error["error"]["code"],
            "websocket_connection_admission_lost"
        );

        let close = tokio::time::timeout(Duration::from_secs(2), client.recv())
            .await
            .expect("gateway should close after the admission error")
            .expect("gateway should send a close frame")
            .expect("close frame should be valid");
        let wreq::ws::message::Message::Close(Some(close)) = close else {
            panic!("expected a close frame after the public error: {close:?}");
        };
        assert_eq!(u16::from(close.code), CLOSE_TRY_AGAIN);
        assert_eq!(close.reason.as_str(), "connection_admission_lost");
        server.abort();
    }

    #[tokio::test]
    async fn connection_limit_teardown_uses_the_official_error_shape() {
        use super::{close_terminated_bootstrap, BootstrapTermination};

        let app = Router::new().route(
            "/v1/responses",
            get(|ws: WebSocketUpgrade| async move {
                ws.on_upgrade(|mut socket| async move {
                    close_terminated_bootstrap(
                        &mut socket,
                        &ResponsesPublicEventSequence::default(),
                        BootstrapTermination::ConnectionLimitReached,
                    )
                    .await;
                })
            }),
        );
        let (mut client, server) = connect_test_websocket(app, "/v1/responses").await;

        let error = tokio::time::timeout(Duration::from_secs(2), client.recv())
            .await
            .expect("connection-limit error should arrive")
            .expect("gateway should send a connection-limit error")
            .expect("connection-limit error frame should be valid");
        let wreq::ws::message::Message::Text(error) = error else {
            panic!("expected the official error event before close: {error:?}");
        };
        let error: serde_json::Value =
            serde_json::from_str(error.as_str()).expect("error event should be JSON");
        assert_eq!(error["type"], "error");
        assert_eq!(error["code"], "websocket_connection_limit_reached");
        assert_eq!(
            error["message"],
            "Responses websocket connection limit reached (60 minutes). Create a new websocket connection to continue."
        );
        assert!(error["param"].is_null());
        assert_eq!(error["sequence_number"], 0);
        assert_eq!(error["status"], 400);
        assert_eq!(error["error"]["type"], "invalid_request_error");
        assert_eq!(error["error"]["code"], "websocket_connection_limit_reached");
        assert_eq!(
            error["error"]["message"],
            "Responses websocket connection limit reached (60 minutes). Create a new websocket connection to continue."
        );

        let close = tokio::time::timeout(Duration::from_secs(2), client.recv())
            .await
            .expect("gateway should close after the connection-limit error")
            .expect("gateway should send a close frame")
            .expect("close frame should be valid");
        let wreq::ws::message::Message::Close(Some(close)) = close else {
            panic!("expected a close frame after the official error: {close:?}");
        };
        assert_eq!(u16::from(close.code), CLOSE_TRY_AGAIN);
        assert_eq!(close.reason.as_str(), "connection_limit_reached");
        server.abort();
    }

    #[tokio::test]
    async fn previous_response_not_found_continues_the_public_sequence_and_names_its_param() {
        let app = Router::new().route(
            "/v1/responses",
            get(|ws: WebSocketUpgrade| async move {
                ws.on_upgrade(|mut socket| async move {
                    let sequence = ResponsesPublicEventSequence::default();
                    assert_eq!(sequence.next(), 0);
                    assert_eq!(sequence.next(), 1);
                    send_previous_response_not_found(&mut socket, &sequence).await;
                })
            }),
        );
        let (mut client, server) = connect_test_websocket(app, "/v1/responses").await;

        let event = tokio::time::timeout(Duration::from_secs(2), client.recv())
            .await
            .expect("previous-response error should arrive")
            .expect("gateway should send a previous-response error")
            .expect("previous-response error frame should be valid");
        let wreq::ws::message::Message::Text(event) = event else {
            panic!("expected a public error event: {event:?}");
        };
        let event: serde_json::Value =
            serde_json::from_str(event.as_str()).expect("error event should be JSON");

        assert_eq!(event["type"], "error");
        assert_eq!(event["code"], "previous_response_not_found");
        assert_eq!(event["sequence_number"], 2);
        assert_eq!(event["param"], "previous_response_id");
        assert_eq!(event["error"]["param"], "previous_response_id");
        server.abort();
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
        let beta_header_present = headers.contains_key("openai-beta");
        ws.on_upgrade(move |socket| async move {
            serve_mock_socket(
                socket,
                state,
                authorization_present,
                account_header_present,
                beta_header_present,
            )
            .await;
        })
    }

    async fn serve_mock_socket(
        socket: WebSocket,
        state: Arc<MockState>,
        authorization_present: bool,
        account_header_present: bool,
        beta_header_present: bool,
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
                beta_header_present,
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

    fn sample_bound_candidate(model: &str) -> SchedulerMinimalCandidateSelectionCandidate {
        SchedulerMinimalCandidateSelectionCandidate {
            provider_id: "provider-1".to_string(),
            provider_name: "Provider".to_string(),
            provider_type: "custom".to_string(),
            provider_priority: 0,
            endpoint_id: "endpoint-1".to_string(),
            endpoint_api_format: "openai:responses".to_string(),
            key_id: "key-1".to_string(),
            key_name: "key-1".to_string(),
            key_auth_type: "api_key".to_string(),
            key_internal_priority: 0,
            key_global_priority_for_format: None,
            key_capabilities: None,
            model_id: "model-1".to_string(),
            global_model_id: "global-model-1".to_string(),
            global_model_name: model.to_string(),
            selected_provider_model_name: model.to_string(),
            supports_streaming: true,
            mapping_matched_model: None,
        }
    }

    fn sample_bound_for_rebind_safety() -> BoundResponsesConnection {
        let backend = resolve_native_responses_websocket_backend(
            crate::orchestration::ResponsesWebSocketBackendKind::NativeResponsesWebSocket,
        );
        let provider_observer = resolve_responses_provider_observer(
            crate::orchestration::ResponsesProviderObserverKind::Codex,
        );
        let decision = sample_decision();
        let binding_identity =
            UpstreamBindingIdentity::from_decision(backend, &decision, "credential-1").unwrap();
        BoundResponsesConnection {
            backend_session: Default::default(),
            backend,
            public_codec: responses_public_wire_codec(),
            public_event_state: Default::default(),
            provider_observer,
            client_model: "gpt-5.6-sol".to_string(),
            provider_model: "gpt-5.6-sol".to_string(),
            decision_template: decision,
            bound_candidate: sample_bound_candidate("gpt-5.6-sol"),
            body_normalization: ResponsesWebSocketBodyNormalization::for_tests("gpt-5.6-sol"),
            binding_identity,
            // Replanning：logical turn 在、attempt 不在。重放安全与配额排除都只看
            // logical turn，所以这些用例不需要真实 socket 或真实 attempt。
            turn_state: ResponsesTurnState::Replanning {
                logical: LogicalTurn::new(
                    json!({"type": "response.create", "model": "gpt-5.6-sol"}),
                    1,
                    "logical-turn".to_string(),
                ),
            },
            public_event_sequence: Default::default(),
            public_teardown: Default::default(),
            latest_public_response_id: None,
            next_turn_index: 2,
            upstream_response_headers: BTreeMap::new(),
            pending_provider_drain: None,
            pending_provider_observation: None,
            exhausted_exclusions: ExhaustedResponsesWebSocketExclusions::default(),
            pending_turn_finalization: None,
            redaction_restorer: ResponsesWebSocketRedactionRestorer::default(),
            session_termination: Default::default(),
        }
    }

    /// 用 mpsc 驱动的 FakeSocket，实现 Stream + Sink 两个 trait。
    /// 测试侧通过 tx 注入消息，通过 pong_rx 观察 Pong 回包。
    struct FakeSocket {
        rx: tokio::sync::mpsc::Receiver<axum::extract::ws::Message>,
        pong_tx: tokio::sync::mpsc::UnboundedSender<axum::extract::ws::Message>,
    }

    struct FakeSocketPair {
        tx: tokio::sync::mpsc::Sender<axum::extract::ws::Message>,
        pong_rx: tokio::sync::mpsc::UnboundedReceiver<axum::extract::ws::Message>,
    }

    fn fake_socket() -> (FakeSocket, FakeSocketPair) {
        let (tx, rx) = tokio::sync::mpsc::channel(16);
        let (pong_tx, pong_rx) = tokio::sync::mpsc::unbounded_channel();
        (FakeSocket { rx, pong_tx }, FakeSocketPair { tx, pong_rx })
    }

    impl futures_util::Stream for FakeSocket {
        type Item = Result<axum::extract::ws::Message, axum::Error>;

        fn poll_next(
            mut self: std::pin::Pin<&mut Self>,
            cx: &mut std::task::Context<'_>,
        ) -> std::task::Poll<Option<Self::Item>> {
            self.rx.poll_recv(cx).map(|opt| opt.map(Ok))
        }
    }

    impl futures_util::Sink<axum::extract::ws::Message> for FakeSocket {
        type Error = axum::Error;

        fn poll_ready(
            self: std::pin::Pin<&mut Self>,
            _cx: &mut std::task::Context<'_>,
        ) -> std::task::Poll<Result<(), Self::Error>> {
            std::task::Poll::Ready(Ok(()))
        }

        fn start_send(
            self: std::pin::Pin<&mut Self>,
            item: axum::extract::ws::Message,
        ) -> Result<(), Self::Error> {
            let _ = self.pong_tx.send(item);
            Ok(())
        }

        fn poll_flush(
            self: std::pin::Pin<&mut Self>,
            _cx: &mut std::task::Context<'_>,
        ) -> std::task::Poll<Result<(), Self::Error>> {
            std::task::Poll::Ready(Ok(()))
        }

        fn poll_close(
            self: std::pin::Pin<&mut Self>,
            _cx: &mut std::task::Context<'_>,
        ) -> std::task::Poll<Result<(), Self::Error>> {
            std::task::Poll::Ready(Ok(()))
        }
    }

    struct StalledPongSocket {
        initial_ping: Option<axum::extract::ws::Message>,
    }

    impl futures_util::Stream for StalledPongSocket {
        type Item = Result<axum::extract::ws::Message, axum::Error>;

        fn poll_next(
            mut self: std::pin::Pin<&mut Self>,
            _cx: &mut std::task::Context<'_>,
        ) -> std::task::Poll<Option<Self::Item>> {
            match self.initial_ping.take() {
                Some(message) => std::task::Poll::Ready(Some(Ok(message))),
                None => std::task::Poll::Pending,
            }
        }
    }

    impl futures_util::Sink<axum::extract::ws::Message> for StalledPongSocket {
        type Error = axum::Error;

        fn poll_ready(
            self: std::pin::Pin<&mut Self>,
            _cx: &mut std::task::Context<'_>,
        ) -> std::task::Poll<Result<(), Self::Error>> {
            std::task::Poll::Pending
        }

        fn start_send(
            self: std::pin::Pin<&mut Self>,
            _item: axum::extract::ws::Message,
        ) -> Result<(), Self::Error> {
            unreachable!("the stalled sink never becomes ready")
        }

        fn poll_flush(
            self: std::pin::Pin<&mut Self>,
            _cx: &mut std::task::Context<'_>,
        ) -> std::task::Poll<Result<(), Self::Error>> {
            std::task::Poll::Pending
        }

        fn poll_close(
            self: std::pin::Pin<&mut Self>,
            _cx: &mut std::task::Context<'_>,
        ) -> std::task::Poll<Result<(), Self::Error>> {
            std::task::Poll::Pending
        }
    }

    #[tokio::test]
    async fn initial_pong_write_cannot_outlive_the_initial_message_deadline() {
        use super::{receive_initial_response_create_with_deadline, InitialMessageError};

        let mut socket = StalledPongSocket {
            initial_ping: Some(axum::extract::ws::Message::Ping(vec![1].into())),
        };

        let result = tokio::time::timeout(
            Duration::from_millis(500),
            receive_initial_response_create_with_deadline(&mut socket, Duration::from_millis(30)),
        )
        .await
        .expect("the absolute deadline must bound a stalled Pong write");

        assert!(matches!(result, Err(InitialMessageError::TimedOut)));
    }

    #[tokio::test]
    async fn bootstrap_supervisor_honors_the_absolute_connection_deadline() {
        use super::{wait_for_bootstrap_termination, BootstrapTermination};

        let result = tokio::time::timeout(
            Duration::from_millis(500),
            wait_for_bootstrap_termination(
                tokio::time::Instant::now() + Duration::from_millis(30),
                None,
            ),
        )
        .await
        .expect("bootstrap supervisor must not outlive the connection deadline");

        assert!(matches!(
            result,
            BootstrapTermination::ConnectionLimitReached
        ));
    }

    #[test]
    fn elapsed_bootstrap_deadline_is_detected_at_resource_boundaries() {
        use super::{bootstrap_termination, BootstrapTermination};

        let result = bootstrap_termination(tokio::time::Instant::now(), None);

        assert!(matches!(
            result,
            Some(BootstrapTermination::ConnectionLimitReached)
        ));
    }

    #[test]
    fn unhealthy_connection_permit_is_detected_during_bootstrap() {
        use super::{bootstrap_termination, BootstrapTermination};

        struct UnhealthyPermit;

        impl aether_runtime::AdmissionPermitHealth for UnhealthyPermit {
            fn is_healthy(&self) -> bool {
                false
            }
        }

        let permit = aether_runtime::AdmissionPermit::from_parts(None, Some(UnhealthyPermit))
            .expect("distributed admission health creates a permit");
        let result = bootstrap_termination(
            tokio::time::Instant::now() + Duration::from_secs(60),
            Some(&permit),
        );

        assert!(matches!(
            result,
            Some(BootstrapTermination::ConnectionAdmissionLost)
        ));
    }

    /// 验证 receive_initial_response_create_with_deadline 的绝对 deadline：
    /// 客户端周期性发送 Ping 帧不会重置计时器，deadline 到期后返回 TimedOut。
    /// 这直接驱动真实的循环逻辑，如果改回每次迭代 timeout(budget, ...) 则会变红。
    ///
    /// 设计思路：deadline_budget = 80ms，Ping 每 30ms 发一次且永不停止。
    /// - 绝对 deadline：~80ms 后函数返回 TimedOut（即使 Ping 仍在到来）。
    /// - 每次迭代 timeout(80ms, ...)：每个 Ping 在 30ms 内到达 < 80ms，函数
    ///   永不超时，500ms 后外层 timeout 判定测试失败。
    #[tokio::test]
    async fn initial_message_times_out_despite_periodic_pings() {
        use super::{receive_initial_response_create_with_deadline, InitialMessageError};

        let (mut fake, pair) = fake_socket();

        let handle = tokio::spawn(async move {
            receive_initial_response_create_with_deadline(&mut fake, Duration::from_millis(80))
                .await
        });

        // 持续发送 Ping，间隔 30ms，永不停止（直到被测函数返回导致 rx drop）
        let ping_task = tokio::spawn(async move {
            let mut i = 0u8;
            loop {
                tokio::time::sleep(Duration::from_millis(30)).await;
                if pair
                    .tx
                    .send(axum::extract::ws::Message::Ping(vec![i].into()))
                    .await
                    .is_err()
                {
                    break;
                }
                i = i.wrapping_add(1);
            }
        });

        // 绝对 deadline 应在 ~80ms 后触发；给 500ms 宽限等待结果。
        let result = tokio::time::timeout(Duration::from_millis(500), handle)
            .await
            .expect("function should return within 500ms (absolute deadline = 80ms)")
            .expect("task should not panic");

        ping_task.abort();

        assert!(
            matches!(result, Err(InitialMessageError::TimedOut)),
            "expected TimedOut after absolute deadline, got: {result:?}"
        );
    }

    /// 验证 deadline 内收到合法 response.create 时正常返回，Ping 被正确回复 Pong。
    #[tokio::test]
    async fn initial_message_succeeds_within_deadline() {
        use super::{receive_initial_response_create_with_deadline, InitialMessageError};

        let (mut fake, mut pair) = fake_socket();

        let handle = tokio::spawn(async move {
            receive_initial_response_create_with_deadline(&mut fake, Duration::from_secs(5)).await
        });

        // 先发一个 Ping，验证 Pong 回包且不影响后续解析
        pair.tx
            .send(axum::extract::ws::Message::Ping(vec![42].into()))
            .await
            .unwrap();
        let pong = tokio::time::timeout(Duration::from_secs(1), pair.pong_rx.recv())
            .await
            .expect("should receive pong within 1s")
            .expect("pong channel should not close");
        assert!(
            matches!(pong, axum::extract::ws::Message::Pong(ref data) if data.as_ref() == [42]),
            "expected Pong([42]), got: {pong:?}"
        );

        // 发送合法的 response.create
        let event_text = r#"{"type":"response.create","model":"gpt-4o"}"#;
        pair.tx
            .send(axum::extract::ws::Message::Text(
                event_text.to_string().into(),
            ))
            .await
            .unwrap();

        let result = tokio::time::timeout(Duration::from_secs(2), handle)
            .await
            .expect("handle should finish within 2s")
            .expect("task should not panic");
        let (text, event) = result.expect("should return Ok");
        assert_eq!(text, event_text);
        assert_eq!(event["type"], "response.create");
        assert_eq!(event["model"], "gpt-4o");
    }
}
