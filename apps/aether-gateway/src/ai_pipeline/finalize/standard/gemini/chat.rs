use base64::Engine as _;
use std::collections::BTreeMap;

use serde_json::{json, Map, Value};

use crate::gateway::ai_pipeline::finalize::common::{
    build_generated_tool_call_id, build_local_success_outcome, canonicalize_tool_arguments,
    local_finalize_allows_envelope, parse_stream_json_events, unwrap_local_finalize_response_value,
    LocalCoreSyncFinalizeOutcome,
};
use crate::gateway::{GatewayControlDecision, GatewayError, GatewaySyncReportRequest};

pub(crate) fn maybe_build_local_gemini_stream_sync_response(
    trace_id: &str,
    decision: &GatewayControlDecision,
    payload: &GatewaySyncReportRequest,
) -> Result<Option<LocalCoreSyncFinalizeOutcome>, GatewayError> {
    if payload.report_kind != "gemini_chat_sync_finalize" || payload.status_code >= 400 {
        return Ok(None);
    }

    let Some(report_context) = payload.report_context.as_ref() else {
        return Ok(None);
    };
    let provider_api_format = report_context
        .get("provider_api_format")
        .and_then(Value::as_str)
        .unwrap_or_default()
        .trim()
        .to_ascii_lowercase();
    let client_api_format = report_context
        .get("client_api_format")
        .and_then(Value::as_str)
        .unwrap_or_default()
        .trim()
        .to_ascii_lowercase();
    let needs_conversion = report_context
        .get("needs_conversion")
        .and_then(Value::as_bool)
        .unwrap_or(false);
    if !local_finalize_allows_envelope(report_context) {
        return Ok(None);
    }
    if provider_api_format != "gemini:chat"
        || client_api_format != "gemini:chat"
        || needs_conversion
    {
        return Ok(None);
    }

    let Some(body_base64) = payload.body_base64.as_deref() else {
        return Ok(None);
    };
    let body_bytes = base64::engine::general_purpose::STANDARD
        .decode(body_base64)
        .map_err(|err| GatewayError::Internal(err.to_string()))?;
    let body_json = match aggregate_gemini_stream_sync_response(&body_bytes) {
        Some(body_json) => body_json,
        None => return Ok(None),
    };
    let Some(body_json) = unwrap_local_finalize_response_value(body_json, report_context)? else {
        return Ok(None);
    };

    Ok(Some(build_local_success_outcome(
        trace_id, decision, payload, body_json,
    )?))
}

pub(crate) fn maybe_build_local_gemini_sync_response(
    trace_id: &str,
    decision: &GatewayControlDecision,
    payload: &GatewaySyncReportRequest,
) -> Result<Option<LocalCoreSyncFinalizeOutcome>, GatewayError> {
    if payload.report_kind != "gemini_chat_sync_finalize" || payload.status_code >= 400 {
        return Ok(None);
    }

    let Some(report_context) = payload.report_context.as_ref() else {
        return Ok(None);
    };
    let provider_api_format = report_context
        .get("provider_api_format")
        .and_then(Value::as_str)
        .unwrap_or_default()
        .trim()
        .to_ascii_lowercase();
    let client_api_format = report_context
        .get("client_api_format")
        .and_then(Value::as_str)
        .unwrap_or_default()
        .trim()
        .to_ascii_lowercase();
    let needs_conversion = report_context
        .get("needs_conversion")
        .and_then(Value::as_bool)
        .unwrap_or(false);
    if !local_finalize_allows_envelope(report_context) {
        return Ok(None);
    }
    if provider_api_format != "gemini:chat"
        || client_api_format != "gemini:chat"
        || needs_conversion
    {
        return Ok(None);
    }

    let Some(body_json) = payload.body_json.as_ref() else {
        return Ok(None);
    };
    let Some(body_json) = unwrap_local_finalize_response_value(body_json.clone(), report_context)?
    else {
        return Ok(None);
    };

    Ok(Some(build_local_success_outcome(
        trace_id, decision, payload, body_json,
    )?))
}

pub(crate) fn convert_gemini_chat_response_to_openai_chat(
    body_json: &Value,
    report_context: &Value,
) -> Option<Value> {
    let body = body_json.as_object()?;
    let candidates = body.get("candidates")?.as_array()?;
    let first_candidate = candidates.first()?.as_object()?;
    let content = first_candidate.get("content")?.as_object()?;
    let parts = content.get("parts")?.as_array()?;
    let mut text = String::new();
    let mut tool_calls = Vec::new();
    for (index, part) in parts.iter().enumerate() {
        let part = part.as_object()?;
        if let Some(piece) = part.get("text").and_then(Value::as_str) {
            text.push_str(piece);
        } else if let Some(function_call) = part.get("functionCall").and_then(Value::as_object) {
            let tool_name = function_call.get("name")?.as_str()?;
            let tool_id = function_call
                .get("id")
                .and_then(Value::as_str)
                .filter(|value| !value.is_empty())
                .map(ToOwned::to_owned)
                .unwrap_or_else(|| build_generated_tool_call_id(index));
            let arguments = canonicalize_tool_arguments(function_call.get("args").cloned());
            tool_calls.push(json!({
                "id": tool_id,
                "type": "function",
                "function": {
                    "name": tool_name,
                    "arguments": arguments,
                }
            }));
        } else {
            return None;
        }
    }
    let mut finish_reason = match first_candidate.get("finishReason").and_then(Value::as_str) {
        Some("STOP") => Some("stop"),
        Some("MAX_TOKENS") => Some("length"),
        Some("SAFETY") => Some("content_filter"),
        Some(other) if !other.is_empty() => Some(other),
        _ => None,
    };
    if !tool_calls.is_empty() && finish_reason.is_none_or(|reason| reason == "stop") {
        finish_reason = Some("tool_calls");
    }
    let usage = body.get("usageMetadata").and_then(Value::as_object);
    let prompt_tokens = usage
        .and_then(|value| value.get("promptTokenCount"))
        .and_then(Value::as_u64)
        .unwrap_or(0);
    let completion_tokens = usage
        .and_then(|value| value.get("candidatesTokenCount"))
        .and_then(Value::as_u64)
        .unwrap_or(0);
    let total_tokens = usage
        .and_then(|value| value.get("totalTokenCount"))
        .and_then(Value::as_u64)
        .unwrap_or(prompt_tokens + completion_tokens);
    let model = body
        .get("modelVersion")
        .and_then(Value::as_str)
        .or_else(|| report_context.get("mapped_model").and_then(Value::as_str))
        .or_else(|| report_context.get("model").and_then(Value::as_str))
        .unwrap_or("unknown");
    let id = body
        .get("responseId")
        .and_then(Value::as_str)
        .unwrap_or("chatcmpl-local-finalize");
    let message_content = if text.is_empty() && !tool_calls.is_empty() {
        Value::Null
    } else {
        Value::String(text)
    };
    let mut message = Map::new();
    message.insert("role".to_string(), Value::String("assistant".to_string()));
    message.insert("content".to_string(), message_content);
    if !tool_calls.is_empty() {
        message.insert("tool_calls".to_string(), Value::Array(tool_calls));
    }
    Some(json!({
        "id": id,
        "object": "chat.completion",
        "model": model,
        "choices": [{
            "index": first_candidate.get("index").and_then(Value::as_u64).unwrap_or(0),
            "message": Value::Object(message),
            "finish_reason": finish_reason,
        }],
        "usage": {
            "prompt_tokens": prompt_tokens,
            "completion_tokens": completion_tokens,
            "total_tokens": total_tokens,
        }
    }))
}

pub(crate) fn convert_openai_chat_response_to_gemini_chat(
    body_json: &Value,
    report_context: &Value,
) -> Option<Value> {
    let body = body_json.as_object()?;
    let choices = body.get("choices")?.as_array()?;
    let first_choice = choices.first()?.as_object()?;
    let message = first_choice.get("message")?.as_object()?;
    let mut parts = Vec::new();

    if let Some(text) = extract_openai_assistant_text(message.get("content")) {
        if !text.trim().is_empty() {
            parts.push(json!({ "text": text }));
        }
    }
    if let Some(tool_call_values) = message.get("tool_calls").and_then(Value::as_array) {
        for (index, tool_call) in tool_call_values.iter().enumerate() {
            let tool_call = tool_call.as_object()?;
            let function = tool_call.get("function")?.as_object()?;
            let tool_name = function
                .get("name")
                .and_then(Value::as_str)
                .map(str::trim)
                .filter(|value| !value.is_empty())?;
            let call_id = tool_call
                .get("id")
                .and_then(Value::as_str)
                .filter(|value| !value.is_empty())
                .map(ToOwned::to_owned)
                .unwrap_or_else(|| build_generated_tool_call_id(index));
            parts.push(json!({
                "functionCall": {
                    "id": call_id,
                    "name": tool_name,
                    "args": parse_openai_function_arguments(function.get("arguments"))?,
                }
            }));
        }
    }
    if parts.is_empty() {
        parts.push(json!({ "text": "" }));
    }

    let usage = body.get("usage").and_then(Value::as_object);
    let prompt_tokens = usage
        .and_then(|value| value.get("prompt_tokens"))
        .and_then(Value::as_u64)
        .unwrap_or(0);
    let completion_tokens = usage
        .and_then(|value| value.get("completion_tokens"))
        .and_then(Value::as_u64)
        .unwrap_or(0);
    let total_tokens = usage
        .and_then(|value| value.get("total_tokens"))
        .and_then(Value::as_u64)
        .unwrap_or(prompt_tokens + completion_tokens);
    let mut finish_reason = match first_choice.get("finish_reason").and_then(Value::as_str) {
        Some("stop") | None => "STOP",
        Some("length") => "MAX_TOKENS",
        Some("content_filter") => "SAFETY",
        Some("tool_calls") | Some("function_call") => "STOP",
        Some(other) => other,
    };
    if parts.iter().any(|part| part.get("functionCall").is_some()) {
        finish_reason = "STOP";
    }
    let model = body
        .get("model")
        .and_then(Value::as_str)
        .or_else(|| report_context.get("mapped_model").and_then(Value::as_str))
        .or_else(|| report_context.get("model").and_then(Value::as_str))
        .unwrap_or("unknown");
    let response_id = body
        .get("id")
        .and_then(Value::as_str)
        .unwrap_or("resp-local-finalize");

    Some(json!({
        "responseId": response_id,
        "modelVersion": model,
        "candidates": [{
            "content": {
                "role": "model",
                "parts": parts,
            },
            "finishReason": finish_reason,
            "index": 0,
        }],
        "usageMetadata": {
            "promptTokenCount": prompt_tokens,
            "candidatesTokenCount": completion_tokens,
            "totalTokenCount": total_tokens,
        }
    }))
}

fn extract_openai_assistant_text(content: Option<&Value>) -> Option<String> {
    match content? {
        Value::Null => Some(String::new()),
        Value::String(text) => Some(text.clone()),
        Value::Array(parts) => {
            let mut text = String::new();
            for part in parts {
                let part = part.as_object()?;
                let part_type = part
                    .get("type")
                    .and_then(Value::as_str)
                    .unwrap_or_default()
                    .trim()
                    .to_ascii_lowercase();
                if matches!(part_type.as_str(), "text" | "output_text") {
                    if let Some(piece) = part.get("text").and_then(Value::as_str) {
                        text.push_str(piece);
                    }
                }
            }
            Some(text)
        }
        _ => None,
    }
}

fn parse_openai_function_arguments(arguments: Option<&Value>) -> Option<Value> {
    match arguments.cloned().unwrap_or(Value::Object(Map::new())) {
        Value::String(text) => serde_json::from_str(&text)
            .ok()
            .or(Some(Value::String(text))),
        other => Some(other),
    }
}

pub(crate) fn aggregate_gemini_stream_sync_response(body: &[u8]) -> Option<Value> {
    let events = parse_stream_json_events(body)?;
    if events.is_empty() {
        return None;
    }

    let mut candidates: BTreeMap<usize, Value> = BTreeMap::new();
    let mut response_id: Option<Value> = None;
    let mut private_response_id: Option<Value> = None;
    let mut model_version: Option<Value> = None;
    let mut usage_metadata: Option<Value> = None;
    let mut prompt_feedback: Option<Value> = None;
    let mut saw_candidate = false;

    for event in events {
        let raw_event_object = event.as_object()?;
        if let Some(id) = raw_event_object.get("responseId") {
            response_id = Some(id.clone());
        }
        if let Some(id) = raw_event_object.get("_v1internal_response_id") {
            private_response_id = Some(id.clone());
        }
        let event_object = if let Some(response) = raw_event_object
            .get("response")
            .and_then(Value::as_object)
            .filter(|response| response.contains_key("candidates"))
        {
            response
        } else {
            raw_event_object
        };
        if let Some(id) = event_object.get("responseId") {
            response_id = Some(id.clone());
        }
        if let Some(id) = event_object.get("_v1internal_response_id") {
            private_response_id = Some(id.clone());
        }
        if let Some(version) = event_object.get("modelVersion") {
            model_version = Some(version.clone());
        }
        if let Some(usage) = event_object.get("usageMetadata") {
            usage_metadata = Some(usage.clone());
        }
        if let Some(prompt) = event_object.get("promptFeedback") {
            prompt_feedback = Some(prompt.clone());
        }
        let Some(event_candidates) = event_object.get("candidates").and_then(Value::as_array)
        else {
            continue;
        };
        for candidate in event_candidates {
            let Some(candidate_object) = candidate.as_object() else {
                continue;
            };
            let index = candidate_object
                .get("index")
                .and_then(Value::as_u64)
                .map(|value| value as usize)
                .unwrap_or(0);
            candidates.insert(index, Value::Object(candidate_object.clone()));
            saw_candidate = true;
        }
    }

    if !saw_candidate {
        return None;
    }

    let mut response = Map::new();
    if let Some(response_id) = response_id {
        response.insert("responseId".to_string(), response_id);
    }
    if let Some(private_response_id) = private_response_id {
        response.insert("_v1internal_response_id".to_string(), private_response_id);
    }
    response.insert(
        "candidates".to_string(),
        Value::Array(candidates.into_values().collect()),
    );
    if let Some(version) = model_version {
        response.insert("modelVersion".to_string(), version);
    }
    if let Some(usage) = usage_metadata {
        response.insert("usageMetadata".to_string(), usage);
    }
    if let Some(prompt) = prompt_feedback {
        response.insert("promptFeedback".to_string(), prompt);
    }
    Some(Value::Object(response))
}
