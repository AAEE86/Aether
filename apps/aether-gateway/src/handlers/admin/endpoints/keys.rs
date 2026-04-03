use super::super::{
    admin_clear_oauth_invalid_key_id, admin_export_key_id, admin_provider_id_for_refresh_quota,
    admin_reveal_key_id, admin_update_key_id, build_admin_create_provider_key_record,
    build_admin_export_key_payload, build_admin_provider_key_response,
    build_admin_provider_keys_payload, build_admin_reveal_key_payload,
    build_admin_update_provider_key_record, normalize_string_id_list,
    refresh_antigravity_provider_quota_locally, refresh_codex_provider_quota_locally,
    refresh_kiro_provider_quota_locally,
};
use crate::gateway::handlers::public::build_admin_keys_grouped_by_format_payload;
use crate::gateway::handlers::{
    admin_provider_id_for_keys, query_param_value, AdminProviderKeyBatchDeleteRequest,
    AdminProviderKeyCreateRequest, AdminProviderKeyUpdateRequest, AdminProviderQuotaRefreshRequest,
    OAUTH_ACCOUNT_BLOCK_PREFIX,
};
use crate::gateway::{AppState, GatewayError, GatewayPublicRequestContext};
use axum::{
    body::{Body, Bytes},
    http,
    response::{IntoResponse, Response},
    Json,
};
use serde_json::json;
use std::collections::BTreeSet;
use std::time::{SystemTime, UNIX_EPOCH};

pub(super) async fn maybe_build_local_admin_endpoints_keys_response(
    state: &AppState,
    request_context: &GatewayPublicRequestContext,
    request_body: Option<&Bytes>,
) -> Result<Option<Response<Body>>, GatewayError> {
    let Some(decision) = request_context.control_decision.as_ref() else {
        return Ok(None);
    };

    if decision.route_family.as_deref() == Some("endpoints_manage")
        && decision.route_kind.as_deref() == Some("keys_grouped_by_format")
        && request_context.request_path == "/api/admin/endpoints/keys/grouped-by-format"
    {
        let Some(payload) = build_admin_keys_grouped_by_format_payload(state).await else {
            return Ok(None);
        };
        return Ok(Some(Json(payload).into_response()));
    }

    if decision.route_family.as_deref() == Some("endpoints_manage")
        && decision.route_kind.as_deref() == Some("reveal_key")
        && request_context
            .request_path
            .starts_with("/api/admin/endpoints/keys/")
        && request_context.request_path.ends_with("/reveal")
    {
        let Some(key_id) = admin_reveal_key_id(&request_context.request_path) else {
            return Ok(Some(
                (
                    http::StatusCode::NOT_FOUND,
                    Json(json!({ "detail": "Key 不存在" })),
                )
                    .into_response(),
            ));
        };
        let Some(key) = state
            .read_provider_catalog_keys_by_ids(std::slice::from_ref(&key_id))
            .await?
            .into_iter()
            .next()
        else {
            return Ok(Some(
                (
                    http::StatusCode::NOT_FOUND,
                    Json(json!({ "detail": format!("Key {key_id} 不存在") })),
                )
                    .into_response(),
            ));
        };
        return Ok(Some(match build_admin_reveal_key_payload(state, &key) {
            Ok(payload) => Json(payload).into_response(),
            Err(detail) => (
                http::StatusCode::BAD_REQUEST,
                Json(json!({ "detail": detail })),
            )
                .into_response(),
        }));
    }

    if decision.route_family.as_deref() == Some("endpoints_manage")
        && decision.route_kind.as_deref() == Some("export_key")
        && request_context
            .request_path
            .starts_with("/api/admin/endpoints/keys/")
        && request_context.request_path.ends_with("/export")
    {
        let Some(key_id) = admin_export_key_id(&request_context.request_path) else {
            return Ok(Some(
                (
                    http::StatusCode::NOT_FOUND,
                    Json(json!({ "detail": "Key 不存在" })),
                )
                    .into_response(),
            ));
        };
        let Some(key) = state
            .read_provider_catalog_keys_by_ids(std::slice::from_ref(&key_id))
            .await?
            .into_iter()
            .next()
        else {
            return Ok(Some(
                (
                    http::StatusCode::NOT_FOUND,
                    Json(json!({ "detail": format!("Key {key_id} 不存在") })),
                )
                    .into_response(),
            ));
        };
        return Ok(Some(
            match build_admin_export_key_payload(state, &key).await {
                Ok(payload) => Json(payload).into_response(),
                Err(detail) => (
                    http::StatusCode::BAD_REQUEST,
                    Json(json!({ "detail": detail })),
                )
                    .into_response(),
            },
        ));
    }

    if decision.route_family.as_deref() == Some("endpoints_manage")
        && decision.route_kind.as_deref() == Some("update_key")
        && request_context.request_method == http::Method::PUT
        && request_context
            .request_path
            .starts_with("/api/admin/endpoints/keys/")
    {
        let Some(key_id) = admin_update_key_id(&request_context.request_path) else {
            return Ok(Some(
                (
                    http::StatusCode::NOT_FOUND,
                    Json(json!({ "detail": "Key 不存在" })),
                )
                    .into_response(),
            ));
        };
        let Some(request_body) = request_body else {
            return Ok(Some(
                (
                    http::StatusCode::BAD_REQUEST,
                    Json(json!({ "detail": "请求体不能为空" })),
                )
                    .into_response(),
            ));
        };
        if !state.has_provider_catalog_data_reader() {
            return Ok(None);
        }
        let raw_value = match serde_json::from_slice::<serde_json::Value>(request_body) {
            Ok(value) => value,
            Err(_) => {
                return Ok(Some(
                    (
                        http::StatusCode::BAD_REQUEST,
                        Json(json!({ "detail": "请求体必须是合法的 JSON 对象" })),
                    )
                        .into_response(),
                ));
            }
        };
        let Some(raw_payload) = raw_value.as_object().cloned() else {
            return Ok(Some(
                (
                    http::StatusCode::BAD_REQUEST,
                    Json(json!({ "detail": "请求体必须是合法的 JSON 对象" })),
                )
                    .into_response(),
            ));
        };
        let payload = match serde_json::from_value::<AdminProviderKeyUpdateRequest>(raw_value) {
            Ok(payload) => payload,
            Err(_) => {
                return Ok(Some(
                    (
                        http::StatusCode::BAD_REQUEST,
                        Json(json!({ "detail": "请求体必须是合法的 JSON 对象" })),
                    )
                        .into_response(),
                ));
            }
        };
        let Some(existing_key) = state
            .read_provider_catalog_keys_by_ids(std::slice::from_ref(&key_id))
            .await?
            .into_iter()
            .next()
        else {
            return Ok(Some(
                (
                    http::StatusCode::NOT_FOUND,
                    Json(json!({ "detail": format!("Key {key_id} 不存在") })),
                )
                    .into_response(),
            ));
        };
        let Some(provider) = state
            .read_provider_catalog_providers_by_ids(std::slice::from_ref(&existing_key.provider_id))
            .await?
            .into_iter()
            .next()
        else {
            return Ok(Some(
                (
                    http::StatusCode::NOT_FOUND,
                    Json(json!({ "detail": format!("Provider {} 不存在", existing_key.provider_id) })),
                )
                    .into_response(),
            ));
        };
        let updated_record = match build_admin_update_provider_key_record(
            state,
            &provider,
            &existing_key,
            &raw_payload,
            payload,
        )
        .await
        {
            Ok(record) => record,
            Err(detail) => {
                return Ok(Some(
                    (
                        http::StatusCode::BAD_REQUEST,
                        Json(json!({ "detail": detail })),
                    )
                        .into_response(),
                ));
            }
        };
        let Some(updated) = state.update_provider_catalog_key(&updated_record).await? else {
            return Ok(None);
        };
        let now_unix_secs = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .ok()
            .map(|duration| duration.as_secs())
            .unwrap_or(0);
        return Ok(Some(
            Json(build_admin_provider_key_response(
                state,
                &updated,
                now_unix_secs,
            ))
            .into_response(),
        ));
    }

    if decision.route_family.as_deref() == Some("endpoints_manage")
        && decision.route_kind.as_deref() == Some("delete_key")
        && request_context.request_method == http::Method::DELETE
        && request_context
            .request_path
            .starts_with("/api/admin/endpoints/keys/")
    {
        let Some(key_id) = admin_update_key_id(&request_context.request_path) else {
            return Ok(Some(
                (
                    http::StatusCode::NOT_FOUND,
                    Json(json!({ "detail": "Key 不存在" })),
                )
                    .into_response(),
            ));
        };
        let Some(_existing_key) = state
            .read_provider_catalog_keys_by_ids(std::slice::from_ref(&key_id))
            .await?
            .into_iter()
            .next()
        else {
            return Ok(Some(
                (
                    http::StatusCode::NOT_FOUND,
                    Json(json!({ "detail": format!("Key {key_id} 不存在") })),
                )
                    .into_response(),
            ));
        };
        if !state.delete_provider_catalog_key(&key_id).await? {
            return Ok(Some(
                (
                    http::StatusCode::NOT_FOUND,
                    Json(json!({ "detail": format!("Key {key_id} 不存在") })),
                )
                    .into_response(),
            ));
        }
        return Ok(Some(
            Json(json!({
                "message": format!("Key {key_id} 已删除")
            }))
            .into_response(),
        ));
    }

    if decision.route_family.as_deref() == Some("endpoints_manage")
        && decision.route_kind.as_deref() == Some("batch_delete_keys")
        && request_context.request_method == http::Method::POST
        && request_context.request_path == "/api/admin/endpoints/keys/batch-delete"
    {
        let Some(request_body) = request_body else {
            return Ok(Some(
                (
                    http::StatusCode::BAD_REQUEST,
                    Json(json!({ "detail": "请求体不能为空" })),
                )
                    .into_response(),
            ));
        };
        let payload =
            match serde_json::from_slice::<AdminProviderKeyBatchDeleteRequest>(request_body) {
                Ok(payload) => payload,
                Err(_) => {
                    return Ok(Some(
                        (
                            http::StatusCode::BAD_REQUEST,
                            Json(json!({ "detail": "请求体必须是合法的 JSON 对象" })),
                        )
                            .into_response(),
                    ));
                }
            };
        if payload.ids.len() > 100 {
            return Ok(Some(
                (
                    http::StatusCode::BAD_REQUEST,
                    Json(json!({ "detail": "ids 最多 100 个" })),
                )
                    .into_response(),
            ));
        }
        if payload.ids.is_empty() {
            return Ok(Some(
                Json(json!({
                    "success_count": 0,
                    "failed_count": 0,
                    "failed": []
                }))
                .into_response(),
            ));
        }

        let found_keys = state
            .read_provider_catalog_keys_by_ids(&payload.ids)
            .await?;
        let found_ids = found_keys
            .iter()
            .map(|key| key.id.clone())
            .collect::<BTreeSet<_>>();
        let mut failed = payload
            .ids
            .iter()
            .filter(|key_id| !found_ids.contains(*key_id))
            .map(|key_id| json!({ "id": key_id, "error": "not found" }))
            .collect::<Vec<_>>();

        let mut success_count = 0usize;
        for key_id in found_ids {
            if state.delete_provider_catalog_key(&key_id).await? {
                success_count += 1;
            } else {
                failed.push(json!({ "id": key_id, "error": "not found" }));
            }
        }

        return Ok(Some(
            Json(json!({
                "success_count": success_count,
                "failed_count": failed.len(),
                "failed": failed,
            }))
            .into_response(),
        ));
    }

    if decision.route_family.as_deref() == Some("endpoints_manage")
        && decision.route_kind.as_deref() == Some("clear_oauth_invalid")
        && request_context.request_method == http::Method::POST
        && request_context
            .request_path
            .starts_with("/api/admin/endpoints/keys/")
        && request_context
            .request_path
            .ends_with("/clear-oauth-invalid")
    {
        let Some(key_id) = admin_clear_oauth_invalid_key_id(&request_context.request_path) else {
            return Ok(Some(
                (
                    http::StatusCode::NOT_FOUND,
                    Json(json!({ "detail": "Key 不存在" })),
                )
                    .into_response(),
            ));
        };
        let Some(key) = state
            .read_provider_catalog_keys_by_ids(std::slice::from_ref(&key_id))
            .await?
            .into_iter()
            .next()
        else {
            return Ok(Some(
                (
                    http::StatusCode::NOT_FOUND,
                    Json(json!({ "detail": format!("Key {key_id} 不存在") })),
                )
                    .into_response(),
            ));
        };
        if key.oauth_invalid_at_unix_secs.is_none() {
            return Ok(Some(
                Json(json!({
                    "message": "该 Key 当前无失效标记，无需清除"
                }))
                .into_response(),
            ));
        }
        state
            .clear_provider_catalog_key_oauth_invalid_marker(&key_id)
            .await?;
        return Ok(Some(
            Json(json!({
                "message": "已清除 OAuth 失效标记"
            }))
            .into_response(),
        ));
    }

    if decision.route_family.as_deref() == Some("endpoints_manage")
        && decision.route_kind.as_deref() == Some("refresh_quota")
        && request_context.request_method == http::Method::POST
        && request_context
            .request_path
            .starts_with("/api/admin/endpoints/providers/")
        && request_context.request_path.ends_with("/refresh-quota")
    {
        let Some(provider_id) = admin_provider_id_for_refresh_quota(&request_context.request_path)
        else {
            return Ok(Some(
                (
                    http::StatusCode::NOT_FOUND,
                    Json(json!({ "detail": "Provider 不存在" })),
                )
                    .into_response(),
            ));
        };
        let Some(provider) = state
            .read_provider_catalog_providers_by_ids(std::slice::from_ref(&provider_id))
            .await?
            .into_iter()
            .next()
        else {
            return Ok(Some(
                (
                    http::StatusCode::NOT_FOUND,
                    Json(json!({ "detail": format!("Provider {provider_id} 不存在") })),
                )
                    .into_response(),
            ));
        };

        let normalized_provider_type = provider.provider_type.trim().to_ascii_lowercase();

        let payload = if let Some(request_body) = request_body {
            match serde_json::from_slice::<AdminProviderQuotaRefreshRequest>(request_body) {
                Ok(payload) => payload,
                Err(_) => {
                    return Ok(Some(
                        (
                            http::StatusCode::BAD_REQUEST,
                            Json(json!({ "detail": "请求体必须是合法的 JSON 对象" })),
                        )
                            .into_response(),
                    ));
                }
            }
        } else {
            AdminProviderQuotaRefreshRequest { key_ids: None }
        };

        let raw_key_ids = payload.key_ids;
        let selected_key_ids = normalize_string_id_list(raw_key_ids.clone());
        let explicit_key_ids_requested = raw_key_ids.is_some();
        let endpoints = state
            .list_provider_catalog_endpoints_by_provider_ids(std::slice::from_ref(&provider_id))
            .await?;
        let endpoint = match normalized_provider_type.as_str() {
            "codex" => endpoints.into_iter().find(|endpoint| {
                endpoint.is_active
                    && endpoint
                        .api_format
                        .trim()
                        .eq_ignore_ascii_case("openai:cli")
            }),
            "antigravity" => endpoints.into_iter().find(|endpoint| {
                endpoint.is_active
                    && (endpoint
                        .api_format
                        .trim()
                        .eq_ignore_ascii_case("gemini:chat")
                        || endpoint
                            .api_format
                            .trim()
                            .eq_ignore_ascii_case("gemini:cli"))
            }),
            "kiro" => endpoints
                .iter()
                .find(|endpoint| {
                    endpoint.is_active
                        && endpoint
                            .api_format
                            .trim()
                            .eq_ignore_ascii_case("claude:cli")
                })
                .cloned()
                .or_else(|| endpoints.into_iter().find(|endpoint| endpoint.is_active)),
            _ => return Ok(None),
        };

        let Some(endpoint) = endpoint else {
            let detail = match normalized_provider_type.as_str() {
                "codex" => "找不到有效的 openai:cli 端点",
                "antigravity" => "找不到有效的 gemini:chat/gemini:cli 端点",
                "kiro" => "找不到有效的 Kiro 端点",
                _ => "找不到有效端点",
            };
            return Ok(Some(
                (
                    http::StatusCode::BAD_REQUEST,
                    Json(json!({ "detail": detail })),
                )
                    .into_response(),
            ));
        };

        let mut keys = state
            .list_provider_catalog_keys_by_provider_ids(std::slice::from_ref(&provider_id))
            .await?;
        keys = if let Some(selected_key_ids) = selected_key_ids.as_ref() {
            if selected_key_ids.is_empty() {
                Vec::new()
            } else {
                let selected = selected_key_ids.iter().cloned().collect::<BTreeSet<_>>();
                keys.into_iter()
                    .filter(|key| selected.contains(&key.id))
                    .collect()
            }
        } else {
            keys.into_iter()
                .filter(|key| {
                    key.is_active
                        || key
                            .oauth_invalid_reason
                            .as_deref()
                            .map(str::trim)
                            .is_some_and(|value| value.starts_with(OAUTH_ACCOUNT_BLOCK_PREFIX))
                })
                .collect()
        };

        if explicit_key_ids_requested && selected_key_ids.is_none() {
            return Ok(Some(
                Json(json!({
                    "success": 0,
                    "failed": 0,
                    "total": 0,
                    "results": [],
                    "message": "未提供可刷新的 Key",
                    "auto_removed": 0,
                }))
                .into_response(),
            ));
        }

        if keys.is_empty() {
            let message = if explicit_key_ids_requested {
                "未提供可刷新的 Key"
            } else {
                "没有可刷新的 Key"
            };
            return Ok(Some(
                Json(json!({
                    "success": 0,
                    "failed": 0,
                    "total": 0,
                    "results": [],
                    "message": message,
                    "auto_removed": 0,
                }))
                .into_response(),
            ));
        }

        let Some(payload) = (match normalized_provider_type.as_str() {
            "codex" => {
                refresh_codex_provider_quota_locally(state, &provider, &endpoint, keys).await?
            }
            "kiro" => {
                refresh_kiro_provider_quota_locally(state, &provider, &endpoint, keys).await?
            }
            "antigravity" => {
                refresh_antigravity_provider_quota_locally(state, &provider, &endpoint, keys)
                    .await?
            }
            _ => None,
        }) else {
            return Ok(None);
        };
        return Ok(Some(Json(payload).into_response()));
    }

    if decision.route_family.as_deref() == Some("endpoints_manage")
        && decision.route_kind.as_deref() == Some("create_provider_key")
        && request_context.request_method == http::Method::POST
        && request_context
            .request_path
            .starts_with("/api/admin/endpoints/providers/")
        && request_context.request_path.ends_with("/keys")
    {
        let Some(provider_id) = admin_provider_id_for_keys(&request_context.request_path) else {
            return Ok(Some(
                (
                    http::StatusCode::NOT_FOUND,
                    Json(json!({ "detail": "Provider 不存在" })),
                )
                    .into_response(),
            ));
        };
        let Some(request_body) = request_body else {
            return Ok(Some(
                (
                    http::StatusCode::BAD_REQUEST,
                    Json(json!({ "detail": "请求体不能为空" })),
                )
                    .into_response(),
            ));
        };
        if !state.has_provider_catalog_data_reader() {
            return Ok(None);
        }
        let payload = match serde_json::from_slice::<AdminProviderKeyCreateRequest>(request_body) {
            Ok(payload) => payload,
            Err(_) => {
                return Ok(Some(
                    (
                        http::StatusCode::BAD_REQUEST,
                        Json(json!({ "detail": "请求体必须是合法的 JSON 对象" })),
                    )
                        .into_response(),
                ));
            }
        };
        let Some(provider) = state
            .read_provider_catalog_providers_by_ids(std::slice::from_ref(&provider_id))
            .await?
            .into_iter()
            .next()
        else {
            return Ok(Some(
                (
                    http::StatusCode::NOT_FOUND,
                    Json(json!({ "detail": format!("Provider {provider_id} 不存在") })),
                )
                    .into_response(),
            ));
        };
        let record = match build_admin_create_provider_key_record(state, &provider, payload).await {
            Ok(record) => record,
            Err(detail) => {
                return Ok(Some(
                    (
                        http::StatusCode::BAD_REQUEST,
                        Json(json!({ "detail": detail })),
                    )
                        .into_response(),
                ));
            }
        };
        let Some(created) = state.create_provider_catalog_key(&record).await? else {
            return Ok(None);
        };
        let now_unix_secs = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .ok()
            .map(|duration| duration.as_secs())
            .unwrap_or(0);
        return Ok(Some(
            Json(build_admin_provider_key_response(
                state,
                &created,
                now_unix_secs,
            ))
            .into_response(),
        ));
    }

    if decision.route_family.as_deref() == Some("endpoints_manage")
        && decision.route_kind.as_deref() == Some("list_provider_keys")
        && request_context
            .request_path
            .starts_with("/api/admin/endpoints/providers/")
        && request_context.request_path.ends_with("/keys")
    {
        let Some(provider_id) = admin_provider_id_for_keys(&request_context.request_path) else {
            return Ok(Some(
                (
                    http::StatusCode::NOT_FOUND,
                    Json(json!({ "detail": "Provider 不存在" })),
                )
                    .into_response(),
            ));
        };
        let skip = query_param_value(request_context.request_query_string.as_deref(), "skip")
            .and_then(|value| value.parse::<usize>().ok())
            .unwrap_or(0);
        let limit = query_param_value(request_context.request_query_string.as_deref(), "limit")
            .and_then(|value| value.parse::<usize>().ok())
            .filter(|value| *value > 0)
            .unwrap_or(100);
        return Ok(Some(
            match build_admin_provider_keys_payload(state, &provider_id, skip, limit).await {
                Some(payload) => Json(payload).into_response(),
                None => (
                    http::StatusCode::NOT_FOUND,
                    Json(json!({ "detail": format!("Provider {provider_id} 不存在") })),
                )
                    .into_response(),
            },
        ));
    }

    Ok(None)
}
