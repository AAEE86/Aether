//! Responses WebSocket request normalization and model-selection helpers.
//!
//! These functions translate client protocol events into the HTTP-shaped
//! planning input and provider `response.create` events. They deliberately do
//! not depend on connection state or perform I/O.

use axum::http::header::{AUTHORIZATION, CONNECTION, CONTENT_TYPE, UPGRADE};
use axum::http::Method;
use serde_json::Value;

use crate::ai_serving::{AiExecutionDecision, ResponsesWebSocketBodyNormalization};
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
    let event = decision
        .provider_request_body
        .clone()
        .unwrap_or_else(|| fallback.clone());
    finish_response_create_event(event, fallback)
}

/// Restores the WebSocket protocol framing that provider-body normalization is
/// not aware of.
///
/// `previous_response_id` is on the Codex unsupported-field list and `generate`
/// is not an HTTP body option at all, so normalization strips both — yet they
/// are the entire point of WebSocket mode. They must be re-grafted from the
/// client event afterwards. `stream`/`background` go the other way: the
/// normalizer inserts `stream`, and the WebSocket protocol has no use for it.
fn finish_response_create_event(
    mut event: Value,
    client_event: &Value,
) -> Result<String, &'static str> {
    let object = event
        .as_object_mut()
        .ok_or("responses_websocket_request_invalid")?;
    object.insert(
        "type".to_string(),
        Value::String("response.create".to_string()),
    );
    for field in ["previous_response_id", "generate"] {
        if let Some(value) = client_event.get(field) {
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

/// Prepares a continuation `response.create` for the already-bound upstream.
///
/// The turn cannot be re-planned without risking a different provider key, so
/// the binding's retained normalizer is replayed instead. That keeps model
/// directives, endpoint body rules and the Codex body contract applied on every
/// turn rather than only on the one that bound the socket.
pub(super) fn normalize_followup_response_create(
    event: &Value,
    provider_model: &str,
    normalization: &ResponsesWebSocketBodyNormalization,
) -> Result<String, &'static str> {
    if event.as_object().is_none() {
        return Err("invalid_response_create");
    }
    if event.get("type").and_then(Value::as_str) != Some("response.create") {
        return Err("invalid_response_create");
    }
    // Normalization is best-effort here: a continuation cannot fall back to
    // another candidate, so a body the contract rejects is still better sent
    // than dropped.
    let mut normalized = normalization
        .normalize_response_create(event)
        .unwrap_or_else(|| event.clone());
    let Some(object) = normalized.as_object_mut() else {
        return Err("invalid_response_create");
    };
    // A continuation must never switch models mid-socket, and normalization is
    // allowed to rewrite `model` (the Codex image-tool path does).
    object.insert(
        "model".to_string(),
        Value::String(provider_model.to_string()),
    );
    finish_response_create_event(normalized, event)
        .map_err(|_| "response_create_serialization_failed")
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::{normalize_followup_response_create, response_create_has_previous_response_id};
    use crate::ai_serving::ResponsesWebSocketBodyNormalization;

    fn normalized_continuation(
        event: &serde_json::Value,
        normalization: &ResponsesWebSocketBodyNormalization,
    ) -> serde_json::Value {
        let outbound = normalize_followup_response_create(event, "provider-model", normalization)
            .expect("continuation should normalize");
        serde_json::from_str(&outbound).expect("normalized event should be JSON")
    }

    #[test]
    fn continuation_keeps_protocol_state_that_provider_normalization_strips() {
        // `previous_response_id` is on the Codex unsupported-field list, so
        // normalization removes it — yet it is what continues the chain. If
        // this regresses, every continuation turn silently starts a new one.
        let event = json!({
            "type": "response.create",
            "model": "public-model",
            "previous_response_id": "resp_123",
            "input": [],
            "stream": true,
            "background": true,
        });

        let normalized = normalized_continuation(
            &event,
            &ResponsesWebSocketBodyNormalization::for_tests("provider-model")
                .with_provider_type_for_tests("codex"),
        );

        assert_eq!(normalized["type"], "response.create");
        assert_eq!(normalized["previous_response_id"], "resp_123");
        assert_eq!(normalized["model"], "provider-model");
        assert!(normalized.get("stream").is_none());
        assert!(normalized.get("background").is_none());
    }

    #[test]
    fn continuation_strips_fields_the_codex_backend_rejects() {
        // The point of the fix: before it, turns 2..N reached Codex with the
        // client's raw body, so a `temperature` that turn 1 had stripped would
        // be rejected upstream. This also proves normalization really runs
        // rather than silently falling back to the unmodified event.
        let event = json!({
            "type": "response.create",
            "model": "public-model",
            "previous_response_id": "resp_123",
            "temperature": 0.7,
            "top_p": 0.9,
            "input": [],
        });

        let normalized = normalized_continuation(
            &event,
            &ResponsesWebSocketBodyNormalization::for_tests("provider-model")
                .with_provider_type_for_tests("codex"),
        );

        assert!(normalized.get("temperature").is_none());
        assert!(normalized.get("top_p").is_none());
        assert_eq!(normalized["store"], false);
        // ...and the protocol state survives the same pass.
        assert_eq!(normalized["previous_response_id"], "resp_123");
    }

    #[test]
    fn continuation_keeps_a_warmup_generate_flag() {
        let event = json!({
            "type": "response.create",
            "model": "public-model",
            "previous_response_id": "resp_123",
            "generate": false,
            "input": [],
        });

        let normalized = normalized_continuation(
            &event,
            &ResponsesWebSocketBodyNormalization::for_tests("provider-model")
                .with_provider_type_for_tests("codex"),
        );

        assert_eq!(normalized["generate"], false);
    }

    #[test]
    fn continuation_applies_the_model_directive_patch_the_binding_turn_received() {
        let event = json!({
            "type": "response.create",
            "model": "public-model",
            "previous_response_id": "resp_123",
            "input": [],
        });

        let normalized = normalized_continuation(
            &event,
            &ResponsesWebSocketBodyNormalization::for_tests("provider-model")
                .with_model_directive_patch_for_tests(json!({"reasoning": {"effort": "high"}})),
        );

        assert_eq!(normalized["reasoning"]["effort"], "high");
    }

    #[test]
    fn continuation_still_forces_the_bound_provider_model() {
        let event = json!({
            "type": "response.create",
            "model": "some-other-model",
            "previous_response_id": "resp_123",
            "input": [],
        });

        let normalized = normalized_continuation(
            &event,
            &ResponsesWebSocketBodyNormalization::for_tests("provider-model"),
        );

        assert_eq!(normalized["model"], "provider-model");
    }

    #[test]
    fn a_continuation_that_is_not_a_response_create_is_rejected() {
        let normalization = ResponsesWebSocketBodyNormalization::for_tests("provider-model");

        assert!(normalize_followup_response_create(
            &json!({"type": "response.cancel"}),
            "provider-model",
            &normalization,
        )
        .is_err());
        assert!(normalize_followup_response_create(
            &json!("not an object"),
            "provider-model",
            &normalization,
        )
        .is_err());
    }

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
