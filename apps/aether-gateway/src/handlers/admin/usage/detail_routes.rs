use super::super::round_to;
use super::replay::{
    admin_usage_build_curl_command, admin_usage_curl_headers, admin_usage_curl_url,
    admin_usage_headers_from_value, admin_usage_id_from_action_path,
    admin_usage_id_from_detail_path, admin_usage_resolve_request_preview_body,
    build_admin_usage_replay_response,
};
use super::{
    admin_usage_bad_request_response, admin_usage_data_unavailable_response,
    admin_usage_record_json, ADMIN_USAGE_DATA_UNAVAILABLE_DETAIL,
};
use crate::gateway::handlers::query_param_bool;
use crate::gateway::{AppState, GatewayError, GatewayPublicRequestContext};
use axum::{
    body::Body,
    http,
    response::{IntoResponse, Response},
    Json,
};
use serde_json::{json, Value};
use std::collections::BTreeMap;

pub(super) async fn maybe_build_local_admin_usage_detail_response(
    state: &AppState,
    request_context: &GatewayPublicRequestContext,
    request_body: Option<&axum::body::Bytes>,
) -> Result<Option<Response<Body>>, GatewayError> {
    let route_kind = request_context
        .control_decision
        .as_ref()
        .and_then(|decision| decision.route_kind.as_deref());

    match route_kind {
        Some("curl")
            if request_context.request_method == http::Method::GET
                && request_context
                    .request_path
                    .starts_with("/api/admin/usage/")
                && request_context.request_path.ends_with("/curl") =>
        {
            if !state.has_usage_data_reader() {
                return Ok(Some(admin_usage_data_unavailable_response(
                    ADMIN_USAGE_DATA_UNAVAILABLE_DETAIL,
                )));
            }

            let Some(usage_id) =
                admin_usage_id_from_action_path(&request_context.request_path, "/curl")
            else {
                return Ok(Some(admin_usage_bad_request_response("usage_id 无效")));
            };

            let Some(item) = state.find_request_usage_by_id(&usage_id).await? else {
                return Ok(Some(
                    (
                        http::StatusCode::NOT_FOUND,
                        Json(json!({ "detail": "Usage record not found" })),
                    )
                        .into_response(),
                ));
            };

            let endpoint = if let Some(endpoint_id) = item.provider_endpoint_id.as_ref() {
                state
                    .read_provider_catalog_endpoints_by_ids(std::slice::from_ref(endpoint_id))
                    .await?
                    .into_iter()
                    .next()
            } else {
                None
            };
            let url = endpoint
                .as_ref()
                .map(|endpoint| admin_usage_curl_url(endpoint, &item));
            let headers_json = item
                .provider_request_headers
                .clone()
                .or_else(|| item.request_headers.clone());
            let headers = headers_json
                .as_ref()
                .and_then(admin_usage_headers_from_value)
                .filter(|headers| !headers.is_empty())
                .unwrap_or_else(admin_usage_curl_headers);
            let body = item
                .provider_request_body
                .clone()
                .or_else(|| item.request_body.clone())
                .unwrap_or_else(|| admin_usage_resolve_request_preview_body(&item, None));
            let curl = admin_usage_build_curl_command(url.as_deref(), &headers, Some(&body));

            return Ok(Some(
                Json(json!({
                    "url": url,
                    "method": "POST",
                    "headers": headers_json.unwrap_or_else(|| json!(headers.clone())),
                    "body": body,
                    "curl": curl,
                    "original_request_body_available": item.request_body.is_some() || item.provider_request_body.is_some(),
                }))
                .into_response(),
            ));
        }
        Some("replay") => {
            return Ok(Some(
                build_admin_usage_replay_response(state, request_context, request_body).await?,
            ));
        }
        Some("detail")
            if request_context.request_method == http::Method::GET
                && request_context
                    .request_path
                    .starts_with("/api/admin/usage/") =>
        {
            if !state.has_usage_data_reader() {
                return Ok(Some(admin_usage_data_unavailable_response(
                    ADMIN_USAGE_DATA_UNAVAILABLE_DETAIL,
                )));
            }

            let Some(usage_id) = admin_usage_id_from_detail_path(&request_context.request_path)
            else {
                return Ok(Some(admin_usage_bad_request_response("usage_id 无效")));
            };
            let include_bodies = query_param_bool(
                request_context.request_query_string.as_deref(),
                "include_bodies",
                true,
            );

            let Some(item) = state.find_request_usage_by_id(&usage_id).await? else {
                return Ok(Some(
                    (
                        http::StatusCode::NOT_FOUND,
                        Json(json!({ "detail": "Usage record not found" })),
                    )
                        .into_response(),
                ));
            };

            let users_by_id: BTreeMap<String, aether_data::repository::users::StoredUserSummary> =
                if state.has_user_data_reader() {
                    if let Some(user_id) = item.user_id.as_ref() {
                        state
                            .list_users_by_ids(std::slice::from_ref(user_id))
                            .await?
                            .into_iter()
                            .map(|user| (user.id.clone(), user))
                            .collect()
                    } else {
                        BTreeMap::new()
                    }
                } else {
                    BTreeMap::new()
                };

            let mut payload = admin_usage_record_json(&item, &users_by_id);
            let request_body = item
                .request_body
                .clone()
                .unwrap_or_else(|| admin_usage_resolve_request_preview_body(&item, None));
            let request_preview_source = if item.request_body.is_some() {
                "stored_original"
            } else {
                "local_reconstruction"
            };
            let mut metadata = match item.request_metadata.clone() {
                Some(serde_json::Value::Object(object)) => serde_json::Value::Object(object),
                Some(value) => json!({ "request_metadata": value }),
                None => json!({}),
            };
            if let Some(object) = metadata.as_object_mut() {
                object.insert(
                    "request_preview_source".to_string(),
                    json!(request_preview_source),
                );
                object.insert(
                    "original_request_body_available".to_string(),
                    json!(item.request_body.is_some()),
                );
                object.insert(
                    "original_response_body_available".to_string(),
                    json!(item.response_body.is_some() || item.client_response_body.is_some()),
                );
            }
            payload["user"] = match item.user_id.as_ref() {
                Some(user_id) => json!({
                    "id": user_id,
                    "email": payload["user_email"].clone(),
                    "username": payload["username"].clone(),
                }),
                None => Value::Null,
            };
            payload["request_id"] = json!(item.request_id);
            payload["billing_status"] = json!(item.billing_status);
            payload["request_type"] = json!(item.request_type);
            payload["provider_id"] = json!(item.provider_id);
            payload["provider_endpoint_id"] = json!(item.provider_endpoint_id);
            payload["provider_api_key_id"] = json!(item.provider_api_key_id);
            payload["error_category"] = json!(item.error_category);
            payload["cache_creation_cost"] = json!(round_to(item.cache_creation_cost_usd, 6));
            payload["cache_read_cost"] = json!(round_to(item.cache_read_cost_usd, 6));
            payload["request_cost"] = json!(round_to(item.total_cost_usd, 6));
            payload["request_headers"] = item
                .request_headers
                .clone()
                .unwrap_or_else(|| json!(admin_usage_curl_headers()));
            payload["provider_request_headers"] = item
                .provider_request_headers
                .clone()
                .unwrap_or_else(|| json!(admin_usage_curl_headers()));
            payload["response_headers"] = item.response_headers.clone().unwrap_or(Value::Null);
            payload["client_response_headers"] =
                item.client_response_headers.clone().unwrap_or(Value::Null);
            payload["metadata"] = metadata;
            payload["has_request_body"] = json!(true);
            payload["has_provider_request_body"] = json!(item.provider_request_body.is_some());
            payload["has_response_body"] = json!(item.response_body.is_some());
            payload["has_client_response_body"] = json!(item.client_response_body.is_some());
            payload["tiered_pricing"] = serde_json::Value::Null;
            if include_bodies {
                payload["request_body"] = request_body;
                payload["provider_request_body"] =
                    item.provider_request_body.clone().unwrap_or(Value::Null);
                payload["response_body"] = item.response_body.clone().unwrap_or(Value::Null);
                payload["client_response_body"] =
                    item.client_response_body.clone().unwrap_or(Value::Null);
            } else {
                payload["request_body"] = serde_json::Value::Null;
                payload["provider_request_body"] = serde_json::Value::Null;
                payload["response_body"] = serde_json::Value::Null;
                payload["client_response_body"] = serde_json::Value::Null;
            }

            return Ok(Some(Json(payload).into_response()));
        }
        _ => {}
    }

    Ok(None)
}
