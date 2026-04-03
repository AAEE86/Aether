use crate::gateway::{AppState, GatewayError, GatewayPublicRequestContext};
use axum::{
    body::{Body, Bytes},
    http,
    response::{IntoResponse, Response},
    Json,
};
use serde_json::json;

const ADMIN_AWS_REGIONS: &[&str] = &[
    "af-south-1",
    "ap-east-1",
    "ap-northeast-1",
    "ap-northeast-2",
    "ap-northeast-3",
    "ap-south-1",
    "ap-south-2",
    "ap-southeast-1",
    "ap-southeast-2",
    "ap-southeast-3",
    "ap-southeast-4",
    "ca-central-1",
    "ca-west-1",
    "eu-central-1",
    "eu-central-2",
    "eu-north-1",
    "eu-south-1",
    "eu-south-2",
    "eu-west-1",
    "eu-west-2",
    "eu-west-3",
    "il-central-1",
    "me-central-1",
    "me-south-1",
    "sa-east-1",
    "us-east-1",
    "us-east-2",
    "us-west-1",
    "us-west-2",
];
const ADMIN_MODEL_CATALOG_DATA_UNAVAILABLE_DETAIL: &str = "Admin model catalog data unavailable";

#[path = "core/management_tokens_routes.rs"]
mod admin_core_management_tokens_routes;
#[path = "core/model_routes.rs"]
mod admin_core_model_routes;
#[path = "core/modules_routes.rs"]
mod admin_core_modules_routes;
#[path = "core/oauth_routes.rs"]
mod admin_core_oauth_routes;
#[path = "core/system_routes.rs"]
mod admin_core_system_routes;

fn build_admin_model_catalog_data_unavailable_response() -> Response<Body> {
    (
        http::StatusCode::SERVICE_UNAVAILABLE,
        Json(json!({ "detail": ADMIN_MODEL_CATALOG_DATA_UNAVAILABLE_DETAIL })),
    )
        .into_response()
}

pub(crate) async fn maybe_build_local_admin_core_response(
    state: &AppState,
    request_context: &GatewayPublicRequestContext,
    request_body: Option<&Bytes>,
) -> Result<Option<Response<Body>>, GatewayError> {
    if let Some(response) = admin_core_management_tokens_routes::maybe_build_local_admin_core_management_tokens_response(
        state,
        request_context,
    )
    .await?
    {
        return Ok(Some(response));
    }
    if let Some(response) = admin_core_oauth_routes::maybe_build_local_admin_core_oauth_response(
        state,
        request_context,
        request_body,
    )
    .await?
    {
        return Ok(Some(response));
    }
    if let Some(response) =
        admin_core_modules_routes::maybe_build_local_admin_core_modules_response(
            state,
            request_context,
            request_body,
        )
        .await?
    {
        return Ok(Some(response));
    }
    if let Some(response) = admin_core_system_routes::maybe_build_local_admin_core_system_response(
        state,
        request_context,
        request_body,
    )
    .await?
    {
        return Ok(Some(response));
    }
    if let Some(response) =
        admin_core_model_routes::maybe_build_local_admin_core_model_response(state, request_context)
            .await?
    {
        return Ok(Some(response));
    }

    Ok(None)
}
