use super::super::build_proxy_error_response;
use axum::{
    body::Body,
    http,
    response::{IntoResponse, Response},
    Json,
};
use serde_json::json;

pub(super) const ADMIN_PROVIDER_STRATEGY_DATA_UNAVAILABLE_DETAIL: &str =
    "Admin provider strategy data unavailable";
pub(super) const ADMIN_PROVIDER_STRATEGY_STATS_DATA_UNAVAILABLE_DETAIL: &str =
    "Admin provider strategy stats data unavailable";

pub(super) fn admin_provider_strategy_data_unavailable_response(detail: &str) -> Response<Body> {
    build_proxy_error_response(
        http::StatusCode::SERVICE_UNAVAILABLE,
        "data_unavailable",
        detail,
        Some(json!({ "error": detail })),
    )
}

pub(super) fn admin_provider_strategy_provider_not_found_response() -> Response<Body> {
    (
        http::StatusCode::NOT_FOUND,
        Json(json!({ "detail": "Provider not found" })),
    )
        .into_response()
}

pub(super) fn admin_provider_strategy_dispatcher_not_found_response() -> Response<Body> {
    (
        http::StatusCode::NOT_FOUND,
        Json(json!({ "detail": "Provider strategy route not found" })),
    )
        .into_response()
}
