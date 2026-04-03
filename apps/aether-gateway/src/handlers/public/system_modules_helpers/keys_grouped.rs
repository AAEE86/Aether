use super::enabled_key_capability_short_names;
use crate::gateway::handlers::{json_string_list, masked_catalog_api_key, unix_secs_to_rfc3339};
use crate::gateway::AppState;
use serde_json::json;
use std::collections::BTreeMap;
use std::time::{SystemTime, UNIX_EPOCH};

pub(crate) async fn build_admin_keys_grouped_by_format_payload(
    state: &AppState,
) -> Option<serde_json::Value> {
    if !state.has_provider_catalog_data_reader() {
        return None;
    }

    let providers = state
        .list_provider_catalog_providers(false)
        .await
        .ok()
        .unwrap_or_default();
    let provider_ids = providers
        .iter()
        .map(|provider| provider.id.clone())
        .collect::<Vec<_>>();
    let provider_by_id = providers
        .iter()
        .map(|provider| (provider.id.clone(), provider.clone()))
        .collect::<BTreeMap<_, _>>();

    let endpoint_base_url_by_provider_and_format = state
        .list_provider_catalog_endpoints_by_provider_ids(&provider_ids)
        .await
        .ok()
        .unwrap_or_default()
        .into_iter()
        .filter(|endpoint| endpoint.is_active)
        .map(|endpoint| {
            (
                (endpoint.provider_id, endpoint.api_format),
                endpoint.base_url,
            )
        })
        .collect::<BTreeMap<_, _>>();

    let mut keys = state
        .list_provider_catalog_keys_by_provider_ids(&provider_ids)
        .await
        .ok()
        .unwrap_or_default();
    keys.sort_by(|left, right| {
        left.internal_priority
            .cmp(&right.internal_priority)
            .then_with(|| left.id.cmp(&right.id))
    });

    let now_unix_secs = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .ok()
        .map(|duration| duration.as_secs())
        .unwrap_or(0);

    let mut grouped = BTreeMap::<String, Vec<serde_json::Value>>::new();
    for key in keys {
        let Some(provider) = provider_by_id.get(&key.provider_id) else {
            continue;
        };
        let request_count = u64::from(key.request_count.unwrap_or(0));
        let success_count = u64::from(key.success_count.unwrap_or(0));
        let success_rate = if request_count > 0 {
            Some(success_count as f64 / request_count as f64)
        } else {
            None
        };
        let avg_response_time_ms = if success_count > 0 {
            Some(f64::from(key.total_response_time_ms.unwrap_or(0)) / success_count as f64)
        } else {
            None
        };
        let priority_by_format = key
            .global_priority_by_format
            .as_ref()
            .and_then(serde_json::Value::as_object)
            .cloned()
            .unwrap_or_default();
        let health_by_format = key
            .health_by_format
            .as_ref()
            .and_then(serde_json::Value::as_object)
            .cloned()
            .unwrap_or_default();
        let circuit_by_format = key
            .circuit_breaker_by_format
            .as_ref()
            .and_then(serde_json::Value::as_object)
            .cloned()
            .unwrap_or_default();
        let capability_names = enabled_key_capability_short_names(key.capabilities.as_ref());
        let api_formats = json_string_list(key.api_formats.as_ref());
        if api_formats.is_empty() {
            continue;
        }

        for api_format in &api_formats {
            let format_health = health_by_format
                .get(api_format)
                .cloned()
                .unwrap_or_else(|| json!({}));
            let format_circuit = circuit_by_format
                .get(api_format)
                .cloned()
                .unwrap_or_else(|| json!({}));
            grouped.entry(api_format.clone()).or_default().push(json!({
                "id": key.id,
                "provider_id": key.provider_id,
                "name": key.name,
                "auth_type": key.auth_type,
                "api_key_masked": masked_catalog_api_key(state, &key),
                "internal_priority": key.internal_priority,
                "global_priority_by_format": key.global_priority_by_format,
                "rate_multipliers": key.rate_multipliers,
                "is_active": key.is_active,
                "provider_active": provider.is_active,
                "provider_name": provider.name,
                "api_formats": api_formats,
                "capabilities": capability_names,
                "success_rate": success_rate,
                "avg_response_time_ms": avg_response_time_ms,
                "request_count": request_count,
                "api_format": api_format,
                "endpoint_base_url": endpoint_base_url_by_provider_and_format
                    .get(&(provider.id.clone(), api_format.clone()))
                    .cloned(),
                "format_priority": priority_by_format
                    .get(api_format)
                    .cloned()
                    .unwrap_or(serde_json::Value::Null),
                "health_score": format_health
                    .get("health_score")
                    .and_then(serde_json::Value::as_f64)
                    .unwrap_or(1.0),
                "circuit_breaker_open": format_circuit
                    .get("open")
                    .and_then(serde_json::Value::as_bool)
                    .unwrap_or(false),
                "last_used_at": key.last_used_at_unix_secs.and_then(unix_secs_to_rfc3339),
                "created_at": unix_secs_to_rfc3339(key.created_at_unix_secs.unwrap_or(now_unix_secs)),
                "updated_at": unix_secs_to_rfc3339(key.updated_at_unix_secs.unwrap_or(now_unix_secs)),
            }));
        }
    }

    Some(serde_json::Value::Object(
        grouped
            .into_iter()
            .map(|(format, items)| (format, serde_json::Value::Array(items)))
            .collect(),
    ))
}
