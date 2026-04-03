use std::collections::BTreeMap;

use aether_contracts::ExecutionPlan;
use aether_data::repository::video_tasks::{
    StoredVideoTask, VideoTaskStatus as StoredVideoTaskStatus,
};
use serde_json::{Map, Value};

use super::super::{
    LocalVideoTaskFollowUpPlan, LocalVideoTaskPersistence, LocalVideoTaskStatus,
    LocalVideoTaskTransport,
};
use crate::gateway::provider_transport::{
    resolve_local_gemini_auth, resolve_local_standard_auth, resolve_transport_execution_timeouts,
    supports_local_gemini_transport, supports_local_standard_transport,
    GatewayProviderTransportSnapshot,
};
use crate::gateway::GatewayControlAuthContext;

use super::{context_text, non_empty_owned};

impl LocalVideoTaskTransport {
    pub(crate) fn from_plan(plan: &ExecutionPlan) -> Option<Self> {
        let upstream_base_url = match plan.provider_api_format.as_str() {
            "openai:video" => plan.url.split("/v1/videos").next()?.to_string(),
            "gemini:video" => plan.url.split("/v1beta/").next()?.to_string(),
            _ => return None,
        };
        if upstream_base_url.is_empty() {
            return None;
        }
        Some(Self {
            upstream_base_url,
            provider_name: plan.provider_name.clone(),
            provider_id: plan.provider_id.clone(),
            endpoint_id: plan.endpoint_id.clone(),
            key_id: plan.key_id.clone(),
            headers: plan.headers.clone(),
            content_type: plan.content_type.clone(),
            model_name: plan.model_name.clone(),
            proxy: plan.proxy.clone(),
            tls_profile: plan.tls_profile.clone(),
            timeouts: plan.timeouts.clone(),
        })
    }

    pub(crate) fn from_provider_transport(
        transport: &GatewayProviderTransportSnapshot,
        api_format: &str,
        model_name: Option<String>,
    ) -> Option<Self> {
        let (auth_header, auth_value) = match api_format {
            "openai:video" => {
                if !supports_local_standard_transport(transport, api_format) {
                    return None;
                }
                resolve_local_standard_auth(transport)?
            }
            "gemini:video" => {
                if !supports_local_gemini_transport(transport, api_format) {
                    return None;
                }
                resolve_local_gemini_auth(transport)?
            }
            _ => return None,
        };

        let mut headers = BTreeMap::new();
        headers.insert(auth_header, auth_value);

        Some(Self {
            upstream_base_url: transport.endpoint.base_url.clone(),
            provider_name: Some(transport.provider.name.clone()),
            provider_id: transport.provider.id.clone(),
            endpoint_id: transport.endpoint.id.clone(),
            key_id: transport.key.id.clone(),
            headers,
            content_type: Some("application/json".to_string()),
            model_name,
            proxy: None,
            tls_profile: None,
            timeouts: resolve_transport_execution_timeouts(transport),
        })
    }
}

impl LocalVideoTaskPersistence {
    pub(crate) fn from_report_context(
        report_context: &Map<String, Value>,
        plan: &ExecutionPlan,
    ) -> Self {
        Self {
            request_id: context_text(report_context, "request_id")
                .unwrap_or_else(|| plan.request_id.clone()),
            username: context_text(report_context, "username"),
            api_key_name: context_text(report_context, "api_key_name"),
            client_api_format: context_text(report_context, "client_api_format")
                .unwrap_or_else(|| plan.client_api_format.clone()),
            provider_api_format: context_text(report_context, "provider_api_format")
                .unwrap_or_else(|| plan.provider_api_format.clone()),
            original_request_body: report_context
                .get("original_request_body")
                .cloned()
                .unwrap_or_else(|| Value::Object(Map::new())),
            format_converted: report_context
                .get("format_converted")
                .and_then(Value::as_bool)
                .unwrap_or(false),
        }
    }

    pub(crate) fn from_stored_task(task: &StoredVideoTask) -> Option<Self> {
        let client_api_format = non_empty_owned(task.client_api_format.as_ref())
            .or_else(|| non_empty_owned(task.provider_api_format.as_ref()))?;
        let provider_api_format = non_empty_owned(task.provider_api_format.as_ref())
            .or_else(|| non_empty_owned(task.client_api_format.as_ref()))?;

        Some(Self {
            request_id: task.request_id.clone(),
            username: task.username.clone(),
            api_key_name: task.api_key_name.clone(),
            client_api_format,
            provider_api_format,
            original_request_body: task
                .original_request_body
                .clone()
                .unwrap_or_else(|| Value::Object(Map::new())),
            format_converted: task.format_converted,
        })
    }
}

impl LocalVideoTaskStatus {
    pub(crate) fn as_database_status(self) -> StoredVideoTaskStatus {
        match self {
            Self::Submitted => StoredVideoTaskStatus::Submitted,
            Self::Queued => StoredVideoTaskStatus::Queued,
            Self::Processing => StoredVideoTaskStatus::Processing,
            Self::Completed => StoredVideoTaskStatus::Completed,
            Self::Failed => StoredVideoTaskStatus::Failed,
            Self::Cancelled => StoredVideoTaskStatus::Cancelled,
            Self::Expired => StoredVideoTaskStatus::Expired,
            Self::Deleted => StoredVideoTaskStatus::Deleted,
        }
    }
}

pub(crate) fn parse_video_content_variant(query_string: Option<&str>) -> Option<&'static str> {
    let mut variant = "video";
    if let Some(query_string) = query_string {
        for (key, value) in url::form_urlencoded::parse(query_string.as_bytes()) {
            if key == "variant" {
                variant = match value.as_ref() {
                    "video" => "video",
                    "thumbnail" => "thumbnail",
                    "spritesheet" => "spritesheet",
                    _ => return None,
                };
            }
        }
    }
    Some(variant)
}

pub(crate) fn gemini_metadata_video_url(metadata: &Value) -> Option<String> {
    metadata
        .get("response")
        .and_then(|value| value.get("generateVideoResponse"))
        .and_then(|value| value.get("generatedSamples"))
        .and_then(Value::as_array)
        .and_then(|value| value.first())
        .and_then(|value| value.get("video"))
        .and_then(|value| value.get("uri"))
        .and_then(Value::as_str)
        .map(str::to_string)
}

pub(crate) fn build_video_follow_up_report_context(
    request_id: &str,
    user_id: &str,
    api_key_id: &str,
    task_id: &str,
    model_name: Option<String>,
    transport: &LocalVideoTaskTransport,
    client_api_format: &str,
    provider_api_format: &str,
) -> Value {
    let mut context = Map::new();
    context.insert(
        "request_id".to_string(),
        Value::String(request_id.to_string()),
    );
    context.insert("user_id".to_string(), Value::String(user_id.to_string()));
    context.insert(
        "api_key_id".to_string(),
        Value::String(api_key_id.to_string()),
    );
    context.insert("task_id".to_string(), Value::String(task_id.to_string()));
    context.insert(
        "provider_id".to_string(),
        Value::String(transport.provider_id.clone()),
    );
    context.insert(
        "endpoint_id".to_string(),
        Value::String(transport.endpoint_id.clone()),
    );
    context.insert(
        "key_id".to_string(),
        Value::String(transport.key_id.clone()),
    );
    context.insert(
        "client_api_format".to_string(),
        Value::String(client_api_format.to_string()),
    );
    context.insert(
        "provider_api_format".to_string(),
        Value::String(provider_api_format.to_string()),
    );
    if let Some(provider_name) = transport.provider_name.clone() {
        context.insert("provider_name".to_string(), Value::String(provider_name));
    }
    if let Some(model_name) = model_name.filter(|value| !value.is_empty()) {
        context.insert("model".to_string(), Value::String(model_name));
    }
    Value::Object(context)
}

pub(crate) fn map_openai_task_status(status: LocalVideoTaskStatus) -> &'static str {
    match status {
        LocalVideoTaskStatus::Submitted | LocalVideoTaskStatus::Queued => "queued",
        LocalVideoTaskStatus::Processing => "processing",
        LocalVideoTaskStatus::Completed => "completed",
        LocalVideoTaskStatus::Failed
        | LocalVideoTaskStatus::Cancelled
        | LocalVideoTaskStatus::Expired => "failed",
        LocalVideoTaskStatus::Deleted => "deleted",
    }
}

pub(crate) fn resolve_follow_up_auth(
    user_id: Option<&str>,
    api_key_id: Option<&str>,
    auth_context: Option<&GatewayControlAuthContext>,
) -> Option<(String, String)> {
    let resolved_user_id = user_id
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string)
        .or_else(|| auth_context.map(|value| value.user_id.clone()))?;
    let resolved_api_key_id = api_key_id
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string)
        .or_else(|| auth_context.map(|value| value.api_key_id.clone()))?;
    Some((resolved_user_id, resolved_api_key_id))
}
