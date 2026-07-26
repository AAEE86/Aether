//! Physical upstream WebSocket binding and transport helpers.

use serde_json::Value;
use wreq::ws::message::Message as WreqWsMessage;

use super::adapter::ResponsesWebSocketProtocolAdapter;
use super::binding::{UpstreamBindingIdentity, UpstreamBindingIdentityError};
use super::request::planned_response_create_event;
use super::state::{BoundResponsesConnection, ExhaustedResponsesWebSocketExclusions};
use crate::ai_serving::{AiExecutionDecision, ResponsesWebSocketBodyNormalization};
use crate::handlers::proxy::websocket::session::RESPONSES_WEBSOCKET_SESSION_LIMITS;
use crate::handlers::proxy::websocket::transport::{
    close_upstream_socket, connect_upstream_websocket, send_upstream_message,
};

pub(super) async fn bind_responses_upstream(
    decision: &AiExecutionDecision,
    normalization: ResponsesWebSocketBodyNormalization,
    initial_event: &Value,
    adapter: &'static dyn ResponsesWebSocketProtocolAdapter,
) -> Result<BoundResponsesConnection, &'static str> {
    let binding_identity =
        UpstreamBindingIdentity::from_decision(adapter, decision).map_err(|error| match error {
            UpstreamBindingIdentityError::MissingUpstreamUrl => {
                adapter.upstream_errors().upstream_url_missing
            }
            UpstreamBindingIdentityError::InvalidUpstreamUrl => {
                adapter.upstream_errors().upstream_url_invalid
            }
            UpstreamBindingIdentityError::InvalidHandshakeHeaders => {
                adapter.upstream_errors().headers_invalid
            }
        })?;
    let mut upstream = connect_upstream_websocket(
        decision,
        RESPONSES_WEBSOCKET_SESSION_LIMITS,
        adapter.upstream_errors(),
    )
    .await?;
    let first_event = planned_response_create_event(decision, initial_event)?;
    send_upstream_message(&mut upstream.socket, WreqWsMessage::text(first_event))
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
        body_normalization: normalization,
        binding_identity,
        active_turn: None,
        active_response_create: None,
        next_turn_index: 2,
        upstream_response_headers: upstream.response_headers,
        pending_adapter_drain: None,
        pending_adapter_observation: None,
        exhausted_exclusions: ExhaustedResponsesWebSocketExclusions::default(),
        pending_turn_finalization: None,
    })
}

pub(super) async fn receive_optional_upstream(
    upstream: &mut Option<wreq::ws::WebSocket>,
) -> Option<Result<WreqWsMessage, ()>> {
    match upstream.as_mut() {
        Some(upstream) => upstream.recv().await.map(|message| message.map_err(|_| ())),
        None => std::future::pending().await,
    }
}

pub(super) async fn close_bound_upstream(bound: &mut BoundResponsesConnection) {
    if let Some(mut upstream) = bound.upstream.take() {
        close_upstream_socket(&mut upstream, None).await;
    }
}

pub(super) fn decision_reuses_bound_upstream(
    bound: &BoundResponsesConnection,
    adapter: &'static dyn ResponsesWebSocketProtocolAdapter,
    decision: &AiExecutionDecision,
) -> bool {
    bound.upstream.is_some()
        && UpstreamBindingIdentity::from_decision(adapter, decision)
            .map(|identity| bound.binding_identity == identity)
            .unwrap_or(false)
}
