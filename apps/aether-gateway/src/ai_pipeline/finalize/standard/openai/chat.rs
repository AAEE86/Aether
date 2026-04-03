use base64::Engine as _;
use std::collections::BTreeMap;

use serde_json::{json, Map, Value};

use super::{
    aggregate_claude_stream_sync_response, aggregate_gemini_stream_sync_response,
    aggregate_openai_cli_stream_sync_response, convert_claude_chat_response_to_openai_chat,
    convert_gemini_chat_response_to_openai_chat,
};
use crate::gateway::ai_pipeline::conversion::sync_chat_response_conversion_kind;
use crate::gateway::ai_pipeline::finalize::common::{
    build_generated_tool_call_id, build_local_success_outcome,
    build_local_success_outcome_with_conversion_report, canonicalize_tool_arguments,
    local_finalize_allows_envelope, unwrap_local_finalize_response_value,
    LocalCoreSyncFinalizeOutcome,
};
use crate::gateway::{GatewayControlDecision, GatewayError, GatewaySyncReportRequest};

#[derive(Debug, Default)]
struct OpenAIChatChoiceState {
    role: Option<String>,
    content: String,
    finish_reason: Option<String>,
    tool_calls: BTreeMap<usize, OpenAIChatToolCallState>,
}

#[derive(Debug, Default)]
struct OpenAIChatToolCallState {
    id: Option<String>,
    tool_type: Option<String>,
    function_name: Option<String>,
    function_arguments: String,
}

pub(crate) fn maybe_build_local_openai_chat_stream_sync_response(
    trace_id: &str,
    decision: &GatewayControlDecision,
    payload: &GatewaySyncReportRequest,
) -> Result<Option<LocalCoreSyncFinalizeOutcome>, GatewayError> {
    if payload.report_kind != "openai_chat_sync_finalize" || payload.status_code >= 400 {
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
    if provider_api_format != "openai:chat"
        || client_api_format != "openai:chat"
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
    let body_json = match aggregate_openai_chat_stream_sync_response(&body_bytes) {
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

pub(crate) fn maybe_build_local_openai_chat_sync_response(
    trace_id: &str,
    decision: &GatewayControlDecision,
    payload: &GatewaySyncReportRequest,
) -> Result<Option<LocalCoreSyncFinalizeOutcome>, GatewayError> {
    if payload.report_kind != "openai_chat_sync_finalize" || payload.status_code >= 400 {
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
    if provider_api_format != "openai:chat"
        || client_api_format != "openai:chat"
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

pub(crate) fn maybe_build_local_openai_chat_cross_format_stream_sync_response(
    trace_id: &str,
    decision: &GatewayControlDecision,
    payload: &GatewaySyncReportRequest,
) -> Result<Option<LocalCoreSyncFinalizeOutcome>, GatewayError> {
    if payload.report_kind != "openai_chat_sync_finalize" || payload.status_code >= 400 {
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
    if client_api_format != "openai:chat" || !local_finalize_allows_envelope(report_context) {
        return Ok(None);
    }
    let Some(conversion_kind) =
        sync_chat_response_conversion_kind(&provider_api_format, &client_api_format)
    else {
        return Ok(None);
    };

    let Some(body_base64) = payload.body_base64.as_deref() else {
        return Ok(None);
    };
    let body_bytes = base64::engine::general_purpose::STANDARD
        .decode(body_base64)
        .map_err(|err| GatewayError::Internal(err.to_string()))?;
    let aggregated = match provider_api_format.as_str() {
        "claude:chat" | "claude:cli" => aggregate_claude_stream_sync_response(&body_bytes),
        "gemini:chat" | "gemini:cli" => aggregate_gemini_stream_sync_response(&body_bytes),
        "openai:cli" | "openai:compact" => aggregate_openai_cli_stream_sync_response(&body_bytes),
        _ => None,
    };
    let Some(aggregated) = aggregated else {
        return Ok(None);
    };
    let Some(aggregated) = unwrap_local_finalize_response_value(aggregated, report_context)? else {
        return Ok(None);
    };
    let converted = match provider_api_format.as_str() {
        "claude:chat" | "claude:cli" => {
            convert_claude_chat_response_to_openai_chat(&aggregated, report_context)
        }
        "gemini:chat" | "gemini:cli" => {
            convert_gemini_chat_response_to_openai_chat(&aggregated, report_context)
        }
        "openai:cli" | "openai:compact" => {
            convert_openai_cli_response_to_openai_chat(&aggregated, report_context)
        }
        _ => None,
    };
    let Some(converted) = converted else {
        return Ok(None);
    };

    Ok(Some(build_local_success_outcome_with_conversion_report(
        trace_id, decision, payload, converted, aggregated,
    )?))
}

pub(crate) fn maybe_build_local_openai_chat_cross_format_sync_response(
    trace_id: &str,
    decision: &GatewayControlDecision,
    payload: &GatewaySyncReportRequest,
) -> Result<Option<LocalCoreSyncFinalizeOutcome>, GatewayError> {
    if payload.report_kind != "openai_chat_sync_finalize" || payload.status_code >= 400 {
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
    if client_api_format != "openai:chat" || !local_finalize_allows_envelope(report_context) {
        return Ok(None);
    }
    let Some(conversion_kind) =
        sync_chat_response_conversion_kind(&provider_api_format, &client_api_format)
    else {
        return Ok(None);
    };

    let Some(body_json) = payload.body_json.as_ref() else {
        return Ok(None);
    };
    let Some(body_json) = unwrap_local_finalize_response_value(body_json.clone(), report_context)?
    else {
        return Ok(None);
    };
    let converted = match provider_api_format.as_str() {
        "claude:chat" | "claude:cli" => {
            convert_claude_chat_response_to_openai_chat(&body_json, report_context)
        }
        "gemini:chat" | "gemini:cli" => {
            convert_gemini_chat_response_to_openai_chat(&body_json, report_context)
        }
        "openai:cli" | "openai:compact" => {
            convert_openai_cli_response_to_openai_chat(&body_json, report_context)
        }
        _ => None,
    };
    let Some(converted) = converted else {
        return Ok(None);
    };

    Ok(Some(build_local_success_outcome_with_conversion_report(
        trace_id, decision, payload, converted, body_json,
    )?))
}

pub(crate) fn convert_openai_cli_response_to_openai_chat(
    body_json: &Value,
    report_context: &Value,
) -> Option<Value> {
    let body = body_json.as_object()?;
    let mut text = String::new();
    let mut tool_calls = Vec::new();

    if let Some(output_items) = body.get("output").and_then(Value::as_array) {
        for (index, item) in output_items.iter().enumerate() {
            let item_object = item.as_object()?;
            let item_type = item_object
                .get("type")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .trim()
                .to_ascii_lowercase();
            match item_type.as_str() {
                "message" => {
                    if let Some(content) = item_object.get("content").and_then(Value::as_array) {
                        for part in content {
                            let part_object = part.as_object()?;
                            let part_type = part_object
                                .get("type")
                                .and_then(Value::as_str)
                                .unwrap_or_default()
                                .trim()
                                .to_ascii_lowercase();
                            if matches!(part_type.as_str(), "output_text" | "text") {
                                if let Some(piece) = part_object.get("text").and_then(Value::as_str)
                                {
                                    text.push_str(piece);
                                }
                            }
                        }
                    }
                }
                "function_call" => {
                    let tool_name = item_object
                        .get("name")
                        .and_then(Value::as_str)
                        .map(str::trim)
                        .filter(|value| !value.is_empty())?;
                    let tool_id = item_object
                        .get("call_id")
                        .and_then(Value::as_str)
                        .filter(|value| !value.is_empty())
                        .or_else(|| {
                            item_object
                                .get("id")
                                .and_then(Value::as_str)
                                .filter(|value| !value.is_empty())
                        })
                        .map(ToOwned::to_owned)
                        .unwrap_or_else(|| build_generated_tool_call_id(index));
                    tool_calls.push(json!({
                        "id": tool_id,
                        "type": "function",
                        "function": {
                            "name": tool_name,
                            "arguments": canonicalize_tool_arguments(item_object.get("arguments").cloned()),
                        }
                    }));
                }
                "output_text" | "text" => {
                    if let Some(piece) = item_object.get("text").and_then(Value::as_str) {
                        text.push_str(piece);
                    }
                }
                _ => {}
            }
        }
    }

    let finish_reason = if tool_calls.is_empty() {
        Some("stop")
    } else {
        Some("tool_calls")
    };
    let model = body
        .get("model")
        .and_then(Value::as_str)
        .or_else(|| report_context.get("mapped_model").and_then(Value::as_str))
        .or_else(|| report_context.get("model").and_then(Value::as_str))
        .unwrap_or("unknown");
    let id = body
        .get("id")
        .and_then(Value::as_str)
        .unwrap_or("chatcmpl-local-openai-cli");

    let usage = body.get("usage").and_then(Value::as_object);
    let prompt_tokens = usage
        .and_then(|value| value.get("input_tokens"))
        .and_then(Value::as_u64)
        .unwrap_or(0);
    let completion_tokens = usage
        .and_then(|value| value.get("output_tokens"))
        .and_then(Value::as_u64)
        .unwrap_or(0);
    let total_tokens = usage
        .and_then(|value| value.get("total_tokens"))
        .and_then(Value::as_u64)
        .unwrap_or(prompt_tokens + completion_tokens);

    let mut message = Map::new();
    message.insert("role".to_string(), Value::String("assistant".to_string()));
    if text.is_empty() && !tool_calls.is_empty() {
        message.insert("content".to_string(), Value::Null);
    } else {
        message.insert("content".to_string(), Value::String(text));
    }
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

pub(crate) fn aggregate_openai_chat_stream_sync_response(body: &[u8]) -> Option<Value> {
    let text = std::str::from_utf8(body).ok()?;
    let mut response_id: Option<String> = None;
    let mut model: Option<String> = None;
    let mut created: Option<u64> = None;
    let mut usage: Option<Value> = None;
    let mut choices: BTreeMap<usize, OpenAIChatChoiceState> = BTreeMap::new();
    let mut saw_chunk = false;

    for raw_line in text.lines() {
        let line = raw_line.trim_matches('\r').trim();
        if line.is_empty() || line.starts_with(':') || line.starts_with("event:") {
            continue;
        }

        let Some(data_line) = line.strip_prefix("data:") else {
            continue;
        };
        let data_line = data_line.trim();
        if data_line.is_empty() || data_line == "[DONE]" {
            continue;
        }

        let chunk: Value = serde_json::from_str(data_line).ok()?;
        let chunk_object = chunk.as_object()?;
        saw_chunk = true;

        if response_id.is_none() {
            response_id = chunk_object
                .get("id")
                .and_then(Value::as_str)
                .map(ToOwned::to_owned);
        }
        if model.is_none() {
            model = chunk_object
                .get("model")
                .and_then(Value::as_str)
                .map(ToOwned::to_owned);
        }
        if created.is_none() {
            created = chunk_object.get("created").and_then(Value::as_u64);
        }
        if let Some(u) = chunk_object.get("usage") {
            usage = Some(u.clone());
        }

        let Some(chunk_choices) = chunk_object.get("choices").and_then(Value::as_array) else {
            continue;
        };
        for chunk_choice in chunk_choices {
            let Some(choice_object) = chunk_choice.as_object() else {
                continue;
            };
            let Some(index) = choice_object
                .get("index")
                .and_then(Value::as_u64)
                .map(|value| value as usize)
            else {
                continue;
            };
            let state = choices.entry(index).or_default();
            if let Some(finish_reason) = choice_object.get("finish_reason").and_then(Value::as_str)
            {
                state.finish_reason = Some(finish_reason.to_string());
            }

            let Some(delta) = choice_object.get("delta").and_then(Value::as_object) else {
                continue;
            };
            if let Some(role) = delta.get("role").and_then(Value::as_str) {
                state.role = Some(role.to_string());
            }
            if let Some(content) = delta.get("content").and_then(Value::as_str) {
                state.content.push_str(content);
            }
            if let Some(tool_calls) = delta.get("tool_calls").and_then(Value::as_array) {
                for tool_call in tool_calls {
                    let Some(tool_call_object) = tool_call.as_object() else {
                        continue;
                    };
                    let tool_index = tool_call_object
                        .get("index")
                        .and_then(Value::as_u64)
                        .map(|value| value as usize)
                        .unwrap_or(0);
                    let tool_state = state.tool_calls.entry(tool_index).or_default();
                    if let Some(id) = tool_call_object.get("id").and_then(Value::as_str) {
                        tool_state.id = Some(id.to_string());
                    }
                    if let Some(tool_type) = tool_call_object.get("type").and_then(Value::as_str) {
                        tool_state.tool_type = Some(tool_type.to_string());
                    }
                    if let Some(function) =
                        tool_call_object.get("function").and_then(Value::as_object)
                    {
                        if let Some(name) = function.get("name").and_then(Value::as_str) {
                            tool_state.function_name = Some(name.to_string());
                        }
                        if let Some(arguments) = function.get("arguments").and_then(Value::as_str) {
                            tool_state.function_arguments.push_str(arguments);
                        }
                    }
                }
            }
        }
    }

    if !saw_chunk {
        return None;
    }

    let mut response_object = Map::new();
    response_object.insert(
        "id".to_string(),
        Value::String(response_id.unwrap_or_else(|| "chatcmpl-local-finalize".to_string())),
    );
    response_object.insert(
        "object".to_string(),
        Value::String("chat.completion".to_string()),
    );
    if let Some(created) = created {
        response_object.insert("created".to_string(), Value::Number(created.into()));
    }
    if let Some(model) = model {
        response_object.insert("model".to_string(), Value::String(model));
    }

    let mut response_choices = Vec::with_capacity(choices.len());
    for (index, state) in choices {
        let mut message = Map::new();
        message.insert(
            "role".to_string(),
            Value::String(state.role.unwrap_or_else(|| "assistant".to_string())),
        );
        if state.tool_calls.is_empty() {
            message.insert("content".to_string(), Value::String(state.content));
        } else {
            if state.content.is_empty() {
                message.insert("content".to_string(), Value::Null);
            } else {
                message.insert("content".to_string(), Value::String(state.content));
            }
            let tool_calls = state
                .tool_calls
                .into_iter()
                .map(|(tool_index, tool_state)| {
                    json!({
                        "index": tool_index,
                        "id": tool_state.id,
                        "type": tool_state.tool_type.unwrap_or_else(|| "function".to_string()),
                        "function": {
                            "name": tool_state.function_name,
                            "arguments": tool_state.function_arguments,
                        },
                    })
                })
                .collect::<Vec<_>>();
            message.insert("tool_calls".to_string(), Value::Array(tool_calls));
        }

        response_choices.push(json!({
            "index": index,
            "message": Value::Object(message),
            "finish_reason": state.finish_reason,
        }));
    }
    response_object.insert("choices".to_string(), Value::Array(response_choices));
    if let Some(usage) = usage {
        response_object.insert("usage".to_string(), usage);
    }

    Some(Value::Object(response_object))
}
