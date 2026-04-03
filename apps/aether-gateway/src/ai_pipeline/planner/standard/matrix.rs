use std::collections::BTreeMap;

use serde_json::{json, Map, Value};

use super::{
    claude::normalize_claude_request_to_openai_chat_request,
    gemini::normalize_gemini_request_to_openai_chat_request,
};
use crate::gateway::ai_pipeline::conversion::request::{
    convert_openai_chat_request_to_claude_request, convert_openai_chat_request_to_gemini_request,
    convert_openai_chat_request_to_openai_cli_request,
    normalize_openai_cli_request_to_openai_chat_request,
};
use crate::gateway::ai_pipeline::conversion::{request_conversion_kind, RequestConversionKind};
use crate::gateway::provider_transport::{
    apply_local_body_rules, build_claude_messages_url, build_gemini_content_url,
    build_openai_chat_url, build_openai_cli_url, build_passthrough_path_url,
};

pub(crate) fn build_standard_request_body(
    body_json: &Value,
    client_api_format: &str,
    mapped_model: &str,
    provider_api_format: &str,
    request_path: &str,
    upstream_is_stream: bool,
    body_rules: Option<&Value>,
) -> Option<Value> {
    let canonical_request = normalize_standard_request_to_openai_chat_request(
        body_json,
        client_api_format,
        request_path,
    )?;
    let conversion_kind = request_conversion_kind(client_api_format, provider_api_format)?;
    let mut provider_request_body = match conversion_kind {
        RequestConversionKind::ToOpenAIChat => {
            build_openai_chat_request_body(&canonical_request, mapped_model, upstream_is_stream)?
        }
        RequestConversionKind::ToOpenAIFamilyCli => {
            convert_openai_chat_request_to_openai_cli_request(
                &canonical_request,
                mapped_model,
                upstream_is_stream,
                false,
            )?
        }
        RequestConversionKind::ToOpenAICompact => {
            convert_openai_chat_request_to_openai_cli_request(
                &canonical_request,
                mapped_model,
                false,
                true,
            )?
        }
        RequestConversionKind::ToClaudeStandard => convert_openai_chat_request_to_claude_request(
            &canonical_request,
            mapped_model,
            upstream_is_stream,
        )?,
        RequestConversionKind::ToGeminiStandard => convert_openai_chat_request_to_gemini_request(
            &canonical_request,
            mapped_model,
            upstream_is_stream,
        )?,
    };

    if !apply_local_body_rules(&mut provider_request_body, body_rules, Some(body_json)) {
        return None;
    }
    Some(provider_request_body)
}

pub(crate) fn build_standard_upstream_url(
    parts: &http::request::Parts,
    transport: &crate::gateway::provider_transport::GatewayProviderTransportSnapshot,
    mapped_model: &str,
    provider_api_format: &str,
    upstream_is_stream: bool,
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
        None => match provider_api_format.trim().to_ascii_lowercase().as_str() {
            "openai:chat" => Some(build_openai_chat_url(
                &transport.endpoint.base_url,
                parts.uri.query(),
            )),
            "openai:cli" => Some(build_openai_cli_url(
                &transport.endpoint.base_url,
                parts.uri.query(),
                false,
            )),
            "openai:compact" => Some(build_openai_cli_url(
                &transport.endpoint.base_url,
                parts.uri.query(),
                true,
            )),
            "claude:chat" | "claude:cli" => Some(build_claude_messages_url(
                &transport.endpoint.base_url,
                parts.uri.query(),
            )),
            "gemini:chat" | "gemini:cli" => build_gemini_content_url(
                &transport.endpoint.base_url,
                mapped_model,
                upstream_is_stream,
                parts.uri.query(),
            ),
            _ => None,
        },
    }
}

pub(crate) fn normalize_standard_request_to_openai_chat_request(
    body_json: &Value,
    client_api_format: &str,
    request_path: &str,
) -> Option<Value> {
    match client_api_format.trim().to_ascii_lowercase().as_str() {
        "openai:chat" => Some(body_json.clone()),
        "openai:cli" | "openai:compact" => {
            normalize_openai_cli_request_to_openai_chat_request(body_json)
        }
        "claude:chat" | "claude:cli" => normalize_claude_request_to_openai_chat_request(body_json),
        "gemini:chat" | "gemini:cli" => {
            normalize_gemini_request_to_openai_chat_request(body_json, request_path)
        }
        _ => None,
    }
}

fn build_openai_chat_request_body(
    body_json: &Value,
    mapped_model: &str,
    upstream_is_stream: bool,
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
    Some(Value::Object(provider_request_body))
}

#[cfg(test)]
mod tests {
    use super::build_standard_request_body;
    use serde_json::json;

    #[test]
    fn builds_openai_chat_request_from_claude_chat_source() {
        let request = json!({
            "model": "claude-3-7-sonnet",
            "system": "You are concise.",
            "messages": [
                {
                    "role": "user",
                    "content": [{"type": "text", "text": "Hello from Claude"}]
                }
            ],
            "max_tokens": 128
        });

        let converted = build_standard_request_body(
            &request,
            "claude:chat",
            "gpt-5",
            "openai:chat",
            "/v1/messages",
            false,
            None,
        )
        .expect("claude chat should convert to openai chat");

        assert_eq!(converted["model"], "gpt-5");
        assert_eq!(converted["messages"][0]["role"], "system");
        assert_eq!(converted["messages"][0]["content"], "You are concise.");
        assert_eq!(converted["messages"][1]["role"], "user");
        assert_eq!(converted["messages"][1]["content"], "Hello from Claude");
    }

    #[test]
    fn builds_claude_chat_request_from_gemini_chat_source() {
        let request = json!({
            "systemInstruction": {
                "parts": [{"text": "Be brief."}]
            },
            "contents": [
                {
                    "role": "user",
                    "parts": [{"text": "Hello from Gemini"}]
                }
            ]
        });

        let converted = build_standard_request_body(
            &request,
            "gemini:chat",
            "claude-sonnet-4-5",
            "claude:chat",
            "/v1beta/models/gemini-2.5-pro:generateContent",
            false,
            None,
        )
        .expect("gemini chat should convert to claude chat");

        assert_eq!(converted["model"], "claude-sonnet-4-5");
        assert_eq!(converted["messages"][0]["role"], "user");
        assert!(
            converted["messages"]
                .to_string()
                .contains("Hello from Gemini"),
            "converted claude payload should retain the gemini user text: {converted}"
        );
    }

    #[test]
    fn builds_gemini_cli_request_from_claude_cli_source() {
        let request = json!({
            "model": "claude-sonnet-4-5",
            "messages": [
                {
                    "role": "user",
                    "content": [{"type": "text", "text": "Need CLI output"}]
                }
            ],
            "max_tokens": 64
        });

        let converted = build_standard_request_body(
            &request,
            "claude:cli",
            "gemini-2.5-pro",
            "gemini:cli",
            "/v1/messages",
            false,
            None,
        )
        .expect("claude cli should convert to gemini cli");

        assert_eq!(converted["contents"][0]["role"], "user");
        assert_eq!(
            converted["contents"][0]["parts"][0]["text"],
            "Need CLI output"
        );
    }
}
