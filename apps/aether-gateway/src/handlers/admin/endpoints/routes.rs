use super::super::{
    admin_default_body_rules_api_format, admin_endpoint_id, admin_provider_id_for_endpoints,
    build_admin_create_provider_endpoint_record, build_admin_endpoint_payload,
    build_admin_provider_endpoint_response, build_admin_provider_endpoints_payload,
    build_admin_update_provider_endpoint_record, endpoint_key_counts_by_format,
    key_api_formats_without_entry,
};
use crate::gateway::api::ai::admin_default_body_rules_for_signature;
use crate::gateway::handlers::{
    query_param_value, AdminProviderEndpointCreateRequest, AdminProviderEndpointUpdateRequest,
};
use crate::gateway::{AppState, GatewayError, GatewayPublicRequestContext};
use axum::{
    body::{Body, Bytes},
    http,
    response::{IntoResponse, Response},
    Json,
};
use serde_json::json;
use std::time::{SystemTime, UNIX_EPOCH};

const ADMIN_ENDPOINTS_DATA_UNAVAILABLE_DETAIL: &str = "Admin endpoint data unavailable";

fn build_admin_endpoints_data_unavailable_response() -> Response<Body> {
    (
        http::StatusCode::SERVICE_UNAVAILABLE,
        Json(json!({ "detail": ADMIN_ENDPOINTS_DATA_UNAVAILABLE_DETAIL })),
    )
        .into_response()
}

pub(super) async fn maybe_build_local_admin_endpoints_routes_response(
    state: &AppState,
    request_context: &GatewayPublicRequestContext,
    request_body: Option<&Bytes>,
) -> Result<Option<Response<Body>>, GatewayError> {
    let Some(decision) = request_context.control_decision.as_ref() else {
        return Ok(None);
    };

    if decision.route_family.as_deref() == Some("endpoints_manage")
        && decision.route_kind.as_deref() == Some("list_provider_endpoints")
        && request_context
            .request_path
            .starts_with("/api/admin/endpoints/providers/")
        && request_context.request_path.ends_with("/endpoints")
    {
        if !state.has_provider_catalog_data_reader() {
            return Ok(Some(build_admin_endpoints_data_unavailable_response()));
        }
        let Some(provider_id) = admin_provider_id_for_endpoints(&request_context.request_path)
        else {
            return Ok(Some(
                (
                    http::StatusCode::NOT_FOUND,
                    Json(json!({ "detail": "Provider 不存在" })),
                )
                    .into_response(),
            ));
        };
        let skip = query_param_value(request_context.request_query_string.as_deref(), "skip")
            .and_then(|value| value.parse::<usize>().ok())
            .unwrap_or(0);
        let limit = query_param_value(request_context.request_query_string.as_deref(), "limit")
            .and_then(|value| value.parse::<usize>().ok())
            .filter(|value| *value > 0)
            .unwrap_or(100);
        return Ok(Some(
            match build_admin_provider_endpoints_payload(state, &provider_id, skip, limit).await {
                Some(payload) => Json(payload).into_response(),
                None => (
                    http::StatusCode::NOT_FOUND,
                    Json(json!({ "detail": format!("Provider {provider_id} 不存在") })),
                )
                    .into_response(),
            },
        ));
    }

    if decision.route_family.as_deref() == Some("endpoints_manage")
        && decision.route_kind.as_deref() == Some("create_endpoint")
        && request_context.request_method == http::Method::POST
        && request_context
            .request_path
            .starts_with("/api/admin/endpoints/providers/")
        && request_context.request_path.ends_with("/endpoints")
    {
        if !state.has_provider_catalog_data_reader() || !state.has_provider_catalog_data_writer() {
            return Ok(Some(build_admin_endpoints_data_unavailable_response()));
        }
        let Some(provider_id) = admin_provider_id_for_endpoints(&request_context.request_path)
        else {
            return Ok(Some(
                (
                    http::StatusCode::NOT_FOUND,
                    Json(json!({ "detail": "Provider 不存在" })),
                )
                    .into_response(),
            ));
        };
        let Some(request_body) = request_body else {
            return Ok(Some(
                (
                    http::StatusCode::BAD_REQUEST,
                    Json(json!({ "detail": "请求体不能为空" })),
                )
                    .into_response(),
            ));
        };
        let payload =
            match serde_json::from_slice::<AdminProviderEndpointCreateRequest>(request_body) {
                Ok(payload) => payload,
                Err(_) => {
                    return Ok(Some(
                        (
                            http::StatusCode::BAD_REQUEST,
                            Json(json!({ "detail": "请求体必须是合法的 JSON 对象" })),
                        )
                            .into_response(),
                    ));
                }
            };
        let Some(provider) = state
            .read_provider_catalog_providers_by_ids(std::slice::from_ref(&provider_id))
            .await?
            .into_iter()
            .next()
        else {
            return Ok(Some(
                (
                    http::StatusCode::NOT_FOUND,
                    Json(json!({ "detail": format!("Provider {provider_id} 不存在") })),
                )
                    .into_response(),
            ));
        };
        let record =
            match build_admin_create_provider_endpoint_record(state, &provider, payload).await {
                Ok(record) => record,
                Err(detail) => {
                    return Ok(Some(
                        (
                            http::StatusCode::BAD_REQUEST,
                            Json(json!({ "detail": detail })),
                        )
                            .into_response(),
                    ));
                }
            };
        let Some(created) = state.create_provider_catalog_endpoint(&record).await? else {
            return Ok(Some(build_admin_endpoints_data_unavailable_response()));
        };
        let now_unix_secs = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .ok()
            .map(|duration| duration.as_secs())
            .unwrap_or(0);
        return Ok(Some(
            Json(build_admin_provider_endpoint_response(
                &created,
                &provider.name,
                0,
                0,
                now_unix_secs,
            ))
            .into_response(),
        ));
    }

    if decision.route_family.as_deref() == Some("endpoints_manage")
        && decision.route_kind.as_deref() == Some("update_endpoint")
        && request_context.request_method == http::Method::PUT
        && request_context
            .request_path
            .starts_with("/api/admin/endpoints/")
    {
        if !state.has_provider_catalog_data_reader() || !state.has_provider_catalog_data_writer() {
            return Ok(Some(build_admin_endpoints_data_unavailable_response()));
        }
        let Some(endpoint_id) = admin_endpoint_id(&request_context.request_path) else {
            return Ok(Some(
                (
                    http::StatusCode::NOT_FOUND,
                    Json(json!({ "detail": "Endpoint 不存在" })),
                )
                    .into_response(),
            ));
        };
        let Some(request_body) = request_body else {
            return Ok(Some(
                (
                    http::StatusCode::BAD_REQUEST,
                    Json(json!({ "detail": "请求体不能为空" })),
                )
                    .into_response(),
            ));
        };
        let raw_value = match serde_json::from_slice::<serde_json::Value>(request_body) {
            Ok(value) => value,
            Err(_) => {
                return Ok(Some(
                    (
                        http::StatusCode::BAD_REQUEST,
                        Json(json!({ "detail": "请求体必须是合法的 JSON 对象" })),
                    )
                        .into_response(),
                ));
            }
        };
        let Some(raw_payload) = raw_value.as_object().cloned() else {
            return Ok(Some(
                (
                    http::StatusCode::BAD_REQUEST,
                    Json(json!({ "detail": "请求体必须是合法的 JSON 对象" })),
                )
                    .into_response(),
            ));
        };
        let payload = match serde_json::from_value::<AdminProviderEndpointUpdateRequest>(raw_value)
        {
            Ok(payload) => payload,
            Err(_) => {
                return Ok(Some(
                    (
                        http::StatusCode::BAD_REQUEST,
                        Json(json!({ "detail": "请求体必须是合法的 JSON 对象" })),
                    )
                        .into_response(),
                ));
            }
        };
        let Some(existing_endpoint) = state
            .read_provider_catalog_endpoints_by_ids(std::slice::from_ref(&endpoint_id))
            .await?
            .into_iter()
            .next()
        else {
            return Ok(Some(
                (
                    http::StatusCode::NOT_FOUND,
                    Json(json!({ "detail": format!("Endpoint {endpoint_id} 不存在") })),
                )
                    .into_response(),
            ));
        };
        let Some(provider) = state
            .read_provider_catalog_providers_by_ids(std::slice::from_ref(
                &existing_endpoint.provider_id,
            ))
            .await?
            .into_iter()
            .next()
        else {
            return Ok(Some(
                (
                    http::StatusCode::NOT_FOUND,
                    Json(json!({ "detail": format!("Provider {} 不存在", existing_endpoint.provider_id) })),
                )
                    .into_response(),
            ));
        };
        let updated_record = match build_admin_update_provider_endpoint_record(
            state,
            &provider,
            &existing_endpoint,
            &raw_payload,
            payload,
        )
        .await
        {
            Ok(record) => record,
            Err(detail) => {
                return Ok(Some(
                    (
                        http::StatusCode::BAD_REQUEST,
                        Json(json!({ "detail": detail })),
                    )
                        .into_response(),
                ));
            }
        };
        let Some(updated) = state
            .update_provider_catalog_endpoint(&updated_record)
            .await?
        else {
            return Ok(Some(build_admin_endpoints_data_unavailable_response()));
        };
        let now_unix_secs = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .ok()
            .map(|duration| duration.as_secs())
            .unwrap_or(0);
        let keys = state
            .list_provider_catalog_keys_by_provider_ids(std::slice::from_ref(&provider.id))
            .await
            .unwrap_or_default();
        let (total_keys_by_format, active_keys_by_format) = endpoint_key_counts_by_format(&keys);
        return Ok(Some(
            Json(build_admin_provider_endpoint_response(
                &updated,
                &provider.name,
                total_keys_by_format
                    .get(updated.api_format.as_str())
                    .copied()
                    .unwrap_or(0),
                active_keys_by_format
                    .get(updated.api_format.as_str())
                    .copied()
                    .unwrap_or(0),
                now_unix_secs,
            ))
            .into_response(),
        ));
    }

    if decision.route_family.as_deref() == Some("endpoints_manage")
        && decision.route_kind.as_deref() == Some("delete_endpoint")
        && request_context.request_method == http::Method::DELETE
        && request_context
            .request_path
            .starts_with("/api/admin/endpoints/")
    {
        if !state.has_provider_catalog_data_reader() || !state.has_provider_catalog_data_writer() {
            return Ok(Some(build_admin_endpoints_data_unavailable_response()));
        }
        let Some(endpoint_id) = admin_endpoint_id(&request_context.request_path) else {
            return Ok(Some(
                (
                    http::StatusCode::NOT_FOUND,
                    Json(json!({ "detail": "Endpoint 不存在" })),
                )
                    .into_response(),
            ));
        };
        let Some(existing_endpoint) = state
            .read_provider_catalog_endpoints_by_ids(std::slice::from_ref(&endpoint_id))
            .await?
            .into_iter()
            .next()
        else {
            return Ok(Some(
                (
                    http::StatusCode::NOT_FOUND,
                    Json(json!({ "detail": format!("Endpoint {endpoint_id} 不存在") })),
                )
                    .into_response(),
            ));
        };
        let now_unix_secs = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .ok()
            .map(|duration| duration.as_secs())
            .unwrap_or(0);
        let keys = state
            .list_provider_catalog_keys_by_provider_ids(std::slice::from_ref(
                &existing_endpoint.provider_id,
            ))
            .await
            .unwrap_or_default();
        let mut affected_keys_count = 0usize;
        for key in keys {
            let Some(updated_formats) =
                key_api_formats_without_entry(&key, existing_endpoint.api_format.as_str())
            else {
                continue;
            };
            let mut updated_key = key.clone();
            updated_key.api_formats = Some(serde_json::Value::Array(
                updated_formats
                    .into_iter()
                    .map(serde_json::Value::String)
                    .collect(),
            ));
            updated_key.updated_at_unix_secs = Some(now_unix_secs);
            if state
                .update_provider_catalog_key(&updated_key)
                .await?
                .is_none()
            {
                return Ok(Some(build_admin_endpoints_data_unavailable_response()));
            }
            affected_keys_count += 1;
        }
        if !state.delete_provider_catalog_endpoint(&endpoint_id).await? {
            return Ok(Some(
                (
                    http::StatusCode::NOT_FOUND,
                    Json(json!({ "detail": format!("Endpoint {endpoint_id} 不存在") })),
                )
                    .into_response(),
            ));
        }
        return Ok(Some(
            Json(json!({
                "message": format!("Endpoint {endpoint_id} 已删除"),
                "affected_keys_count": affected_keys_count,
            }))
            .into_response(),
        ));
    }

    if decision.route_family.as_deref() == Some("endpoints_manage")
        && decision.route_kind.as_deref() == Some("get_endpoint")
        && request_context
            .request_path
            .starts_with("/api/admin/endpoints/")
    {
        if !state.has_provider_catalog_data_reader() {
            return Ok(Some(build_admin_endpoints_data_unavailable_response()));
        }
        let Some(endpoint_id) = admin_endpoint_id(&request_context.request_path) else {
            return Ok(Some(
                (
                    http::StatusCode::NOT_FOUND,
                    Json(json!({ "detail": "Endpoint 不存在" })),
                )
                    .into_response(),
            ));
        };
        return Ok(Some(
            match build_admin_endpoint_payload(state, &endpoint_id).await {
                Some(payload) => Json(payload).into_response(),
                None => (
                    http::StatusCode::NOT_FOUND,
                    Json(json!({ "detail": format!("Endpoint {endpoint_id} 不存在") })),
                )
                    .into_response(),
            },
        ));
    }

    if decision.route_family.as_deref() == Some("endpoints_manage")
        && decision.route_kind.as_deref() == Some("default_body_rules")
        && request_context
            .request_path
            .starts_with("/api/admin/endpoints/defaults/")
        && request_context.request_path.ends_with("/body-rules")
    {
        let Some(api_format) = admin_default_body_rules_api_format(&request_context.request_path)
        else {
            return Ok(Some(
                (
                    http::StatusCode::BAD_REQUEST,
                    Json(json!({ "detail": "无效的 api_format" })),
                )
                    .into_response(),
            ));
        };
        let provider_type = query_param_value(
            request_context.request_query_string.as_deref(),
            "provider_type",
        );
        return Ok(Some(
            match admin_default_body_rules_for_signature(&api_format, provider_type.as_deref()) {
                Some((normalized_api_format, body_rules)) => Json(json!({
                    "api_format": normalized_api_format,
                    "body_rules": body_rules,
                }))
                .into_response(),
                None => (
                    http::StatusCode::BAD_REQUEST,
                    Json(json!({ "detail": format!("无效的 api_format: {api_format}") })),
                )
                    .into_response(),
            },
        ));
    }

    Ok(None)
}
