use super::{
    ADMIN_MONITORING_CACHE_AFFINITY_REDIS_REQUIRED_DETAIL, ADMIN_MONITORING_REDIS_REQUIRED_DETAIL,
};
use crate::gateway::handlers::query_param_value;
use axum::{
    body::Body,
    http,
    response::{IntoResponse, Response},
    Json,
};
use serde_json::json;

pub(super) fn parse_admin_monitoring_keyword_filter(query: Option<&str>) -> Option<String> {
    query_param_value(query, "keyword")
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
}

fn admin_monitoring_cache_path_identifier_from_path(
    request_path: &str,
    prefix: &str,
) -> Option<String> {
    let value = request_path
        .strip_prefix(prefix)?
        .trim()
        .trim_matches('/')
        .to_string();
    if value.is_empty() || value.contains('/') {
        None
    } else {
        Some(value)
    }
}

pub(super) fn admin_monitoring_cache_affinity_user_identifier_from_path(
    request_path: &str,
) -> Option<String> {
    admin_monitoring_cache_path_identifier_from_path(
        request_path,
        "/api/admin/monitoring/cache/affinity/",
    )
}

pub(super) fn admin_monitoring_cache_users_user_identifier_from_path(
    request_path: &str,
) -> Option<String> {
    admin_monitoring_cache_path_identifier_from_path(
        request_path,
        "/api/admin/monitoring/cache/users/",
    )
}

pub(super) fn admin_monitoring_cache_provider_id_from_path(request_path: &str) -> Option<String> {
    admin_monitoring_cache_path_identifier_from_path(
        request_path,
        "/api/admin/monitoring/cache/providers/",
    )
}

pub(super) fn admin_monitoring_cache_model_name_from_path(request_path: &str) -> Option<String> {
    admin_monitoring_cache_path_identifier_from_path(
        request_path,
        "/api/admin/monitoring/cache/model-mapping/",
    )
}

pub(super) fn admin_monitoring_cache_redis_category_from_path(
    request_path: &str,
) -> Option<String> {
    admin_monitoring_cache_path_identifier_from_path(
        request_path,
        "/api/admin/monitoring/cache/redis-keys/",
    )
}

pub(super) fn admin_monitoring_cache_model_mapping_provider_params_from_path(
    request_path: &str,
) -> Option<(String, String)> {
    let suffix = request_path
        .strip_prefix("/api/admin/monitoring/cache/model-mapping/provider/")?
        .trim()
        .trim_matches('/');
    let segments = suffix.split('/').collect::<Vec<_>>();
    if segments.len() != 2 || segments.iter().any(|segment| segment.trim().is_empty()) {
        return None;
    }
    Some((
        segments[0].trim().to_string(),
        segments[1].trim().to_string(),
    ))
}

pub(super) fn admin_monitoring_cache_affinity_delete_params_from_path(
    request_path: &str,
) -> Option<(String, String, String, String)> {
    let suffix = request_path
        .strip_prefix("/api/admin/monitoring/cache/affinity/")?
        .trim()
        .trim_matches('/');
    let segments = suffix.split('/').collect::<Vec<_>>();
    if segments.len() != 4 || segments.iter().any(|segment| segment.trim().is_empty()) {
        return None;
    }
    Some((
        segments[0].trim().to_string(),
        segments[1].trim().to_string(),
        segments[2].trim().to_string(),
        segments[3].trim().to_string(),
    ))
}

pub(super) fn admin_monitoring_cache_affinity_unavailable_response() -> Response<Body> {
    (
        http::StatusCode::SERVICE_UNAVAILABLE,
        Json(json!({ "detail": ADMIN_MONITORING_CACHE_AFFINITY_REDIS_REQUIRED_DETAIL })),
    )
        .into_response()
}

pub(super) fn admin_monitoring_redis_unavailable_response() -> Response<Body> {
    (
        http::StatusCode::SERVICE_UNAVAILABLE,
        Json(json!({ "detail": ADMIN_MONITORING_REDIS_REQUIRED_DETAIL })),
    )
        .into_response()
}

pub(super) fn admin_monitoring_cache_affinity_not_found_response(
    user_identifier: &str,
) -> Response<Body> {
    (
        http::StatusCode::NOT_FOUND,
        Json(json!({
            "detail": format!(
                "无法识别的用户标识符: {user_identifier}。支持用户名、邮箱、User ID或API Key ID"
            )
        })),
    )
        .into_response()
}

pub(super) fn admin_monitoring_cache_users_not_found_response(
    user_identifier: &str,
) -> Response<Body> {
    (
        http::StatusCode::NOT_FOUND,
        Json(json!({
            "detail": format!(
                "无法识别的标识符: {user_identifier}。支持用户名、邮箱、User ID或API Key ID"
            )
        })),
    )
        .into_response()
}
