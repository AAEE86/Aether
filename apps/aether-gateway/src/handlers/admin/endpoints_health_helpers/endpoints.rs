use crate::gateway::api::ai::{
    admin_default_body_rules_for_signature, admin_endpoint_signature_parts,
};
use crate::gateway::handlers::public::{admin_requested_force_stream, normalize_admin_base_url};
use crate::gateway::provider_transport::provider_type_is_fixed;
use crate::gateway::AppState;
use aether_data::repository::provider_catalog::{
    StoredProviderCatalogEndpoint, StoredProviderCatalogProvider,
};
use serde_json::json;
use std::time::{SystemTime, UNIX_EPOCH};
use uuid::Uuid;

use super::super::{build_admin_provider_endpoint_response, endpoint_key_counts_by_format};
use crate::gateway::handlers::{
    AdminProviderEndpointCreateRequest, AdminProviderEndpointUpdateRequest,
};

pub(crate) async fn build_admin_provider_endpoints_payload(
    state: &AppState,
    provider_id: &str,
    skip: usize,
    limit: usize,
) -> Option<serde_json::Value> {
    if !state.has_provider_catalog_data_reader() {
        return None;
    }

    let provider = state
        .read_provider_catalog_providers_by_ids(&[provider_id.to_string()])
        .await
        .ok()
        .and_then(|mut providers| providers.drain(..).next())?;
    let mut endpoints = state
        .list_provider_catalog_endpoints_by_provider_ids(std::slice::from_ref(&provider.id))
        .await
        .ok()
        .unwrap_or_default();
    endpoints.sort_by(|left, right| {
        right
            .created_at_unix_secs
            .unwrap_or_default()
            .cmp(&left.created_at_unix_secs.unwrap_or_default())
            .then_with(|| left.id.cmp(&right.id))
    });
    let keys = state
        .list_provider_catalog_keys_by_provider_ids(std::slice::from_ref(&provider.id))
        .await
        .ok()
        .unwrap_or_default();
    let (total_keys_by_format, active_keys_by_format) = endpoint_key_counts_by_format(&keys);
    let now_unix_secs = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .ok()
        .map(|duration| duration.as_secs())
        .unwrap_or(0);

    Some(serde_json::Value::Array(
        endpoints
            .into_iter()
            .skip(skip)
            .take(limit)
            .map(|endpoint| {
                build_admin_provider_endpoint_response(
                    &endpoint,
                    &provider.name,
                    total_keys_by_format
                        .get(endpoint.api_format.as_str())
                        .copied()
                        .unwrap_or(0),
                    active_keys_by_format
                        .get(endpoint.api_format.as_str())
                        .copied()
                        .unwrap_or(0),
                    now_unix_secs,
                )
            })
            .collect(),
    ))
}

pub(crate) async fn build_admin_endpoint_payload(
    state: &AppState,
    endpoint_id: &str,
) -> Option<serde_json::Value> {
    if !state.has_provider_catalog_data_reader() {
        return None;
    }

    let endpoint = state
        .read_provider_catalog_endpoints_by_ids(&[endpoint_id.to_string()])
        .await
        .ok()
        .and_then(|mut endpoints| endpoints.drain(..).next())?;
    let provider = state
        .read_provider_catalog_providers_by_ids(std::slice::from_ref(&endpoint.provider_id))
        .await
        .ok()
        .and_then(|mut providers| providers.drain(..).next())?;
    let keys = state
        .list_provider_catalog_keys_by_provider_ids(std::slice::from_ref(&endpoint.provider_id))
        .await
        .ok()
        .unwrap_or_default();
    let (total_keys_by_format, active_keys_by_format) = endpoint_key_counts_by_format(&keys);
    let now_unix_secs = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .ok()
        .map(|duration| duration.as_secs())
        .unwrap_or(0);

    Some(build_admin_provider_endpoint_response(
        &endpoint,
        &provider.name,
        total_keys_by_format
            .get(endpoint.api_format.as_str())
            .copied()
            .unwrap_or(0),
        active_keys_by_format
            .get(endpoint.api_format.as_str())
            .copied()
            .unwrap_or(0),
        now_unix_secs,
    ))
}

pub(crate) async fn build_admin_create_provider_endpoint_record(
    state: &AppState,
    provider: &StoredProviderCatalogProvider,
    payload: AdminProviderEndpointCreateRequest,
) -> Result<StoredProviderCatalogEndpoint, String> {
    if payload.provider_id.trim() != provider.id {
        return Err("provider_id 不匹配".to_string());
    }
    if provider_type_is_fixed(&provider.provider_type) {
        return Err("固定类型 Provider 不允许手动新增 Endpoint".to_string());
    }
    if !(0..=999).contains(&payload.max_retries) {
        return Err("max_retries 必须在 0 到 999 之间".to_string());
    }

    let (normalized_api_format, api_family, endpoint_kind) =
        admin_endpoint_signature_parts(&payload.api_format)
            .ok_or_else(|| format!("无效的 api_format: {}", payload.api_format))?;
    let base_url = normalize_admin_base_url(&payload.base_url)?;

    let existing_endpoints = state
        .list_provider_catalog_endpoints_by_provider_ids(std::slice::from_ref(&provider.id))
        .await
        .map_err(|err| format!("{err:?}"))?;
    if existing_endpoints
        .iter()
        .any(|endpoint| endpoint.api_format == normalized_api_format)
    {
        return Err(format!(
            "Provider {} 已存在 {} 格式的 Endpoint",
            provider.name, normalized_api_format
        ));
    }

    let body_rules = match payload.body_rules {
        Some(value) => Some(value),
        None => admin_default_body_rules_for_signature(
            normalized_api_format,
            Some(provider.provider_type.as_str()),
        )
        .and_then(|(_, rules)| (!rules.is_empty()).then_some(serde_json::Value::Array(rules))),
    };

    let now_unix_secs = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .ok()
        .map(|duration| duration.as_secs())
        .unwrap_or(0);

    StoredProviderCatalogEndpoint::new(
        Uuid::new_v4().to_string(),
        provider.id.clone(),
        normalized_api_format.to_string(),
        Some(api_family.to_string()),
        Some(endpoint_kind.to_string()),
        true,
    )
    .map_err(|err| err.to_string())?
    .with_timestamps(Some(now_unix_secs), Some(now_unix_secs))
    .with_transport_fields(
        base_url,
        payload.header_rules,
        body_rules,
        Some(payload.max_retries),
        payload.custom_path.and_then(|value| {
            let trimmed = value.trim().to_string();
            (!trimmed.is_empty()).then_some(trimmed)
        }),
        payload.config,
        payload.format_acceptance_config,
        payload.proxy,
    )
    .map_err(|err| err.to_string())
}

pub(crate) async fn build_admin_update_provider_endpoint_record(
    state: &AppState,
    provider: &StoredProviderCatalogProvider,
    existing_endpoint: &StoredProviderCatalogEndpoint,
    raw_payload: &serde_json::Map<String, serde_json::Value>,
    payload: AdminProviderEndpointUpdateRequest,
) -> Result<StoredProviderCatalogEndpoint, String> {
    let mut updated = existing_endpoint.clone();

    if provider_type_is_fixed(&provider.provider_type)
        && (raw_payload.contains_key("base_url") || raw_payload.contains_key("custom_path"))
    {
        return Err("固定类型 Provider 的 Endpoint 不允许修改 base_url/custom_path".to_string());
    }

    if let Some(value) = raw_payload.get("base_url") {
        let Some(base_url) = payload.base_url.as_deref() else {
            return Err(if value.is_null() {
                "base_url 不能为空".to_string()
            } else {
                "base_url 必须是字符串".to_string()
            });
        };
        updated.base_url = normalize_admin_base_url(base_url)?;
    }

    if raw_payload.contains_key("custom_path") {
        updated.custom_path = payload.custom_path;
    }

    if let Some(value) = raw_payload.get("header_rules") {
        if !value.is_null() && !value.is_array() {
            return Err("header_rules 必须是数组或 null".to_string());
        }
        updated.header_rules = if value.is_null() {
            None
        } else {
            payload.header_rules
        };
    }

    if let Some(value) = raw_payload.get("body_rules") {
        if !value.is_null() && !value.is_array() {
            return Err("body_rules 必须是数组或 null".to_string());
        }
        updated.body_rules = if value.is_null() {
            None
        } else {
            payload.body_rules
        };
    }

    if let Some(value) = raw_payload.get("max_retries") {
        let Some(max_retries) = payload.max_retries else {
            return Err(if value.is_null() {
                "max_retries 必须是 0 到 999 之间的整数".to_string()
            } else {
                "max_retries 必须是整数".to_string()
            });
        };
        if !(0..=999).contains(&max_retries) {
            return Err("max_retries 必须在 0 到 999 之间".to_string());
        }
        updated.max_retries = Some(max_retries);
    }

    if raw_payload.contains_key("is_active") {
        let Some(is_active) = payload.is_active else {
            return Err("is_active 必须是布尔值".to_string());
        };
        updated.is_active = is_active;
    }

    if let Some(value) = raw_payload.get("config") {
        if !value.is_null() && !value.is_object() {
            return Err("config 必须是对象或 null".to_string());
        }
        updated.config = if value.is_null() {
            None
        } else {
            payload.config
        };
    }

    if let Some(value) = raw_payload.get("proxy") {
        if value.is_null() {
            updated.proxy = None;
        } else {
            let Some(mut proxy) = payload.proxy.and_then(|value| value.as_object().cloned()) else {
                return Err("proxy 必须是对象或 null".to_string());
            };
            if !proxy.contains_key("password") {
                if let Some(old_password) = existing_endpoint
                    .proxy
                    .as_ref()
                    .and_then(serde_json::Value::as_object)
                    .and_then(|proxy| proxy.get("password"))
                    .and_then(serde_json::Value::as_str)
                    .filter(|value| !value.is_empty())
                {
                    proxy.insert("password".to_string(), json!(old_password));
                }
            }
            updated.proxy = Some(serde_json::Value::Object(proxy));
        }
    }

    if let Some(value) = raw_payload.get("format_acceptance_config") {
        if !value.is_null() && !value.is_object() {
            return Err("format_acceptance_config 必须是对象或 null".to_string());
        }
        updated.format_acceptance_config = if value.is_null() {
            None
        } else {
            payload.format_acceptance_config
        };
    }

    let provider_type = provider.provider_type.trim().to_ascii_lowercase();
    if provider_type == "codex" && existing_endpoint.api_format == "openai:cli" {
        let has_config_in_payload = raw_payload.contains_key("config");
        let config_payload = if has_config_in_payload {
            updated.config.clone().unwrap_or_else(|| json!({}))
        } else {
            existing_endpoint
                .config
                .clone()
                .unwrap_or_else(|| json!({}))
        };
        let mut config = config_payload.as_object().cloned().unwrap_or_default();
        let requested = config
            .get("upstream_stream_policy")
            .or_else(|| config.get("upstreamStreamPolicy"))
            .or_else(|| config.get("upstream_stream"));
        if has_config_in_payload
            && requested.is_some()
            && !admin_requested_force_stream(requested.expect("checked above"))
        {
            return Err("Codex OpenAI CLI 端点固定为强制流式，不允许修改".to_string());
        }
        config.remove("upstreamStreamPolicy");
        config.remove("upstream_stream");
        config.insert("upstream_stream_policy".to_string(), json!("force_stream"));
        updated.config = Some(serde_json::Value::Object(config));
    }

    let (_, api_family, endpoint_kind) = admin_endpoint_signature_parts(&updated.api_format)
        .ok_or_else(|| format!("无效的 api_format: {}", updated.api_format))?;
    updated.api_family = Some(api_family.to_string());
    updated.endpoint_kind = Some(endpoint_kind.to_string());
    updated.updated_at_unix_secs = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .ok()
        .map(|duration| duration.as_secs());

    let _ = state;
    Ok(updated)
}
