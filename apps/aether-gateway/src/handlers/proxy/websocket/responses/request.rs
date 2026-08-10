//! Responses WebSocket request normalization and model-selection helpers.
//!
//! These functions translate client protocol events into the HTTP-shaped
//! planning input and provider `response.create` events. They deliberately do
//! not depend on connection state or perform I/O.

use axum::http::header::{AUTHORIZATION, CONNECTION, CONTENT_TYPE, UPGRADE};
use axum::http::Method;
use serde_json::Value;

use crate::ai_serving::{AiExecutionDecision, ResponsesWebSocketBodyNormalization};
use crate::handlers::proxy::websocket::ingress::credential_free_websocket_planning_uri;
use crate::handlers::proxy::websocket::ingress::WebSocketRequestContext;
use crate::headers::request_origin_from_headers_and_remote_addr;
use crate::privacy::RedactionSessionSlot;

/// Current top-level OpenAI Responses field snapshot used by codec tests.
///
/// The WebSocket `response.create` payload mirrors `POST /v1/responses`, plus
/// the WebSocket-only `type` and `generate` fields. This is deliberately not a
/// runtime allowlist: the codec preserves ordinary future fields and rejects
/// only explicitly unsupported or provider-private fields.
#[cfg(test)]
const PUBLIC_RESPONSE_CREATE_FIELDS: &[&str] = &[
    "type",
    "context_management",
    "conversation",
    "include",
    "input",
    "instructions",
    "max_output_tokens",
    "max_tool_calls",
    "metadata",
    "model",
    "moderation",
    "parallel_tool_calls",
    "previous_response_id",
    "prompt",
    "prompt_cache_key",
    "prompt_cache_options",
    "prompt_cache_retention",
    "reasoning",
    "safety_identifier",
    "service_tier",
    "store",
    "temperature",
    "text",
    "tool_choice",
    "tools",
    "top_logprobs",
    "top_p",
    "truncation",
    "user",
    "generate",
];

const WEBSOCKET_UNSUPPORTED_RESPONSE_CREATE_FIELDS: &[&str] =
    &["stream", "stream_options", "background", "multi_agent"];

const MULTI_AGENT_INPUT_ITEM_TYPES: &[&str] = &[
    "agent_message",
    "multi_agent_call",
    "multi_agent_call_output",
];

const PROVIDER_PRIVATE_RESPONSE_CREATE_FIELDS: &[&str] = &[
    "client_metadata",
    "clientMetadata",
    "agent_identity",
    "agentIdentity",
    "auth_mode",
    "authMode",
    "account_id",
    "accountId",
    "chatgpt_account_id",
    "chatgptAccountId",
    "chatgpt_account_is_fedramp",
    "chatgptAccountIsFedramp",
    "is_fedramp",
    "isFedramp",
    "originator",
    "provider",
    "provider_type",
    "providerType",
    "provider_options",
    "providerOptions",
    "provider_metadata",
    "providerMetadata",
    "provider_debug",
    "providerDebug",
    "transport",
    "transport_options",
    "transportOptions",
    "websocket_transport",
    "websocketTransport",
    "use_responses_lite",
    "useResponsesLite",
    "reasoning_summary_delivery",
    "reasoningSummaryDelivery",
    "base_instructions",
    "baseInstructions",
    "model_messages",
    "modelMessages",
    "session_id",
    "sessionId",
    "thread_id",
    "threadId",
];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum ResponsesPublicRequestError {
    InvalidEventShape,
    UnsupportedEventType,
    UnsupportedWebSocketField { field: &'static str },
    ProviderPrivateField { field: &'static str },
    InvalidField { field: &'static str },
}

impl ResponsesPublicRequestError {
    pub(super) const fn code(self) -> &'static str {
        match self {
            Self::InvalidEventShape => "invalid_response_create",
            Self::UnsupportedEventType => "unsupported_client_event",
            Self::UnsupportedWebSocketField { .. } => "unsupported_response_create_field",
            Self::ProviderPrivateField { .. } => "provider_private_response_create_field",
            Self::InvalidField { .. } => "invalid_response_create_field",
        }
    }

    pub(super) fn message(self) -> &'static str {
        match self {
            Self::InvalidEventShape => "response.create must be a JSON object",
            Self::UnsupportedEventType => "Only response.create client events are supported",
            Self::UnsupportedWebSocketField { .. } => {
                "This Responses request field is not supported over WebSocket"
            }
            Self::ProviderPrivateField { .. } => {
                "Provider-private fields are not part of the public Responses WebSocket protocol"
            }
            Self::InvalidField { field: "generate" } => {
                "response.create.generate must be a boolean or null"
            }
            Self::InvalidField { .. } => "response.create contains an invalid field value",
        }
    }

    pub(super) const fn param(self) -> Option<&'static str> {
        match self {
            Self::UnsupportedWebSocketField { field }
            | Self::ProviderPrivateField { field }
            | Self::InvalidField { field } => Some(field),
            Self::InvalidEventShape | Self::UnsupportedEventType => None,
        }
    }
}

/// Canonicalizes client input into Aether's one public Responses WebSocket
/// protocol before authorization, redaction, planning, or provider handling.
pub(super) trait ResponsesPublicRequestCodec: Send + Sync {
    fn response_create(&self, client_event: &Value) -> Result<Value, ResponsesPublicRequestError>;
}

struct OpenAiResponsesPublicRequestCodec;

static OPENAI_RESPONSES_PUBLIC_REQUEST_CODEC: OpenAiResponsesPublicRequestCodec =
    OpenAiResponsesPublicRequestCodec;

impl ResponsesPublicRequestCodec for OpenAiResponsesPublicRequestCodec {
    fn response_create(&self, client_event: &Value) -> Result<Value, ResponsesPublicRequestError> {
        let object = client_event
            .as_object()
            .ok_or(ResponsesPublicRequestError::InvalidEventShape)?;
        if object.get("type").and_then(Value::as_str) != Some("response.create") {
            return Err(ResponsesPublicRequestError::UnsupportedEventType);
        }

        for &field in WEBSOCKET_UNSUPPORTED_RESPONSE_CREATE_FIELDS {
            if object.contains_key(field) {
                return Err(ResponsesPublicRequestError::UnsupportedWebSocketField { field });
            }
        }
        for &field in PROVIDER_PRIVATE_RESPONSE_CREATE_FIELDS {
            if object.contains_key(field) {
                return Err(ResponsesPublicRequestError::ProviderPrivateField { field });
            }
        }
        if object
            .keys()
            .any(|field| provider_private_field_prefix(field))
        {
            return Err(ResponsesPublicRequestError::ProviderPrivateField {
                field: "provider_extension",
            });
        }
        if object
            .get("generate")
            .is_some_and(|value| !value.is_boolean() && !value.is_null())
        {
            return Err(ResponsesPublicRequestError::InvalidField { field: "generate" });
        }
        reject_multi_agent_item_markers(object.get("input"), "input")?;
        reject_multi_agent_item_markers(object.get("instructions"), "instructions")?;

        // The public contract mirrors POST /v1/responses. Preserve ordinary
        // fields that a newer OpenAI schema may add while this gateway version
        // is still on an older field snapshot; only explicitly private or
        // transport-incompatible fields are rejected above.
        let mut public = object.clone();
        // Existing clients have treated null as an omitted optional warmup
        // flag. Preserve that compatibility without putting null on the
        // canonical public wire sent to a provider.
        if public.get("generate").is_some_and(Value::is_null) {
            public.remove("generate");
        }
        Ok(Value::Object(public))
    }
}

fn reject_multi_agent_item_markers(
    value: Option<&Value>,
    field: &'static str,
) -> Result<(), ResponsesPublicRequestError> {
    let Some(items) = value.and_then(Value::as_array) else {
        return Ok(());
    };
    for item in items {
        let Some(item) = item.as_object() else {
            continue;
        };
        if item.contains_key("agent")
            || item
                .get("type")
                .and_then(Value::as_str)
                .is_some_and(|item_type| MULTI_AGENT_INPUT_ITEM_TYPES.contains(&item_type))
            || item
                .get("caller")
                .and_then(Value::as_object)
                .and_then(|caller| caller.get("type"))
                .and_then(Value::as_str)
                .is_some_and(|caller_type| caller_type == "multi_agent")
        {
            return Err(ResponsesPublicRequestError::InvalidField { field });
        }
    }
    Ok(())
}

fn provider_private_field_prefix(field: &str) -> bool {
    let field = field.to_ascii_lowercase();
    field.starts_with("codex.")
        || field.starts_with("codex_")
        || field.starts_with("provider_")
        || field.starts_with("x-openai-internal-")
        || field.starts_with("x_openai_internal_")
}

pub(super) fn responses_public_request_codec() -> &'static dyn ResponsesPublicRequestCodec {
    &OPENAI_RESPONSES_PUBLIC_REQUEST_CODEC
}

/// 把一条 WebSocket turn 还原成 planner 需要的 HTTP 形状请求头部。
///
/// 这里必须和 HTTP 前门（`handlers/proxy/mod.rs`）保持同一份 extension 契约：
/// planner 只在 `parts.extensions` 里拿到 `RedactionSessionSlot` 时才做请求脱敏
/// （`ai_serving/planner/redaction.rs`），少插这一项等于整条 WS 链路静默绕过
/// 已启用的 PII 脱敏。
pub(super) fn build_planning_parts(
    context: &WebSocketRequestContext,
    request_id: &str,
) -> http::request::Parts {
    let mut request = http::Request::builder()
        .method(Method::POST)
        // The ingress context is already credential-free. Keep this second
        // guard at the planning boundary so synthetic/test contexts and future
        // callers cannot accidentally turn Aether's `?key=` into provider
        // query authentication.
        .uri(credential_free_websocket_planning_uri(&context.uri))
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
    // Aether exposes the stable Responses WebSocket contract. Client beta
    // negotiation must not influence provider planning or handshake headers.
    headers.remove("openai-beta");
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
    // slot 必须每个 turn 新建，不能按连接复用：planner 侧的请求脱敏缓存键是
    // `{format:?}:{body_json 指针地址}`（`ai_serving/planner/redaction.rs:169`），
    // 连接级复用同一个 slot 时，上一轮 client_event 释放后这一轮的 `Value` 很可能
    // 落在同一地址，会命中上一轮缓存，把上一轮的脱敏 body 当成这一轮的发出去。
    // 每个 `response.create` 本身就是独立计费/审计请求，per-turn 也正好对应
    // HTTP 前门「一个请求一个 slot」的语义。
    request
        .extensions_mut()
        .insert(RedactionSessionSlot::default());
    request
        .extensions_mut()
        .insert(crate::execution_identity::ExecutionRequestId::server_owned(
            request_id.to_string(),
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
/// client event afterwards. `stream`/`stream_options`/`background` go the other
/// way: HTTP streaming normalization or provider configuration can insert
/// them, while the WebSocket protocol has no use for them.
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
    match response_create_previous_response_id(client_event)? {
        Some(previous_response_id) => {
            object.insert(
                "previous_response_id".to_string(),
                Value::String(previous_response_id.to_string()),
            );
        }
        None => {
            object.remove("previous_response_id");
        }
    }
    if let Some(value) = client_event.get("generate") {
        if value.is_null() {
            object.remove("generate");
        } else {
            object.insert("generate".to_string(), value.clone());
        }
    }
    object.remove("stream");
    object.remove("stream_options");
    object.remove("background");
    if object.contains_key("multi_agent")
        || reject_multi_agent_item_markers(object.get("input"), "input").is_err()
        || reject_multi_agent_item_markers(object.get("instructions"), "instructions").is_err()
    {
        return Err("responses_websocket_multi_agent_unsupported");
    }
    serde_json::to_string(&event).map_err(|_| "responses_websocket_request_invalid")
}

pub(super) fn response_create_previous_response_id(
    event: &Value,
) -> Result<Option<&str>, &'static str> {
    let Some(value) = event.get("previous_response_id") else {
        return Ok(None);
    };
    if value.is_null() {
        return Ok(None);
    }
    value
        .as_str()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(Some)
        .ok_or("invalid_previous_response_id")
}

pub(super) fn response_create_has_previous_response_id(event: &Value) -> bool {
    response_create_previous_response_id(event)
        .ok()
        .flatten()
        .is_some()
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
    // A configured provider normalization is part of the selected binding's
    // contract. If it rejects this continuation, forwarding the original body
    // would bypass provider-specific safety and compatibility rules.
    let mut normalized = normalization
        .normalize_response_create(event)
        .ok_or("response_create_normalization_failed")?;
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
}

#[cfg(test)]
mod tests {
    use std::net::SocketAddr;

    use axum::http::{HeaderMap, Uri};
    use serde_json::json;

    use super::{
        build_planning_parts, normalize_followup_response_create,
        response_create_has_previous_response_id, response_create_previous_response_id,
        responses_public_request_codec, ResponsesPublicRequestError, PUBLIC_RESPONSE_CREATE_FIELDS,
    };
    use crate::ai_serving::ResponsesWebSocketBodyNormalization;
    use crate::control::GatewayControlDecision;
    use crate::execution_identity::{execution_request_id_from_parts, ExecutionRequestId};
    use crate::handlers::proxy::websocket::ingress::WebSocketRequestContext;
    use crate::privacy::RedactionSessionSlot;

    fn websocket_context() -> WebSocketRequestContext {
        WebSocketRequestContext {
            trace_id: "trace-planning-parts".to_string(),
            headers: HeaderMap::new(),
            uri: Uri::from_static("/v1/responses"),
            remote_addr: "127.0.0.1:65001"
                .parse::<SocketAddr>()
                .expect("remote address should parse"),
            decision: GatewayControlDecision::synthetic(
                "/v1/responses".to_string(),
                Some("ai_public".to_string()),
                Some("openai".to_string()),
                Some("responses_websocket".to_string()),
                Some("openai:responses".to_string()),
            ),
            websocket_connection_permit: None,
        }
    }

    #[test]
    fn public_request_codec_preserves_every_public_top_level_field() {
        let mut input = serde_json::Map::new();
        for field in PUBLIC_RESPONSE_CREATE_FIELDS {
            let value = match *field {
                "type" => json!("response.create"),
                "generate" => json!(false),
                _ => json!({"field": field}),
            };
            input.insert((*field).to_string(), value);
        }
        input.insert(
            "future_responses_option".to_string(),
            json!({"enabled": true}),
        );

        let canonical = responses_public_request_codec()
            .response_create(&serde_json::Value::Object(input.clone()))
            .expect("all public fields should be accepted");
        let canonical = canonical
            .as_object()
            .expect("canonical response.create should remain an object");

        for field in PUBLIC_RESPONSE_CREATE_FIELDS {
            assert_eq!(
                canonical.get(*field),
                input.get(*field),
                "public field should be preserved: {field}"
            );
        }
        assert_eq!(
            canonical.get("future_responses_option"),
            input.get("future_responses_option")
        );
    }

    #[test]
    fn public_request_codec_rejects_http_only_websocket_fields() {
        for field in ["stream", "stream_options", "background", "multi_agent"] {
            let mut event = json!({
                "type": "response.create",
                "model": "public-model",
                "input": [],
            });
            event
                .as_object_mut()
                .expect("test event should be an object")
                .insert(field.to_string(), json!(true));

            assert_eq!(
                responses_public_request_codec().response_create(&event),
                Err(ResponsesPublicRequestError::UnsupportedWebSocketField { field }),
                "field should be rejected by the stable WebSocket protocol: {field}"
            );
            let error = responses_public_request_codec()
                .response_create(&event)
                .expect_err("unsupported WebSocket field must be rejected");
            assert_eq!(error.param(), Some(field));
        }
    }

    #[test]
    fn public_request_codec_rejects_multi_agent_beta_item_markers() {
        for items in [
            json!([{"type": "agent_message"}]),
            json!([{"type": "multi_agent_call"}]),
            json!([{"type": "multi_agent_call_output"}]),
            json!([{"type": "message", "agent": null}]),
            json!([{"type": "message", "caller": {"type": "multi_agent"}}]),
        ] {
            for field in ["input", "instructions"] {
                let mut event = json!({
                    "type": "response.create",
                    "model": "public-model",
                });
                event
                    .as_object_mut()
                    .expect("response.create test event")
                    .insert(field.to_string(), items.clone());
                let error = responses_public_request_codec()
                    .response_create(&event)
                    .expect_err("multi-agent beta items must not enter the stable protocol");
                assert_eq!(error, ResponsesPublicRequestError::InvalidField { field });
                assert_eq!(error.param(), Some(field));
            }
        }
    }

    #[test]
    fn public_request_codec_rejects_provider_private_fields() {
        for &field in super::PROVIDER_PRIVATE_RESPONSE_CREATE_FIELDS {
            let mut event = json!({
                "type": "response.create",
                "model": "public-model",
                "input": [],
            });
            event
                .as_object_mut()
                .expect("test event should be an object")
                .insert(field.to_string(), json!({"private": true}));

            assert_eq!(
                responses_public_request_codec()
                    .response_create(&event)
                    .expect_err("provider-private field should be rejected")
                    .param(),
                Some(field),
                "provider-private field should identify its public error parameter: {field}"
            );
        }

        for field in [
            "codex_debug",
            "provider_secret",
            "x-openai-internal-routing",
        ] {
            let mut event = json!({
                "type": "response.create",
                "model": "public-model",
                "input": [],
            });
            event
                .as_object_mut()
                .expect("test event should be an object")
                .insert(field.to_string(), json!({"private": true}));

            assert_eq!(
                responses_public_request_codec()
                    .response_create(&event)
                    .expect_err("provider-private prefix should be rejected")
                    .param(),
                Some("provider_extension"),
                "provider-private prefix should use the stable public parameter: {field}"
            );
        }
    }

    #[test]
    fn public_request_codec_validates_and_canonicalizes_generate() {
        assert_eq!(
            responses_public_request_codec().response_create(&json!({
                "type": "response.create",
                "generate": "false",
            })),
            Err(ResponsesPublicRequestError::InvalidField { field: "generate" })
        );

        let canonical = responses_public_request_codec()
            .response_create(&json!({
                "type": "response.create",
                "generate": null,
            }))
            .expect("null generate remains backward-compatible as omission");
        assert!(canonical.get("generate").is_none());
    }

    #[test]
    fn planning_parts_carry_a_fresh_redaction_session_slot_per_turn() {
        // 没有这个 extension，planner 会静默跳过已启用的 PII 脱敏
        // （ai_serving/planner/redaction.rs），整条 WS 链路都按原文发上游。
        let context = websocket_context();
        let first = build_planning_parts(&context, "request-turn-1");
        let second = build_planning_parts(&context, "request-turn-2");

        assert_eq!(
            execution_request_id_from_parts(&first, &context.trace_id),
            "request-turn-1"
        );
        assert_eq!(
            execution_request_id_from_parts(&second, &context.trace_id),
            "request-turn-2"
        );
        assert!(first.extensions.get::<ExecutionRequestId>().is_some());
        assert!(second.extensions.get::<ExecutionRequestId>().is_some());

        let first_slot = first
            .extensions
            .get::<RedactionSessionSlot>()
            .expect("planning parts must carry a redaction session slot");
        let second_slot = second
            .extensions
            .get::<RedactionSessionSlot>()
            .expect("planning parts must carry a redaction session slot");

        // 每轮必须是独立 slot：slot 内的请求缓存以 body 指针地址为键，跨轮共享会
        // 命中上一轮缓存。用缓存条目相互不可见来证明两者不是同一个 slot。
        first_slot.put_cached_request_redaction(
            "turn-1",
            crate::privacy::CachedRequestRedaction::unredacted(),
        );
        assert!(first_slot.cached_request_redaction("turn-1").is_some());
        assert!(second_slot.cached_request_redaction("turn-1").is_none());
    }

    #[test]
    fn planning_parts_never_forward_the_public_query_key() {
        let mut context = websocket_context();
        context.uri = Uri::from_static("/v1/responses?key=aether-secret&mode=debug&key=second");

        let parts = build_planning_parts(&context, "request-turn");

        assert_eq!(parts.uri, Uri::from_static("/v1/responses?mode=debug"));
    }

    #[test]
    fn planning_parts_never_forward_client_beta_negotiation() {
        let mut context = websocket_context();
        context.headers.insert(
            "openai-beta",
            "responses_multi_agent=2026-05-01"
                .parse()
                .expect("test header should parse"),
        );

        let parts = build_planning_parts(&context, "request-turn");

        assert!(!parts.headers.contains_key("openai-beta"));
    }

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
        assert!(normalized.get("stream_options").is_none());
        assert!(normalized.get("background").is_none());
    }

    #[test]
    fn provider_configuration_cannot_reintroduce_http_stream_options() {
        let event = json!({
            "type": "response.create",
            "model": "public-model",
            "input": [],
        });

        let from_body_rule = normalized_continuation(
            &event,
            &ResponsesWebSocketBodyNormalization::for_tests("provider-model")
                .with_body_rules_for_tests(json!([{
                    "action": "set",
                    "path": "stream_options",
                    "value": {"include_usage": true}
                }])),
        );
        assert!(from_body_rule.get("stream_options").is_none());

        let from_model_directive = normalized_continuation(
            &event,
            &ResponsesWebSocketBodyNormalization::for_tests("provider-model")
                .with_model_directive_patch_for_tests(json!({
                    "stream_options": {"include_usage": true}
                })),
        );
        assert!(from_model_directive.get("stream_options").is_none());
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
    fn continuation_normalization_failure_is_not_forwarded_unnormalized() {
        let normalization = ResponsesWebSocketBodyNormalization::for_tests("provider-model")
            .with_body_rules_for_tests(json!({"not": "a rule list"}));

        assert_eq!(
            normalize_followup_response_create(
                &json!({
                    "type": "response.create",
                    "model": "public-model",
                    "previous_response_id": "resp_123",
                    "input": "sensitive input"
                }),
                "provider-model",
                &normalization,
            ),
            Err("response_create_normalization_failed")
        );
    }

    #[test]
    fn provider_normalization_cannot_enable_multi_agent_beta() {
        for patch in [
            json!({"multi_agent": {"enabled": true}}),
            json!({
                "input": [{
                    "type": "message",
                    "caller": {"type": "multi_agent"}
                }]
            }),
        ] {
            let normalization = ResponsesWebSocketBodyNormalization::for_tests("provider-model")
                .with_model_directive_patch_for_tests(patch);

            assert_eq!(
                normalize_followup_response_create(
                    &json!({
                        "type": "response.create",
                        "model": "public-model",
                        "input": [{"type": "message", "role": "user", "content": "hello"}]
                    }),
                    "provider-model",
                    &normalization,
                ),
                Err("responses_websocket_multi_agent_unsupported")
            );
        }
    }

    #[test]
    fn previous_response_id_must_be_a_non_empty_string() {
        assert_eq!(
            response_create_previous_response_id(&json!({"previous_response_id": 42})),
            Err("invalid_previous_response_id")
        );
        assert_eq!(
            response_create_previous_response_id(&json!({"previous_response_id": "  "})),
            Err("invalid_previous_response_id")
        );
        assert!(response_create_has_previous_response_id(
            &json!({"previous_response_id": "resp_1"})
        ));
        assert!(!response_create_has_previous_response_id(
            &json!({"previous_response_id": null})
        ));
        assert!(!response_create_has_previous_response_id(&json!({})));
    }
}
