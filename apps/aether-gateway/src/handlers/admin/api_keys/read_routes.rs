use super::admin_api_keys_shared::{
    admin_api_key_total_tokens_by_ids, admin_api_keys_id_from_path, admin_api_keys_parse_limit,
    admin_api_keys_parse_skip, build_admin_api_key_detail_payload,
    build_admin_api_key_list_item_payload, build_admin_api_keys_bad_request_response,
    build_admin_api_keys_data_unavailable_response, build_admin_api_keys_not_found_response,
};
use super::{decrypt_catalog_secret_with_fallbacks, query_param_bool, query_param_optional_bool};
use crate::gateway::{AppState, GatewayError, GatewayPublicRequestContext};
use axum::{
    body::Body,
    http,
    response::{IntoResponse, Response},
    Json,
};
use serde_json::json;

pub(super) async fn build_admin_list_api_keys_response(
    state: &AppState,
    request_context: &GatewayPublicRequestContext,
) -> Result<Response<Body>, GatewayError> {
    let query = request_context.request_query_string.as_deref();
    let skip = match admin_api_keys_parse_skip(query) {
        Ok(value) => value,
        Err(detail) => return Ok(build_admin_api_keys_bad_request_response(detail)),
    };
    let limit = match admin_api_keys_parse_limit(query) {
        Ok(value) => value,
        Err(detail) => return Ok(build_admin_api_keys_bad_request_response(detail)),
    };
    let is_active = query_param_optional_bool(query, "is_active");

    let total = state
        .count_auth_api_key_export_standalone_records(is_active)
        .await? as usize;
    let paged_records = state
        .list_auth_api_key_export_standalone_records_page(
            &aether_data::repository::auth::StandaloneApiKeyExportListQuery {
                skip,
                limit,
                is_active,
            },
        )
        .await?;
    let api_key_ids = paged_records
        .iter()
        .map(|record| record.api_key_id.clone())
        .collect::<Vec<_>>();
    let total_tokens_by_api_key_id = admin_api_key_total_tokens_by_ids(state, &api_key_ids).await?;

    let api_keys = paged_records
        .iter()
        .map(|record| {
            build_admin_api_key_list_item_payload(
                state,
                record,
                total_tokens_by_api_key_id
                    .get(&record.api_key_id)
                    .copied()
                    .unwrap_or(0),
            )
        })
        .collect::<Vec<_>>();

    Ok(Json(json!({
        "api_keys": api_keys,
        "total": total,
        "limit": limit,
        "skip": skip,
    }))
    .into_response())
}

pub(super) async fn build_admin_api_key_detail_response(
    state: &AppState,
    request_context: &GatewayPublicRequestContext,
) -> Result<Response<Body>, GatewayError> {
    let Some(api_key_id) = admin_api_keys_id_from_path(&request_context.request_path) else {
        return Ok(build_admin_api_keys_data_unavailable_response());
    };

    if state
        .read_auth_api_key_snapshots_by_ids(std::slice::from_ref(&api_key_id))
        .await?
        .into_iter()
        .any(|snapshot| snapshot.api_key_id == api_key_id && !snapshot.api_key_is_standalone)
    {
        return Ok(build_admin_api_keys_bad_request_response(
            "仅支持查看独立密钥",
        ));
    }

    let Some(record) = state
        .find_auth_api_key_export_standalone_record_by_id(&api_key_id)
        .await?
    else {
        return Ok(build_admin_api_keys_not_found_response());
    };

    if query_param_bool(
        request_context.request_query_string.as_deref(),
        "include_key",
        false,
    ) {
        let Some(ciphertext) = record
            .key_encrypted
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty())
        else {
            return Ok(build_admin_api_keys_bad_request_response(
                "该密钥没有存储完整密钥信息",
            ));
        };
        let Some(key) = decrypt_catalog_secret_with_fallbacks(state.encryption_key(), ciphertext)
        else {
            return Ok((
                http::StatusCode::INTERNAL_SERVER_ERROR,
                Json(json!({ "detail": "解密密钥失败" })),
            )
                .into_response());
        };
        return Ok(Json(json!({ "key": key })).into_response());
    }

    let wallet = state
        .list_wallet_snapshots_by_api_key_ids(std::slice::from_ref(&api_key_id))
        .await?
        .into_iter()
        .find(|wallet| wallet.api_key_id.as_deref() == Some(api_key_id.as_str()));
    let total_tokens_by_api_key_id =
        admin_api_key_total_tokens_by_ids(state, std::slice::from_ref(&api_key_id)).await?;
    let total_tokens = total_tokens_by_api_key_id
        .get(&api_key_id)
        .copied()
        .unwrap_or(0);

    Ok(Json(build_admin_api_key_detail_payload(
        state,
        &record,
        total_tokens,
        wallet.as_ref(),
    ))
    .into_response())
}
