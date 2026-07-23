//! Responses WebSocket request normalization and model-selection helpers.
//!
//! These functions translate client protocol events into the HTTP-shaped
//! planning input and provider `response.create` events. They deliberately do
//! not depend on connection state or perform I/O.

use axum::http::header::{AUTHORIZATION, CONNECTION, CONTENT_TYPE, UPGRADE};
use axum::http::Method;
use serde_json::Value;

use crate::ai_serving::AiExecutionDecision;
use crate::handlers::proxy::websocket::ingress::WebSocketRequestContext;
use crate::headers::request_origin_from_headers_and_remote_addr;

pub(super) fn build_planning_parts(context: &WebSocketRequestContext) -> http::request::Parts {
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

pub(super) fn planned_response_create_event(
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
    // These fields are WebSocket protocol state, not ordinary HTTP body
    // options. Provider request normalization may omit them, but moving or
    // dropping either one changes the meaning of the client session.
    for field in ["previous_response_id", "generate"] {
        if let Some(value) = fallback.get(field) {
            if value.is_null() {
                object.remove(field);
            } else {
                object.insert(field.to_string(), value.clone());
            }
        }
    }
    object.remove("stream");
    object.remove("background");
    serde_json::to_string(&event).map_err(|_| "responses_websocket_request_invalid")
}

pub(super) fn response_create_has_previous_response_id(event: &Value) -> bool {
    event
        .get("previous_response_id")
        .is_some_and(|value| !value.is_null())
}

pub(super) fn continuation_requires_same_upstream(
    event: &Value,
    reuses_bound_upstream: bool,
) -> bool {
    response_create_has_previous_response_id(event) && !reuses_bound_upstream
}

pub(super) fn changed_followup_response_create_model(
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

pub(super) fn response_create_model_or_current(
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

pub(super) fn provider_model_from_decision(decision: &AiExecutionDecision) -> Option<String> {
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

pub(super) fn normalize_followup_response_create(
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

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::response_create_has_previous_response_id;

    #[test]
    fn previous_response_id_is_protocol_state_even_when_not_a_string() {
        assert!(response_create_has_previous_response_id(
            &json!({"previous_response_id": 42})
        ));
        assert!(!response_create_has_previous_response_id(
            &json!({"previous_response_id": null})
        ));
        assert!(!response_create_has_previous_response_id(&json!({})));
    }
}
