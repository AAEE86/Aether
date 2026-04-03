use super::{
    admin_usage_bad_request_response, admin_usage_data_unavailable_response,
    ADMIN_USAGE_DATA_UNAVAILABLE_DETAIL,
};
use crate::gateway::{AppState, GatewayError, GatewayPublicRequestContext};
use axum::{
    body::Body,
    http,
    response::{IntoResponse, Response},
    Json,
};
use serde_json::{json, Value};
use std::collections::BTreeMap;

fn admin_usage_id_from_path_suffix(request_path: &str, suffix: Option<&str>) -> Option<String> {
    let mut value = request_path
        .strip_prefix("/api/admin/usage/")?
        .trim()
        .trim_matches('/')
        .to_string();
    if let Some(suffix) = suffix {
        value = value.strip_suffix(suffix)?.trim_matches('/').to_string();
    }
    if value.is_empty() || value.contains('/') {
        None
    } else {
        Some(value)
    }
}

pub(super) fn admin_usage_id_from_detail_path(request_path: &str) -> Option<String> {
    admin_usage_id_from_path_suffix(request_path, None)
}

pub(super) fn admin_usage_id_from_action_path(request_path: &str, action: &str) -> Option<String> {
    admin_usage_id_from_path_suffix(request_path, Some(action))
}

#[derive(Debug, Default, serde::Deserialize)]
struct AdminUsageReplayRequest {
    #[serde(default, alias = "target_provider_id")]
    provider_id: Option<String>,
    #[serde(default, alias = "target_endpoint_id")]
    endpoint_id: Option<String>,
    #[serde(default, alias = "target_api_key_id")]
    api_key_id: Option<String>,
    #[serde(default)]
    body_override: Option<serde_json::Value>,
}

fn admin_usage_resolve_replay_mode(same_provider: bool, same_endpoint: bool) -> &'static str {
    if same_provider && same_endpoint {
        "same_endpoint_reuse"
    } else if same_provider {
        "same_provider_remap"
    } else {
        "cross_provider_remap"
    }
}

pub(super) fn admin_usage_resolve_request_preview_body(
    item: &aether_data::repository::usage::StoredRequestUsageAudit,
    body_override: Option<serde_json::Value>,
) -> serde_json::Value {
    let resolved_model = item.model.clone();
    let mut request_body = body_override
        .or_else(|| item.request_body.clone())
        .unwrap_or_else(|| {
            json!({
                "model": resolved_model,
                "stream": item.is_stream,
            })
        });
    if let Some(body) = request_body.as_object_mut() {
        body.entry("model".to_string())
            .or_insert_with(|| json!(resolved_model));
        if !body.contains_key("stream") {
            body.insert("stream".to_string(), json!(item.is_stream));
        }
        if let Some(target_model) = item
            .target_model
            .as_ref()
            .filter(|value| !value.trim().is_empty())
        {
            body.entry("target_model".to_string())
                .or_insert_with(|| json!(target_model));
        }
        if let Some(request_type) = item
            .request_type
            .as_ref()
            .filter(|value| !value.trim().is_empty())
        {
            body.entry("request_type".to_string())
                .or_insert_with(|| json!(request_type));
        }
        if let Some(api_format) = item
            .api_format
            .as_ref()
            .filter(|value| !value.trim().is_empty())
        {
            body.entry("api_format".to_string())
                .or_insert_with(|| json!(api_format));
        }
    }
    request_body
}

pub(super) async fn build_admin_usage_replay_response(
    state: &AppState,
    request_context: &GatewayPublicRequestContext,
    request_body: Option<&axum::body::Bytes>,
) -> Result<Response<Body>, GatewayError> {
    if !state.has_usage_data_reader() || !state.has_provider_catalog_data_reader() {
        return Ok(admin_usage_data_unavailable_response(
            ADMIN_USAGE_DATA_UNAVAILABLE_DETAIL,
        ));
    }

    let Some(usage_id) = admin_usage_id_from_action_path(&request_context.request_path, "/replay")
    else {
        return Ok(admin_usage_bad_request_response("usage_id 无效"));
    };

    let payload = match request_body {
        Some(body) if !body.is_empty() => {
            serde_json::from_slice::<AdminUsageReplayRequest>(body).unwrap_or_default()
        }
        _ => AdminUsageReplayRequest::default(),
    };

    let Some(item) = state.find_request_usage_by_id(&usage_id).await? else {
        return Ok((
            http::StatusCode::NOT_FOUND,
            Json(json!({ "detail": "Usage record not found" })),
        )
            .into_response());
    };

    let target_provider_id = payload
        .provider_id
        .clone()
        .or_else(|| item.provider_id.clone())
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty());
    let Some(target_provider_id) = target_provider_id else {
        return Ok(admin_usage_bad_request_response(
            "Replay target provider is unavailable",
        ));
    };
    let Some(target_provider) = state
        .read_provider_catalog_providers_by_ids(std::slice::from_ref(&target_provider_id))
        .await?
        .into_iter()
        .next()
    else {
        return Ok((
            http::StatusCode::NOT_FOUND,
            Json(json!({ "detail": format!("Provider {target_provider_id} 不存在") })),
        )
            .into_response());
    };

    let requested_endpoint_id = payload
        .endpoint_id
        .clone()
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty());
    let target_endpoint = if let Some(endpoint_id) = requested_endpoint_id.clone() {
        let Some(endpoint) = state
            .read_provider_catalog_endpoints_by_ids(std::slice::from_ref(&endpoint_id))
            .await?
            .into_iter()
            .next()
        else {
            return Ok((
                http::StatusCode::NOT_FOUND,
                Json(json!({ "detail": format!("Endpoint {endpoint_id} 不存在") })),
            )
                .into_response());
        };
        if endpoint.provider_id != target_provider.id {
            return Ok(admin_usage_bad_request_response(
                "Target endpoint does not belong to the target provider",
            ));
        }
        endpoint
    } else {
        let preferred_endpoint_id = item
            .provider_endpoint_id
            .clone()
            .filter(|_| item.provider_id.as_deref() == Some(target_provider.id.as_str()));
        if let Some(endpoint_id) = preferred_endpoint_id {
            if let Some(endpoint) = state
                .read_provider_catalog_endpoints_by_ids(std::slice::from_ref(&endpoint_id))
                .await?
                .into_iter()
                .find(|endpoint| endpoint.provider_id == target_provider.id)
            {
                endpoint
            } else {
                let mut endpoints = state
                    .list_provider_catalog_endpoints_by_provider_ids(std::slice::from_ref(
                        &target_provider.id,
                    ))
                    .await?;
                let preferred_api_format = item
                    .endpoint_api_format
                    .as_deref()
                    .or(item.api_format.as_deref())
                    .unwrap_or_default();
                endpoints
                    .iter()
                    .find(|endpoint| {
                        endpoint.is_active && endpoint.api_format == preferred_api_format
                    })
                    .cloned()
                    .or_else(|| {
                        endpoints
                            .iter()
                            .find(|endpoint| endpoint.is_active)
                            .cloned()
                    })
                    .or_else(|| endpoints.into_iter().next())
                    .ok_or_else(|| {
                        GatewayError::Internal("target provider has no endpoints".to_string())
                    })?
            }
        } else {
            let mut endpoints = state
                .list_provider_catalog_endpoints_by_provider_ids(std::slice::from_ref(
                    &target_provider.id,
                ))
                .await?;
            let preferred_api_format = item
                .endpoint_api_format
                .as_deref()
                .or(item.api_format.as_deref())
                .unwrap_or_default();
            endpoints
                .iter()
                .find(|endpoint| endpoint.is_active && endpoint.api_format == preferred_api_format)
                .cloned()
                .or_else(|| {
                    endpoints
                        .iter()
                        .find(|endpoint| endpoint.is_active)
                        .cloned()
                })
                .or_else(|| endpoints.into_iter().next())
                .ok_or_else(|| {
                    GatewayError::Internal("target provider has no endpoints".to_string())
                })?
        }
    };

    let same_provider = item.provider_id.as_deref() == Some(target_provider.id.as_str());
    let same_endpoint = item.provider_endpoint_id.as_deref() == Some(target_endpoint.id.as_str());
    let resolved_model = item.model.clone();
    let mapping_source = "none";
    let request_body = admin_usage_resolve_request_preview_body(&item, payload.body_override);

    let url = admin_usage_curl_url(&target_endpoint, &item);
    let headers = admin_usage_curl_headers();
    let curl = admin_usage_build_curl_command(Some(&url), &headers, Some(&request_body));
    Ok(Json(json!({
        "dry_run": true,
        "usage_id": item.id,
        "request_id": item.request_id,
        "mode": admin_usage_resolve_replay_mode(same_provider, same_endpoint),
        "target_provider_id": target_provider.id,
        "target_provider_name": target_provider.name,
        "target_endpoint_id": target_endpoint.id,
        "target_api_key_id": payload.api_key_id.or(item.provider_api_key_id.clone()),
        "target_api_format": target_endpoint.api_format,
        "resolved_model": resolved_model,
        "mapping_source": mapping_source,
        "method": "POST",
        "url": url,
        "request_headers": headers,
        "request_body": request_body,
        "original_request_body_available": item.request_body.is_some(),
        "note": "Rust local replay currently exposes a dry-run plan and does not dispatch upstream",
        "curl": curl,
    }))
    .into_response())
}

pub(super) fn admin_usage_headers_from_value(
    value: &serde_json::Value,
) -> Option<BTreeMap<String, String>> {
    let object = value.as_object()?;
    Some(BTreeMap::from_iter(object.iter().filter_map(
        |(key, value)| {
            if value.is_null() {
                return None;
            }
            let value = value
                .as_str()
                .map(ToOwned::to_owned)
                .unwrap_or_else(|| value.to_string());
            Some((key.clone(), value))
        },
    )))
}

pub(super) fn admin_usage_curl_headers() -> BTreeMap<String, String> {
    BTreeMap::from([("Content-Type".to_string(), "application/json".to_string())])
}

pub(super) fn admin_usage_curl_url(
    endpoint: &aether_data::repository::provider_catalog::StoredProviderCatalogEndpoint,
    item: &aether_data::repository::usage::StoredRequestUsageAudit,
) -> String {
    let api_format = item
        .endpoint_api_format
        .as_deref()
        .or(item.api_format.as_deref())
        .unwrap_or(endpoint.api_format.as_str());

    if let Some(custom_path) = endpoint
        .custom_path
        .as_deref()
        .filter(|value| !value.is_empty())
    {
        return crate::gateway::provider_transport::build_passthrough_path_url(
            &endpoint.base_url,
            custom_path,
            None,
            &[],
        )
        .unwrap_or_else(|| endpoint.base_url.clone());
    }

    match api_format {
        value if value.starts_with("claude:") => {
            crate::gateway::provider_transport::build_claude_messages_url(&endpoint.base_url, None)
        }
        value if value.starts_with("gemini:") => {
            crate::gateway::provider_transport::build_gemini_content_url(
                &endpoint.base_url,
                item.target_model.as_deref().unwrap_or(item.model.as_str()),
                item.is_stream,
                None,
            )
            .unwrap_or_else(|| endpoint.base_url.clone())
        }
        value if value.starts_with("openai:") => {
            crate::gateway::provider_transport::build_openai_chat_url(&endpoint.base_url, None)
        }
        _ => endpoint.base_url.clone(),
    }
}

fn admin_usage_curl_shell_quote(value: &str) -> String {
    format!("'{}'", value.replace('\'', "'\"'\"'"))
}

pub(super) fn admin_usage_build_curl_command(
    url: Option<&str>,
    headers: &BTreeMap<String, String>,
    body: Option<&serde_json::Value>,
) -> String {
    let mut parts = vec!["curl".to_string()];
    if let Some(url) = url {
        parts.push(admin_usage_curl_shell_quote(url));
    }
    parts.push("-X POST".to_string());
    for (key, value) in headers {
        parts.push(format!(
            "-H {}",
            admin_usage_curl_shell_quote(&format!("{key}: {value}"))
        ));
    }
    if let Some(body) = body {
        parts.push(format!(
            "-d {}",
            admin_usage_curl_shell_quote(&body.to_string())
        ));
    }
    parts.join(" \\\n  ")
}
