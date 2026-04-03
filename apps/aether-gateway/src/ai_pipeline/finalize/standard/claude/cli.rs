use base64::Engine as _;

use super::aggregate_claude_stream_sync_response;
use serde_json::{json, Value};

use crate::gateway::ai_pipeline::finalize::common::{
    build_generated_tool_call_id, build_local_success_outcome, canonicalize_tool_arguments,
    local_finalize_allows_envelope, unwrap_local_finalize_response_value,
    LocalCoreSyncFinalizeOutcome,
};
use crate::gateway::ai_pipeline::finalize::standard::build_openai_cli_response;
use crate::gateway::{GatewayControlDecision, GatewayError, GatewaySyncReportRequest};

pub(crate) fn maybe_build_local_claude_cli_stream_sync_response(
    trace_id: &str,
    decision: &GatewayControlDecision,
    payload: &GatewaySyncReportRequest,
) -> Result<Option<LocalCoreSyncFinalizeOutcome>, GatewayError> {
    if payload.report_kind != "claude_cli_sync_finalize" || payload.status_code >= 400 {
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
    if provider_api_format != "claude:cli" || client_api_format != "claude:cli" || needs_conversion
    {
        return Ok(None);
    }

    let Some(body_base64) = payload.body_base64.as_deref() else {
        return Ok(None);
    };
    let body_bytes = base64::engine::general_purpose::STANDARD
        .decode(body_base64)
        .map_err(|err| GatewayError::Internal(err.to_string()))?;
    let body_json =
        match aggregate_provider_claude_cli_stream_sync_response(&body_bytes, report_context)? {
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

fn aggregate_provider_claude_cli_stream_sync_response(
    body_bytes: &[u8],
    _report_context: &Value,
) -> Result<Option<Value>, GatewayError> {
    Ok(aggregate_claude_stream_sync_response(body_bytes))
}

pub(crate) fn convert_claude_cli_response_to_openai_cli(
    body_json: &Value,
    report_context: &Value,
) -> Option<Value> {
    let body = body_json.as_object()?;
    let content = body.get("content")?.as_array()?;
    let mut text = String::new();
    let mut function_calls = Vec::new();
    for (index, block) in content.iter().enumerate() {
        let block = block.as_object()?;
        match block.get("type")?.as_str()? {
            "text" => {
                text.push_str(block.get("text")?.as_str()?);
            }
            "tool_use" => {
                let tool_name = block.get("name")?.as_str()?;
                let call_id = block
                    .get("id")
                    .and_then(Value::as_str)
                    .filter(|value| !value.is_empty())
                    .map(ToOwned::to_owned)
                    .unwrap_or_else(|| build_generated_tool_call_id(index));
                let arguments = canonicalize_tool_arguments(block.get("input").cloned());
                function_calls.push(json!({
                    "type": "function_call",
                    "call_id": call_id,
                    "name": tool_name,
                    "arguments": arguments,
                }));
            }
            _ => return None,
        }
    }

    let usage = body.get("usage").and_then(Value::as_object);
    let prompt_tokens = usage
        .and_then(|value| value.get("input_tokens"))
        .and_then(Value::as_u64)
        .unwrap_or(0);
    let output_tokens = usage
        .and_then(|value| value.get("output_tokens"))
        .and_then(Value::as_u64)
        .unwrap_or(0);
    let total_tokens = prompt_tokens + output_tokens;
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

    Some(build_openai_cli_response(
        response_id,
        model,
        &text,
        function_calls,
        prompt_tokens,
        output_tokens,
        total_tokens,
    ))
}
