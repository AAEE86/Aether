use serde_json::Value;

use super::super::{
    apply_local_body_rules, build_kiro_provider_request_body, sanitize_claude_code_request_body,
    LocalSameFormatProviderFamily, LocalSameFormatProviderSpec,
};

pub(crate) fn build_same_format_provider_request_body(
    body_json: &Value,
    mapped_model: &str,
    spec: LocalSameFormatProviderSpec,
    body_rules: Option<&Value>,
    upstream_is_stream: bool,
    kiro_auth: Option<&crate::ai_pipeline::transport::kiro::KiroRequestAuth>,
    is_claude_code: bool,
) -> Option<Value> {
    if let Some(kiro_auth) = kiro_auth {
        return build_kiro_provider_request_body(
            body_json,
            mapped_model,
            &kiro_auth.auth_config,
            body_rules,
        );
    }

    let request_body_object = body_json.as_object()?;
    let mut provider_request_body = serde_json::Map::from_iter(
        request_body_object
            .iter()
            .map(|(key, value)| (key.clone(), value.clone())),
    );
    match spec.family {
        LocalSameFormatProviderFamily::Standard => {
            provider_request_body
                .insert("model".to_string(), Value::String(mapped_model.to_string()));
            if upstream_is_stream {
                provider_request_body.insert("stream".to_string(), Value::Bool(true));
            }
        }
        LocalSameFormatProviderFamily::Gemini => {
            provider_request_body.remove("model");
        }
    }
    let mut provider_request_body = Value::Object(provider_request_body);
    if is_claude_code {
        sanitize_claude_code_request_body(&mut provider_request_body);
    }
    if !apply_local_body_rules(&mut provider_request_body, body_rules, Some(body_json)) {
        return None;
    }
    Some(provider_request_body)
}
