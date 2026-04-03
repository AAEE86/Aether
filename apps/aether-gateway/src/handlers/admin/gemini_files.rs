use crate::gateway::{AppState, GatewayError, GatewayPublicRequestContext};
use aether_data::repository::provider_catalog::StoredProviderCatalogKey;
use axum::body::{Body, Bytes};
use axum::http::{self, Response};
use axum::response::IntoResponse;
use axum::Json;
use serde_json::json;
use std::time::{SystemTime, UNIX_EPOCH};

const ADMIN_GEMINI_FILES_DATA_UNAVAILABLE_DETAIL: &str = "Admin Gemini files data unavailable";
const ADMIN_GEMINI_FILE_UPLOAD_DETAIL: &str = "Admin Gemini file upload requires Rust uploader";
const ADMIN_GEMINI_FILES_DEFAULT_PAGE: usize = 1;
const ADMIN_GEMINI_FILES_DEFAULT_PAGE_SIZE: usize = 20;
const ADMIN_GEMINI_FILES_MAX_PAGE_SIZE: usize = 100;

#[path = "gemini_files/read_routes.rs"]
mod admin_gemini_files_read_routes;
#[path = "gemini_files/upload.rs"]
mod admin_gemini_files_upload;

pub(crate) async fn maybe_build_local_admin_gemini_files_response(
    state: &AppState,
    request_context: &GatewayPublicRequestContext,
    request_body: Option<&Bytes>,
) -> Result<Option<Response<Body>>, GatewayError> {
    let Some(decision) = request_context.control_decision.as_ref() else {
        return Ok(None);
    };
    if decision.route_family.as_deref() != Some("gemini_files_manage") {
        return Ok(None);
    }

    if let Some(response) =
        admin_gemini_files_read_routes::maybe_build_local_admin_gemini_files_read_response(
            state,
            request_context,
        )
        .await?
    {
        return Ok(Some(response));
    }

    if let Some(response) =
        admin_gemini_files_upload::maybe_build_local_admin_gemini_files_upload_response(
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

fn admin_gemini_files_key_capable(key: &StoredProviderCatalogKey) -> bool {
    key.is_active
        && key
            .capabilities
            .as_ref()
            .and_then(|value| value.get("gemini_files"))
            .and_then(serde_json::Value::as_bool)
            .unwrap_or(false)
}

fn admin_gemini_files_error_response(
    status: http::StatusCode,
    detail: impl Into<String>,
) -> Response<Body> {
    (status, Json(json!({ "detail": detail.into() }))).into_response()
}

fn admin_gemini_files_now_unix_secs() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_secs())
        .unwrap_or_default()
}
