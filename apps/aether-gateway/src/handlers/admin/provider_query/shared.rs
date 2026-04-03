use axum::{
    body::{Body, Bytes},
    http,
    response::{IntoResponse, Response},
    Json,
};
use serde_json::json;

pub(super) const ADMIN_PROVIDER_QUERY_INVALID_JSON_DETAIL: &str = "Invalid JSON request body";
pub(super) const ADMIN_PROVIDER_QUERY_PROVIDER_ID_REQUIRED_DETAIL: &str = "provider_id is required";
pub(super) const ADMIN_PROVIDER_QUERY_MODEL_REQUIRED_DETAIL: &str = "model is required";
pub(super) const ADMIN_PROVIDER_QUERY_FAILOVER_MODELS_REQUIRED_DETAIL: &str =
    "failover_models should not be empty";
pub(super) const ADMIN_PROVIDER_QUERY_PROVIDER_NOT_FOUND_DETAIL: &str = "Provider not found";
pub(super) const ADMIN_PROVIDER_QUERY_API_KEY_NOT_FOUND_DETAIL: &str = "API Key not found";
pub(super) const ADMIN_PROVIDER_QUERY_NO_ACTIVE_API_KEY_DETAIL: &str =
    "No active API Key found for this provider";
pub(super) const ADMIN_PROVIDER_QUERY_NO_LOCAL_MODELS_DETAIL: &str =
    "No models available from local provider catalog";

pub(super) fn build_admin_provider_query_bad_request_response(
    detail: &'static str,
) -> Response<Body> {
    (
        http::StatusCode::BAD_REQUEST,
        Json(json!({ "detail": detail })),
    )
        .into_response()
}

pub(super) fn build_admin_provider_query_not_found_response(
    detail: &'static str,
) -> Response<Body> {
    (
        http::StatusCode::NOT_FOUND,
        Json(json!({ "detail": detail })),
    )
        .into_response()
}

pub(super) fn parse_admin_provider_query_body(
    request_body: Option<&Bytes>,
) -> Result<serde_json::Value, Response<Body>> {
    let Some(raw_body) = request_body else {
        return Ok(json!({}));
    };
    if raw_body.is_empty() {
        return Ok(json!({}));
    }
    serde_json::from_slice::<serde_json::Value>(raw_body).map_err(|_| {
        build_admin_provider_query_bad_request_response(ADMIN_PROVIDER_QUERY_INVALID_JSON_DETAIL)
    })
}

pub(super) fn provider_query_extract_provider_id(payload: &serde_json::Value) -> Option<String> {
    payload
        .get("provider_id")
        .and_then(serde_json::Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(ToOwned::to_owned)
}

pub(super) fn provider_query_extract_api_key_id(payload: &serde_json::Value) -> Option<String> {
    payload
        .get("api_key_id")
        .and_then(serde_json::Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(ToOwned::to_owned)
}

pub(super) fn provider_query_extract_model(payload: &serde_json::Value) -> Option<String> {
    payload
        .get("model")
        .or_else(|| payload.get("model_name"))
        .and_then(serde_json::Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(ToOwned::to_owned)
}

pub(super) fn provider_query_extract_failover_models(payload: &serde_json::Value) -> Vec<String> {
    payload
        .get("failover_models")
        .or_else(|| payload.get("models"))
        .and_then(serde_json::Value::as_array)
        .map(|items| {
            items
                .iter()
                .filter_map(serde_json::Value::as_str)
                .map(str::trim)
                .filter(|value| !value.is_empty())
                .map(ToOwned::to_owned)
                .collect::<Vec<_>>()
        })
        .unwrap_or_default()
}
