use crate::gateway::{AppState, GatewayError, GatewayPublicRequestContext};
use axum::{
    body::Body,
    http,
    response::{IntoResponse, Response},
    Json,
};
use serde_json::json;

const ADMIN_USAGE_DATA_UNAVAILABLE_DETAIL: &str = "Admin usage data unavailable";

#[path = "usage/analytics.rs"]
mod analytics;
#[path = "usage/replay.rs"]
mod replay;
#[path = "usage/analytics_routes.rs"]
mod usage_analytics_routes;
#[path = "usage/detail_routes.rs"]
mod usage_detail_routes;
#[path = "usage/summary_routes.rs"]
mod usage_summary_routes;

use self::analytics::*;

pub(crate) async fn maybe_build_local_admin_usage_response(
    state: &AppState,
    request_context: &GatewayPublicRequestContext,
    request_body: Option<&axum::body::Bytes>,
) -> Result<Option<Response<Body>>, GatewayError> {
    let Some(decision) = request_context.control_decision.as_ref() else {
        return Ok(None);
    };

    if decision.route_family.as_deref() != Some("usage_manage") {
        return Ok(None);
    }

    if let Some(response) = usage_detail_routes::maybe_build_local_admin_usage_detail_response(
        state,
        request_context,
        request_body,
    )
    .await?
    {
        return Ok(Some(response));
    }
    if let Some(response) =
        usage_summary_routes::maybe_build_local_admin_usage_summary_response(state, request_context)
            .await?
    {
        return Ok(Some(response));
    }
    if let Some(response) =
        usage_analytics_routes::maybe_build_local_admin_usage_analytics_response(
            state,
            request_context,
        )
        .await?
    {
        return Ok(Some(response));
    }

    Ok(None)
}

fn admin_usage_data_unavailable_response(detail: &'static str) -> Response<Body> {
    (
        http::StatusCode::SERVICE_UNAVAILABLE,
        Json(json!({ "detail": detail })),
    )
        .into_response()
}

fn admin_usage_bad_request_response(detail: impl Into<String>) -> Response<Body> {
    (
        http::StatusCode::BAD_REQUEST,
        Json(json!({ "detail": detail.into() })),
    )
        .into_response()
}
