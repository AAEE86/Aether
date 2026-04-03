use std::collections::BTreeMap;

use serde_json::{json, Map, Value};
use url::form_urlencoded;

use crate::gateway::ai_pipeline::conversion::request::{
    convert_openai_chat_request_to_claude_request, convert_openai_chat_request_to_gemini_request,
    convert_openai_chat_request_to_openai_cli_request, extract_openai_text_content,
    normalize_openai_cli_request_to_openai_chat_request, parse_openai_tool_result_content,
};
use crate::gateway::ai_pipeline::conversion::{request_conversion_kind, RequestConversionKind};
use crate::gateway::provider_transport::{
    apply_local_body_rules, build_antigravity_v1internal_url, build_claude_messages_url,
    build_gemini_content_url, build_openai_chat_url, build_openai_cli_url,
    build_passthrough_path_url, AntigravityRequestUrlAction,
};

pub(crate) fn build_local_openai_chat_request_body(
    body_json: &Value,
    mapped_model: &str,
    upstream_is_stream: bool,
    body_rules: Option<&Value>,
) -> Option<Value> {
    let request_body_object = body_json.as_object()?;
    let mut provider_request_body = serde_json::Map::from_iter(
        request_body_object
            .iter()
            .map(|(key, value)| (key.clone(), value.clone())),
    );
    provider_request_body.insert("model".to_string(), Value::String(mapped_model.to_string()));
    if upstream_is_stream {
        provider_request_body.insert("stream".to_string(), Value::Bool(true));
    }
    let mut provider_request_body = Value::Object(provider_request_body);
    if !apply_local_body_rules(&mut provider_request_body, body_rules, Some(body_json)) {
        return None;
    }
    Some(provider_request_body)
}

pub(crate) fn build_local_openai_chat_upstream_url(
    parts: &http::request::Parts,
    transport: &crate::gateway::provider_transport::GatewayProviderTransportSnapshot,
) -> Option<String> {
    let custom_path = transport
        .endpoint
        .custom_path
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty());

    match custom_path {
        Some(path) => {
            build_passthrough_path_url(&transport.endpoint.base_url, path, parts.uri.query(), &[])
        }
        None => Some(build_openai_chat_url(
            &transport.endpoint.base_url,
            parts.uri.query(),
        )),
    }
}

pub(crate) fn build_cross_format_openai_chat_request_body(
    body_json: &Value,
    mapped_model: &str,
    provider_api_format: &str,
    upstream_is_stream: bool,
    body_rules: Option<&Value>,
) -> Option<Value> {
    let conversion_kind = request_conversion_kind("openai:chat", provider_api_format)?;
    let mut provider_request_body = match conversion_kind {
        RequestConversionKind::ToClaudeStandard => convert_openai_chat_request_to_claude_request(
            body_json,
            mapped_model,
            upstream_is_stream,
        )?,
        RequestConversionKind::ToGeminiStandard => convert_openai_chat_request_to_gemini_request(
            body_json,
            mapped_model,
            upstream_is_stream,
        )?,
        RequestConversionKind::ToOpenAIFamilyCli => {
            convert_openai_chat_request_to_openai_cli_request(
                body_json,
                mapped_model,
                upstream_is_stream,
                false,
            )?
        }
        RequestConversionKind::ToOpenAICompact => {
            convert_openai_chat_request_to_openai_cli_request(body_json, mapped_model, false, true)?
        }
        _ => return None,
    };

    if !apply_local_body_rules(&mut provider_request_body, body_rules, Some(body_json)) {
        return None;
    }
    Some(provider_request_body)
}

pub(crate) fn build_cross_format_openai_chat_upstream_url(
    parts: &http::request::Parts,
    transport: &crate::gateway::provider_transport::GatewayProviderTransportSnapshot,
    mapped_model: &str,
    provider_api_format: &str,
    upstream_is_stream: bool,
) -> Option<String> {
    let conversion_kind = request_conversion_kind("openai:chat", provider_api_format)?;
    let custom_path = transport
        .endpoint
        .custom_path
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty());

    match custom_path {
        Some(path) => {
            build_passthrough_path_url(&transport.endpoint.base_url, path, parts.uri.query(), &[])
        }
        None => match conversion_kind {
            RequestConversionKind::ToClaudeStandard => Some(build_claude_messages_url(
                &transport.endpoint.base_url,
                parts.uri.query(),
            )),
            RequestConversionKind::ToGeminiStandard => build_gemini_content_url(
                &transport.endpoint.base_url,
                mapped_model,
                upstream_is_stream,
                parts.uri.query(),
            ),
            RequestConversionKind::ToOpenAIFamilyCli => Some(build_openai_cli_url(
                &transport.endpoint.base_url,
                parts.uri.query(),
                false,
            )),
            RequestConversionKind::ToOpenAICompact => Some(build_openai_cli_url(
                &transport.endpoint.base_url,
                parts.uri.query(),
                true,
            )),
            _ => None,
        },
    }
}

pub(crate) fn build_local_openai_cli_request_body(
    body_json: &Value,
    mapped_model: &str,
    require_streaming: bool,
    body_rules: Option<&Value>,
) -> Option<Value> {
    let request_body_object = body_json.as_object()?;
    let mut provider_request_body = serde_json::Map::from_iter(
        request_body_object
            .iter()
            .map(|(key, value)| (key.clone(), value.clone())),
    );
    provider_request_body.insert("model".to_string(), Value::String(mapped_model.to_string()));
    if require_streaming {
        provider_request_body.insert("stream".to_string(), Value::Bool(true));
    }
    let mut provider_request_body = Value::Object(provider_request_body);
    if !apply_local_body_rules(&mut provider_request_body, body_rules, Some(body_json)) {
        return None;
    }
    Some(provider_request_body)
}

pub(crate) fn build_cross_format_openai_cli_request_body(
    body_json: &Value,
    mapped_model: &str,
    client_api_format: &str,
    provider_api_format: &str,
    upstream_is_stream: bool,
    body_rules: Option<&Value>,
) -> Option<Value> {
    let chat_like_request = normalize_openai_cli_request_to_openai_chat_request(body_json)?;
    let conversion_kind = request_conversion_kind(client_api_format, provider_api_format)?;
    let mut provider_request_body = match conversion_kind {
        RequestConversionKind::ToOpenAIFamilyCli => {
            convert_openai_chat_request_to_openai_cli_request(
                &chat_like_request,
                mapped_model,
                upstream_is_stream,
                false,
            )?
        }
        RequestConversionKind::ToOpenAICompact => {
            convert_openai_chat_request_to_openai_cli_request(
                &chat_like_request,
                mapped_model,
                false,
                true,
            )?
        }
        RequestConversionKind::ToClaudeStandard => convert_openai_chat_request_to_claude_request(
            &chat_like_request,
            mapped_model,
            upstream_is_stream,
        )?,
        RequestConversionKind::ToGeminiStandard => convert_openai_chat_request_to_gemini_request(
            &chat_like_request,
            mapped_model,
            upstream_is_stream,
        )?,
        _ => return None,
    };
    if !apply_local_body_rules(&mut provider_request_body, body_rules, Some(body_json)) {
        return None;
    }
    Some(provider_request_body)
}

pub(crate) fn build_local_openai_cli_upstream_url(
    parts: &http::request::Parts,
    transport: &crate::gateway::provider_transport::GatewayProviderTransportSnapshot,
    compact: bool,
) -> Option<String> {
    let custom_path = transport
        .endpoint
        .custom_path
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty());

    match custom_path {
        Some(path) => {
            build_passthrough_path_url(&transport.endpoint.base_url, path, parts.uri.query(), &[])
        }
        None => Some(build_openai_cli_url(
            &transport.endpoint.base_url,
            parts.uri.query(),
            compact,
        )),
    }
}

pub(crate) fn build_cross_format_openai_cli_upstream_url(
    parts: &http::request::Parts,
    transport: &crate::gateway::provider_transport::GatewayProviderTransportSnapshot,
    mapped_model: &str,
    client_api_format: &str,
    provider_api_format: &str,
    upstream_is_stream: bool,
) -> Option<String> {
    let conversion_kind = request_conversion_kind(client_api_format, provider_api_format)?;
    if transport
        .provider
        .provider_type
        .trim()
        .eq_ignore_ascii_case("antigravity")
    {
        let query = parts.uri.query().map(|query| {
            form_urlencoded::parse(query.as_bytes())
                .into_owned()
                .collect::<BTreeMap<String, String>>()
        });
        return build_antigravity_v1internal_url(
            &transport.endpoint.base_url,
            if upstream_is_stream {
                AntigravityRequestUrlAction::StreamGenerateContent
            } else {
                AntigravityRequestUrlAction::GenerateContent
            },
            query.as_ref(),
        );
    }

    let custom_path = transport
        .endpoint
        .custom_path
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty());

    match custom_path {
        Some(path) => {
            build_passthrough_path_url(&transport.endpoint.base_url, path, parts.uri.query(), &[])
        }
        None => match conversion_kind {
            RequestConversionKind::ToOpenAIFamilyCli => Some(build_openai_cli_url(
                &transport.endpoint.base_url,
                parts.uri.query(),
                false,
            )),
            RequestConversionKind::ToOpenAICompact => Some(build_openai_cli_url(
                &transport.endpoint.base_url,
                parts.uri.query(),
                true,
            )),
            RequestConversionKind::ToClaudeStandard => Some(build_claude_messages_url(
                &transport.endpoint.base_url,
                parts.uri.query(),
            )),
            RequestConversionKind::ToGeminiStandard => build_gemini_content_url(
                &transport.endpoint.base_url,
                mapped_model,
                upstream_is_stream,
                parts.uri.query(),
            ),
            _ => None,
        },
    }
}

#[cfg(test)]
mod tests {
    use super::build_cross_format_openai_cli_request_body;
    use serde_json::json;

    #[test]
    fn builds_openai_family_cross_format_request_body_from_compact_source() {
        let body_json = json!({
            "model": "gpt-5",
            "input": "hello",
        });

        let provider_request_body = build_cross_format_openai_cli_request_body(
            &body_json,
            "gpt-5-upstream",
            "openai:compact",
            "openai:cli",
            false,
            None,
        )
        .expect("compact to openai cli body should build");

        assert_eq!(provider_request_body["model"], "gpt-5-upstream");
        assert_eq!(provider_request_body["input"][0]["type"], "message");
        assert_eq!(provider_request_body["input"][0]["role"], "user");
    }
}
