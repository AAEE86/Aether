use axum::{
    body::Body,
    http,
    response::{IntoResponse, Response},
    Json,
};
use serde_json::json;

pub(super) const ADMIN_GLOBAL_MODELS_DATA_UNAVAILABLE_DETAIL: &str =
    "Admin global model data unavailable";

pub(super) fn build_admin_global_models_data_unavailable_response() -> Response<Body> {
    (
        http::StatusCode::SERVICE_UNAVAILABLE,
        Json(json!({ "detail": ADMIN_GLOBAL_MODELS_DATA_UNAVAILABLE_DETAIL })),
    )
        .into_response()
}
