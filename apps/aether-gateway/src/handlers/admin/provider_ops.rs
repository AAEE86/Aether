use crate::gateway::{AppState, GatewayError, GatewayPublicRequestContext};
use axum::body::{Body, Bytes};
use axum::http::Response;

#[path = "provider_ops/architectures.rs"]
mod provider_ops_architectures;

#[path = "provider_ops/providers.rs"]
mod provider_ops_providers;

pub(crate) use self::provider_ops_providers::admin_provider_ops_local_action_response;

pub(crate) async fn maybe_build_local_admin_provider_ops_response(
    state: &AppState,
    request_context: &GatewayPublicRequestContext,
    request_body: Option<&Bytes>,
) -> Result<Option<Response<Body>>, GatewayError> {
    if let Some(response) =
        provider_ops_architectures::maybe_build_local_admin_provider_ops_architectures_response(
            request_context,
        )
        .await?
    {
        return Ok(Some(response));
    }

    if let Some(response) =
        provider_ops_providers::maybe_build_local_admin_provider_ops_providers_response(
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
