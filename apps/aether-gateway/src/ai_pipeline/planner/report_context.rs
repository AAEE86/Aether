use std::collections::BTreeMap;

use serde_json::{Map, Value};

use crate::ai_pipeline::contracts::ExecutionRuntimeAuthContext;

pub(crate) struct LocalExecutionReportContextParts<'a> {
    pub(crate) auth_context: &'a ExecutionRuntimeAuthContext,
    pub(crate) request_id: &'a str,
    pub(crate) candidate_id: &'a str,
    pub(crate) candidate_index: u32,
    pub(crate) retry_index: u32,
    pub(crate) model: &'a str,
    pub(crate) provider_name: &'a str,
    pub(crate) provider_id: &'a str,
    pub(crate) endpoint_id: &'a str,
    pub(crate) key_id: &'a str,
    pub(crate) key_name: Option<&'a str>,
    pub(crate) provider_api_format: &'a str,
    pub(crate) client_api_format: &'a str,
    pub(crate) mapped_model: Option<&'a str>,
    pub(crate) upstream_url: Option<&'a str>,
    pub(crate) provider_request_method: Option<Value>,
    pub(crate) provider_request_headers: Option<&'a BTreeMap<String, String>>,
    pub(crate) original_headers: &'a http::HeaderMap,
    pub(crate) original_request_body: &'a Value,
    pub(crate) has_envelope: bool,
    pub(crate) needs_conversion: bool,
    pub(crate) extra_fields: Map<String, Value>,
}

pub(crate) fn build_local_execution_report_context(
    parts: LocalExecutionReportContextParts<'_>,
) -> Value {
    let mut object = Map::new();
    object.insert(
        "user_id".to_string(),
        Value::String(parts.auth_context.user_id.clone()),
    );
    object.insert(
        "api_key_id".to_string(),
        Value::String(parts.auth_context.api_key_id.clone()),
    );
    object.insert(
        "username".to_string(),
        parts
            .auth_context
            .username
            .clone()
            .map(Value::String)
            .unwrap_or(Value::Null),
    );
    object.insert(
        "api_key_name".to_string(),
        parts
            .auth_context
            .api_key_name
            .clone()
            .map(Value::String)
            .unwrap_or(Value::Null),
    );
    object.insert(
        "request_id".to_string(),
        Value::String(parts.request_id.to_string()),
    );
    object.insert(
        "candidate_id".to_string(),
        Value::String(parts.candidate_id.to_string()),
    );
    object.insert(
        "candidate_index".to_string(),
        Value::Number(parts.candidate_index.into()),
    );
    object.insert(
        "retry_index".to_string(),
        Value::Number(parts.retry_index.into()),
    );
    object.insert("model".to_string(), Value::String(parts.model.to_string()));
    object.insert(
        "provider_name".to_string(),
        Value::String(parts.provider_name.to_string()),
    );
    object.insert(
        "provider_id".to_string(),
        Value::String(parts.provider_id.to_string()),
    );
    object.insert(
        "endpoint_id".to_string(),
        Value::String(parts.endpoint_id.to_string()),
    );
    object.insert(
        "key_id".to_string(),
        Value::String(parts.key_id.to_string()),
    );
    object.insert(
        "provider_api_format".to_string(),
        Value::String(parts.provider_api_format.to_string()),
    );
    object.insert(
        "client_api_format".to_string(),
        Value::String(parts.client_api_format.to_string()),
    );
    object.insert(
        "original_headers".to_string(),
        serde_json::to_value(crate::ai_pipeline::collect_control_headers(
            parts.original_headers,
        ))
        .expect("control headers should serialize"),
    );
    object.insert(
        "original_request_body".to_string(),
        crate::ai_pipeline::build_report_context_original_request_echo(parts.original_request_body)
            .unwrap_or(Value::Null),
    );
    object.insert("has_envelope".to_string(), Value::Bool(parts.has_envelope));
    object.insert(
        "needs_conversion".to_string(),
        Value::Bool(parts.needs_conversion),
    );

    if let Some(key_name) = parts.key_name {
        object.insert("key_name".to_string(), Value::String(key_name.to_string()));
    }
    if let Some(mapped_model) = parts.mapped_model {
        object.insert(
            "mapped_model".to_string(),
            Value::String(mapped_model.to_string()),
        );
    }
    if let Some(upstream_url) = parts.upstream_url {
        object.insert(
            "upstream_url".to_string(),
            Value::String(upstream_url.to_string()),
        );
    }
    if let Some(provider_request_method) = parts.provider_request_method {
        object.insert(
            "provider_request_method".to_string(),
            provider_request_method,
        );
    }
    if let Some(provider_request_headers) = parts.provider_request_headers {
        object.insert(
            "provider_request_headers".to_string(),
            serde_json::to_value(provider_request_headers)
                .expect("provider request headers should serialize"),
        );
    }

    object.extend(parts.extra_fields);
    Value::Object(object)
}
