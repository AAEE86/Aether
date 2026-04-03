use crate::gateway::handlers::json_string_list;
use crate::gateway::{AppState, GatewayError};
use aether_data::repository::provider_catalog::StoredProviderCatalogKey;
use axum::{
    body::Body,
    http,
    response::{IntoResponse, Response},
    Json,
};
use serde_json::json;

pub(super) fn admin_adaptive_effective_limit(key: &StoredProviderCatalogKey) -> Option<u32> {
    if key.rpm_limit.is_none() {
        key.learned_rpm_limit
    } else {
        key.rpm_limit
    }
}

pub(super) fn admin_adaptive_adjustment_items(
    value: Option<&serde_json::Value>,
) -> Vec<serde_json::Map<String, serde_json::Value>> {
    value
        .and_then(serde_json::Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(serde_json::Value::as_object)
        .cloned()
        .collect()
}

pub(super) fn admin_adaptive_key_payload(key: &StoredProviderCatalogKey) -> serde_json::Value {
    json!({
        "id": key.id,
        "name": key.name,
        "provider_id": key.provider_id,
        "api_formats": json_string_list(key.api_formats.as_ref()),
        "is_adaptive": key.rpm_limit.is_none(),
        "rpm_limit": key.rpm_limit,
        "effective_limit": admin_adaptive_effective_limit(key),
        "learned_rpm_limit": key.learned_rpm_limit,
        "concurrent_429_count": key.concurrent_429_count.unwrap_or(0),
        "rpm_429_count": key.rpm_429_count.unwrap_or(0),
    })
}

pub(super) fn admin_adaptive_key_not_found_response(key_id: &str) -> Response<Body> {
    (
        http::StatusCode::NOT_FOUND,
        Json(json!({ "detail": format!("Key {key_id} 不存在") })),
    )
        .into_response()
}

pub(super) fn admin_adaptive_dispatcher_not_found_response() -> Response<Body> {
    (
        http::StatusCode::NOT_FOUND,
        Json(json!({ "detail": "Adaptive route not found" })),
    )
        .into_response()
}

pub(super) fn admin_adaptive_key_id_from_path(path: &str) -> Option<String> {
    let normalized = path.trim_end_matches('/');
    let mut segments = normalized.split('/').filter(|segment| !segment.is_empty());
    match (
        segments.next(),
        segments.next(),
        segments.next(),
        segments.next(),
        segments.next(),
    ) {
        (Some("api"), Some("admin"), Some("adaptive"), Some("keys"), Some(key_id))
            if !key_id.is_empty() =>
        {
            Some(key_id.to_string())
        }
        _ => None,
    }
}

pub(super) async fn admin_adaptive_find_key(
    state: &AppState,
    key_id: &str,
) -> Result<Option<StoredProviderCatalogKey>, GatewayError> {
    Ok(state
        .read_provider_catalog_keys_by_ids(std::slice::from_ref(&key_id.to_string()))
        .await?
        .into_iter()
        .next())
}

pub(super) async fn admin_adaptive_load_candidate_keys(
    state: &AppState,
    provider_id: Option<&str>,
) -> Result<Vec<StoredProviderCatalogKey>, GatewayError> {
    if let Some(provider_id) = provider_id.filter(|value| !value.trim().is_empty()) {
        return state
            .list_provider_catalog_keys_by_provider_ids(std::slice::from_ref(
                &provider_id.to_string(),
            ))
            .await;
    }

    let provider_ids = state
        .list_provider_catalog_providers(false)
        .await?
        .into_iter()
        .map(|provider| provider.id)
        .collect::<Vec<_>>();
    if provider_ids.is_empty() {
        return Ok(vec![]);
    }
    state
        .list_provider_catalog_keys_by_provider_ids(&provider_ids)
        .await
}
