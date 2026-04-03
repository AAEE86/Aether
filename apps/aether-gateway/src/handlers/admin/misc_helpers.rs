use crate::gateway::handlers::{json_string_list, unix_secs_to_rfc3339};
use crate::gateway::GatewayPublicRequestContext;
use aether_data::repository::provider_catalog::StoredProviderCatalogKey;
use axum::{
    body::Body,
    http,
    response::{IntoResponse, Response},
    Json,
};
use serde_json::json;
use std::collections::BTreeMap;

pub(crate) fn build_unhandled_admin_proxy_response(
    request_context: &GatewayPublicRequestContext,
) -> Response<Body> {
    let decision = request_context.control_decision.as_ref();
    (
        http::StatusCode::NOT_IMPLEMENTED,
        Json(json!({
            "detail": "admin proxy route not implemented in rust frontdoor",
            "route_family": decision.and_then(|value| value.route_family.as_deref()),
            "route_kind": decision.and_then(|value| value.route_kind.as_deref()),
            "request_path": request_context.request_path,
        })),
    )
        .into_response()
}

pub(crate) fn build_admin_proxy_auth_required_response(
    request_context: &GatewayPublicRequestContext,
) -> Response<Body> {
    let decision = request_context.control_decision.as_ref();
    (
        http::StatusCode::UNAUTHORIZED,
        Json(json!({
            "detail": "admin authentication required",
            "route_family": decision.and_then(|value| value.route_family.as_deref()),
            "route_kind": decision.and_then(|value| value.route_kind.as_deref()),
            "request_path": request_context.request_path,
        })),
    )
        .into_response()
}

pub(crate) fn provider_catalog_key_supports_format(
    key: &StoredProviderCatalogKey,
    api_format: &str,
) -> bool {
    let Some(value) = key.api_formats.as_ref() else {
        return true;
    };
    let Some(values) = value.as_array() else {
        return true;
    };
    values
        .iter()
        .filter_map(serde_json::Value::as_str)
        .any(|candidate| candidate.trim().eq_ignore_ascii_case(api_format))
}

pub(crate) fn key_api_formats_without_entry(
    key: &StoredProviderCatalogKey,
    api_format: &str,
) -> Option<Vec<String>> {
    let current_formats = json_string_list(key.api_formats.as_ref());
    if !current_formats
        .iter()
        .any(|candidate| candidate == api_format)
    {
        return None;
    }
    Some(
        current_formats
            .into_iter()
            .filter(|candidate| candidate != api_format)
            .collect(),
    )
}

pub(crate) fn admin_health_key_id(request_path: &str) -> Option<String> {
    let raw = request_path.strip_prefix("/api/admin/endpoints/health/key/")?;
    let normalized = raw.trim().trim_matches('/');
    if normalized.is_empty() {
        None
    } else {
        Some(normalized.to_string())
    }
}

pub(crate) fn admin_recover_key_id(request_path: &str) -> Option<String> {
    let raw = request_path.strip_prefix("/api/admin/endpoints/health/keys/")?;
    let normalized = raw.trim().trim_matches('/');
    if normalized.is_empty() {
        None
    } else {
        Some(normalized.to_string())
    }
}

pub(crate) fn admin_rpm_key_id(request_path: &str) -> Option<String> {
    let raw = request_path.strip_prefix("/api/admin/endpoints/rpm/key/")?;
    let normalized = raw.trim().trim_matches('/');
    if normalized.is_empty() {
        None
    } else {
        Some(normalized.to_string())
    }
}

pub(crate) fn admin_provider_id_for_endpoints(request_path: &str) -> Option<String> {
    let raw = request_path.strip_prefix("/api/admin/endpoints/providers/")?;
    let raw = raw.strip_suffix("/endpoints")?;
    let normalized = raw.trim().trim_matches('/');
    if normalized.is_empty() {
        None
    } else {
        Some(normalized.to_string())
    }
}

pub(crate) fn admin_endpoint_id(request_path: &str) -> Option<String> {
    let raw = request_path.strip_prefix("/api/admin/endpoints/")?;
    let normalized = raw.trim().trim_matches('/');
    if normalized.is_empty() || normalized.contains('/') {
        None
    } else {
        Some(normalized.to_string())
    }
}

pub(crate) fn admin_default_body_rules_api_format(request_path: &str) -> Option<String> {
    let raw = request_path.strip_prefix("/api/admin/endpoints/defaults/")?;
    let raw = raw.strip_suffix("/body-rules")?;
    let normalized = raw.trim().trim_matches('/');
    if normalized.is_empty() {
        None
    } else {
        Some(normalized.to_string())
    }
}

pub(crate) fn masked_proxy_value(proxy: Option<&serde_json::Value>) -> serde_json::Value {
    let Some(proxy) = proxy.and_then(serde_json::Value::as_object) else {
        return serde_json::Value::Null;
    };
    let mut masked = proxy.clone();
    if masked
        .get("password")
        .and_then(serde_json::Value::as_str)
        .is_some_and(|value| !value.trim().is_empty())
    {
        masked.insert("password".to_string(), json!("***"));
    }
    serde_json::Value::Object(masked)
}

pub(crate) fn json_truthy(value: &serde_json::Value) -> bool {
    match value {
        serde_json::Value::Null => false,
        serde_json::Value::Bool(value) => *value,
        serde_json::Value::Number(value) => value.as_f64().is_some_and(|value| value != 0.0),
        serde_json::Value::String(value) => !value.trim().is_empty(),
        serde_json::Value::Array(value) => !value.is_empty(),
        serde_json::Value::Object(value) => !value.is_empty(),
    }
}

pub(crate) fn endpoint_timestamp_or_now(
    value: Option<u64>,
    now_unix_secs: u64,
) -> serde_json::Value {
    unix_secs_to_rfc3339(value.unwrap_or(now_unix_secs))
        .map(serde_json::Value::String)
        .unwrap_or(serde_json::Value::Null)
}

pub(crate) fn endpoint_key_counts_by_format(
    keys: &[aether_data::repository::provider_catalog::StoredProviderCatalogKey],
) -> (BTreeMap<String, usize>, BTreeMap<String, usize>) {
    let mut total = BTreeMap::new();
    let mut active = BTreeMap::new();
    for key in keys {
        let Some(formats) = key
            .api_formats
            .as_ref()
            .and_then(serde_json::Value::as_array)
        else {
            continue;
        };
        for api_format in formats.iter().filter_map(serde_json::Value::as_str) {
            *total.entry(api_format.to_string()).or_insert(0) += 1;
            if key.is_active {
                *active.entry(api_format.to_string()).or_insert(0) += 1;
            }
        }
    }
    (total, active)
}

pub(crate) fn build_admin_provider_endpoint_response(
    endpoint: &aether_data::repository::provider_catalog::StoredProviderCatalogEndpoint,
    provider_name: &str,
    total_keys: usize,
    active_keys: usize,
    now_unix_secs: u64,
) -> serde_json::Value {
    json!({
        "id": endpoint.id,
        "provider_id": endpoint.provider_id,
        "provider_name": provider_name,
        "api_format": endpoint.api_format,
        "base_url": endpoint.base_url,
        "custom_path": endpoint.custom_path,
        "header_rules": endpoint.header_rules,
        "body_rules": endpoint.body_rules,
        "max_retries": endpoint.max_retries.unwrap_or(2),
        "is_active": endpoint.is_active,
        "config": endpoint.config,
        "proxy": masked_proxy_value(endpoint.proxy.as_ref()),
        "format_acceptance_config": endpoint.format_acceptance_config,
        "total_keys": total_keys,
        "active_keys": active_keys,
        "created_at": endpoint_timestamp_or_now(endpoint.created_at_unix_secs, now_unix_secs),
        "updated_at": endpoint_timestamp_or_now(endpoint.updated_at_unix_secs, now_unix_secs),
    })
}
