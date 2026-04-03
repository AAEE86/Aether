use crate::gateway::{AppState, GatewayError, GatewayPublicRequestContext};
use axum::body::{Body, Bytes};
use axum::http::Response;

#[path = "provider_query/models.rs"]
mod provider_query_models;
#[path = "provider_query/routes.rs"]
mod provider_query_routes;
#[path = "provider_query/shared.rs"]
mod provider_query_shared;

pub(crate) async fn maybe_build_local_admin_provider_query_response(
    state: &AppState,
    request_context: &GatewayPublicRequestContext,
    request_body: Option<&Bytes>,
) -> Result<Option<Response<Body>>, GatewayError> {
    provider_query_routes::maybe_build_local_admin_provider_query_response(
        state,
        request_context,
        request_body,
    )
    .await
}
