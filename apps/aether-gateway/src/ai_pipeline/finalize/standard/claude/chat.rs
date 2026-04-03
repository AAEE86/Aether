use base64::Engine as _;
use std::collections::BTreeMap;

use serde_json::{json, Map, Value};

use crate::gateway::ai_pipeline::finalize::common::{
    build_generated_tool_call_id, build_local_success_outcome, canonicalize_tool_arguments,
    local_finalize_allows_envelope, parse_stream_json_events, unwrap_local_finalize_response_value,
    LocalCoreSyncFinalizeOutcome,
};
use crate::gateway::{GatewayControlDecision, GatewayError, GatewaySyncReportRequest};

#[derive(Debug, Default)]
struct ClaudeContentBlockState {
    object: Map<String, Value>,
    text: String,
    partial_json: String,
}

pub(crate) fn maybe_build_local_claude_stream_sync_response(
    trace_id: &str,
    decision: &GatewayControlDecision,
    payload: &GatewaySyncReportRequest,
) -> Result<Option<LocalCoreSyncFinalizeOutcome>, GatewayError> {
    if payload.report_kind != "claude_chat_sync_finalize" || payload.status_code >= 400 {
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
    if provider_api_format != "claude:chat"
        || client_api_format != "claude:chat"
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
    let body_json = match aggregate_claude_stream_sync_response(&body_bytes) {
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

pub(crate) fn maybe_build_local_claude_sync_response(
    trace_id: &str,
    decision: &GatewayControlDecision,
    payload: &GatewaySyncReportRequest,
) -> Result<Option<LocalCoreSyncFinalizeOutcome>, GatewayError> {
    if payload.report_kind != "claude_chat_sync_finalize" || payload.status_code >= 400 {
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
    if provider_api_format != "claude:chat"
        || client_api_format != "claude:chat"
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

pub(crate) fn convert_claude_chat_response_to_openai_chat(
    body_json: &Value,
    report_context: &Value,
) -> Option<Value> {
    let body = body_json.as_object()?;
    let content = body.get("content")?.as_array()?;
    let mut text = String::new();
    let mut tool_calls = Vec::new();
    for (index, block) in content.iter().enumerate() {
        let block = block.as_object()?;
        match block.get("type")?.as_str()? {
            "text" => {
                text.push_str(block.get("text")?.as_str()?);
            }
            "tool_use" => {
                let tool_name = block.get("name")?.as_str()?;
                let tool_id = block
                    .get("id")
                    .and_then(Value::as_str)
                    .filter(|value| !value.is_empty())
                    .map(ToOwned::to_owned)
                    .unwrap_or_else(|| build_generated_tool_call_id(index));
                let arguments = canonicalize_tool_arguments(block.get("input").cloned());
                tool_calls.push(json!({
                    "id": tool_id,
                    "type": "function",
                    "function": {
                        "name": tool_name,
                        "arguments": arguments,
                    }
                }));
            }
            _ => return None,
        }
    }
    let mut finish_reason = match body.get("stop_reason").and_then(Value::as_str) {
        Some("end_turn") | Some("stop_sequence") => Some("stop"),
        Some("max_tokens") => Some("length"),
        Some("tool_use") => Some("tool_calls"),
        Some(other) if !other.is_empty() => Some(other),
        _ => None,
    };
    if !tool_calls.is_empty() && finish_reason.is_none_or(|reason| reason == "stop") {
        finish_reason = Some("tool_calls");
    }
    let usage = body.get("usage").and_then(Value::as_object);
    let prompt_tokens = usage
        .and_then(|value| value.get("input_tokens"))
        .and_then(Value::as_u64)
        .unwrap_or(0);
    let completion_tokens = usage
        .and_then(|value| value.get("output_tokens"))
        .and_then(Value::as_u64)
        .unwrap_or(0);
    let total_tokens = prompt_tokens + completion_tokens;
    let model = body
        .get("model")
        .and_then(Value::as_str)
        .or_else(|| report_context.get("mapped_model").and_then(Value::as_str))
        .or_else(|| report_context.get("model").and_then(Value::as_str))
        .unwrap_or("unknown");
    let id = body
        .get("id")
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
            "index": 0,
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

pub(crate) fn convert_openai_chat_response_to_claude_chat(
    body_json: &Value,
    report_context: &Value,
) -> Option<Value> {
    let body = body_json.as_object()?;
    let choices = body.get("choices")?.as_array()?;
    let first_choice = choices.first()?.as_object()?;
    let message = first_choice.get("message")?.as_object()?;
    let mut content = Vec::new();

    if let Some(text) = extract_openai_assistant_text(message.get("content")) {
        if !text.trim().is_empty() {
            content.push(json!({
                "type": "text",
                "text": text,
            }));
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
            let tool_id = tool_call
                .get("id")
                .and_then(Value::as_str)
                .filter(|value| !value.is_empty())
                .map(ToOwned::to_owned)
                .unwrap_or_else(|| build_generated_tool_call_id(index));
            let input = parse_openai_function_arguments(function.get("arguments"))?;
            content.push(json!({
                "type": "tool_use",
                "id": tool_id,
                "name": tool_name,
                "input": input,
            }));
        }
    }
    if content.is_empty() {
        content.push(json!({
            "type": "text",
            "text": "",
        }));
    }

    let stop_reason = match first_choice.get("finish_reason").and_then(Value::as_str) {
        Some("stop") | None => "end_turn",
        Some("length") => "max_tokens",
        Some("tool_calls") | Some("function_call") => "tool_use",
        Some("content_filter") => "content_filtered",
        Some(other) => other,
    };
    let usage = body.get("usage").and_then(Value::as_object);
    let input_tokens = usage
        .and_then(|value| value.get("prompt_tokens"))
        .and_then(Value::as_u64)
        .unwrap_or(0);
    let output_tokens = usage
        .and_then(|value| value.get("completion_tokens"))
        .and_then(Value::as_u64)
        .unwrap_or(0);
    let model = body
        .get("model")
        .and_then(Value::as_str)
        .or_else(|| report_context.get("mapped_model").and_then(Value::as_str))
        .or_else(|| report_context.get("model").and_then(Value::as_str))
        .unwrap_or("unknown");
    let id = body
        .get("id")
        .and_then(Value::as_str)
        .unwrap_or("msg-local-finalize");

    Some(json!({
        "id": id,
        "type": "message",
        "role": "assistant",
        "model": model,
        "content": content,
        "stop_reason": stop_reason,
        "usage": {
            "input_tokens": input_tokens,
            "output_tokens": output_tokens,
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

pub(crate) fn aggregate_claude_stream_sync_response(body: &[u8]) -> Option<Value> {
    let events = parse_stream_json_events(body)?;
    if events.is_empty() {
        return None;
    }

    let mut message_object: Option<Map<String, Value>> = None;
    let mut content_blocks: BTreeMap<usize, ClaudeContentBlockState> = BTreeMap::new();
    let mut usage: Option<Value> = None;
    let mut saw_message_start = false;

    for event in events {
        let event_object = event.as_object()?;
        let event_type = event_object
            .get("type")
            .and_then(Value::as_str)
            .unwrap_or_default();

        match event_type {
            "message_start" => {
                let mut message = event_object.get("message")?.as_object()?.clone();
                usage = message.remove("usage");
                message_object = Some(message);
                saw_message_start = true;
            }
            "content_block_start" => {
                let index = event_object
                    .get("index")
                    .and_then(Value::as_u64)
                    .map(|value| value as usize)
                    .unwrap_or(0);
                let object = event_object
                    .get("content_block")
                    .and_then(Value::as_object)
                    .cloned()
                    .unwrap_or_default();
                content_blocks.insert(
                    index,
                    ClaudeContentBlockState {
                        object,
                        ..Default::default()
                    },
                );
            }
            "content_block_delta" => {
                let index = event_object
                    .get("index")
                    .and_then(Value::as_u64)
                    .map(|value| value as usize)
                    .unwrap_or(0);
                let state = content_blocks.entry(index).or_default();
                let Some(delta) = event_object.get("delta").and_then(Value::as_object) else {
                    continue;
                };
                match delta
                    .get("type")
                    .and_then(Value::as_str)
                    .unwrap_or_default()
                {
                    "text_delta" => {
                        if let Some(text) = delta.get("text").and_then(Value::as_str) {
                            state.text.push_str(text);
                        }
                    }
                    "input_json_delta" => {
                        if let Some(partial_json) =
                            delta.get("partial_json").and_then(Value::as_str)
                        {
                            state.partial_json.push_str(partial_json);
                        }
                    }
                    _ => {}
                }
            }
            "message_delta" => {
                if let Some(message) = message_object.as_mut() {
                    if let Some(delta) = event_object.get("delta").and_then(Value::as_object) {
                        if let Some(stop_reason) = delta.get("stop_reason") {
                            message.insert("stop_reason".to_string(), stop_reason.clone());
                        }
                        if let Some(stop_sequence) = delta.get("stop_sequence") {
                            message.insert("stop_sequence".to_string(), stop_sequence.clone());
                        }
                    }
                }
                if let Some(delta_usage) = event_object.get("usage") {
                    usage = Some(delta_usage.clone());
                }
            }
            "message_stop" => {}
            _ => {}
        }
    }

    if !saw_message_start {
        return None;
    }

    let mut message = message_object?;
    let mut content = Vec::with_capacity(content_blocks.len());
    for (_index, state) in content_blocks {
        let mut block = state.object;
        let block_type = block
            .get("type")
            .and_then(Value::as_str)
            .unwrap_or("text")
            .to_string();
        match block_type.as_str() {
            "text" => {
                block.insert(
                    "text".to_string(),
                    Value::String(if state.text.is_empty() {
                        block
                            .get("text")
                            .and_then(Value::as_str)
                            .unwrap_or_default()
                            .to_string()
                    } else {
                        state.text
                    }),
                );
            }
            "tool_use" => {
                if !state.partial_json.is_empty() {
                    let input = serde_json::from_str::<Value>(&state.partial_json)
                        .unwrap_or(Value::String(state.partial_json));
                    block.insert("input".to_string(), input);
                }
            }
            _ => {
                if !state.text.is_empty() {
                    block.insert("text".to_string(), Value::String(state.text));
                }
            }
        }
        content.push(Value::Object(block));
    }
    message.insert("content".to_string(), Value::Array(content));
    if let Some(usage_value) = usage {
        message.insert("usage".to_string(), usage_value);
    }

    Some(Value::Object(message))
}
