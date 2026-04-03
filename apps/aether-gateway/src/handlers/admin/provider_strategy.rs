use crate::gateway::{AppState, GatewayError, GatewayPublicRequestContext};
use axum::body::{Body, Bytes};
use axum::http::Response;

#[path = "provider_strategy/builders.rs"]
mod provider_strategy_builders;
#[path = "provider_strategy/routes.rs"]
mod provider_strategy_routes;
#[path = "provider_strategy/shared.rs"]
mod provider_strategy_shared;

pub(crate) async fn maybe_build_local_admin_provider_strategy_response(
    state: &AppState,
    request_context: &GatewayPublicRequestContext,
    request_body: Option<&Bytes>,
) -> Result<Option<Response<Body>>, GatewayError> {
    provider_strategy_routes::maybe_build_local_admin_provider_strategy_response(
        state,
        request_context,
        request_body,
    )
    .await
}
