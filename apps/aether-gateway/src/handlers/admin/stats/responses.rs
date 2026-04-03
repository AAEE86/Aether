use super::{AdminStatsLeaderboardMetric, AdminStatsTimeRange};
use axum::{
    body::Body,
    http,
    response::{IntoResponse, Response},
    Json,
};
use serde_json::json;

pub(super) fn admin_stats_provider_quota_usage_empty_response() -> Response<Body> {
    Json(json!({
        "providers": [],
        "data_source_available": false,
    }))
    .into_response()
}

pub(super) fn admin_stats_cost_forecast_empty_response() -> Response<Body> {
    Json(json!({
        "history": [],
        "forecast": [],
        "slope": 0.0,
        "intercept": 0.0,
        "data_source_available": false,
    }))
    .into_response()
}

pub(super) fn admin_stats_comparison_empty_response(
    current_range: &AdminStatsTimeRange,
    comparison_range: &AdminStatsTimeRange,
) -> Response<Body> {
    Json(json!({
        "current": {
            "total_requests": 0,
            "total_tokens": 0,
            "total_cost": 0.0,
            "actual_total_cost": 0.0,
            "avg_response_time_ms": 0.0,
            "error_requests": 0,
        },
        "comparison": {
            "total_requests": 0,
            "total_tokens": 0,
            "total_cost": 0.0,
            "actual_total_cost": 0.0,
            "avg_response_time_ms": 0.0,
            "error_requests": 0,
        },
        "change_percent": {
            "total_requests": serde_json::Value::Null,
            "total_tokens": serde_json::Value::Null,
            "total_cost": serde_json::Value::Null,
            "actual_total_cost": serde_json::Value::Null,
            "avg_response_time_ms": serde_json::Value::Null,
            "error_requests": serde_json::Value::Null,
        },
        "current_start": current_range.start_date.to_string(),
        "current_end": current_range.end_date.to_string(),
        "comparison_start": comparison_range.start_date.to_string(),
        "comparison_end": comparison_range.end_date.to_string(),
    }))
    .into_response()
}

pub(super) fn admin_stats_error_distribution_empty_response() -> Response<Body> {
    Json(json!({
        "distribution": [],
        "trend": [],
    }))
    .into_response()
}

pub(super) fn admin_stats_performance_percentiles_empty_response() -> Response<Body> {
    Json(json!([])).into_response()
}

pub(super) fn admin_stats_cost_savings_empty_response() -> Response<Body> {
    Json(json!({
        "cache_read_tokens": 0,
        "cache_read_cost": 0.0,
        "cache_creation_cost": 0.0,
        "estimated_full_cost": 0.0,
        "cache_savings": 0.0,
    }))
    .into_response()
}

pub(super) fn admin_stats_leaderboard_empty_response(
    metric: AdminStatsLeaderboardMetric,
    time_range: Option<&AdminStatsTimeRange>,
) -> Response<Body> {
    Json(json!({
        "items": [],
        "total": 0,
        "metric": metric.as_str(),
        "start_date": time_range.map(|value| value.start_date.to_string()),
        "end_date": time_range.map(|value| value.end_date.to_string()),
    }))
    .into_response()
}

pub(super) fn admin_stats_time_series_empty_response() -> Response<Body> {
    Json(json!([])).into_response()
}

pub(crate) fn admin_stats_bad_request_response(detail: String) -> Response<Body> {
    (
        http::StatusCode::BAD_REQUEST,
        Json(json!({ "detail": detail })),
    )
        .into_response()
}
