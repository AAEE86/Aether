use super::super::{
    build_admin_model_catalog_payload, clear_admin_external_models_cache,
    read_admin_external_models_cache,
};
use super::build_admin_model_catalog_data_unavailable_response;
use crate::gateway::{AppState, GatewayError, GatewayPublicRequestContext};
use axum::{
    body::Body,
    http,
    response::{IntoResponse, Response},
    Json,
};
use serde_json::json;

pub(super) async fn maybe_build_local_admin_core_model_response(
    state: &AppState,
    request_context: &GatewayPublicRequestContext,
) -> Result<Option<Response<Body>>, GatewayError> {
    let Some(decision) = request_context.control_decision.as_ref() else {
        return Ok(None);
    };

    if decision.route_family.as_deref() == Some("model_catalog_manage")
        && decision.route_kind.as_deref() == Some("catalog")
        && request_context.request_method == http::Method::GET
        && request_context.request_path == "/api/admin/models/catalog"
    {
        if !state.has_global_model_data_reader() || !state.has_provider_catalog_data_reader() {
            return Ok(Some(build_admin_model_catalog_data_unavailable_response()));
        }
        let Some(payload) = build_admin_model_catalog_payload(state).await else {
            return Ok(Some(build_admin_model_catalog_data_unavailable_response()));
        };
        return Ok(Some(Json(payload).into_response()));
    }

    if decision.route_family.as_deref() == Some("model_external_manage")
        && decision.route_kind.as_deref() == Some("external")
        && request_context.request_method == http::Method::GET
        && request_context.request_path == "/api/admin/models/external"
    {
        return Ok(Some(
            match read_admin_external_models_cache(state).await? {
                Some(payload) => Json(payload).into_response(),
                None => (
                    http::StatusCode::SERVICE_UNAVAILABLE,
                    Json(json!({
                        "detail": "External models catalog requires Rust admin backend"
                    })),
                )
                    .into_response(),
            },
        ));
    }

    if decision.route_family.as_deref() == Some("model_external_manage")
        && decision.route_kind.as_deref() == Some("clear_external_cache")
        && request_context.request_method == http::Method::DELETE
        && request_context.request_path == "/api/admin/models/external/cache"
    {
        return Ok(Some(
            Json(clear_admin_external_models_cache(state).await?).into_response(),
        ));
    }

    Ok(None)
}
