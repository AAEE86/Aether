use super::provider_oauth_state::{
    build_admin_provider_oauth_backend_unavailable_response,
    build_admin_provider_oauth_supported_types_payload,
};
use crate::gateway::{AppState, GatewayError, GatewayPublicRequestContext};
use axum::{
    body::{Body, Bytes},
    http,
    response::{IntoResponse, Response},
    Json,
};

#[path = "dispatch/batch.rs"]
mod dispatch_batch;
#[path = "dispatch/complete.rs"]
mod dispatch_complete;
#[path = "dispatch/device.rs"]
mod dispatch_device;
#[path = "dispatch/import.rs"]
mod dispatch_import;
#[path = "dispatch/refresh.rs"]
mod dispatch_refresh;
#[path = "dispatch/start.rs"]
mod dispatch_start;
#[path = "dispatch/tasks.rs"]
mod dispatch_tasks;

pub(crate) async fn maybe_build_local_admin_provider_oauth_response(
    state: &AppState,
    request_context: &GatewayPublicRequestContext,
    request_body: Option<&Bytes>,
) -> Result<Option<Response<Body>>, GatewayError> {
    let Some(decision) = request_context.control_decision.as_ref() else {
        return Ok(None);
    };
    if decision.route_family.as_deref() != Some("provider_oauth_manage") {
        return Ok(None);
    }

    let route_kind = decision.route_kind.as_deref();
    let method = &request_context.request_method;

    if route_kind == Some("supported_types")
        && *method == http::Method::GET
        && request_context.request_path == "/api/admin/provider-oauth/supported-types"
    {
        return Ok(Some(
            Json(build_admin_provider_oauth_supported_types_payload()).into_response(),
        ));
    }

    if route_kind == Some("start_key_oauth") && *method == http::Method::POST {
        return Ok(Some(
            dispatch_start::handle_admin_provider_oauth_start_key(state, request_context).await?,
        ));
    }

    if route_kind == Some("start_provider_oauth") && *method == http::Method::POST {
        return Ok(Some(
            dispatch_start::handle_admin_provider_oauth_start_provider(state, request_context)
                .await?,
        ));
    }

    if route_kind == Some("get_batch_import_task_status") && *method == http::Method::GET {
        return Ok(Some(
            dispatch_tasks::handle_admin_provider_oauth_batch_import_task_status(
                state,
                request_context,
            )
            .await?,
        ));
    }

    if route_kind == Some("complete_key_oauth") && *method == http::Method::POST {
        return Ok(Some(
            dispatch_complete::handle_admin_provider_oauth_complete_key(
                state,
                request_context,
                request_body,
            )
            .await?,
        ));
    }

    if route_kind == Some("refresh_key_oauth") && *method == http::Method::POST {
        return Ok(Some(
            dispatch_refresh::handle_admin_provider_oauth_refresh_key(state, request_context)
                .await?,
        ));
    }

    if route_kind == Some("complete_provider_oauth") && *method == http::Method::POST {
        return Ok(Some(
            dispatch_complete::handle_admin_provider_oauth_complete_provider(
                state,
                request_context,
                request_body,
            )
            .await?,
        ));
    }

    if route_kind == Some("import_refresh_token") && *method == http::Method::POST {
        return Ok(Some(
            dispatch_import::handle_admin_provider_oauth_import_refresh_token(
                state,
                request_context,
                request_body,
            )
            .await?,
        ));
    }

    if route_kind == Some("batch_import_oauth") && *method == http::Method::POST {
        return Ok(Some(
            dispatch_batch::handle_admin_provider_oauth_batch_import(
                state,
                request_context,
                request_body,
            )
            .await?,
        ));
    }

    if route_kind == Some("start_batch_import_oauth_task") && *method == http::Method::POST {
        return Ok(Some(
            dispatch_batch::handle_admin_provider_oauth_start_batch_import_task(
                state,
                request_context,
                request_body,
            )
            .await?,
        ));
    }

    if route_kind == Some("device_authorize") && *method == http::Method::POST {
        return Ok(Some(
            dispatch_device::handle_admin_provider_oauth_device_authorize(
                state,
                request_context,
                request_body,
            )
            .await?,
        ));
    }

    if route_kind == Some("device_poll") && *method == http::Method::POST {
        return Ok(Some(
            dispatch_device::handle_admin_provider_oauth_device_poll(
                state,
                request_context,
                request_body,
            )
            .await?,
        ));
    }

    if matches!(
        route_kind,
        Some("refresh_key_oauth" | "import_refresh_token")
    ) {
        return Ok(Some(
            build_admin_provider_oauth_backend_unavailable_response(),
        ));
    }

    Ok(None)
}
