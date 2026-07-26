//! Standard OpenAI Responses WebSocket session engine.
//!
//! An incoming client socket is authenticated at Upgrade time. Its first
//! `response.create` selects a provider through the normal Responses planner.
//! Later turns reuse that upstream while the requested model remains eligible
//! on the selected key. A model change is planned again and keeps the current
//! upstream when the planner resolves to the same target; an independent
//! request may replace it, but a continuation must stay on the original
//! connection and account.

use axum::body::Bytes;
use axum::extract::ws::{Message as AxumWsMessage, WebSocket};
use futures_util::{SinkExt, StreamExt};
use serde_json::Value;
use tokio::time::timeout;
use uuid::Uuid;

use super::adapter::resolve_responses_websocket_adapter;
use super::client::consume_response_create_rate_limit;
use super::connection::relay_bound_connection;
use super::lifecycle::{
    await_pending_adapter_observation, await_pending_turn_finalization,
    await_turn_finalization_handle, finalize_unbound_turn, responses_websocket_turn_start_close,
    send_responses_websocket_turn_start_error,
};
use super::request::{build_planning_parts, planned_response_create_event};
use super::state::ActiveResponsesWebSocketRequest;
use super::turn::{
    begin_responses_websocket_turn, prepare_responses_websocket_turn_decision,
    ResponsesWebSocketTurnOutcome,
};
use super::upstream::bind_responses_upstream;

use crate::ai_serving::maybe_build_responses_websocket_decision;
use crate::control::request_model_local_rejection;
use crate::handlers::proxy::websocket::ingress::{
    WebSocketConnectionLog, WebSocketConnectionLogSpec, WebSocketRequestContext,
};
use crate::handlers::proxy::websocket::session::{
    CLOSE_INTERNAL_ERROR, CLOSE_POLICY_VIOLATION, CLOSE_TRY_AGAIN,
    RESPONSES_WEBSOCKET_SESSION_LIMITS, WEBSOCKET_LOG_TRANSPORT,
};
use crate::handlers::proxy::websocket::transport::{
    close_client_socket, send_client_message, send_gateway_error, send_gateway_error_with_status,
};
use crate::orchestration::release_pool_key_lease_from_report_context;
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
    mut context: WebSocketRequestContext,
) {
    let connection_permit = context.websocket_connection_permit.take();
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
            send_gateway_error_with_status(
                &mut client_socket,
                429,
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
            send_gateway_error_with_status(
                &mut client_socket,
                503,
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
        None,
    )
    .await
    {
        Ok(Some(decision)) => decision,
        Ok(None) => {
            send_gateway_error_with_status(
                &mut client_socket,
                503,
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
            send_gateway_error_with_status(
                &mut client_socket,
                503,
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
    let normalization = planned.normalization;
    let decision = planned.execution;
    let first_provider_event = match planned_response_create_event(&decision, &first_event)
        .and_then(|event| {
            serde_json::from_str::<Value>(&event).map_err(|_| "responses_websocket_request_invalid")
        }) {
        Ok(event) => event,
        Err(code) => {
            release_pool_key_lease_from_report_context(&state, decision.report_context.as_ref())
                .await;
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
    let first_logical_turn_id = Uuid::new_v4().to_string();
    let first_turn_decision = prepare_responses_websocket_turn_decision(
        &decision,
        context.trace_id.clone(),
        true,
        &first_event,
        &first_provider_event,
        &context.trace_id,
        1,
        &first_logical_turn_id,
        1,
    );
    let mut first_turn = match begin_responses_websocket_turn(
        &state,
        &planning_parts,
        &context.decision,
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
            send_responses_websocket_turn_start_error(&mut client_socket, &error).await;
            let (close_code, close_reason) = responses_websocket_turn_start_close(&error);
            close_client_socket(&mut client_socket, close_code, close_reason).await;
            return;
        }
    };

    let mut bound =
        match bind_responses_upstream(&decision, normalization, &first_event, adapter).await {
            Ok(connection) => connection,
            Err(code) => {
                let finalizer = finalize_unbound_turn(
                    state.clone(),
                    first_turn,
                    ResponsesWebSocketTurnOutcome::upstream_connect_failed(code),
                )
                .await;
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
                    &mut client_socket,
                    502,
                    code,
                    "Gateway could not establish the Provider connection",
                )
                .await;
                close_client_socket(&mut client_socket, CLOSE_TRY_AGAIN, code).await;
                await_turn_finalization_handle(finalizer).await;
                return;
            }
        };
    first_turn.mark_upstream_request_sent();
    first_turn.set_provider_response_headers(bound.upstream_response_headers.clone());
    bound.active_turn = Some(first_turn);
    bound.active_response_create = Some(ActiveResponsesWebSocketRequest::new(
        first_event,
        1,
        first_logical_turn_id,
    ));

    relay_bound_connection(
        &mut client_socket,
        &mut bound,
        &state,
        &context,
        connection_permit,
    )
    .await;
    await_pending_turn_finalization(&mut bound).await;
    await_pending_adapter_observation(&mut bound).await;
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
                send_client_message(client_socket, AxumWsMessage::Pong(payload))
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

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;
    use std::sync::Arc;
    use std::time::{Duration, Instant};

    use super::super::adapter::{
        resolve_responses_websocket_adapter, ResponsesWebSocketDrainDirective,
    };
    use super::super::binding::UpstreamBindingIdentity;
    use super::super::client::adapter_drain_ready;
    use super::super::quota::{
        active_continuation_can_retry_from_full_input, is_usage_limit_error_event,
        observe_active_response_rebind_safety, record_exhausted_bound_key,
        should_request_full_continuation_retry,
    };
    use super::super::request::{
        changed_followup_response_create_model, continuation_requires_same_upstream,
        normalize_followup_response_create, planned_response_create_event,
        response_create_model_or_current,
    };
    use super::super::state::{
        ActiveResponsesWebSocketRequest, BoundResponsesConnection,
        ExhaustedResponsesWebSocketExclusions,
    };
    use super::super::turn::{
        ResponsesWebSocketTurnDeadline, ResponsesWebSocketTurnObservation,
        ResponsesWebSocketTurnOutcome, ResponsesWebSocketTurnTimeoutPhase,
    };
    use super::super::upstream::bind_responses_upstream;
    use crate::ai_serving::{AiExecutionDecision, ResponsesWebSocketBodyNormalization};
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
            retry_exclusion_until_unix_secs: None,
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
    fn quota_error_can_request_a_full_retry_only_before_public_response_state() {
        let mut bound = sample_bound_for_rebind_safety();
        bound.active_response_create = Some(ActiveResponsesWebSocketRequest::new(
            json!({
                "type": "response.create",
                "previous_response_id": "resp-previous",
            }),
            2,
            "logical-turn".to_string(),
        ));

        assert!(active_continuation_can_retry_from_full_input(&bound));
        bound
            .active_response_create
            .as_mut()
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
        bound.active_response_create = Some(ActiveResponsesWebSocketRequest::new(
            json!({
                "type": "response.create",
                "previous_response_id": "resp-previous",
            }),
            2,
            "logical-turn".to_string(),
        ));

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
        let mut request = ActiveResponsesWebSocketRequest::new(
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

        let mut retried = ActiveResponsesWebSocketRequest::new(
            json!({"type": "response.create", "model": "gpt-5.6-sol"}),
            2,
            "logical-turn".to_string(),
        );
        retried.retry_attempted = true;
        assert_eq!(
            retried.quota_retry_block_reason(),
            Some("quota_retry_already_attempted")
        );

        let mut client_control = ActiveResponsesWebSocketRequest::new(
            json!({"type": "response.create", "model": "gpt-5.6-sol"}),
            2,
            "logical-turn".to_string(),
        );
        client_control.mark_retry_unsafe("client_control_event");
        assert_eq!(
            client_control.quota_retry_block_reason(),
            Some("client_control_event")
        );

        let continuation = ActiveResponsesWebSocketRequest::new(
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
    fn adapter_safety_contract_controls_transparent_rebind_eligibility() {
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
                .active_response_create
                .as_ref()
                .and_then(ActiveResponsesWebSocketRequest::quota_retry_block_reason),
            None
        );

        observe_active_response_rebind_safety(&mut bound, &json!({"type": "response.created"}));
        assert_eq!(
            bound
                .active_response_create
                .as_ref()
                .and_then(ActiveResponsesWebSocketRequest::quota_retry_block_reason),
            Some("standard_response_event")
        );

        let mut unknown = sample_bound_for_rebind_safety();
        observe_active_response_rebind_safety(&mut unknown, &json!({"type": "codex.unknown"}));
        assert_eq!(
            unknown
                .active_response_create
                .as_ref()
                .and_then(ActiveResponsesWebSocketRequest::quota_retry_block_reason),
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
        ]);
        decision.provider_request_body = Some(json!({
            "model": "provider-model",
            "input": "hello",
            "stream": true,
            "background": true,
        }));

        let mut bound = bind_responses_upstream(
            &decision,
            ResponsesWebSocketBodyNormalization::for_tests("provider-model"),
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

    fn sample_bound_for_rebind_safety() -> BoundResponsesConnection {
        let adapter = resolve_responses_websocket_adapter(
            crate::orchestration::ResponsesWebSocketAdapter::Codex,
        );
        let decision = sample_decision();
        let binding_identity = UpstreamBindingIdentity::from_decision(adapter, &decision).unwrap();
        BoundResponsesConnection {
            upstream: None,
            adapter,
            client_model: "gpt-5.6-sol".to_string(),
            provider_model: "gpt-5.6-sol".to_string(),
            response_in_flight: true,
            decision_template: decision,
            body_normalization: ResponsesWebSocketBodyNormalization::for_tests("gpt-5.6-sol"),
            binding_identity,
            active_turn: None,
            active_response_create: Some(ActiveResponsesWebSocketRequest::new(
                json!({"type": "response.create", "model": "gpt-5.6-sol"}),
                1,
                "logical-turn".to_string(),
            )),
            next_turn_index: 2,
            upstream_response_headers: BTreeMap::new(),
            pending_adapter_drain: None,
            pending_adapter_observation: None,
            exhausted_exclusions: ExhaustedResponsesWebSocketExclusions::default(),
            pending_turn_finalization: None,
        }
    }
}
