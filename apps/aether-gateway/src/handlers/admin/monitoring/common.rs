use super::ADMIN_MONITORING_DATA_UNAVAILABLE_DETAIL;
use axum::{
    body::Body,
    http,
    response::{IntoResponse, Response},
    Json,
};
use serde_json::json;

pub(super) struct AdminMonitoringCacheSnapshot {
    pub(super) scheduler_name: String,
    pub(super) scheduling_mode: String,
    pub(super) provider_priority_mode: String,
    pub(super) storage_type: &'static str,
    pub(super) total_affinities: usize,
    pub(super) cache_hits: usize,
    pub(super) cache_misses: usize,
    pub(super) cache_hit_rate: f64,
    pub(super) provider_switches: usize,
    pub(super) key_switches: usize,
    pub(super) cache_invalidations: usize,
}

#[derive(Debug, Clone)]
pub(super) struct AdminMonitoringCacheAffinityRecord {
    pub(super) raw_key: String,
    pub(super) affinity_key: String,
    pub(super) api_format: String,
    pub(super) model_name: String,
    pub(super) provider_id: Option<String>,
    pub(super) endpoint_id: Option<String>,
    pub(super) key_id: Option<String>,
    pub(super) created_at: Option<serde_json::Value>,
    pub(super) expire_at: Option<serde_json::Value>,
    pub(super) request_count: u64,
}

pub(super) fn admin_monitoring_data_unavailable_response() -> Response<Body> {
    (
        http::StatusCode::SERVICE_UNAVAILABLE,
        Json(json!({ "detail": ADMIN_MONITORING_DATA_UNAVAILABLE_DETAIL })),
    )
        .into_response()
}

pub(super) fn admin_monitoring_bad_request_response(detail: impl Into<String>) -> Response<Body> {
    (
        http::StatusCode::BAD_REQUEST,
        Json(json!({ "detail": detail.into() })),
    )
        .into_response()
}

pub(super) fn admin_monitoring_not_found_response(detail: &'static str) -> Response<Body> {
    (
        http::StatusCode::NOT_FOUND,
        Json(json!({ "detail": detail })),
    )
        .into_response()
}

pub(super) fn admin_monitoring_usage_is_error(
    item: &aether_data::repository::usage::StoredRequestUsageAudit,
) -> bool {
    item.status_code.is_some_and(|value| value >= 400)
        || item.status.trim().eq_ignore_ascii_case("failed")
        || item.status.trim().eq_ignore_ascii_case("error")
        || item.error_message.is_some()
        || item.error_category.is_some()
}

pub(super) fn admin_monitoring_user_behavior_user_id_from_path(
    request_path: &str,
) -> Option<String> {
    let value = request_path
        .strip_prefix("/api/admin/monitoring/user-behavior/")?
        .trim()
        .trim_matches('/')
        .to_string();
    if value.is_empty() || value.contains('/') {
        None
    } else {
        Some(value)
    }
}
