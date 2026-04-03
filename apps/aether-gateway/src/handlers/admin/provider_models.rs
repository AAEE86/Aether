use crate::gateway::{AppState, GatewayControlDecision, GatewayError, GatewayPublicRequestContext};
use axum::body::{Body, Bytes};
use axum::http::Response;

#[path = "provider_models/assign_global.rs"]
mod provider_models_assign_global;
#[path = "provider_models/available_source.rs"]
mod provider_models_available_source;
#[path = "provider_models/batch.rs"]
mod provider_models_batch;
#[path = "provider_models/create.rs"]
mod provider_models_create;
#[path = "provider_models/delete.rs"]
mod provider_models_delete;
#[path = "provider_models/detail.rs"]
mod provider_models_detail;
#[path = "provider_models/import.rs"]
mod provider_models_import;
#[path = "provider_models/list.rs"]
mod provider_models_list;
#[path = "provider_models/update.rs"]
mod provider_models_update;

pub(crate) async fn maybe_build_local_admin_provider_models_response(
    state: &AppState,
    request_context: &GatewayPublicRequestContext,
    request_body: Option<&Bytes>,
) -> Result<Option<Response<Body>>, GatewayError> {
    let Some(decision) = request_context.control_decision.as_ref() else {
        return Ok(None);
    };

    if let Some(response) =
        provider_models_list::maybe_handle(state, request_context, request_body, decision).await?
    {
        return Ok(Some(response));
    }

    if let Some(response) =
        provider_models_detail::maybe_handle(state, request_context, request_body, decision).await?
    {
        return Ok(Some(response));
    }

    if let Some(response) =
        provider_models_create::maybe_handle(state, request_context, request_body, decision).await?
    {
        return Ok(Some(response));
    }

    if let Some(response) =
        provider_models_update::maybe_handle(state, request_context, request_body, decision).await?
    {
        return Ok(Some(response));
    }

    if let Some(response) =
        provider_models_delete::maybe_handle(state, request_context, request_body, decision).await?
    {
        return Ok(Some(response));
    }

    if let Some(response) =
        provider_models_batch::maybe_handle(state, request_context, request_body, decision).await?
    {
        return Ok(Some(response));
    }

    if let Some(response) = provider_models_available_source::maybe_handle(
        state,
        request_context,
        request_body,
        decision,
    )
    .await?
    {
        return Ok(Some(response));
    }

    if let Some(response) =
        provider_models_assign_global::maybe_handle(state, request_context, request_body, decision)
            .await?
    {
        return Ok(Some(response));
    }

    if let Some(response) =
        provider_models_import::maybe_handle(state, request_context, request_body, decision).await?
    {
        return Ok(Some(response));
    }

    Ok(None)
}
