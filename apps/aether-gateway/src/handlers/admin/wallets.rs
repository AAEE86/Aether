use crate::gateway::{AppState, GatewayError, GatewayPublicRequestContext};
use axum::{body::Body, response::Response};

#[path = "wallets/routes.rs"]
mod admin_wallets_routes;
#[path = "wallets/shared.rs"]
mod admin_wallets_shared;
#[path = "wallets/mutations.rs"]
mod mutations;
#[path = "wallets/reads.rs"]
mod reads;

pub(crate) async fn maybe_build_local_admin_wallets_response(
    state: &AppState,
    request_context: &GatewayPublicRequestContext,
    request_body: Option<&axum::body::Bytes>,
) -> Result<Option<Response<Body>>, GatewayError> {
    admin_wallets_routes::maybe_build_local_admin_wallets_routes_response(
        state,
        request_context,
        request_body,
    )
    .await
}
