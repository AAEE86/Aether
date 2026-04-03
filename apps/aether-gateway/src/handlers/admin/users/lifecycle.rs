use super::{
    admin_default_user_initial_gift, build_admin_users_bad_request_response,
    build_admin_users_data_unavailable_response, build_admin_users_read_only_response,
    format_optional_datetime_iso8601, normalize_admin_optional_user_email,
    normalize_admin_user_api_formats, normalize_admin_user_role, normalize_admin_user_string_list,
    normalize_admin_username, validate_admin_user_password, AdminCreateUserRequest,
    AdminUpdateUserFieldPresence, AdminUpdateUserRequest,
};
use crate::gateway::handlers::{query_param_optional_bool, query_param_value};
use crate::gateway::{AppState, GatewayError, GatewayPublicRequestContext};
use axum::{
    body::Body,
    http,
    response::{IntoResponse, Response},
    Json,
};
use serde_json::json;

async fn admin_user_password_policy(state: &AppState) -> Result<String, GatewayError> {
    let config = state
        .read_system_config_json_value("password_policy_level")
        .await?;
    Ok(
        match config
            .as_ref()
            .and_then(|value| value.as_str())
            .unwrap_or("weak")
            .trim()
            .to_ascii_lowercase()
            .as_str()
        {
            "medium" => "medium".to_string(),
            "strong" => "strong".to_string(),
            _ => "weak".to_string(),
        },
    )
}

async fn find_admin_export_user(
    state: &AppState,
    user_id: &str,
) -> Result<Option<aether_data::repository::users::StoredUserExportRow>, GatewayError> {
    state.find_export_user_by_id(user_id).await
}

fn build_admin_user_payload(
    user: &aether_data::repository::users::StoredUserAuthRecord,
    rate_limit: Option<i32>,
    unlimited: bool,
) -> serde_json::Value {
    json!({
        "id": user.id,
        "email": user.email,
        "username": user.username,
        "role": user.role,
        "allowed_providers": user.allowed_providers,
        "allowed_api_formats": user.allowed_api_formats,
        "allowed_models": user.allowed_models,
        "rate_limit": rate_limit,
        "unlimited": unlimited,
        "is_active": user.is_active,
        "created_at": format_optional_datetime_iso8601(user.created_at),
        "updated_at": serde_json::Value::Null,
        "last_login_at": format_optional_datetime_iso8601(user.last_login_at),
    })
}

fn admin_user_id_from_detail_path(request_path: &str) -> Option<String> {
    let value = request_path
        .strip_prefix("/api/admin/users/")?
        .trim()
        .trim_matches('/')
        .to_string();
    if value.is_empty() || value.contains('/') {
        None
    } else {
        Some(value)
    }
}

pub(super) async fn build_admin_list_users_response(
    state: &AppState,
    request_context: &GatewayPublicRequestContext,
) -> Result<Response<Body>, GatewayError> {
    let skip = query_param_value(request_context.request_query_string.as_deref(), "skip")
        .and_then(|value| value.parse::<usize>().ok())
        .unwrap_or(0);
    let limit = query_param_value(request_context.request_query_string.as_deref(), "limit")
        .and_then(|value| value.parse::<usize>().ok())
        .unwrap_or(100)
        .clamp(1, 1000);
    let role = query_param_value(request_context.request_query_string.as_deref(), "role")
        .map(|value| value.trim().to_ascii_lowercase())
        .filter(|value| !value.is_empty());
    let is_active =
        query_param_optional_bool(request_context.request_query_string.as_deref(), "is_active");

    let paged_rows = state
        .list_export_users_page(&aether_data::repository::users::UserExportListQuery {
            skip,
            limit,
            role: role.clone(),
            is_active,
        })
        .await?;
    let user_ids = paged_rows
        .iter()
        .map(|row| row.id.clone())
        .collect::<Vec<_>>();
    let auth_by_user_id = state
        .list_user_auth_by_ids(&user_ids)
        .await?
        .into_iter()
        .map(|user| (user.id.clone(), user))
        .collect::<std::collections::BTreeMap<_, _>>();
    let wallet_by_user_id = state
        .list_wallet_snapshots_by_user_ids(&user_ids)
        .await?
        .into_iter()
        .filter_map(|wallet| wallet.user_id.clone().map(|user_id| (user_id, wallet)))
        .collect::<std::collections::BTreeMap<_, _>>();

    let mut payload = Vec::with_capacity(paged_rows.len());
    for row in paged_rows {
        let auth = auth_by_user_id.get(&row.id);
        let unlimited = wallet_by_user_id
            .get(&row.id)
            .is_some_and(|wallet| wallet.limit_mode.eq_ignore_ascii_case("unlimited"));
        payload.push(json!({
            "id": row.id,
            "email": row.email,
            "username": row.username,
            "role": row.role,
            "allowed_providers": row.allowed_providers,
            "allowed_api_formats": row.allowed_api_formats,
            "allowed_models": row.allowed_models,
            "rate_limit": row.rate_limit,
            "unlimited": unlimited,
            "is_active": row.is_active,
            "created_at": format_optional_datetime_iso8601(auth.as_ref().and_then(|user| user.created_at)),
            "updated_at": serde_json::Value::Null,
            "last_login_at": format_optional_datetime_iso8601(
                auth.as_ref().and_then(|user| user.last_login_at),
            ),
        }));
    }

    Ok(Json(payload).into_response())
}

pub(super) async fn build_admin_get_user_response(
    state: &AppState,
    request_context: &GatewayPublicRequestContext,
) -> Result<Response<Body>, GatewayError> {
    let Some(user_id) = admin_user_id_from_detail_path(&request_context.request_path) else {
        return Ok(build_admin_users_bad_request_response("缺少 user_id"));
    };
    let Some(user) = state.find_user_auth_by_id(&user_id).await? else {
        return Ok((
            http::StatusCode::NOT_FOUND,
            Json(json!({ "detail": "用户不存在" })),
        )
            .into_response());
    };

    let wallet = state
        .find_wallet(aether_data::repository::wallet::WalletLookupKey::UserId(
            &user_id,
        ))
        .await?;
    let export_row = find_admin_export_user(state, &user_id).await?;
    let unlimited = wallet
        .as_ref()
        .is_some_and(|wallet| wallet.limit_mode.eq_ignore_ascii_case("unlimited"));
    Ok(Json(build_admin_user_payload(
        &user,
        export_row.as_ref().and_then(|row| row.rate_limit),
        unlimited,
    ))
    .into_response())
}

pub(super) async fn build_admin_create_user_response(
    state: &AppState,
    request_context: &GatewayPublicRequestContext,
    request_body: Option<&axum::body::Bytes>,
) -> Result<Response<Body>, GatewayError> {
    let _ = request_context;
    if !state.has_auth_user_write_capability() {
        return Ok(build_admin_users_read_only_response(
            "当前为只读模式，无法创建用户",
        ));
    }
    if !state.has_auth_wallet_write_capability() {
        return Ok(build_admin_users_read_only_response(
            "当前为只读模式，无法初始化用户钱包",
        ));
    }
    let Some(request_body) = request_body else {
        return Ok((
            http::StatusCode::BAD_REQUEST,
            Json(json!({ "detail": "请求数据验证失败" })),
        )
            .into_response());
    };
    let payload = match serde_json::from_slice::<AdminCreateUserRequest>(request_body) {
        Ok(value) => value,
        Err(_) => {
            return Ok((
                http::StatusCode::BAD_REQUEST,
                Json(json!({ "detail": "请求数据验证失败" })),
            )
                .into_response())
        }
    };

    let email = match normalize_admin_optional_user_email(payload.email.as_deref()) {
        Ok(value) => value,
        Err(detail) => {
            return Ok((
                http::StatusCode::BAD_REQUEST,
                Json(json!({ "detail": detail })),
            )
                .into_response())
        }
    };
    let username = match normalize_admin_username(&payload.username) {
        Ok(value) => value,
        Err(detail) => {
            return Ok((
                http::StatusCode::BAD_REQUEST,
                Json(json!({ "detail": detail })),
            )
                .into_response())
        }
    };
    let role = match normalize_admin_user_role(payload.role.as_deref()) {
        Ok(value) => value,
        Err(detail) => {
            return Ok((
                http::StatusCode::BAD_REQUEST,
                Json(json!({ "detail": detail })),
            )
                .into_response())
        }
    };
    let password_policy = admin_user_password_policy(state).await?;
    if let Err(detail) = validate_admin_user_password(&payload.password, &password_policy) {
        return Ok((
            http::StatusCode::BAD_REQUEST,
            Json(json!({ "detail": detail })),
        )
            .into_response());
    }
    if payload.rate_limit.is_some_and(|value| value < 0) {
        return Ok((
            http::StatusCode::BAD_REQUEST,
            Json(json!({ "detail": "rate_limit 必须大于等于 0" })),
        )
            .into_response());
    }
    if payload
        .initial_gift_usd
        .is_some_and(|value| !value.is_finite() || !(0.0..=10000.0).contains(&value))
    {
        return Ok((
            http::StatusCode::BAD_REQUEST,
            Json(json!({ "detail": "初始赠款必须在 0-10000 范围内" })),
        )
            .into_response());
    }
    let allowed_providers =
        match normalize_admin_user_string_list(payload.allowed_providers, "allowed_providers") {
            Ok(value) => value,
            Err(detail) => {
                return Ok((
                    http::StatusCode::BAD_REQUEST,
                    Json(json!({ "detail": detail })),
                )
                    .into_response())
            }
        };
    let allowed_api_formats = match normalize_admin_user_api_formats(payload.allowed_api_formats) {
        Ok(value) => value,
        Err(detail) => {
            return Ok((
                http::StatusCode::BAD_REQUEST,
                Json(json!({ "detail": detail })),
            )
                .into_response())
        }
    };
    let allowed_models =
        match normalize_admin_user_string_list(payload.allowed_models, "allowed_models") {
            Ok(value) => value,
            Err(detail) => {
                return Ok((
                    http::StatusCode::BAD_REQUEST,
                    Json(json!({ "detail": detail })),
                )
                    .into_response())
            }
        };

    if let Some(email) = email.as_deref() {
        if state.find_user_auth_by_identifier(email).await?.is_some() {
            return Ok((
                http::StatusCode::BAD_REQUEST,
                Json(json!({ "detail": format!("邮箱已存在: {email}") })),
            )
                .into_response());
        }
    }
    if state
        .find_user_auth_by_identifier(&username)
        .await?
        .is_some()
    {
        return Ok((
            http::StatusCode::BAD_REQUEST,
            Json(json!({ "detail": format!("用户名已存在: {username}") })),
        )
            .into_response());
    }

    let password_hash = match bcrypt::hash(&payload.password, bcrypt::DEFAULT_COST) {
        Ok(value) => value,
        Err(_) => {
            return Ok((
                http::StatusCode::BAD_REQUEST,
                Json(json!({ "detail": "密码长度不能超过72字节" })),
            )
                .into_response())
        }
    };
    let initial_gift_usd = if payload.unlimited {
        0.0
    } else if let Some(value) = payload.initial_gift_usd {
        value
    } else {
        admin_default_user_initial_gift(
            state
                .read_system_config_json_value("default_user_initial_gift_usd")
                .await?
                .as_ref(),
        )
    };

    let Some(user) = state
        .create_local_auth_user_with_settings(
            email,
            false,
            username,
            password_hash,
            role,
            allowed_providers,
            allowed_api_formats,
            allowed_models,
            payload.rate_limit,
        )
        .await?
    else {
        return Ok(build_admin_users_read_only_response(
            "当前为只读模式，无法创建用户",
        ));
    };

    if state
        .initialize_auth_user_wallet(&user.id, initial_gift_usd, payload.unlimited)
        .await?
        .is_none()
    {
        return Ok(build_admin_users_read_only_response(
            "当前为只读模式，无法初始化用户钱包",
        ));
    }

    Ok(Json(build_admin_user_payload(
        &user,
        payload.rate_limit,
        payload.unlimited,
    ))
    .into_response())
}

pub(super) async fn build_admin_update_user_response(
    state: &AppState,
    request_context: &GatewayPublicRequestContext,
    request_body: Option<&axum::body::Bytes>,
) -> Result<Response<Body>, GatewayError> {
    let Some(user_id) = admin_user_id_from_detail_path(&request_context.request_path) else {
        return Ok(build_admin_users_bad_request_response("缺少 user_id"));
    };
    let Some(_existing_user) = state.find_user_auth_by_id(&user_id).await? else {
        return Ok((
            http::StatusCode::NOT_FOUND,
            Json(json!({ "detail": "用户不存在" })),
        )
            .into_response());
    };
    let Some(request_body) = request_body else {
        return Ok((
            http::StatusCode::BAD_REQUEST,
            Json(json!({ "detail": "请求数据验证失败" })),
        )
            .into_response());
    };
    let raw_payload = match serde_json::from_slice::<serde_json::Value>(request_body) {
        Ok(serde_json::Value::Object(map)) => map,
        _ => {
            return Ok((
                http::StatusCode::BAD_REQUEST,
                Json(json!({ "detail": "请求数据验证失败" })),
            )
                .into_response())
        }
    };
    let field_presence = AdminUpdateUserFieldPresence {
        allowed_providers: raw_payload.contains_key("allowed_providers"),
        allowed_api_formats: raw_payload.contains_key("allowed_api_formats"),
        allowed_models: raw_payload.contains_key("allowed_models"),
    };
    let payload = match serde_json::from_value::<AdminUpdateUserRequest>(serde_json::Value::Object(
        raw_payload.clone(),
    )) {
        Ok(value) => value,
        Err(_) => {
            return Ok((
                http::StatusCode::BAD_REQUEST,
                Json(json!({ "detail": "请求数据验证失败" })),
            )
                .into_response())
        }
    };

    let email = match payload.email.as_deref() {
        Some(value) => match normalize_admin_optional_user_email(Some(value)) {
            Ok(value) => value,
            Err(detail) => {
                return Ok((
                    http::StatusCode::BAD_REQUEST,
                    Json(json!({ "detail": detail })),
                )
                    .into_response())
            }
        },
        None => None,
    };
    if let Some(email) = email.as_deref() {
        if state
            .is_other_user_auth_email_taken(email, &user_id)
            .await?
        {
            return Ok((
                http::StatusCode::BAD_REQUEST,
                Json(json!({ "detail": format!("邮箱已存在: {email}") })),
            )
                .into_response());
        }
    }

    let username = match payload.username.as_deref() {
        Some(value) => match normalize_admin_username(value) {
            Ok(value) => Some(value),
            Err(detail) => {
                return Ok((
                    http::StatusCode::BAD_REQUEST,
                    Json(json!({ "detail": detail })),
                )
                    .into_response())
            }
        },
        None => None,
    };
    if let Some(username) = username.as_deref() {
        if state
            .is_other_user_auth_username_taken(username, &user_id)
            .await?
        {
            return Ok((
                http::StatusCode::BAD_REQUEST,
                Json(json!({ "detail": format!("用户名已存在: {username}") })),
            )
                .into_response());
        }
    }

    let role = match payload.role.as_deref() {
        Some(value) => match normalize_admin_user_role(Some(value)) {
            Ok(value) => Some(value),
            Err(detail) => {
                return Ok((
                    http::StatusCode::BAD_REQUEST,
                    Json(json!({ "detail": detail })),
                )
                    .into_response())
            }
        },
        None => None,
    };
    if payload.rate_limit.is_some_and(|value| value < 0) {
        return Ok((
            http::StatusCode::BAD_REQUEST,
            Json(json!({ "detail": "rate_limit 必须大于等于 0" })),
        )
            .into_response());
    }
    let allowed_providers = if field_presence.allowed_providers {
        match normalize_admin_user_string_list(payload.allowed_providers, "allowed_providers") {
            Ok(value) => value,
            Err(detail) => {
                return Ok((
                    http::StatusCode::BAD_REQUEST,
                    Json(json!({ "detail": detail })),
                )
                    .into_response())
            }
        }
    } else {
        None
    };
    let allowed_api_formats = if field_presence.allowed_api_formats {
        match normalize_admin_user_api_formats(payload.allowed_api_formats) {
            Ok(value) => value,
            Err(detail) => {
                return Ok((
                    http::StatusCode::BAD_REQUEST,
                    Json(json!({ "detail": detail })),
                )
                    .into_response())
            }
        }
    } else {
        None
    };
    let allowed_models = if field_presence.allowed_models {
        match normalize_admin_user_string_list(payload.allowed_models, "allowed_models") {
            Ok(value) => value,
            Err(detail) => {
                return Ok((
                    http::StatusCode::BAD_REQUEST,
                    Json(json!({ "detail": detail })),
                )
                    .into_response())
            }
        }
    } else {
        None
    };
    let needs_auth_user_write = email.is_some()
        || username.is_some()
        || payload.password.is_some()
        || role.is_some()
        || field_presence.allowed_providers
        || field_presence.allowed_api_formats
        || field_presence.allowed_models
        || payload.rate_limit.is_some()
        || payload.is_active.is_some();
    if needs_auth_user_write && !state.has_auth_user_write_capability() {
        return Ok(build_admin_users_read_only_response(
            "当前为只读模式，无法更新用户",
        ));
    }
    if payload.unlimited.is_some() && !state.has_auth_wallet_write_capability() {
        return Ok(build_admin_users_read_only_response(
            "当前为只读模式，无法更新用户钱包",
        ));
    }

    if email.is_some() || username.is_some() {
        if state
            .update_local_auth_user_profile(&user_id, email.clone(), username.clone())
            .await?
            .is_none()
        {
            return Ok((
                http::StatusCode::NOT_FOUND,
                Json(json!({ "detail": "用户不存在" })),
            )
                .into_response());
        }
    }

    if let Some(password) = payload.password.as_deref() {
        let password_policy = admin_user_password_policy(state).await?;
        if let Err(detail) = validate_admin_user_password(password, &password_policy) {
            return Ok((
                http::StatusCode::BAD_REQUEST,
                Json(json!({ "detail": detail })),
            )
                .into_response());
        }
        let password_hash = match bcrypt::hash(password, bcrypt::DEFAULT_COST) {
            Ok(value) => value,
            Err(_) => {
                return Ok((
                    http::StatusCode::BAD_REQUEST,
                    Json(json!({ "detail": "密码长度不能超过72字节" })),
                )
                    .into_response())
            }
        };
        if state
            .update_local_auth_user_password_hash(&user_id, password_hash, chrono::Utc::now())
            .await?
            .is_none()
        {
            return Ok((
                http::StatusCode::NOT_FOUND,
                Json(json!({ "detail": "用户不存在" })),
            )
                .into_response());
        }
    }

    if role.is_some()
        || field_presence.allowed_providers
        || field_presence.allowed_api_formats
        || field_presence.allowed_models
        || payload.rate_limit.is_some()
        || payload.is_active.is_some()
    {
        if state
            .update_local_auth_user_admin_fields(
                &user_id,
                role,
                field_presence.allowed_providers,
                allowed_providers,
                field_presence.allowed_api_formats,
                allowed_api_formats,
                field_presence.allowed_models,
                allowed_models,
                payload.rate_limit,
                payload.is_active,
            )
            .await?
            .is_none()
        {
            return Ok((
                http::StatusCode::NOT_FOUND,
                Json(json!({ "detail": "用户不存在" })),
            )
                .into_response());
        }
    }

    if let Some(unlimited) = payload.unlimited {
        match state
            .find_wallet(aether_data::repository::wallet::WalletLookupKey::UserId(
                &user_id,
            ))
            .await?
        {
            Some(wallet) => {
                let desired_limit_mode = if unlimited { "unlimited" } else { "finite" };
                if !wallet.limit_mode.eq_ignore_ascii_case(desired_limit_mode) {
                    if state
                        .update_auth_user_wallet_limit_mode(&user_id, desired_limit_mode)
                        .await?
                        .is_none()
                    {
                        return Ok(build_admin_users_data_unavailable_response());
                    }
                }
            }
            None => {
                if state
                    .initialize_auth_user_wallet(&user_id, 0.0, unlimited)
                    .await?
                    .is_none()
                {
                    return Ok(build_admin_users_data_unavailable_response());
                }
            }
        }
    }

    let Some(user) = state.find_user_auth_by_id(&user_id).await? else {
        return Ok((
            http::StatusCode::NOT_FOUND,
            Json(json!({ "detail": "用户不存在" })),
        )
            .into_response());
    };
    let wallet = state
        .find_wallet(aether_data::repository::wallet::WalletLookupKey::UserId(
            &user_id,
        ))
        .await?;
    let unlimited = wallet
        .as_ref()
        .is_some_and(|wallet| wallet.limit_mode.eq_ignore_ascii_case("unlimited"));
    let export_row = find_admin_export_user(state, &user_id).await?;
    let rate_limit = export_row
        .as_ref()
        .and_then(|row| row.rate_limit)
        .or(payload.rate_limit);

    Ok(Json(build_admin_user_payload(&user, rate_limit, unlimited)).into_response())
}

pub(super) async fn build_admin_delete_user_response(
    state: &AppState,
    request_context: &GatewayPublicRequestContext,
) -> Result<Response<Body>, GatewayError> {
    let Some(user_id) = admin_user_id_from_detail_path(&request_context.request_path) else {
        return Ok(build_admin_users_bad_request_response("缺少 user_id"));
    };
    let Some(user) = state.find_user_auth_by_id(&user_id).await? else {
        return Ok((
            http::StatusCode::NOT_FOUND,
            Json(json!({ "detail": "用户不存在" })),
        )
            .into_response());
    };

    if user.role.eq_ignore_ascii_case("admin") && state.count_active_admin_users().await? <= 1 {
        return Ok((
            http::StatusCode::BAD_REQUEST,
            Json(json!({ "detail": "不能删除最后一个管理员账户" })),
        )
            .into_response());
    }
    if state.count_user_pending_refunds(&user_id).await? > 0 {
        return Ok((
            http::StatusCode::BAD_REQUEST,
            Json(json!({ "detail": "用户存在未完结退款，禁止删除" })),
        )
            .into_response());
    }
    if state.count_user_pending_payment_orders(&user_id).await? > 0 {
        return Ok((
            http::StatusCode::BAD_REQUEST,
            Json(json!({ "detail": "用户存在未完结充值订单，禁止删除" })),
        )
            .into_response());
    }

    if !state.delete_local_auth_user(&user_id).await? {
        return Ok((
            http::StatusCode::NOT_FOUND,
            Json(json!({ "detail": "用户不存在" })),
        )
            .into_response());
    }

    Ok(Json(json!({ "message": "用户删除成功" })).into_response())
}
