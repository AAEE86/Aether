use crate::gateway::{AppState, GatewayError, GatewayPublicRequestContext};
use axum::{
    body::{Body, Bytes},
    response::Response,
};

#[path = "ldap/builders.rs"]
mod ldap_builders;
#[path = "ldap/routes.rs"]
mod ldap_routes;
#[path = "ldap/shared.rs"]
mod ldap_shared;

pub(crate) async fn maybe_build_local_admin_ldap_response(
    state: &AppState,
    request_context: &GatewayPublicRequestContext,
    request_body: Option<&Bytes>,
) -> Result<Option<Response<Body>>, GatewayError> {
    ldap_routes::maybe_build_local_admin_ldap_response(state, request_context, request_body).await
}
