use base64::Engine as _;

use serde_json::{json, Value};

use super::{
    aggregate_claude_stream_sync_response, aggregate_gemini_stream_sync_response,
    convert_claude_cli_response_to_openai_cli, convert_gemini_cli_response_to_openai_cli,
};
use crate::gateway::ai_pipeline::conversion::sync_cli_response_conversion_kind;
use crate::gateway::ai_pipeline::finalize::common::{
    build_local_success_outcome, build_local_success_outcome_with_conversion_report,
    canonicalize_tool_arguments, local_finalize_allows_envelope,
    unwrap_local_finalize_response_value, LocalCoreSyncFinalizeOutcome,
};
use crate::gateway::{GatewayControlDecision, GatewayError, GatewaySyncReportRequest};

pub(crate) fn maybe_build_local_openai_cli_stream_sync_response(
    trace_id: &str,
    decision: &GatewayControlDecision,
    payload: &GatewaySyncReportRequest,
) -> Result<Option<LocalCoreSyncFinalizeOutcome>, GatewayError> {
    if let Some(response) =
        maybe_build_local_openai_cli_direct_stream_sync_response(trace_id, decision, payload)?
    {
        return Ok(Some(response));
    }
    if let Some(response) = maybe_build_local_openai_cli_openai_family_stream_sync_response(
        trace_id, decision, payload,
    )? {
        return Ok(Some(response));
    }
    maybe_build_local_openai_cli_direct_sync_response(trace_id, decision, payload)
}

pub(crate) fn maybe_build_local_openai_cli_cross_format_stream_sync_response(
    trace_id: &str,
    decision: &GatewayControlDecision,
    payload: &GatewaySyncReportRequest,
) -> Result<Option<LocalCoreSyncFinalizeOutcome>, GatewayError> {
    if let Some(response) =
        maybe_build_local_openai_cli_antigravity_cross_format_stream_sync_response(
            trace_id, decision, payload,
        )?
    {
        return Ok(Some(response));
    }

    if !matches!(
        payload.report_kind.as_str(),
        "openai_cli_sync_finalize" | "openai_compact_sync_finalize"
    ) || payload.status_code >= 400
    {
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
    let _has_envelope = report_context
        .get("has_envelope")
        .and_then(Value::as_bool)
        .unwrap_or(false);
    if !matches!(client_api_format.as_str(), "openai:cli" | "openai:compact")
        || !local_finalize_allows_envelope(report_context)
    {
        return Ok(None);
    }
    let Some(conversion_kind) =
        sync_cli_response_conversion_kind(&provider_api_format, &client_api_format)
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
        "openai:cli" | "openai:compact" => aggregate_openai_cli_stream_sync_response(&body_bytes),
        "claude:chat" | "claude:cli" => aggregate_claude_stream_sync_response(&body_bytes),
        "gemini:chat" | "gemini:cli" => aggregate_gemini_stream_sync_response(&body_bytes),
        _ => None,
    };
    let Some(aggregated) = aggregated else {
        return Ok(None);
    };
    let Some(aggregated) = unwrap_local_finalize_response_value(aggregated, report_context)? else {
        return Ok(None);
    };
    let converted = match provider_api_format.as_str() {
        "openai:cli" | "openai:compact" => Some(aggregated.clone()),
        "claude:chat" | "claude:cli" => {
            convert_claude_cli_response_to_openai_cli(&aggregated, report_context)
        }
        "gemini:chat" | "gemini:cli" => {
            convert_gemini_cli_response_to_openai_cli(&aggregated, report_context)
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

pub(crate) fn maybe_build_local_openai_cli_cross_format_sync_response(
    trace_id: &str,
    decision: &GatewayControlDecision,
    payload: &GatewaySyncReportRequest,
) -> Result<Option<LocalCoreSyncFinalizeOutcome>, GatewayError> {
    if let Some(response) = maybe_build_local_openai_cli_antigravity_cross_format_sync_response(
        trace_id, decision, payload,
    )? {
        return Ok(Some(response));
    }

    if !matches!(
        payload.report_kind.as_str(),
        "openai_cli_sync_finalize" | "openai_compact_sync_finalize"
    ) || payload.status_code >= 400
    {
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
    if !matches!(client_api_format.as_str(), "openai:cli" | "openai:compact")
        || !local_finalize_allows_envelope(report_context)
    {
        return Ok(None);
    }
    let Some(conversion_kind) =
        sync_cli_response_conversion_kind(&provider_api_format, &client_api_format)
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
        "openai:cli" | "openai:compact" => Some(body_json.clone()),
        "claude:chat" | "claude:cli" => {
            convert_claude_cli_response_to_openai_cli(&body_json, report_context)
        }
        "gemini:chat" | "gemini:cli" => {
            convert_gemini_cli_response_to_openai_cli(&body_json, report_context)
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

fn maybe_build_local_openai_cli_direct_sync_response(
    trace_id: &str,
    decision: &GatewayControlDecision,
    payload: &GatewaySyncReportRequest,
) -> Result<Option<LocalCoreSyncFinalizeOutcome>, GatewayError> {
    if !matches!(
        payload.report_kind.as_str(),
        "openai_cli_sync_finalize" | "openai_compact_sync_finalize"
    ) || payload.status_code >= 400
    {
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

    if !local_finalize_allows_envelope(report_context)
        || !is_openai_cli_family_api_format(provider_api_format.as_str())
        || !is_openai_cli_family_api_format(client_api_format.as_str())
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

fn maybe_build_local_openai_cli_direct_stream_sync_response(
    trace_id: &str,
    decision: &GatewayControlDecision,
    payload: &GatewaySyncReportRequest,
) -> Result<Option<LocalCoreSyncFinalizeOutcome>, GatewayError> {
    if !matches!(
        payload.report_kind.as_str(),
        "openai_cli_sync_finalize" | "openai_compact_sync_finalize"
    ) || payload.status_code >= 400
    {
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
    if !matches!(
        provider_api_format.as_str(),
        "openai:cli" | "openai:compact"
    ) || provider_api_format != client_api_format
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
    let body_json = match aggregate_openai_cli_stream_sync_response(&body_bytes) {
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

fn maybe_build_local_openai_cli_openai_family_stream_sync_response(
    trace_id: &str,
    decision: &GatewayControlDecision,
    payload: &GatewaySyncReportRequest,
) -> Result<Option<LocalCoreSyncFinalizeOutcome>, GatewayError> {
    if !matches!(
        payload.report_kind.as_str(),
        "openai_cli_sync_finalize" | "openai_compact_sync_finalize"
    ) || payload.status_code >= 400
    {
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
    if !local_finalize_allows_envelope(report_context)
        || !is_openai_cli_family_api_format(provider_api_format.as_str())
        || !is_openai_cli_family_api_format(client_api_format.as_str())
        || provider_api_format == client_api_format
    {
        return Ok(None);
    }

    let Some(body_base64) = payload.body_base64.as_deref() else {
        return Ok(None);
    };
    let body_bytes = base64::engine::general_purpose::STANDARD
        .decode(body_base64)
        .map_err(|err| GatewayError::Internal(err.to_string()))?;
    let Some(body_json) = aggregate_openai_cli_stream_sync_response(&body_bytes) else {
        return Ok(None);
    };
    let Some(body_json) = unwrap_local_finalize_response_value(body_json, report_context)? else {
        return Ok(None);
    };

    Ok(Some(build_local_success_outcome(
        trace_id, decision, payload, body_json,
    )?))
}

fn maybe_build_local_openai_cli_antigravity_cross_format_stream_sync_response(
    trace_id: &str,
    decision: &GatewayControlDecision,
    payload: &GatewaySyncReportRequest,
) -> Result<Option<LocalCoreSyncFinalizeOutcome>, GatewayError> {
    if !matches!(
        payload.report_kind.as_str(),
        "openai_cli_sync_finalize" | "openai_compact_sync_finalize"
    ) || payload.status_code >= 400
    {
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
    if provider_api_format != "gemini:cli"
        || !is_openai_cli_family_api_format(client_api_format.as_str())
        || !is_antigravity_v1internal_envelope(report_context)
        || !local_finalize_allows_envelope(report_context)
    {
        return Ok(None);
    }

    let Some(body_base64) = payload.body_base64.as_deref() else {
        return Ok(None);
    };
    let body_bytes = base64::engine::general_purpose::STANDARD
        .decode(body_base64)
        .map_err(|err| GatewayError::Internal(err.to_string()))?;
    let Some(aggregated) = aggregate_gemini_stream_sync_response(&body_bytes) else {
        return Ok(None);
    };
    let Some(provider_body_json) =
        unwrap_cli_conversion_response_value(aggregated, report_context)?
    else {
        return Ok(None);
    };
    let Some(converted) =
        convert_gemini_cli_response_to_openai_cli(&provider_body_json, report_context)
    else {
        return Ok(None);
    };

    Ok(Some(build_local_success_outcome_with_conversion_report(
        trace_id,
        decision,
        payload,
        converted,
        provider_body_json,
    )?))
}

fn maybe_build_local_openai_cli_antigravity_cross_format_sync_response(
    trace_id: &str,
    decision: &GatewayControlDecision,
    payload: &GatewaySyncReportRequest,
) -> Result<Option<LocalCoreSyncFinalizeOutcome>, GatewayError> {
    if !matches!(
        payload.report_kind.as_str(),
        "openai_cli_sync_finalize" | "openai_compact_sync_finalize"
    ) || payload.status_code >= 400
    {
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
    if provider_api_format != "gemini:cli"
        || !is_openai_cli_family_api_format(client_api_format.as_str())
        || !is_antigravity_v1internal_envelope(report_context)
        || !local_finalize_allows_envelope(report_context)
    {
        return Ok(None);
    }

    let Some(body_json) = payload.body_json.as_ref() else {
        return Ok(None);
    };
    let Some(provider_body_json) =
        unwrap_cli_conversion_response_value(body_json.clone(), report_context)?
    else {
        return Ok(None);
    };
    let Some(converted) =
        convert_gemini_cli_response_to_openai_cli(&provider_body_json, report_context)
    else {
        return Ok(None);
    };

    Ok(Some(build_local_success_outcome_with_conversion_report(
        trace_id,
        decision,
        payload,
        converted,
        provider_body_json,
    )?))
}

fn unwrap_cli_conversion_response_value(
    data: Value,
    report_context: &Value,
) -> Result<Option<Value>, GatewayError> {
    if !is_antigravity_v1internal_envelope(report_context) {
        return unwrap_local_finalize_response_value(data, report_context);
    }

    let mut unwrapped = if let Some(response) = data
        .get("response")
        .and_then(Value::as_object)
        .filter(|response| !response.contains_key("response"))
    {
        let mut response = response.clone();
        if let Some(response_id) = data.get("responseId").cloned() {
            response
                .entry("responseId".to_string())
                .or_insert(response_id);
        }
        Value::Object(response)
    } else {
        data
    };

    if let Some(object) = unwrapped.as_object_mut() {
        if !object.contains_key("responseId") {
            if let Some(response_id) = object.get("_v1internal_response_id").cloned() {
                object.insert("responseId".to_string(), response_id);
            }
        }
    }

    Ok(Some(unwrapped))
}

fn is_antigravity_v1internal_envelope(report_context: &Value) -> bool {
    report_context
        .get("has_envelope")
        .and_then(Value::as_bool)
        .unwrap_or(false)
        && report_context
            .get("envelope_name")
            .and_then(Value::as_str)
            .is_some_and(|value| value.eq_ignore_ascii_case("antigravity:v1internal"))
}

fn is_openai_cli_family_api_format(api_format: &str) -> bool {
    matches!(api_format, "openai:cli" | "openai:compact")
}

pub(crate) fn build_openai_cli_response(
    response_id: &str,
    model: &str,
    text: &str,
    function_calls: Vec<Value>,
    prompt_tokens: u64,
    output_tokens: u64,
    total_tokens: u64,
) -> Value {
    let mut output = Vec::new();
    if !text.is_empty() {
        output.push(json!({
            "type": "message",
            "id": format!("{response_id}_msg"),
            "role": "assistant",
            "status": "completed",
            "content": [{
                "type": "output_text",
                "text": text,
                "annotations": []
            }]
        }));
    }
    output.extend(function_calls);
    json!({
        "id": response_id,
        "object": "response",
        "status": "completed",
        "model": model,
        "output": output,
        "usage": {
            "input_tokens": prompt_tokens,
            "output_tokens": output_tokens,
            "total_tokens": total_tokens,
        }
    })
}

pub(crate) fn convert_openai_chat_response_to_openai_cli(
    body_json: &Value,
    report_context: &Value,
    compact: bool,
) -> Option<Value> {
    let body = body_json.as_object()?;
    let choices = body.get("choices")?.as_array()?;
    let first_choice = choices.first()?.as_object()?;
    let message = first_choice.get("message")?.as_object()?;
    let mut text = String::new();
    match message.get("content") {
        Some(Value::String(value)) => text.push_str(value),
        Some(Value::Array(parts)) => {
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
        }
        Some(Value::Null) | None => {}
        _ => return None,
    }

    let mut function_calls = Vec::new();
    if let Some(tool_call_values) = message.get("tool_calls").and_then(Value::as_array) {
        for tool_call in tool_call_values {
            let tool_call = tool_call.as_object()?;
            let function = tool_call.get("function")?.as_object()?;
            let tool_name = function
                .get("name")
                .and_then(Value::as_str)
                .map(str::trim)
                .filter(|value| !value.is_empty())?;
            function_calls.push(json!({
                "type": "function_call",
                "id": tool_call.get("id").cloned().unwrap_or(Value::Null),
                "call_id": tool_call.get("id").cloned().unwrap_or(Value::Null),
                "name": tool_name,
                "arguments": canonicalize_tool_arguments(function.get("arguments").cloned()),
            }));
        }
    }

    let usage = body.get("usage").and_then(Value::as_object);
    let prompt_tokens = usage
        .and_then(|value| value.get("prompt_tokens"))
        .and_then(Value::as_u64)
        .unwrap_or(0);
    let output_tokens = usage
        .and_then(|value| value.get("completion_tokens"))
        .and_then(Value::as_u64)
        .unwrap_or(0);
    let total_tokens = usage
        .and_then(|value| value.get("total_tokens"))
        .and_then(Value::as_u64)
        .unwrap_or(prompt_tokens + output_tokens);
    let response_id = if compact {
        body.get("id")
            .and_then(Value::as_str)
            .map(|value| value.replace("chatcmpl", "resp"))
            .unwrap_or_else(|| "resp-local-finalize".to_string())
    } else {
        body.get("id")
            .and_then(Value::as_str)
            .map(|value| value.replace("chatcmpl", "resp"))
            .unwrap_or_else(|| "resp-local-finalize".to_string())
    };
    let model = body
        .get("model")
        .and_then(Value::as_str)
        .or_else(|| report_context.get("mapped_model").and_then(Value::as_str))
        .or_else(|| report_context.get("model").and_then(Value::as_str))
        .unwrap_or("unknown");

    Some(build_openai_cli_response(
        &response_id,
        model,
        &text,
        function_calls,
        prompt_tokens,
        output_tokens,
        total_tokens,
    ))
}

pub(crate) fn aggregate_openai_cli_stream_sync_response(body: &[u8]) -> Option<Value> {
    let text = std::str::from_utf8(body).ok()?;

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

        let event: Value = serde_json::from_str(data_line).ok()?;
        let event_type = event
            .get("type")
            .and_then(Value::as_str)
            .unwrap_or_default();
        if event_type == "response.completed" {
            let response = event.get("response")?.as_object()?.clone();
            return Some(Value::Object(response));
        }
    }

    None
}
