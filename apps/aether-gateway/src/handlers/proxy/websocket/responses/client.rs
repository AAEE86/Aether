//! Client-side Responses WebSocket event forwarding and follow-up planning.

use axum::body::Bytes;
use axum::extract::ws::{Message as AxumWsMessage, WebSocket};
use futures_util::SinkExt;
use serde_json::Value;
use uuid::Uuid;
use wreq::ws::message::Message as WreqWsMessage;

use super::adapter::{resolve_responses_websocket_adapter, ResponsesWebSocketDrainDirective};
use super::lifecycle::{
    await_pending_turn_finalization, queue_turn_finalization,
    send_responses_websocket_turn_start_error, ActiveResponsesWebSocketTurn,
};
use super::quota::{mark_active_response_retry_unsafe, send_previous_response_not_found};
use super::redaction::redact_responses_websocket_client_event;
use super::request::{
    build_planning_parts, changed_followup_response_create_model,
    continuation_requires_same_upstream, normalize_followup_response_create,
    planned_response_create_event, provider_model_from_decision,
    response_create_has_previous_response_id, response_create_model_or_current,
};
use super::state::{ActiveResponsesWebSocketRequest, BoundResponsesConnection};
use super::turn::{
    begin_responses_websocket_turn, prepare_responses_websocket_turn_decision,
    ResponsesWebSocketTurnObservation, ResponsesWebSocketTurnOutcome,
};
use super::upstream::{bind_responses_upstream, decision_reuses_bound_upstream};
use crate::ai_serving::maybe_build_responses_websocket_decision;
use crate::clock::current_unix_secs;
use crate::control::{request_model_local_rejection, GatewayControlDecision};
use crate::handlers::proxy::websocket::ingress::WebSocketRequestContext;
use crate::handlers::proxy::websocket::session::{CLOSE_INTERNAL_ERROR, WEBSOCKET_LOG_TRANSPORT};
use crate::handlers::proxy::websocket::transport::{
    client_close_to_upstream, close_client_socket, close_upstream_socket, send_client_message,
    send_gateway_error, send_gateway_error_with_status, send_upstream_message,
};
use crate::orchestration::release_pool_key_lease_from_report_context;
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
    UpstreamError(&'static str),
}

pub(super) fn adapter_drain_ready(
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

pub(super) async fn forward_client_message(
    client_message: AxumWsMessage,
    bound: &mut BoundResponsesConnection,
    client_socket: &mut WebSocket,
    state: &AppState,
    context: &WebSocketRequestContext,
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
                if bound.upstream.is_none() {
                    send_gateway_error(
                        client_socket,
                        "responses_websocket_upstream_rebind_required",
                        "Send a new response.create to select another Provider connection",
                    )
                    .await;
                    return RelayDisposition::Continue;
                }
                // We cannot reconstruct arbitrary Responses control events on
                // a replacement socket.  A concurrent quota error must be
                // surfaced rather than replaying only the response.create.
                mark_active_response_retry_unsafe(bound, "client_control_event");
                return send_upstream_message(
                    bound
                        .upstream
                        .as_mut()
                        .expect("upstream presence was checked above"),
                    WreqWsMessage::text(text),
                )
                .await
                .map(|()| RelayDisposition::Continue)
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

            // A prior terminal turn may still be writing usage/audit and
            // projecting provider effects. Do not let a new independent turn
            // plan against stale health, adaptive, or pool state.
            await_pending_turn_finalization(bound).await;

            match consume_response_create_rate_limit(state, &context.decision, context.rpm_bypassed)
                .await
            {
                Ok(true) => {}
                Ok(false) => {
                    send_gateway_error_with_status(
                        client_socket,
                        429,
                        "rate_limit_exceeded",
                        "Too many response.create events; retry later",
                    )
                    .await;
                    return RelayDisposition::Continue;
                }
                Err(()) => {
                    send_gateway_error_with_status(
                        client_socket,
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

            let Some(client_event) = client_event else {
                send_gateway_error(
                    client_socket,
                    "invalid_response_create",
                    "response.create must be valid JSON",
                )
                .await;
                return RelayDisposition::Continue;
            };
            // 这一轮的 planning Parts 只构造一次（它携带 per-turn 的
            // RedactionSessionSlot），并且客户端事件也只在这里脱敏一次：
            // 复用已绑定 upstream 的 continuation 根本不进 planner，只靠 planner
            // 内部脱敏拦不住它。之后 re-plan / continuation / 配额重试都只看脱敏
            // 后的事件，上游请求体与审计 original_request_body 因此一致。
            let planning_parts = build_planning_parts(context);
            let redacted_client_event = redact_responses_websocket_client_event(
                state,
                &planning_parts,
                &context.decision,
                &client_event,
            )
            .await;
            let client_event = match redacted_client_event {
                Ok(Some(redacted)) => redacted,
                Ok(None) => client_event,
                Err(error) => {
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
            if bound.upstream.is_none() {
                if response_create_has_previous_response_id(&client_event) {
                    send_previous_response_not_found(client_socket).await;
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
                    &planning_parts,
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
                    &planning_parts,
                    client_event,
                    requested_model,
                )
                .await;
            }
            if !response_create_has_previous_response_id(&client_event) {
                return forward_replanned_response_create(
                    bound,
                    client_socket,
                    state,
                    context,
                    &planning_parts,
                    client_event,
                    bound.client_model.clone(),
                )
                .await;
            }

            let outbound = match normalize_followup_response_create(
                &client_event,
                &bound.provider_model,
                &bound.body_normalization,
            ) {
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
            let turn_request_id = Uuid::new_v4().to_string();
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
                &bound.decision_template,
                turn_request_id,
                false,
                &client_event,
                &provider_event,
                &context.trace_id,
                turn_index,
                &logical_turn_id,
                1,
            );
            let mut turn = match begin_responses_websocket_turn(
                state,
                &planning_parts,
                &context.decision,
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
                    send_responses_websocket_turn_start_error(client_socket, &error).await;
                    return RelayDisposition::Continue;
                }
            };
            turn.set_provider_response_headers(bound.upstream_response_headers.clone());
            bound.active_turn = Some(ActiveResponsesWebSocketTurn::new(state, turn));
            bound.active_response_create = Some(ActiveResponsesWebSocketRequest::new(
                client_event.clone(),
                turn_index,
                logical_turn_id,
            ));
            bound.next_turn_index = bound.next_turn_index.saturating_add(1);
            bound.response_in_flight = true;

            let Some(upstream) = bound.upstream.as_mut() else {
                return RelayDisposition::UpstreamError("responses_websocket_send_failed");
            };
            match send_upstream_message(upstream, WreqWsMessage::text(outbound)).await {
                Ok(()) => {
                    if let Some(turn) = bound.active_turn.as_mut() {
                        turn.mark_upstream_request_sent();
                    }
                    RelayDisposition::Continue
                }
                Err(_) => RelayDisposition::UpstreamError("responses_websocket_send_failed"),
            }
        }
        AxumWsMessage::Binary(data) => {
            if bound.upstream.is_some() {
                mark_active_response_retry_unsafe(bound, "client_binary_frame");
                send_upstream_message(
                    bound
                        .upstream
                        .as_mut()
                        .expect("upstream presence was checked above"),
                    WreqWsMessage::Binary(data),
                )
                .await
                .map(|()| RelayDisposition::Continue)
                .unwrap_or(RelayDisposition::UpstreamError(
                    "responses_websocket_send_failed",
                ))
            } else {
                send_gateway_error(
                    client_socket,
                    "responses_websocket_upstream_rebind_required",
                    "Send a new response.create to select another Provider connection",
                )
                .await;
                RelayDisposition::Continue
            }
        }
        AxumWsMessage::Ping(data) => match bound.upstream.as_mut() {
            Some(upstream) => send_upstream_message(upstream, WreqWsMessage::Ping(data))
                .await
                .map(|()| RelayDisposition::Continue)
                .unwrap_or(RelayDisposition::UpstreamError(
                    "responses_websocket_send_failed",
                )),
            None => send_client_message(client_socket, AxumWsMessage::Pong(data))
                .await
                .map(|()| RelayDisposition::Continue)
                .unwrap_or(RelayDisposition::Close),
        },
        AxumWsMessage::Pong(data) => match bound.upstream.as_mut() {
            Some(upstream) => send_upstream_message(upstream, WreqWsMessage::Pong(data))
                .await
                .map(|()| RelayDisposition::Continue)
                .unwrap_or(RelayDisposition::UpstreamError(
                    "responses_websocket_send_failed",
                )),
            None => RelayDisposition::Continue,
        },
        AxumWsMessage::Close(frame) => {
            if let Some(upstream) = bound.upstream.as_mut() {
                close_upstream_socket(upstream, client_close_to_upstream(frame)).await;
            }
            RelayDisposition::Close
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
    planning_parts: &http::request::Parts,
    client_event: Value,
    requested_model: String,
) -> RelayDisposition {
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
    let logical_turn_id = Uuid::new_v4().to_string();
    let now_unix_secs = current_unix_secs();
    let excluded_key_ids = bound.exhausted_exclusions.key_ids(now_unix_secs);
    let excluded_codex_account_ids = bound.exhausted_exclusions.codex_account_ids(now_unix_secs);
    let excluded_key_ids = (!excluded_key_ids.is_empty()).then_some(&excluded_key_ids);
    let excluded_codex_account_ids =
        (!excluded_codex_account_ids.is_empty()).then_some(&excluded_codex_account_ids);
    let planned = match maybe_build_responses_websocket_decision(
        state,
        planning_parts,
        &turn_request_id,
        &context.decision,
        &client_event,
        excluded_key_ids,
        excluded_codex_account_ids,
    )
    .await
    {
        Ok(Some(decision)) => decision,
        Ok(None) => {
            send_gateway_error_with_status(
                client_socket,
                503,
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
            send_gateway_error_with_status(
                client_socket,
                503,
                "responses_provider_unavailable",
                "Gateway could not prepare the requested model",
            )
            .await;
            return RelayDisposition::Continue;
        }
    };
    let adapter = resolve_responses_websocket_adapter(planned.adapter);
    let normalization = planned.normalization;
    let decision = planned.execution;
    let reuses_bound_upstream = decision_reuses_bound_upstream(bound, adapter, &decision);
    if continuation_requires_same_upstream(&client_event, reuses_bound_upstream) {
        release_pool_key_lease_from_report_context(state, decision.report_context.as_ref()).await;
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
        send_previous_response_not_found(client_socket).await;
        return RelayDisposition::Continue;
    }
    let provider_event =
        match planned_response_create_event(&decision, &client_event).and_then(|event| {
            serde_json::from_str::<Value>(&event)
                .map_err(|_| "response_create_serialization_failed")
        }) {
            Ok(event) => event,
            Err(code) => {
                release_pool_key_lease_from_report_context(state, decision.report_context.as_ref())
                    .await;
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
        &logical_turn_id,
        1,
    );
    let mut turn = match begin_responses_websocket_turn(
        state,
        planning_parts,
        &context.decision,
        turn_decision,
        &client_event,
    )
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
            send_responses_websocket_turn_start_error(client_socket, &error).await;
            return RelayDisposition::Continue;
        }
    };

    if reuses_bound_upstream {
        let outbound = match serde_json::to_string(&provider_event) {
            Ok(outbound) => outbound,
            Err(_) => {
                queue_turn_finalization(
                    bound,
                    state,
                    turn,
                    ResponsesWebSocketTurnOutcome::upstream_send_failed(),
                )
                .await;
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
            queue_turn_finalization(
                bound,
                state,
                turn,
                ResponsesWebSocketTurnOutcome::upstream_send_failed(),
            )
            .await;
            return RelayDisposition::UpstreamError("responses_websocket_send_failed");
        };
        if send_upstream_message(upstream, WreqWsMessage::text(outbound))
            .await
            .is_err()
        {
            queue_turn_finalization(
                bound,
                state,
                turn,
                ResponsesWebSocketTurnOutcome::upstream_send_failed(),
            )
            .await;
            return RelayDisposition::UpstreamError("responses_websocket_send_failed");
        }

        turn.mark_upstream_request_sent();
        turn.set_provider_response_headers(bound.upstream_response_headers.clone());
        let provider_model =
            provider_model_from_decision(&decision).unwrap_or_else(|| bound.provider_model.clone());
        let previous_client_model = std::mem::replace(&mut bound.client_model, requested_model);
        let previous_provider_model = std::mem::replace(&mut bound.provider_model, provider_model);
        bound.decision_template = decision;
        // The re-plan keeps this upstream but resolved a new model, so later
        // continuations must normalize against the new plan, not the old one.
        bound.body_normalization = normalization;
        bound.active_turn = Some(ActiveResponsesWebSocketTurn::new(state, turn));
        bound.active_response_create = Some(ActiveResponsesWebSocketRequest::new(
            client_event.clone(),
            turn_index,
            logical_turn_id.clone(),
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

    let mut replacement =
        match bind_responses_upstream(&decision, normalization, &client_event, adapter).await {
            Ok(connection) => connection,
            Err(code) => {
                queue_turn_finalization(
                    bound,
                    state,
                    turn,
                    ResponsesWebSocketTurnOutcome::upstream_connect_failed(code),
                )
                .await;
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
    let replacement_upstream = replacement
        .upstream
        .take()
        .expect("newly bound Responses upstream should be present");
    if let Some(mut previous_upstream) = bound.upstream.replace(replacement_upstream) {
        close_upstream_socket(&mut previous_upstream, None).await;
    }
    bound.adapter = replacement.adapter;
    bound.client_model = replacement.client_model;
    bound.provider_model = replacement.provider_model;
    bound.response_in_flight = true;
    bound.decision_template = replacement.decision_template;
    bound.body_normalization = replacement.body_normalization;
    bound.binding_identity = replacement.binding_identity;
    bound.active_turn = Some(ActiveResponsesWebSocketTurn::new(state, turn));
    bound.active_response_create = Some(ActiveResponsesWebSocketRequest::new(
        client_event,
        turn_index,
        logical_turn_id,
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

pub(super) async fn consume_response_create_rate_limit(
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
