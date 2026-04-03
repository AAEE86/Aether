use super::{
    build_admin_payment_callback_payload, build_admin_payment_callback_payload_from_record,
    build_admin_payments_bad_request_response, parse_admin_payments_limit,
    parse_admin_payments_offset,
};
use crate::gateway::handlers::query_param_value;
use crate::gateway::{AppState, GatewayError, GatewayPublicRequestContext};
use axum::{
    body::Body,
    response::{IntoResponse, Response},
    Json,
};
use serde_json::json;
use sqlx::Row;

pub(super) async fn maybe_build_local_admin_payment_callbacks_response(
    state: &AppState,
    request_context: &GatewayPublicRequestContext,
    route_kind: Option<&str>,
) -> Result<Option<Response<Body>>, GatewayError> {
    match route_kind {
        Some("list_callbacks") => Ok(Some(
            build_admin_payment_callbacks_response(state, request_context).await?,
        )),
        _ => Ok(None),
    }
}

async fn build_admin_payment_callbacks_response(
    state: &AppState,
    request_context: &GatewayPublicRequestContext,
) -> Result<Response<Body>, GatewayError> {
    let query = request_context.request_query_string.as_deref();
    let limit = match parse_admin_payments_limit(query) {
        Ok(value) => value,
        Err(detail) => return Ok(build_admin_payments_bad_request_response(detail)),
    };
    let offset = match parse_admin_payments_offset(query) {
        Ok(value) => value,
        Err(detail) => return Ok(build_admin_payments_bad_request_response(detail)),
    };
    let payment_method = query_param_value(query, "payment_method");

    if let Some((items, total)) = state
        .list_admin_payment_callbacks(payment_method.as_deref(), limit, offset)
        .await?
    {
        return Ok(Json(json!({
            "items": items
                .iter()
                .map(build_admin_payment_callback_payload_from_record)
                .collect::<Vec<_>>(),
            "total": total,
            "limit": limit,
            "offset": offset,
        }))
        .into_response());
    }

    let mut total = 0_u64;
    let mut items = Vec::new();
    if let Some(pool) = state.postgres_pool() {
        let count_row = sqlx::query(
            r#"
SELECT COUNT(*) AS total
FROM payment_callbacks
WHERE ($1::TEXT IS NULL OR payment_method = $1)
            "#,
        )
        .bind(payment_method.as_deref())
        .fetch_one(&pool)
        .await
        .map_err(|err| GatewayError::Internal(err.to_string()))?;
        total = count_row
            .try_get::<i64, _>("total")
            .map_err(|err| GatewayError::Internal(err.to_string()))?
            .max(0) as u64;
        let rows = sqlx::query(
            r#"
SELECT
  id,
  payment_order_id,
  payment_method,
  callback_key,
  order_no,
  gateway_order_id,
  payload_hash,
  signature_valid,
  status,
  payload,
  error_message,
  CAST(EXTRACT(EPOCH FROM created_at) AS BIGINT) AS created_at_unix_secs,
  CAST(EXTRACT(EPOCH FROM processed_at) AS BIGINT) AS processed_at_unix_secs
FROM payment_callbacks
WHERE ($1::TEXT IS NULL OR payment_method = $1)
ORDER BY created_at DESC
OFFSET $2
LIMIT $3
            "#,
        )
        .bind(payment_method.as_deref())
        .bind(i64::try_from(offset).map_err(|err| GatewayError::Internal(err.to_string()))?)
        .bind(i64::try_from(limit).map_err(|err| GatewayError::Internal(err.to_string()))?)
        .fetch_all(&pool)
        .await
        .map_err(|err| GatewayError::Internal(err.to_string()))?;
        items = rows
            .iter()
            .map(build_admin_payment_callback_payload)
            .collect::<Result<Vec<_>, GatewayError>>()?;
    } else {
        return Ok(Json(json!({
            "items": [],
            "total": 0,
            "limit": limit,
            "offset": offset,
        }))
        .into_response());
    }

    Ok(Json(json!({
        "items": items,
        "total": total,
        "limit": limit,
        "offset": offset,
    }))
    .into_response())
}
