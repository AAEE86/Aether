//! Client-side Responses WebSocket event forwarding and follow-up planning.

use axum::body::Bytes;
use axum::extract::ws::{Message as AxumWsMessage, WebSocket};
use futures_util::SinkExt;
use serde_json::Value;
use uuid::Uuid;

use super::adapter::{
    resolve_responses_provider_observer, ResponsesProviderObserver, ResponsesPublicEventState,
    ResponsesWebSocketDrainDirective,
};
use super::backend::resolve_native_responses_websocket_backend;
use super::lifecycle::{
    await_pending_turn_finalization, queue_turn_finalization, responses_websocket_turn_start_close,
    send_responses_websocket_turn_start_error,
};
use super::ownership::{
    await_owned_responses_websocket_plan, await_owned_responses_websocket_turn, disarm_owned_turn,
    spawn_owned_responses_websocket_plan, spawn_owned_responses_websocket_turn,
    PlannedPoolKeyLeaseGuard,
};
use super::quota::send_previous_response_not_found;
use super::redaction::redact_responses_websocket_client_event;
use super::request::{
    build_planning_parts, changed_followup_response_create_model,
    continuation_requires_same_upstream, planned_response_create_event,
    provider_model_from_decision, response_create_has_previous_response_id,
    response_create_model_or_current, response_create_previous_response_id,
    responses_public_request_codec, ResponsesPublicRequestError,
};
use super::state::{
    evict_referenced_public_response_id, BoundResponsesConnection, ResponsesPublicEventSequence,
};
use super::turn::{
    prepare_responses_websocket_turn_decision, refresh_responses_websocket_turn_auth,
    ResponsesWebSocketTurnObservation, ResponsesWebSocketTurnOutcome,
};
use super::turn_state::LogicalTurn;
use super::upstream::{
    bind_responses_upstream, canonicalize_responses_websocket_decision,
    decision_reuses_bound_upstream, ResponsesUpstreamBindError,
};
use crate::ai_serving::{
    revalidate_bound_responses_candidate, BoundResponsesCandidateRevalidation,
};
use crate::clock::current_unix_secs;
use crate::control::{request_model_local_rejection, GatewayControlDecision};
use crate::handlers::proxy::websocket::ingress::WebSocketRequestContext;
use crate::handlers::proxy::websocket::session::{
    CLOSE_INTERNAL_ERROR, CLOSE_POLICY_VIOLATION, WEBSOCKET_LOG_TRANSPORT,
};
use crate::handlers::proxy::websocket::transport::{
    close_client_socket, send_client_message, send_gateway_error, send_gateway_error_with_status,
    send_responses_websocket_error,
};
use crate::headers::effective_client_ip;
use crate::rate_limit::FrontdoorUserRpmOutcome;
use crate::AppState;

const LOG_TARGET: &str = "aether_gateway::handlers::proxy::responses_ws";

macro_rules! debug {
    ($($arg:tt)*) => {
        tracing::debug!(target: LOG_TARGET, $($arg)*)
    };
}

macro_rules! warn {
    ($($arg:tt)*) => {
        tracing::warn!(target: LOG_TARGET, $($arg)*)
    };
}

pub(super) enum RelayDisposition {
    Continue,
    Close,
    ExecutionReservationLost,
    UpstreamError(&'static str),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct ClientProtocolError {
    code: &'static str,
    message: &'static str,
    param: Option<&'static str>,
}

const INVALID_CLIENT_EVENT: ClientProtocolError = ClientProtocolError {
    code: "invalid_client_event",
    message: "Client WebSocket messages must be valid JSON response.create events",
    param: None,
};
const UNSUPPORTED_CLIENT_EVENT: ClientProtocolError = ClientProtocolError {
    code: "unsupported_client_event",
    message: "Only response.create client events are supported",
    param: None,
};
const UNSUPPORTED_CLIENT_FRAME: ClientProtocolError = ClientProtocolError {
    code: "unsupported_client_frame",
    message: "Responses WebSocket client events must use JSON text frames",
    param: None,
};
#[derive(Debug, Clone, PartialEq, Eq)]
enum ClientControlFrameAction {
    ReplyPong(Bytes),
    Consume,
    CloseUpstream,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ClientProtocolRejectionAction {
    Continue,
    TerminateActiveTurn,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum PreviousResponseValidationError {
    Invalid,
    NotOwned,
}

fn validate_followup_previous_response_id<'a>(
    event: &'a Value,
    latest_public_response_id: Option<&str>,
) -> Result<Option<&'a str>, PreviousResponseValidationError> {
    let previous_response_id = response_create_previous_response_id(event)
        .map_err(|_| PreviousResponseValidationError::Invalid)?;
    if previous_response_id
        .is_some_and(|response_id| Some(response_id) != latest_public_response_id)
    {
        return Err(PreviousResponseValidationError::NotOwned);
    }
    Ok(previous_response_id)
}

/// Applies OpenAI's connection-local failure rule to a request that has
/// already passed previous-response ownership validation.  Only a continuation
/// can evict the committed ID; an independent failed turn leaves it intact.
fn evict_failed_continuation(bound: &mut BoundResponsesConnection, event: &Value) {
    if event.get("type").and_then(Value::as_str) != Some("response.create") {
        return;
    }
    let previous_response_id = response_create_previous_response_id(event)
        .ok()
        .flatten()
        .map(str::to_string);
    evict_public_response_id(bound, previous_response_id.as_deref());
}

fn evict_public_response_id(
    bound: &mut BoundResponsesConnection,
    previous_response_id: Option<&str>,
) {
    evict_referenced_public_response_id(&mut bound.latest_public_response_id, previous_response_id);
}

fn client_protocol_rejection_action(response_in_flight: bool) -> ClientProtocolRejectionAction {
    if response_in_flight {
        ClientProtocolRejectionAction::TerminateActiveTurn
    } else {
        ClientProtocolRejectionAction::Continue
    }
}

fn client_control_frame_action(message: &AxumWsMessage) -> Option<ClientControlFrameAction> {
    match message {
        AxumWsMessage::Ping(payload) => Some(ClientControlFrameAction::ReplyPong(payload.clone())),
        AxumWsMessage::Pong(_) => Some(ClientControlFrameAction::Consume),
        AxumWsMessage::Close(_) => Some(ClientControlFrameAction::CloseUpstream),
        AxumWsMessage::Text(_) | AxumWsMessage::Binary(_) => None,
    }
}

fn parse_public_response_create(text: &str) -> Result<Value, ClientProtocolError> {
    let event = serde_json::from_str::<Value>(text).map_err(|_| INVALID_CLIENT_EVENT)?;
    responses_public_request_codec()
        .response_create(&event)
        .map_err(|error| match error {
            ResponsesPublicRequestError::InvalidEventShape
            | ResponsesPublicRequestError::UnsupportedEventType => UNSUPPORTED_CLIENT_EVENT,
            error => ClientProtocolError {
                code: error.code(),
                message: error.message(),
                param: error.param(),
            },
        })
}

async fn reject_client_protocol_event(
    client_socket: &mut WebSocket,
    bound: &mut BoundResponsesConnection,
    error: ClientProtocolError,
) -> RelayDisposition {
    let action = client_protocol_rejection_action(bound.turn_state.response_in_flight());
    if action == ClientProtocolRejectionAction::TerminateActiveTurn {
        if !bound.public_teardown.try_claim() {
            return RelayDisposition::Close;
        }
        if bound
            .public_event_state
            .accept_local_terminal_error()
            .is_err()
        {
            return RelayDisposition::Close;
        }
    } else {
        begin_local_error_response(&bound.public_event_sequence, &mut bound.public_event_state);
    }
    send_responses_websocket_error(
        client_socket,
        &bound.public_event_sequence,
        400,
        "invalid_request_error",
        error.code,
        error.message,
        error.param,
    )
    .await;
    if action == ClientProtocolRejectionAction::Continue {
        return RelayDisposition::Continue;
    }
    bound.backend_session.close().await;
    close_client_socket(client_socket, CLOSE_POLICY_VIOLATION, error.code).await;
    RelayDisposition::Close
}

/// Starts a new public response whose only server event will be a local error.
/// This keeps follow-up validation/admission failures on the same per-response
/// sequence and FSM contract as provider-backed turns.
fn begin_local_error_response(
    sequence: &ResponsesPublicEventSequence,
    state: &mut ResponsesPublicEventState,
) {
    sequence.reset();
    state.reset();
    state
        .accept_local_terminal_error()
        .expect("a fresh public response accepts a local error terminal");
}

fn response_create_access_check_event(
    client_event: &Value,
    current_model: &str,
) -> Result<Value, &'static str> {
    let mut event = client_event.clone();
    response_create_model_or_current(&mut event, current_model)?;
    Ok(event)
}

fn provider_observers_allow_socket_reuse(
    bound: &dyn ResponsesProviderObserver,
    planned: &dyn ResponsesProviderObserver,
) -> bool {
    bound.kind() == planned.kind()
}

pub(super) fn provider_drain_ready(
    pending_provider_drain: Option<ResponsesWebSocketDrainDirective>,
    response_in_flight: bool,
    observation: Option<ResponsesWebSocketTurnObservation>,
    upstream_closed: bool,
) -> bool {
    pending_provider_drain.is_some()
        && (upstream_closed
            || !response_in_flight
            || matches!(
                observation,
                Some(ResponsesWebSocketTurnObservation::Terminal(_))
            ))
}

pub(super) async fn forward_client_message(
    client_message: AxumWsMessage,
    bound: &mut BoundResponsesConnection,
    client_socket: &mut WebSocket,
    state: &AppState,
    context: &WebSocketRequestContext,
) -> RelayDisposition {
    if let Some(action) = client_control_frame_action(&client_message) {
        return match action {
            ClientControlFrameAction::ReplyPong(payload) => {
                send_client_message(client_socket, AxumWsMessage::Pong(payload))
                    .await
                    .map(|()| RelayDisposition::Continue)
                    .unwrap_or(RelayDisposition::Close)
            }
            ClientControlFrameAction::Consume => RelayDisposition::Continue,
            ClientControlFrameAction::CloseUpstream => {
                // The public and provider sockets are separate WebSocket
                // endpoints. Do not expose the client's close code or reason
                // to the provider transport.
                bound.backend_session.close().await;
                RelayDisposition::Close
            }
        };
    }

    match client_message {
        AxumWsMessage::Text(text) => {
            let text = text.to_string();
            let client_event = match parse_public_response_create(&text) {
                Ok(event) => event,
                Err(error) => {
                    if let Ok(rejected_event) = serde_json::from_str::<Value>(&text) {
                        evict_failed_continuation(bound, &rejected_event);
                    }
                    return reject_client_protocol_event(client_socket, bound, error).await;
                }
            };

            if !bound.turn_state.accepts_new_response_create() {
                evict_failed_continuation(bound, &client_event);
                return reject_client_protocol_event(
                    client_socket,
                    bound,
                    ClientProtocolError {
                        code: "response_already_in_progress",
                        message: "This connection runs one response at a time",
                        param: None,
                    },
                )
                .await;
            }
            match validate_followup_previous_response_id(
                &client_event,
                bound.latest_public_response_id.as_deref(),
            ) {
                Ok(_) => {}
                Err(PreviousResponseValidationError::Invalid) => {
                    begin_local_error_response(
                        &bound.public_event_sequence,
                        &mut bound.public_event_state,
                    );
                    send_responses_websocket_error(
                        client_socket,
                        &bound.public_event_sequence,
                        400,
                        "invalid_request_error",
                        "invalid_previous_response_id",
                        "response.create.previous_response_id must be a non-empty string or null",
                        Some("previous_response_id"),
                    )
                    .await;
                    return RelayDisposition::Continue;
                }
                Err(PreviousResponseValidationError::NotOwned) => {
                    begin_local_error_response(
                        &bound.public_event_sequence,
                        &mut bound.public_event_state,
                    );
                    send_previous_response_not_found(client_socket, &bound.public_event_sequence)
                        .await;
                    return RelayDisposition::Continue;
                }
            }
            // A syntactically valid response.create starts a fresh public
            // response even when a later auth/model/admission step rejects it.
            // Provider-backed paths reset again when their attempt begins;
            // both resets occur before the first public event.
            begin_local_error_response(&bound.public_event_sequence, &mut bound.public_event_state);
            // A prior terminal turn may still be writing usage/audit and
            // projecting provider effects. Do not let a new independent turn
            // plan against stale health, adaptive, or pool state.
            await_pending_turn_finalization(bound).await;

            let client_ip = effective_client_ip(&context.headers, &context.remote_addr);
            let turn_control_decision =
                match refresh_responses_websocket_turn_auth(state, &context.decision, client_ip)
                    .await
                {
                    Ok(authorization) => authorization,
                    Err(error) => {
                        evict_failed_continuation(bound, &client_event);
                        if !bound.public_teardown.try_claim() {
                            return RelayDisposition::Close;
                        }
                        send_responses_websocket_turn_start_error(
                            client_socket,
                            &bound.public_event_sequence,
                            &error,
                        )
                        .await;
                        let (close_code, close_reason) =
                            responses_websocket_turn_start_close(&error);
                        close_client_socket(client_socket, close_code, close_reason).await;
                        return RelayDisposition::Close;
                    }
                };
            match consume_response_create_rate_limit(
                state,
                &turn_control_decision.decision,
                client_ip,
                &context.trace_id,
            )
            .await
            {
                Ok(true) => {}
                Ok(false) => {
                    evict_failed_continuation(bound, &client_event);
                    send_gateway_error_with_status(
                        client_socket,
                        &bound.public_event_sequence,
                        429,
                        "rate_limit_exceeded",
                        "Too many response.create events; retry later",
                    )
                    .await;
                    return RelayDisposition::Continue;
                }
                Err(()) => {
                    evict_failed_continuation(bound, &client_event);
                    if !bound.public_teardown.try_claim() {
                        return RelayDisposition::Close;
                    }
                    send_gateway_error_with_status(
                        client_socket,
                        &bound.public_event_sequence,
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
                    return RelayDisposition::Close;
                }
            }

            // 这一轮的 planning Parts 只构造一次（它携带 per-turn 的
            // RedactionSessionSlot），并且客户端事件也只在这里脱敏一次：
            // 复用已绑定 upstream 的 continuation 根本不进 planner，只靠 planner
            // 内部脱敏拦不住它。之后 re-plan / continuation / 配额重试都只看脱敏
            // 后的事件，上游请求体与审计 original_request_body 因此一致。
            let turn_request_id =
                crate::execution_identity::ExecutionRequestId::generate().into_string();
            let planning_parts = build_planning_parts(context, &turn_request_id);
            // A follow-up may omit `model`, but its effective model is still
            // the public model bound to this connection. Inject that model
            // only into the access-check copy so a per-turn allowed-model
            // refresh cannot be bypassed by omission; the original event keeps
            // its protocol semantics for redaction and provider forwarding.
            let access_check_event =
                match response_create_access_check_event(&client_event, &bound.client_model) {
                    Ok(event) => event,
                    Err(code) => {
                        evict_failed_continuation(bound, &client_event);
                        send_gateway_error(
                            client_socket,
                            &bound.public_event_sequence,
                            code,
                            "response.create.model must be a non-empty string",
                        )
                        .await;
                        return RelayDisposition::Continue;
                    }
                };
            let client_event_text = match serde_json::to_vec(&access_check_event) {
                Ok(value) => Bytes::from(value),
                Err(_) => {
                    evict_failed_continuation(bound, &client_event);
                    send_gateway_error(
                        client_socket,
                        &bound.public_event_sequence,
                        "invalid_response_create",
                        "response.create must be valid JSON",
                    )
                    .await;
                    return RelayDisposition::Continue;
                }
            };
            match request_model_local_rejection(
                state,
                Some(&turn_control_decision.decision),
                &planning_parts.uri,
                &planning_parts.headers,
                &client_event_text,
            )
            .await
            {
                Ok(Some(_)) => {
                    evict_failed_continuation(bound, &client_event);
                    send_gateway_error(
                        client_socket,
                        &bound.public_event_sequence,
                        "model_not_allowed",
                        "The requested model is not available to this API key",
                    )
                    .await;
                    return RelayDisposition::Continue;
                }
                Ok(None) => {}
                Err(error) => {
                    evict_failed_continuation(bound, &client_event);
                    if !bound.public_teardown.try_claim() {
                        return RelayDisposition::Close;
                    }
                    warn!(
                        event_name = "responses_websocket_followup_model_access_check_failed",
                        log_type = "ops",
                        transport = WEBSOCKET_LOG_TRANSPORT,
                        websocket = true,
                        trace_id = %context.trace_id,
                        error = ?error,
                        "gateway failed to evaluate follow-up WebSocket model access policy"
                    );
                    send_gateway_error(
                        client_socket,
                        &bound.public_event_sequence,
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
            let redacted_client_event = redact_responses_websocket_client_event(
                state,
                &planning_parts,
                &turn_control_decision.decision,
                &client_event,
            )
            .await;
            let (client_event, redaction_session) = match redacted_client_event {
                Ok(Some(redaction)) => (redaction.client_event, Some(redaction.session)),
                Ok(None) => (client_event, None),
                Err(error) => {
                    evict_failed_continuation(bound, &client_event);
                    if !bound.public_teardown.try_claim() {
                        return RelayDisposition::Close;
                    }
                    warn!(
                        event_name = "responses_websocket_followup_redaction_failed",
                        log_type = "ops",
                        transport = WEBSOCKET_LOG_TRANSPORT,
                        websocket = true,
                        trace_id = %context.trace_id,
                        error = ?error,
                        "gateway could not apply chat PII redaction to a Responses WebSocket turn"
                    );
                    send_gateway_error_with_status(
                        client_socket,
                        &bound.public_event_sequence,
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
                    return RelayDisposition::Close;
                }
            };
            if !bound.backend_session.is_bound() {
                if response_create_has_previous_response_id(&client_event) {
                    evict_failed_continuation(bound, &client_event);
                    send_previous_response_not_found(client_socket, &bound.public_event_sequence)
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
                        evict_failed_continuation(bound, &client_event);
                        send_gateway_error(
                            client_socket,
                            &bound.public_event_sequence,
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
                    turn_request_id.clone(),
                    planning_parts,
                    &turn_control_decision.decision,
                    client_event,
                    requested_model,
                    redaction_session,
                    turn_control_decision.auth_snapshot,
                )
                .await;
            }
            let changed_model =
                match changed_followup_response_create_model(&client_event, &bound.client_model) {
                    Ok(model) => model,
                    Err(code) => {
                        evict_failed_continuation(bound, &client_event);
                        send_gateway_error(
                            client_socket,
                            &bound.public_event_sequence,
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
                    turn_request_id.clone(),
                    planning_parts,
                    &turn_control_decision.decision,
                    client_event,
                    requested_model,
                    redaction_session,
                    turn_control_decision.auth_snapshot,
                )
                .await;
            }
            if !response_create_has_previous_response_id(&client_event) {
                return forward_replanned_response_create(
                    bound,
                    client_socket,
                    state,
                    context,
                    turn_request_id.clone(),
                    planning_parts,
                    &turn_control_decision.decision,
                    client_event,
                    bound.client_model.clone(),
                    redaction_session,
                    turn_control_decision.auth_snapshot,
                )
                .await;
            }

            let Some(auth_snapshot) = turn_control_decision.auth_snapshot.as_ref() else {
                evict_failed_continuation(bound, &client_event);
                if !bound.public_teardown.try_claim() {
                    return RelayDisposition::Close;
                }
                send_gateway_error_with_status(
                    client_socket,
                    &bound.public_event_sequence,
                    503,
                    "responses_websocket_revalidation_unavailable",
                    "Gateway could not revalidate the bound Responses provider",
                )
                .await;
                close_client_socket(
                    client_socket,
                    CLOSE_INTERNAL_ERROR,
                    "responses_websocket_revalidation_unavailable",
                )
                .await;
                return RelayDisposition::Close;
            };
            let (fresh_decision, credential_binding_fingerprint) =
                match revalidate_bound_responses_candidate(
                    state,
                    &planning_parts,
                    &context.trace_id,
                    &turn_control_decision.decision,
                    auth_snapshot,
                    &client_event,
                    &bound.bound_candidate,
                    &bound.provider_model,
                    bound.backend.kind(),
                    bound.provider_observer.kind(),
                )
                .await
                {
                    BoundResponsesCandidateRevalidation::Prepared {
                        decision,
                        credential_binding_fingerprint,
                    } => (
                        canonicalize_responses_websocket_decision(decision),
                        credential_binding_fingerprint,
                    ),
                    BoundResponsesCandidateRevalidation::CapacityLimited { reason } => {
                        evict_failed_continuation(bound, &client_event);
                        debug!(
                            event_name = "responses_websocket_bound_candidate_capacity_limited",
                            log_type = "event",
                            transport = WEBSOCKET_LOG_TRANSPORT,
                            websocket = true,
                            trace_id = %context.trace_id,
                            provider_id = %bound.bound_candidate.provider_id,
                            endpoint_id = %bound.bound_candidate.endpoint_id,
                            key_id = %bound.bound_candidate.key_id,
                            reason,
                            "gateway retained the bound Responses WebSocket while its concrete candidate was temporarily unavailable"
                        );
                        send_gateway_error_with_status(
                            client_socket,
                            &bound.public_event_sequence,
                            429,
                            "responses_websocket_candidate_capacity_limited",
                            "The bound Responses provider is temporarily at capacity; retry later",
                        )
                        .await;
                        return RelayDisposition::Continue;
                    }
                    BoundResponsesCandidateRevalidation::Denied { reason } => {
                        evict_failed_continuation(bound, &client_event);
                        if !bound.public_teardown.try_claim() {
                            return RelayDisposition::Close;
                        }
                        warn!(
                            event_name = "responses_websocket_bound_candidate_denied",
                            log_type = "ops",
                            transport = WEBSOCKET_LOG_TRANSPORT,
                            websocket = true,
                            trace_id = %context.trace_id,
                            provider_id = %bound.bound_candidate.provider_id,
                            endpoint_id = %bound.bound_candidate.endpoint_id,
                            key_id = %bound.bound_candidate.key_id,
                            reason,
                            "gateway denied a continuation after the bound candidate was revoked"
                        );
                        send_gateway_error_with_status(
                        client_socket,
                        &bound.public_event_sequence,
                        403,
                        "responses_websocket_candidate_denied",
                        "The bound Responses provider is no longer authorized for this connection",
                    )
                    .await;
                        close_client_socket(
                            client_socket,
                            CLOSE_POLICY_VIOLATION,
                            "responses_websocket_candidate_denied",
                        )
                        .await;
                        return RelayDisposition::Close;
                    }
                    BoundResponsesCandidateRevalidation::Unavailable { reason } => {
                        evict_failed_continuation(bound, &client_event);
                        if !bound.public_teardown.try_claim() {
                            return RelayDisposition::Close;
                        }
                        warn!(
                            event_name = "responses_websocket_bound_candidate_revalidation_failed",
                            log_type = "ops",
                            transport = WEBSOCKET_LOG_TRANSPORT,
                            websocket = true,
                            trace_id = %context.trace_id,
                            provider_id = %bound.bound_candidate.provider_id,
                            endpoint_id = %bound.bound_candidate.endpoint_id,
                            key_id = %bound.bound_candidate.key_id,
                            reason,
                            "gateway could not strongly revalidate the bound Responses candidate"
                        );
                        send_gateway_error_with_status(
                            client_socket,
                            &bound.public_event_sequence,
                            503,
                            "responses_websocket_revalidation_unavailable",
                            "Gateway could not revalidate the bound Responses provider",
                        )
                        .await;
                        close_client_socket(
                            client_socket,
                            CLOSE_INTERNAL_ERROR,
                            "responses_websocket_revalidation_unavailable",
                        )
                        .await;
                        return RelayDisposition::Close;
                    }
                };

            if !decision_reuses_bound_upstream(
                bound,
                bound.backend,
                &fresh_decision,
                &credential_binding_fingerprint,
            ) {
                evict_failed_continuation(bound, &client_event);
                if !bound.public_teardown.try_claim() {
                    return RelayDisposition::Close;
                }
                warn!(
                    event_name = "responses_websocket_bound_transport_changed",
                    log_type = "ops",
                    transport = WEBSOCKET_LOG_TRANSPORT,
                    websocket = true,
                    trace_id = %context.trace_id,
                    provider_id = %bound.bound_candidate.provider_id,
                    endpoint_id = %bound.bound_candidate.endpoint_id,
                    key_id = %bound.bound_candidate.key_id,
                    "gateway denied a continuation whose current transport no longer matches the bound socket"
                );
                send_gateway_error_with_status(
                    client_socket,
                    &bound.public_event_sequence,
                    403,
                    "responses_websocket_binding_changed",
                    "The bound Responses provider connection is no longer authorized",
                )
                .await;
                close_client_socket(
                    client_socket,
                    CLOSE_POLICY_VIOLATION,
                    "responses_websocket_binding_changed",
                )
                .await;
                return RelayDisposition::Close;
            }

            let provider_event = match planned_response_create_event(&fresh_decision, &client_event)
                .and_then(|value| {
                    serde_json::from_str::<Value>(&value)
                        .map_err(|_| "response_create_serialization_failed")
                }) {
                Ok(value) => value,
                Err(code) => {
                    evict_failed_continuation(bound, &client_event);
                    send_gateway_error(
                        client_socket,
                        &bound.public_event_sequence,
                        code,
                        "Gateway could not prepare the response.create event",
                    )
                    .await;
                    return RelayDisposition::Continue;
                }
            };
            let turn_index = bound.next_turn_index;
            let logical_turn_id = Uuid::new_v4().to_string();
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
                &fresh_decision,
                turn_request_id.clone(),
                None,
                false,
                &client_event,
                &provider_event,
                &context.trace_id,
                turn_index,
                &logical_turn_id,
                1,
            );
            let planned_lease =
                PlannedPoolKeyLeaseGuard::new(state, turn_decision.report_context.as_ref());
            let turn_start = spawn_owned_responses_websocket_turn(
                state.clone(),
                planning_parts,
                turn_control_decision.decision.clone(),
                turn_decision,
                client_event.clone(),
                planned_lease,
                bound.session_termination.clone(),
            );
            let mut turn = match await_owned_responses_websocket_turn(turn_start).await {
                Ok(turn) => turn,
                Err(error) => {
                    evict_failed_continuation(bound, &client_event);
                    warn!(
                        event_name = "responses_websocket_followup_turn_lifecycle_start_failed",
                        log_type = "ops",
                        transport = WEBSOCKET_LOG_TRANSPORT,
                        websocket = true,
                        trace_id = %context.trace_id,
                        error = ?error,
                        "gateway could not start Responses WebSocket follow-up usage/audit lifecycle"
                    );
                    send_responses_websocket_turn_start_error(
                        client_socket,
                        &bound.public_event_sequence,
                        &error,
                    )
                    .await;
                    return RelayDisposition::Continue;
                }
            };
            turn.set_provider_response_headers(bound.upstream_response_headers.clone());
            bound.turn_state.begin(
                LogicalTurn::authorized(
                    turn_request_id,
                    client_event.clone(),
                    turn_control_decision.decision,
                    turn_control_decision.auth_snapshot,
                    turn_index,
                    logical_turn_id,
                ),
                turn,
            );
            bound.public_event_sequence.reset();
            bound.public_event_state.reset();
            bound.next_turn_index = bound.next_turn_index.saturating_add(1);

            if !bound
                .turn_state
                .attempt()
                .is_some_and(|turn| turn.admission_is_healthy())
            {
                return RelayDisposition::ExecutionReservationLost;
            }
            match bound
                .backend_session
                .send_response_create(&provider_event)
                .await
            {
                Ok(()) => {
                    if let Some(session) = redaction_session {
                        bound.redaction_restorer.register(session);
                    }
                    if let Some(turn) = bound.turn_state.attempt_mut() {
                        turn.mark_upstream_request_sent();
                    }
                    RelayDisposition::Continue
                }
                Err(_) => {
                    let previous_response_id = bound
                        .turn_state
                        .logical()
                        .and_then(|logical| {
                            response_create_previous_response_id(&logical.client_event).ok()
                        })
                        .flatten()
                        .map(str::to_string);
                    evict_public_response_id(bound, previous_response_id.as_deref());
                    RelayDisposition::UpstreamError("responses_websocket_send_failed")
                }
            }
        }
        AxumWsMessage::Binary(_) => {
            reject_client_protocol_event(client_socket, bound, UNSUPPORTED_CLIENT_FRAME).await
        }
        AxumWsMessage::Ping(_) | AxumWsMessage::Pong(_) | AxumWsMessage::Close(_) => {
            unreachable!("client control frames are terminated before public event handling")
        }
    }
}

/// 重新规划一轮 `response.create`（换模型或独立轮）。
///
/// `planning_parts` 与 `client_event` 都由调用方准备：事件已经过请求侧脱敏，
/// Parts 携带这一轮的 `RedactionSessionSlot`，所以 planner 里的候选级脱敏对
/// 已脱敏内容是幂等的 no-op，上游请求体与审计 body 都保持脱敏态。
async fn forward_replanned_response_create(
    bound: &mut BoundResponsesConnection,
    client_socket: &mut WebSocket,
    state: &AppState,
    context: &WebSocketRequestContext,
    turn_request_id: String,
    planning_parts: http::request::Parts,
    turn_control_decision: &GatewayControlDecision,
    client_event: Value,
    requested_model: String,
    redaction_session: Option<crate::privacy::RedactionSession>,
    auth_snapshot: Option<crate::data::auth::GatewayAuthApiKeySnapshot>,
) -> RelayDisposition {
    let failed_continuation_id = response_create_previous_response_id(&client_event)
        .ok()
        .flatten()
        .map(str::to_string);
    let logical_turn_id = Uuid::new_v4().to_string();
    let now_unix_secs = current_unix_secs();
    let excluded_key_ids = bound.exhausted_exclusions.key_ids(now_unix_secs);
    let excluded_codex_account_ids = bound.exhausted_exclusions.codex_account_ids(now_unix_secs);
    let excluded_key_ids = (!excluded_key_ids.is_empty()).then_some(excluded_key_ids);
    let excluded_codex_account_ids =
        (!excluded_codex_account_ids.is_empty()).then_some(excluded_codex_account_ids);
    let planning = spawn_owned_responses_websocket_plan(
        state.clone(),
        planning_parts,
        turn_request_id.clone(),
        turn_control_decision.clone(),
        client_event.clone(),
        excluded_key_ids,
        excluded_codex_account_ids,
        auth_snapshot.clone(),
    );
    let owned_plan = match await_owned_responses_websocket_plan(planning).await {
        Ok(Some(decision)) => decision,
        Ok(None) => {
            evict_public_response_id(bound, failed_continuation_id.as_deref());
            send_gateway_error_with_status(
                client_socket,
                &bound.public_event_sequence,
                503,
                "responses_provider_unavailable",
                "No eligible WebSocket-enabled Responses provider is available for the requested model",
            )
            .await;
            return RelayDisposition::Continue;
        }
        Err(error) => {
            evict_public_response_id(bound, failed_continuation_id.as_deref());
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
            send_gateway_error_with_status(
                client_socket,
                &bound.public_event_sequence,
                503,
                "responses_provider_unavailable",
                "Gateway could not prepare the requested model",
            )
            .await;
            return RelayDisposition::Continue;
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
    // Provider observation is part of the physical binding contract. Reusing
    // the socket with a different observer would either leak provider-private
    // events or silently stop observing the metadata required by that target.
    let reuses_bound_upstream =
        provider_observers_allow_socket_reuse(bound.provider_observer, provider_observer)
            && decision_reuses_bound_upstream(
                bound,
                backend,
                &decision,
                &credential_binding_fingerprint,
            );
    if continuation_requires_same_upstream(&client_event, reuses_bound_upstream) {
        planned_lease.release().await;
        evict_public_response_id(bound, failed_continuation_id.as_deref());
        debug!(
            event_name = "responses_websocket_continuation_rebind_rejected",
            log_type = "event",
            transport = WEBSOCKET_LOG_TRANSPORT,
            websocket = true,
            trace_id = %context.trace_id,
            requested_model = %requested_model,
            previous_key_id = ?bound.decision_template.key_id,
            planned_key_id = ?decision.key_id,
            error_code = "previous_response_not_found",
            "gateway refused to move a Responses continuation to a different upstream account or connection"
        );
        send_previous_response_not_found(client_socket, &bound.public_event_sequence).await;
        return RelayDisposition::Continue;
    }
    let provider_event =
        match planned_response_create_event(&decision, &client_event).and_then(|event| {
            serde_json::from_str::<Value>(&event)
                .map_err(|_| "response_create_serialization_failed")
        }) {
            Ok(event) => event,
            Err(code) => {
                planned_lease.release().await;
                evict_public_response_id(bound, failed_continuation_id.as_deref());
                send_gateway_error(
                    client_socket,
                    &bound.public_event_sequence,
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
        turn_request_id.clone(),
        None,
        true,
        &client_event,
        &provider_event,
        &context.trace_id,
        turn_index,
        &logical_turn_id,
        1,
    );
    let turn_start = spawn_owned_responses_websocket_turn(
        state.clone(),
        planning_parts,
        turn_control_decision.clone(),
        turn_decision,
        client_event.clone(),
        planned_lease,
        bound.session_termination.clone(),
    );
    let mut turn = match await_owned_responses_websocket_turn(turn_start).await {
        Ok(turn) => turn,
        Err(error) => {
            evict_public_response_id(bound, failed_continuation_id.as_deref());
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
            send_responses_websocket_turn_start_error(
                client_socket,
                &bound.public_event_sequence,
                &error,
            )
            .await;
            return RelayDisposition::Continue;
        }
    };

    if reuses_bound_upstream {
        if !turn.admission_is_healthy() {
            evict_public_response_id(bound, failed_continuation_id.as_deref());
            queue_turn_finalization(
                bound,
                state,
                disarm_owned_turn(turn),
                ResponsesWebSocketTurnOutcome::execution_reservation_lost(),
            );
            return RelayDisposition::ExecutionReservationLost;
        }
        if bound
            .backend_session
            .send_response_create(&provider_event)
            .await
            .is_err()
        {
            evict_public_response_id(bound, failed_continuation_id.as_deref());
            queue_turn_finalization(
                bound,
                state,
                disarm_owned_turn(turn),
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
        // Observer kind equality is required by `reuses_bound_upstream`; keep
        // the binding's concrete observer synchronized with the fresh plan.
        bound.provider_observer = provider_observer;
        bound.decision_template = decision;
        bound.bound_candidate = bound_candidate;
        // The re-plan keeps this upstream but resolved a new model, so later
        // continuations must normalize against the new plan, not the old one.
        bound.body_normalization = normalization;
        bound.turn_state.begin(
            LogicalTurn::authorized(
                turn_request_id.clone(),
                client_event.clone(),
                turn_control_decision.clone(),
                auth_snapshot,
                turn_index,
                logical_turn_id.clone(),
            ),
            turn,
        );
        bound.public_event_sequence.reset();
        bound.public_event_state.reset();
        if let Some(session) = redaction_session {
            bound.redaction_restorer.register(session);
        }
        bound.next_turn_index = bound.next_turn_index.saturating_add(1);
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

    let mut replacement = match bind_responses_upstream(
        &decision,
        bound_candidate,
        credential_binding_fingerprint,
        normalization,
        &client_event,
        backend,
        provider_observer,
        || turn.admission_is_healthy(),
    )
    .await
    {
        Ok(connection) => connection,
        Err(ResponsesUpstreamBindError::ExecutionReservationLost) => {
            evict_public_response_id(bound, failed_continuation_id.as_deref());
            queue_turn_finalization(
                bound,
                state,
                disarm_owned_turn(turn),
                ResponsesWebSocketTurnOutcome::execution_reservation_lost(),
            );
            return RelayDisposition::ExecutionReservationLost;
        }
        Err(ResponsesUpstreamBindError::Upstream(code)) => {
            evict_public_response_id(bound, failed_continuation_id.as_deref());
            queue_turn_finalization(
                bound,
                state,
                disarm_owned_turn(turn),
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
            send_gateway_error_with_status(
                client_socket,
                &bound.public_event_sequence,
                502,
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
    bound
        .backend_session
        .replace_from(&mut replacement.backend_session);
    bound.backend = replacement.backend;
    bound.public_codec = replacement.public_codec;
    bound.provider_observer = replacement.provider_observer;
    bound.client_model = replacement.client_model;
    bound.provider_model = replacement.provider_model;
    bound.decision_template = replacement.decision_template;
    bound.bound_candidate = replacement.bound_candidate;
    bound.body_normalization = replacement.body_normalization;
    bound.binding_identity = replacement.binding_identity;
    bound.turn_state.begin(
        LogicalTurn::authorized(
            turn_request_id,
            client_event,
            turn_control_decision.clone(),
            auth_snapshot,
            turn_index,
            logical_turn_id,
        ),
        turn,
    );
    bound.public_event_sequence.reset();
    bound.public_event_state.reset();
    if let Some(session) = redaction_session {
        bound.redaction_restorer.register(session);
    }
    bound.next_turn_index = bound.next_turn_index.saturating_add(1);
    bound.upstream_response_headers = replacement.upstream_response_headers;
    bound.pending_provider_drain = replacement.pending_provider_drain;
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

pub(super) async fn consume_response_create_rate_limit(
    state: &AppState,
    decision: &GatewayControlDecision,
    client_ip: std::net::IpAddr,
    trace_id: &str,
) -> Result<bool, ()> {
    match state.admin_security_ip_whitelisted(client_ip).await {
        Ok(true) => return Ok(true),
        Ok(false) => {}
        Err(error) => {
            warn!(
                event_name = "responses_websocket_ip_whitelist_check_failed",
                log_type = "ops",
                transport = WEBSOCKET_LOG_TRANSPORT,
                websocket = true,
                trace_id,
                client_ip = %client_ip,
                error = ?error,
                "gateway continued with WebSocket rate limiting after IP whitelist check error"
            );
        }
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

#[cfg(test)]
mod tests {
    use axum::extract::ws::Message as AxumWsMessage;
    use axum::http::StatusCode;
    use serde_json::json;

    use super::{
        begin_local_error_response, client_control_frame_action, client_protocol_rejection_action,
        parse_public_response_create, provider_observers_allow_socket_reuse,
        response_create_access_check_event, validate_followup_previous_response_id,
        ClientControlFrameAction, ClientProtocolRejectionAction, PreviousResponseValidationError,
        INVALID_CLIENT_EVENT, UNSUPPORTED_CLIENT_EVENT, UNSUPPORTED_CLIENT_FRAME,
    };
    use crate::handlers::proxy::websocket::responses::adapter::resolve_responses_provider_observer;
    use crate::handlers::proxy::websocket::responses::adapter::ResponsesPublicEventState;
    use crate::handlers::proxy::websocket::responses::lifecycle::responses_websocket_turn_start_close;
    use crate::handlers::proxy::websocket::responses::state::ResponsesPublicEventSequence;
    use crate::handlers::proxy::websocket::session::{
        CLOSE_INTERNAL_ERROR, CLOSE_POLICY_VIOLATION,
    };
    use crate::orchestration::ResponsesProviderObserverKind;
    use crate::GatewayError;

    #[test]
    fn public_client_protocol_accepts_only_response_create_json() {
        let event = parse_public_response_create(
            r#"{"type":"response.create","model":"public-model","input":[],"future_responses_option":{"enabled":true}}"#,
        )
        .expect("response.create should be accepted");

        assert_eq!(event["type"], "response.create");
        assert_eq!(event["future_responses_option"], json!({"enabled": true}));
    }

    #[test]
    fn followup_public_codec_rejects_provider_private_request_fields() {
        let error = parse_public_response_create(
            r#"{"type":"response.create","model":"public-model","client_metadata":{"thread_id":"private"}}"#,
        )
        .expect_err("provider-private request fields must not reach follow-up planning");

        assert_eq!(error.code, "provider_private_response_create_field");
        assert_eq!(error.param, Some("client_metadata"));
    }

    #[test]
    fn followup_public_codec_rejects_multi_agent_beta() {
        let error = parse_public_response_create(
            r#"{"type":"response.create","model":"public-model","multi_agent":{"enabled":true}}"#,
        )
        .expect_err("multi-agent beta must not enter the stable follow-up path");

        assert_eq!(error.code, "unsupported_response_create_field");
        assert_eq!(error.param, Some("multi_agent"));

        let error = parse_public_response_create(
            r#"{"type":"response.create","model":"public-model","input":[{"type":"message","caller":{"type":"multi_agent"}}]}"#,
        )
        .expect_err("multi-agent callers must not enter the stable follow-up path");

        assert_eq!(error.code, "invalid_response_create_field");
        assert_eq!(error.param, Some("input"));
    }

    #[test]
    fn followup_previous_response_id_accepts_only_the_latest_id_on_this_connection() {
        let current = serde_json::json!({"previous_response_id": "resp_current"});
        let old = serde_json::json!({"previous_response_id": "resp_old"});

        assert_eq!(
            validate_followup_previous_response_id(&current, Some("resp_current")),
            Ok(Some("resp_current"))
        );
        assert_eq!(
            validate_followup_previous_response_id(&old, Some("resp_current")),
            Err(PreviousResponseValidationError::NotOwned)
        );
        assert_eq!(
            validate_followup_previous_response_id(&current, None),
            Err(PreviousResponseValidationError::NotOwned)
        );
    }

    #[test]
    fn followup_previous_response_id_rejects_invalid_types_and_empty_strings() {
        for event in [
            serde_json::json!({"previous_response_id": 42}),
            serde_json::json!({"previous_response_id": "  "}),
        ] {
            assert_eq!(
                validate_followup_previous_response_id(&event, Some("resp_current")),
                Err(PreviousResponseValidationError::Invalid)
            );
        }
    }

    #[test]
    fn invalid_json_is_a_gateway_protocol_error() {
        assert_eq!(
            parse_public_response_create("not-json"),
            Err(INVALID_CLIENT_EVENT)
        );
    }

    #[test]
    fn followup_local_error_starts_a_fresh_public_sequence_and_terminal_state() {
        let sequence = ResponsesPublicEventSequence::default();
        let mut state = ResponsesPublicEventState::default();
        state
            .accept_local_terminal_error()
            .expect("previous response terminal");
        assert_eq!(sequence.next(), 0);
        assert_eq!(sequence.next(), 1);

        begin_local_error_response(&sequence, &mut state);

        assert!(matches!(
            state,
            ResponsesPublicEventState::Terminal { response_id: None }
        ));
        assert_eq!(sequence.next(), 0);
    }

    #[test]
    fn provider_private_and_unknown_events_are_not_public_client_events() {
        for event in [
            r#"{"type":"codex.rate_limits"}"#,
            r#"{"type":"response.cancel"}"#,
            r#"{"type":"unknown.control"}"#,
            r#"{"type":42}"#,
            r#"[]"#,
        ] {
            assert_eq!(
                parse_public_response_create(event),
                Err(UNSUPPORTED_CLIENT_EVENT),
                "event should be rejected: {event}"
            );
        }
    }

    #[test]
    fn binary_frames_have_a_stable_public_protocol_error() {
        assert_eq!(UNSUPPORTED_CLIENT_FRAME.code, "unsupported_client_frame");
        assert!(UNSUPPORTED_CLIENT_FRAME.message.contains("JSON text"));
    }

    #[test]
    fn client_protocol_errors_only_terminate_an_active_turn() {
        assert_eq!(
            client_protocol_rejection_action(false),
            ClientProtocolRejectionAction::Continue
        );
        assert_eq!(
            client_protocol_rejection_action(true),
            ClientProtocolRejectionAction::TerminateActiveTurn
        );
    }

    #[test]
    fn client_control_frames_terminate_at_the_public_socket() {
        let ping_payload = axum::body::Bytes::from_static(b"private-client-ping");
        assert_eq!(
            client_control_frame_action(&AxumWsMessage::Ping(ping_payload.clone())),
            Some(ClientControlFrameAction::ReplyPong(ping_payload))
        );
        assert_eq!(
            client_control_frame_action(&AxumWsMessage::Pong(axum::body::Bytes::from_static(
                b"private-client-pong"
            ))),
            Some(ClientControlFrameAction::Consume)
        );
        assert_eq!(
            client_control_frame_action(&AxumWsMessage::Close(Some(
                axum::extract::ws::CloseFrame {
                    code: 4321,
                    reason: "private-client-close-reason".into(),
                }
            ))),
            Some(ClientControlFrameAction::CloseUpstream)
        );
    }

    #[test]
    fn omitted_followup_model_is_injected_into_the_per_turn_access_check() {
        for event in [
            serde_json::json!({"type": "response.create", "input": "independent"}),
            serde_json::json!({
                "type": "response.create",
                "previous_response_id": "resp_1",
                "input": "continuation"
            }),
        ] {
            let checked = response_create_access_check_event(&event, "bound-public-model")
                .expect("omitted model should inherit the bound public model");

            assert_eq!(checked["model"], "bound-public-model");
            assert!(
                event.get("model").is_none(),
                "wire event must stay unchanged"
            );
        }
    }

    #[test]
    fn explicit_followup_model_remains_the_access_check_model() {
        let event = serde_json::json!({
            "type": "response.create",
            "model": "requested-model",
            "input": "turn"
        });

        let checked = response_create_access_check_event(&event, "bound-public-model")
            .expect("explicit model should be valid");

        assert_eq!(checked["model"], "requested-model");
    }

    #[test]
    fn a_physical_socket_is_not_reused_across_provider_observer_kinds() {
        let standard = resolve_responses_provider_observer(ResponsesProviderObserverKind::Standard);
        let codex = resolve_responses_provider_observer(ResponsesProviderObserverKind::Codex);

        assert!(provider_observers_allow_socket_reuse(standard, standard));
        assert!(provider_observers_allow_socket_reuse(codex, codex));
        assert!(!provider_observers_allow_socket_reuse(standard, codex));
        assert!(!provider_observers_allow_socket_reuse(codex, standard));
    }

    #[test]
    fn turn_auth_refresh_failures_map_to_terminal_close_codes() {
        let revoked = GatewayError::Client {
            status: StatusCode::UNAUTHORIZED,
            message: "API key revoked".to_string(),
        };
        let control_unavailable = GatewayError::ControlUnavailable {
            trace_id: "trace-refresh".to_string(),
            message: "strong auth refresh failed".to_string(),
        };

        assert_eq!(
            responses_websocket_turn_start_close(&revoked).0,
            CLOSE_POLICY_VIOLATION
        );
        assert_eq!(
            responses_websocket_turn_start_close(&control_unavailable).0,
            CLOSE_INTERNAL_ERROR
        );
    }
}
