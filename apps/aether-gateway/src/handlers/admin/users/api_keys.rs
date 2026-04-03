use super::{
    build_admin_users_bad_request_response, build_admin_users_data_unavailable_response,
    build_admin_users_read_only_response, AdminCreateUserApiKeyRequest,
    AdminToggleUserApiKeyLockRequest, AdminUpdateUserApiKeyRequest,
};
use crate::gateway::handlers::{
    decrypt_catalog_secret_with_fallbacks, encrypt_catalog_secret_with_fallbacks,
    query_param_optional_bool,
};
use crate::gateway::{AppState, GatewayError, GatewayPublicRequestContext};
use axum::{
    body::Body,
    http,
    response::{IntoResponse, Response},
    Json,
};
use serde_json::json;

fn admin_user_api_key_full_key_parts(request_path: &str) -> Option<(String, String)> {
    let raw = request_path.strip_prefix("/api/admin/users/")?;
    let (user_id, key_id) = raw.split_once("/api-keys/")?;
    let user_id = user_id.trim().trim_matches('/');
    let key_id = key_id
        .trim()
        .trim_matches('/')
        .strip_suffix("/full-key")?
        .trim()
        .trim_matches('/');
    if user_id.is_empty() || key_id.is_empty() || user_id.contains('/') || key_id.contains('/') {
        None
    } else {
        Some((user_id.to_string(), key_id.to_string()))
    }
}

fn admin_user_api_key_parts(request_path: &str) -> Option<(String, String)> {
    let raw = request_path.strip_prefix("/api/admin/users/")?;
    let (user_id, key_id) = raw.split_once("/api-keys/")?;
    let user_id = user_id.trim().trim_matches('/');
    let key_id = key_id.trim().trim_matches('/');
    if user_id.is_empty() || key_id.is_empty() || user_id.contains('/') || key_id.contains('/') {
        None
    } else {
        Some((user_id.to_string(), key_id.to_string()))
    }
}

fn admin_user_api_key_lock_parts(request_path: &str) -> Option<(String, String)> {
    let raw = request_path.strip_prefix("/api/admin/users/")?;
    let (user_id, key_id) = raw.split_once("/api-keys/")?;
    let user_id = user_id.trim().trim_matches('/');
    let key_id = key_id
        .trim()
        .trim_matches('/')
        .strip_suffix("/lock")?
        .trim()
        .trim_matches('/');
    if user_id.is_empty() || key_id.is_empty() || user_id.contains('/') || key_id.contains('/') {
        None
    } else {
        Some((user_id.to_string(), key_id.to_string()))
    }
}

pub(crate) fn format_optional_unix_secs_iso8601(value: Option<u64>) -> Option<String> {
    let secs = value?;
    let secs = i64::try_from(secs).ok()?;
    chrono::DateTime::<chrono::Utc>::from_timestamp(secs, 0).map(|value| value.to_rfc3339())
}

fn admin_user_id_from_api_keys_path(request_path: &str) -> Option<String> {
    request_path
        .strip_prefix("/api/admin/users/")?
        .strip_suffix("/api-keys")
        .map(|value| value.trim().trim_matches('/').to_string())
        .filter(|value| !value.is_empty() && !value.contains('/'))
}

pub(crate) fn masked_user_api_key_display(state: &AppState, ciphertext: Option<&str>) -> String {
    let Some(ciphertext) = ciphertext.map(str::trim).filter(|value| !value.is_empty()) else {
        return "sk-****".to_string();
    };
    let Some(full_key) = decrypt_catalog_secret_with_fallbacks(state.encryption_key(), ciphertext)
    else {
        return "sk-****".to_string();
    };
    let prefix_len = full_key.len().min(10);
    let prefix = &full_key[..prefix_len];
    let suffix = if full_key.len() >= 4 {
        &full_key[full_key.len() - 4..]
    } else {
        ""
    };
    format!("{prefix}...{suffix}")
}

fn build_admin_user_api_key_detail_payload(
    state: &AppState,
    record: &aether_data::repository::auth::StoredAuthApiKeyExportRecord,
    is_locked: bool,
) -> serde_json::Value {
    json!({
        "id": record.api_key_id,
        "name": record.name,
        "key_display": masked_user_api_key_display(state, record.key_encrypted.as_deref()),
        "is_active": record.is_active,
        "is_locked": is_locked,
        "total_requests": record.total_requests,
        "total_cost_usd": record.total_cost_usd,
        "rate_limit": record.rate_limit,
        "expires_at": format_optional_unix_secs_iso8601(record.expires_at_unix_secs),
        "last_used_at": serde_json::Value::Null,
        "created_at": serde_json::Value::Null,
    })
}

pub(crate) fn normalize_admin_optional_api_key_name(
    value: Option<String>,
) -> Result<Option<String>, String> {
    match value {
        None => Ok(None),
        Some(value) => {
            let trimmed = value.trim();
            if trimmed.is_empty() {
                return Err("API密钥名称不能为空".to_string());
            }
            Ok(Some(trimmed.chars().take(100).collect()))
        }
    }
}

fn normalize_admin_api_key_providers(
    value: Option<Vec<String>>,
) -> Result<Option<Vec<String>>, String> {
    let Some(values) = value else {
        return Ok(None);
    };
    let mut normalized = Vec::new();
    let mut seen = std::collections::BTreeSet::new();
    for provider_id in values {
        let provider_id = provider_id.trim();
        if provider_id.is_empty() {
            return Err("提供商ID不能为空".to_string());
        }
        if seen.insert(provider_id.to_string()) {
            normalized.push(provider_id.to_string());
        }
    }
    Ok(Some(normalized))
}

pub(crate) fn generate_admin_user_api_key_plaintext() -> String {
    let first = uuid::Uuid::new_v4().simple().to_string();
    let second = uuid::Uuid::new_v4().simple().to_string();
    format!("sk-{}{}", first, &second[..16])
}

pub(crate) fn hash_admin_user_api_key(value: &str) -> String {
    use sha2::Digest;

    let mut hasher = sha2::Sha256::new();
    hasher.update(value.as_bytes());
    format!("{:x}", hasher.finalize())
}

pub(crate) fn default_admin_user_api_key_name() -> String {
    format!("API Key {}", chrono::Utc::now().format("%Y%m%d%H%M%S"))
}

pub(super) async fn build_admin_list_user_api_keys_response(
    state: &AppState,
    request_context: &GatewayPublicRequestContext,
) -> Result<Response<Body>, GatewayError> {
    let Some(user_id) = admin_user_id_from_api_keys_path(&request_context.request_path) else {
        return Ok(build_admin_users_bad_request_response("缺少 user_id"));
    };
    let Some(user) = state.find_user_auth_by_id(&user_id).await? else {
        return Ok((
            http::StatusCode::NOT_FOUND,
            Json(json!({ "detail": "用户不存在" })),
        )
            .into_response());
    };

    let active_filter =
        query_param_optional_bool(request_context.request_query_string.as_deref(), "is_active");
    let mut export_records = state
        .list_auth_api_key_export_records_by_user_ids(std::slice::from_ref(&user_id))
        .await?;
    if let Some(is_active) = active_filter {
        export_records.retain(|record| record.is_active == is_active);
    }

    let snapshot_ids = export_records
        .iter()
        .map(|record| record.api_key_id.clone())
        .collect::<Vec<_>>();
    let snapshot_by_id = state
        .read_auth_api_key_snapshots_by_ids(&snapshot_ids)
        .await?
        .into_iter()
        .map(|snapshot| (snapshot.api_key_id.clone(), snapshot))
        .collect::<std::collections::BTreeMap<_, _>>();

    let api_keys = export_records
        .into_iter()
        .map(|record| {
            let is_locked = snapshot_by_id
                .get(&record.api_key_id)
                .map(|snapshot| snapshot.api_key_is_locked)
                .unwrap_or(false);
            json!({
                "id": record.api_key_id,
                "name": record.name,
                "key_display": masked_user_api_key_display(state, record.key_encrypted.as_deref()),
                "is_active": record.is_active,
                "is_locked": is_locked,
                "total_requests": record.total_requests,
                "total_cost_usd": record.total_cost_usd,
                "rate_limit": record.rate_limit,
                "expires_at": format_optional_unix_secs_iso8601(record.expires_at_unix_secs),
                "last_used_at": serde_json::Value::Null,
                "created_at": serde_json::Value::Null,
            })
        })
        .collect::<Vec<_>>();

    Ok(Json(json!({
        "api_keys": api_keys,
        "total": api_keys.len(),
        "user_email": user.email,
        "username": user.username,
    }))
    .into_response())
}

pub(super) async fn build_admin_create_user_api_key_response(
    state: &AppState,
    request_context: &GatewayPublicRequestContext,
    request_body: Option<&axum::body::Bytes>,
) -> Result<Response<Body>, GatewayError> {
    if !state.has_auth_api_key_writer() {
        return Ok(build_admin_users_read_only_response(
            "当前为只读模式，无法创建用户 API Key",
        ));
    }

    let Some(user_id) = admin_user_id_from_api_keys_path(&request_context.request_path) else {
        return Ok(build_admin_users_bad_request_response("缺少 user_id"));
    };
    if state.find_user_auth_by_id(&user_id).await?.is_none() {
        return Ok((
            http::StatusCode::NOT_FOUND,
            Json(json!({ "detail": "用户不存在" })),
        )
            .into_response());
    }

    let Some(request_body) = request_body else {
        return Ok((
            http::StatusCode::BAD_REQUEST,
            Json(json!({ "detail": "请求数据验证失败" })),
        )
            .into_response());
    };
    let payload = match serde_json::from_slice::<AdminCreateUserApiKeyRequest>(request_body) {
        Ok(value) => value,
        Err(_) => {
            return Ok((
                http::StatusCode::BAD_REQUEST,
                Json(json!({ "detail": "请求数据验证失败" })),
            )
                .into_response())
        }
    };
    if payload.allowed_api_formats.is_some()
        || payload.allowed_models.is_some()
        || payload.expire_days.is_some()
        || payload.expires_at.is_some()
        || payload.initial_balance_usd.is_some()
        || payload.unlimited_balance.unwrap_or(false)
        || payload.is_standalone.unwrap_or(false)
        || payload.auto_delete_on_expiry.unwrap_or(false)
    {
        return Ok((
            http::StatusCode::BAD_REQUEST,
            Json(json!({ "detail": "当前仅支持 name、rate_limit、allowed_providers 字段" })),
        )
            .into_response());
    }

    let name = match normalize_admin_optional_api_key_name(payload.name) {
        Ok(Some(value)) => value,
        Ok(None) => default_admin_user_api_key_name(),
        Err(detail) => {
            return Ok((
                http::StatusCode::BAD_REQUEST,
                Json(json!({ "detail": detail })),
            )
                .into_response())
        }
    };
    let allowed_providers = match normalize_admin_api_key_providers(payload.allowed_providers) {
        Ok(value) => value,
        Err(detail) => {
            return Ok((
                http::StatusCode::BAD_REQUEST,
                Json(json!({ "detail": detail })),
            )
                .into_response())
        }
    };
    let rate_limit = payload.rate_limit.unwrap_or(0);
    if rate_limit < 0 {
        return Ok((
            http::StatusCode::BAD_REQUEST,
            Json(json!({ "detail": "rate_limit 必须大于等于 0" })),
        )
            .into_response());
    }

    let plaintext_key = generate_admin_user_api_key_plaintext();
    let Some(key_encrypted) = encrypt_catalog_secret_with_fallbacks(state, &plaintext_key) else {
        return Ok((
            http::StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({ "detail": "API密钥加密失败" })),
        )
            .into_response());
    };

    let Some(created) = state
        .create_user_api_key(aether_data::repository::auth::CreateUserApiKeyRecord {
            user_id: user_id.clone(),
            api_key_id: uuid::Uuid::new_v4().to_string(),
            key_hash: hash_admin_user_api_key(&plaintext_key),
            key_encrypted: Some(key_encrypted),
            name: Some(name.clone()),
            rate_limit,
            concurrent_limit: 5,
        })
        .await?
    else {
        return Ok(build_admin_users_data_unavailable_response());
    };

    let created = if allowed_providers.is_some() {
        match state
            .set_user_api_key_allowed_providers(&user_id, &created.api_key_id, allowed_providers)
            .await?
        {
            Some(updated) => updated,
            None => created,
        }
    } else {
        created
    };

    Ok(Json(json!({
        "id": created.api_key_id,
        "key": plaintext_key,
        "name": created.name,
        "key_display": masked_user_api_key_display(state, created.key_encrypted.as_deref()),
        "rate_limit": created.rate_limit,
        "expires_at": format_optional_unix_secs_iso8601(created.expires_at_unix_secs),
        "created_at": chrono::Utc::now().to_rfc3339(),
        "message": "API Key创建成功，请妥善保存完整密钥",
    }))
    .into_response())
}

pub(super) async fn build_admin_update_user_api_key_response(
    state: &AppState,
    request_context: &GatewayPublicRequestContext,
    request_body: Option<&axum::body::Bytes>,
) -> Result<Response<Body>, GatewayError> {
    if !state.has_auth_api_key_writer() {
        return Ok(build_admin_users_read_only_response(
            "当前为只读模式，无法更新用户 API Key",
        ));
    }

    let Some((user_id, api_key_id)) = admin_user_api_key_parts(&request_context.request_path)
    else {
        return Ok(build_admin_users_bad_request_response(
            "缺少 user_id 或 key_id",
        ));
    };
    let Some(request_body) = request_body else {
        return Ok((
            http::StatusCode::BAD_REQUEST,
            Json(json!({ "detail": "请求数据验证失败" })),
        )
            .into_response());
    };
    let payload = match serde_json::from_slice::<AdminUpdateUserApiKeyRequest>(request_body) {
        Ok(value) => value,
        Err(_) => {
            return Ok((
                http::StatusCode::BAD_REQUEST,
                Json(json!({ "detail": "请求数据验证失败" })),
            )
                .into_response())
        }
    };
    let name = match normalize_admin_optional_api_key_name(payload.name) {
        Ok(value) => value,
        Err(detail) => {
            return Ok((
                http::StatusCode::BAD_REQUEST,
                Json(json!({ "detail": detail })),
            )
                .into_response())
        }
    };
    if payload.rate_limit.is_some_and(|value| value < 0) {
        return Ok((
            http::StatusCode::BAD_REQUEST,
            Json(json!({ "detail": "rate_limit 必须大于等于 0" })),
        )
            .into_response());
    }

    let Some(updated) = state
        .update_user_api_key_basic(aether_data::repository::auth::UpdateUserApiKeyBasicRecord {
            user_id,
            api_key_id: api_key_id.clone(),
            name,
            rate_limit: payload.rate_limit,
        })
        .await?
    else {
        return Ok((
            http::StatusCode::NOT_FOUND,
            Json(json!({ "detail": "API Key不存在或不属于该用户" })),
        )
            .into_response());
    };

    let is_locked = state
        .read_auth_api_key_snapshots_by_ids(std::slice::from_ref(&api_key_id))
        .await?
        .into_iter()
        .find(|snapshot| snapshot.api_key_id == api_key_id)
        .map(|snapshot| snapshot.api_key_is_locked)
        .unwrap_or(false);
    let mut payload = build_admin_user_api_key_detail_payload(state, &updated, is_locked);
    payload["message"] = json!("API Key更新成功");
    Ok(Json(payload).into_response())
}

pub(super) async fn build_admin_delete_user_api_key_response(
    state: &AppState,
    request_context: &GatewayPublicRequestContext,
) -> Result<Response<Body>, GatewayError> {
    if !state.has_auth_api_key_writer() {
        return Ok(build_admin_users_read_only_response(
            "当前为只读模式，无法删除用户 API Key",
        ));
    }

    let Some((user_id, api_key_id)) = admin_user_api_key_parts(&request_context.request_path)
    else {
        return Ok(build_admin_users_bad_request_response(
            "缺少 user_id 或 key_id",
        ));
    };

    match state.delete_user_api_key(&user_id, &api_key_id).await? {
        true => Ok(Json(json!({ "message": "API Key已删除" })).into_response()),
        false => Ok((
            http::StatusCode::NOT_FOUND,
            Json(json!({ "detail": "API Key不存在或不属于该用户" })),
        )
            .into_response()),
    }
}

pub(super) async fn build_admin_toggle_user_api_key_lock_response(
    state: &AppState,
    request_context: &GatewayPublicRequestContext,
    request_body: Option<&axum::body::Bytes>,
) -> Result<Response<Body>, GatewayError> {
    if !state.has_auth_api_key_writer() {
        return Ok(build_admin_users_read_only_response(
            "当前为只读模式，无法锁定或解锁用户 API Key",
        ));
    }

    let Some((user_id, api_key_id)) = admin_user_api_key_lock_parts(&request_context.request_path)
    else {
        return Ok(build_admin_users_bad_request_response(
            "缺少 user_id 或 key_id",
        ));
    };

    let Some(snapshot) = state
        .read_auth_api_key_snapshots_by_ids(std::slice::from_ref(&api_key_id))
        .await?
        .into_iter()
        .find(|snapshot| snapshot.user_id == user_id && snapshot.api_key_id == api_key_id)
    else {
        return Ok((
            http::StatusCode::NOT_FOUND,
            Json(json!({ "detail": "API Key不存在或不属于该用户" })),
        )
            .into_response());
    };

    if snapshot.api_key_is_standalone {
        return Ok((
            http::StatusCode::NOT_FOUND,
            Json(json!({ "detail": "API Key不存在或不属于该用户" })),
        )
            .into_response());
    }

    let desired_is_locked = match request_body {
        None => !snapshot.api_key_is_locked,
        Some(body) if body.is_empty() => !snapshot.api_key_is_locked,
        Some(body) => match serde_json::from_slice::<AdminToggleUserApiKeyLockRequest>(body) {
            Ok(payload) => payload.locked.unwrap_or(!snapshot.api_key_is_locked),
            Err(_) => {
                return Ok((
                    http::StatusCode::BAD_REQUEST,
                    Json(json!({ "detail": "请求数据验证失败" })),
                )
                    .into_response())
            }
        },
    };

    if !state
        .set_user_api_key_locked(&user_id, &api_key_id, desired_is_locked)
        .await?
    {
        return Ok((
            http::StatusCode::NOT_FOUND,
            Json(json!({ "detail": "API Key不存在或不属于该用户" })),
        )
            .into_response());
    }

    Ok(Json(json!({
        "id": api_key_id,
        "is_locked": desired_is_locked,
        "message": if desired_is_locked {
            "API密钥已锁定"
        } else {
            "API密钥已解锁"
        },
    }))
    .into_response())
}

pub(super) async fn build_admin_reveal_user_api_key_response(
    state: &AppState,
    request_context: &GatewayPublicRequestContext,
) -> Result<Response<Body>, GatewayError> {
    let Some((user_id, key_id)) = admin_user_api_key_full_key_parts(&request_context.request_path)
    else {
        return Ok(build_admin_users_bad_request_response(
            "缺少 user_id 或 key_id",
        ));
    };

    let records = state
        .list_auth_api_key_export_records_by_user_ids(std::slice::from_ref(&user_id))
        .await?;
    let Some(record) = records
        .into_iter()
        .find(|record| record.api_key_id == key_id)
    else {
        return Ok((
            http::StatusCode::NOT_FOUND,
            Json(json!({ "detail": "API Key不存在或不属于该用户" })),
        )
            .into_response());
    };

    let Some(ciphertext) = record.key_encrypted.as_deref().map(str::trim) else {
        return Ok((
            http::StatusCode::BAD_REQUEST,
            Json(json!({ "detail": "该密钥没有存储完整密钥信息" })),
        )
            .into_response());
    };
    if ciphertext.is_empty() {
        return Ok((
            http::StatusCode::BAD_REQUEST,
            Json(json!({ "detail": "该密钥没有存储完整密钥信息" })),
        )
            .into_response());
    }

    let Some(full_key) = decrypt_catalog_secret_with_fallbacks(state.encryption_key(), ciphertext)
    else {
        return Ok((
            http::StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({ "detail": "解密密钥失败" })),
        )
            .into_response());
    };

    Ok(Json(json!({ "key": full_key })).into_response())
}
