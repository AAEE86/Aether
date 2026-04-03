use crate::gateway::{AppState, GatewayError, GatewayPublicRequestContext};
use axum::body::{Body, Bytes};
use axum::http::Response;

#[path = "endpoints/health.rs"]
mod endpoints_health;
#[path = "endpoints/keys.rs"]
mod endpoints_keys;
#[path = "endpoints/routes.rs"]
mod endpoints_routes;
#[path = "endpoints/rpm.rs"]
mod endpoints_rpm;

pub(crate) async fn maybe_build_local_admin_endpoints_response(
    state: &AppState,
    request_context: &GatewayPublicRequestContext,
    request_body: Option<&Bytes>,
) -> Result<Option<Response<Body>>, GatewayError> {
    if let Some(response) =
        endpoints_health::maybe_build_local_admin_endpoints_health_response(state, request_context)
            .await?
    {
        return Ok(Some(response));
    }

    if let Some(response) =
        endpoints_rpm::maybe_build_local_admin_endpoints_rpm_response(state, request_context)
            .await?
    {
        return Ok(Some(response));
    }

    if let Some(response) = endpoints_keys::maybe_build_local_admin_endpoints_keys_response(
        state,
        request_context,
        request_body,
    )
    .await?
    {
        return Ok(Some(response));
    }

    if let Some(response) = endpoints_routes::maybe_build_local_admin_endpoints_routes_response(
        state,
        request_context,
        request_body,
    )
    .await?
    {
        return Ok(Some(response));
    }

    Ok(None)
}
