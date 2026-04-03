use super::admin_wallets_shared::{
    admin_wallet_id_from_detail_path, admin_wallet_id_from_suffix_path,
    build_admin_wallet_not_found_response, build_admin_wallet_refund_payload,
    build_admin_wallet_summary_payload, build_admin_wallets_bad_request_response,
    optional_epoch_value, parse_admin_wallets_limit, parse_admin_wallets_offset,
    parse_admin_wallets_owner_type_filter, resolve_admin_wallet_owner_summary,
    wallet_owner_summary_from_fields, ADMIN_WALLETS_API_KEY_REFUND_DETAIL,
};
use crate::gateway::handlers::{query_param_value, unix_secs_to_rfc3339};
use crate::gateway::{AppState, GatewayError, GatewayPublicRequestContext};
use axum::{
    body::Body,
    response::{IntoResponse, Response},
    Json,
};
use serde_json::json;
use sqlx::Row;

pub(super) async fn build_admin_wallet_detail_response(
    state: &AppState,
    request_context: &GatewayPublicRequestContext,
) -> Result<Response<Body>, GatewayError> {
    let Some(wallet_id) = admin_wallet_id_from_detail_path(&request_context.request_path) else {
        return Ok(build_admin_wallets_bad_request_response("wallet_id 无效"));
    };

    let Some(wallet) = state
        .find_wallet(aether_data::repository::wallet::WalletLookupKey::WalletId(
            &wallet_id,
        ))
        .await?
    else {
        return Ok(build_admin_wallet_not_found_response());
    };

    let owner = resolve_admin_wallet_owner_summary(state, &wallet).await?;
    let mut payload = build_admin_wallet_summary_payload(&wallet, &owner);
    if let Some(object) = payload.as_object_mut() {
        object.insert("pending_refund_count".to_string(), serde_json::Value::Null);
    }
    Ok(Json(payload).into_response())
}

pub(super) async fn build_admin_wallet_list_response(
    state: &AppState,
    request_context: &GatewayPublicRequestContext,
) -> Result<Response<Body>, GatewayError> {
    let limit = match parse_admin_wallets_limit(request_context.request_query_string.as_deref()) {
        Ok(value) => value,
        Err(detail) => return Ok(build_admin_wallets_bad_request_response(detail)),
    };
    let offset = match parse_admin_wallets_offset(request_context.request_query_string.as_deref()) {
        Ok(value) => value,
        Err(detail) => return Ok(build_admin_wallets_bad_request_response(detail)),
    };
    let status = query_param_value(request_context.request_query_string.as_deref(), "status");

    let mut total = 0_u64;
    let mut items = Vec::new();
    if let Some(pool) = state.postgres_pool() {
        let count_row = sqlx::query(
            r#"
SELECT COUNT(*) AS total
FROM wallets
WHERE ($1::TEXT IS NULL OR status = $1)
            "#,
        )
        .bind(status.as_deref())
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
  w.id,
  w.user_id,
  w.api_key_id,
  CAST(w.balance AS DOUBLE PRECISION) AS balance,
  CAST(w.gift_balance AS DOUBLE PRECISION) AS gift_balance,
  w.limit_mode,
  w.currency,
  w.status,
  CAST(w.total_recharged AS DOUBLE PRECISION) AS total_recharged,
  CAST(w.total_consumed AS DOUBLE PRECISION) AS total_consumed,
  CAST(w.total_refunded AS DOUBLE PRECISION) AS total_refunded,
  CAST(w.total_adjusted AS DOUBLE PRECISION) AS total_adjusted,
  users.username AS user_name,
  api_keys.name AS api_key_name,
  CAST(EXTRACT(EPOCH FROM w.created_at) AS BIGINT) AS created_at_unix_secs,
  CAST(EXTRACT(EPOCH FROM w.updated_at) AS BIGINT) AS updated_at_unix_secs
FROM wallets w
LEFT JOIN users ON users.id = w.user_id
LEFT JOIN api_keys ON api_keys.id = w.api_key_id
WHERE ($1::TEXT IS NULL OR w.status = $1)
ORDER BY w.updated_at DESC
OFFSET $2
LIMIT $3
            "#,
        )
        .bind(status.as_deref())
        .bind(i64::try_from(offset).map_err(|err| GatewayError::Internal(err.to_string()))?)
        .bind(i64::try_from(limit).map_err(|err| GatewayError::Internal(err.to_string()))?)
        .fetch_all(&pool)
        .await
        .map_err(|err| GatewayError::Internal(err.to_string()))?;

        items = rows
            .into_iter()
            .map(|row| {
                let wallet_id = row
                    .try_get::<String, _>("id")
                    .map_err(|err| GatewayError::Internal(err.to_string()))?;
                let user_id = row
                    .try_get::<Option<String>, _>("user_id")
                    .map_err(|err| GatewayError::Internal(err.to_string()))?;
                let api_key_id = row
                    .try_get::<Option<String>, _>("api_key_id")
                    .map_err(|err| GatewayError::Internal(err.to_string()))?;
                let owner = wallet_owner_summary_from_fields(
                    user_id.as_deref(),
                    row.try_get::<Option<String>, _>("user_name")
                        .map_err(|err| GatewayError::Internal(err.to_string()))?,
                    api_key_id.as_deref(),
                    row.try_get::<Option<String>, _>("api_key_name")
                        .map_err(|err| GatewayError::Internal(err.to_string()))?,
                );
                Ok(json!({
                    "id": wallet_id,
                    "user_id": user_id,
                    "api_key_id": api_key_id,
                    "owner_type": owner.owner_type,
                    "owner_name": owner.owner_name,
                    "balance": row.try_get::<f64, _>("balance").map_err(|err| GatewayError::Internal(err.to_string()))?
                        + row.try_get::<f64, _>("gift_balance").map_err(|err| GatewayError::Internal(err.to_string()))?,
                    "recharge_balance": row.try_get::<f64, _>("balance").map_err(|err| GatewayError::Internal(err.to_string()))?,
                    "gift_balance": row.try_get::<f64, _>("gift_balance").map_err(|err| GatewayError::Internal(err.to_string()))?,
                    "refundable_balance": row.try_get::<f64, _>("balance").map_err(|err| GatewayError::Internal(err.to_string()))?,
                    "currency": row.try_get::<String, _>("currency").map_err(|err| GatewayError::Internal(err.to_string()))?,
                    "status": row.try_get::<String, _>("status").map_err(|err| GatewayError::Internal(err.to_string()))?,
                    "limit_mode": row.try_get::<String, _>("limit_mode").map_err(|err| GatewayError::Internal(err.to_string()))?,
                    "unlimited": row.try_get::<String, _>("limit_mode").map_err(|err| GatewayError::Internal(err.to_string()))?.eq_ignore_ascii_case("unlimited"),
                    "total_recharged": row.try_get::<f64, _>("total_recharged").map_err(|err| GatewayError::Internal(err.to_string()))?,
                    "total_consumed": row.try_get::<f64, _>("total_consumed").map_err(|err| GatewayError::Internal(err.to_string()))?,
                    "total_refunded": row.try_get::<f64, _>("total_refunded").map_err(|err| GatewayError::Internal(err.to_string()))?,
                    "total_adjusted": row.try_get::<f64, _>("total_adjusted").map_err(|err| GatewayError::Internal(err.to_string()))?,
                    "created_at": optional_epoch_value(&row, "created_at_unix_secs")?,
                    "updated_at": optional_epoch_value(&row, "updated_at_unix_secs")?,
                }))
            })
            .collect::<Result<Vec<_>, GatewayError>>()?;
    }

    Ok(Json(json!({
        "items": items,
        "total": total,
        "limit": limit,
        "offset": offset,
    }))
    .into_response())
}

pub(super) async fn build_admin_wallet_ledger_response(
    state: &AppState,
    request_context: &GatewayPublicRequestContext,
) -> Result<Response<Body>, GatewayError> {
    let query = request_context.request_query_string.as_deref();
    let limit = match parse_admin_wallets_limit(query) {
        Ok(value) => value,
        Err(detail) => return Ok(build_admin_wallets_bad_request_response(detail)),
    };
    let offset = match parse_admin_wallets_offset(query) {
        Ok(value) => value,
        Err(detail) => return Ok(build_admin_wallets_bad_request_response(detail)),
    };
    let category = query_param_value(query, "category");
    let reason_code = query_param_value(query, "reason_code");
    let owner_type = parse_admin_wallets_owner_type_filter(query);

    let mut total = 0_u64;
    let mut items = Vec::new();
    if let Some(pool) = state.postgres_pool() {
        let count_row = sqlx::query(
            r#"
SELECT COUNT(*) AS total
FROM wallet_transactions tx
JOIN wallets w ON w.id = tx.wallet_id
WHERE ($1::TEXT IS NULL OR tx.category = $1)
  AND ($2::TEXT IS NULL OR tx.reason_code = $2)
  AND (
    $3::TEXT IS NULL
    OR ($3 = 'user' AND w.user_id IS NOT NULL)
    OR ($3 = 'api_key' AND w.api_key_id IS NOT NULL)
  )
            "#,
        )
        .bind(category.as_deref())
        .bind(reason_code.as_deref())
        .bind(owner_type.as_deref())
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
  tx.id,
  tx.wallet_id,
  tx.category,
  tx.reason_code,
  CAST(tx.amount AS DOUBLE PRECISION) AS amount,
  CAST(tx.balance_before AS DOUBLE PRECISION) AS balance_before,
  CAST(tx.balance_after AS DOUBLE PRECISION) AS balance_after,
  CAST(tx.recharge_balance_before AS DOUBLE PRECISION) AS recharge_balance_before,
  CAST(tx.recharge_balance_after AS DOUBLE PRECISION) AS recharge_balance_after,
  CAST(tx.gift_balance_before AS DOUBLE PRECISION) AS gift_balance_before,
  CAST(tx.gift_balance_after AS DOUBLE PRECISION) AS gift_balance_after,
  tx.link_type,
  tx.link_id,
  tx.operator_id,
  tx.description,
  w.user_id,
  w.api_key_id,
  w.status AS wallet_status,
  wallet_users.username AS wallet_user_name,
  api_keys.name AS api_key_name,
  operator_users.username AS operator_name,
  operator_users.email AS operator_email,
  CAST(EXTRACT(EPOCH FROM tx.created_at) AS BIGINT) AS created_at_unix_secs
FROM wallet_transactions tx
JOIN wallets w ON w.id = tx.wallet_id
LEFT JOIN users wallet_users ON wallet_users.id = w.user_id
LEFT JOIN api_keys ON api_keys.id = w.api_key_id
LEFT JOIN users operator_users ON operator_users.id = tx.operator_id
WHERE ($1::TEXT IS NULL OR tx.category = $1)
  AND ($2::TEXT IS NULL OR tx.reason_code = $2)
  AND (
    $3::TEXT IS NULL
    OR ($3 = 'user' AND w.user_id IS NOT NULL)
    OR ($3 = 'api_key' AND w.api_key_id IS NOT NULL)
  )
ORDER BY tx.created_at DESC
OFFSET $4
LIMIT $5
            "#,
        )
        .bind(category.as_deref())
        .bind(reason_code.as_deref())
        .bind(owner_type.as_deref())
        .bind(i64::try_from(offset).map_err(|err| GatewayError::Internal(err.to_string()))?)
        .bind(i64::try_from(limit).map_err(|err| GatewayError::Internal(err.to_string()))?)
        .fetch_all(&pool)
        .await
        .map_err(|err| GatewayError::Internal(err.to_string()))?;

        items = rows
            .into_iter()
            .map(|row| {
                let user_id = row
                    .try_get::<Option<String>, _>("user_id")
                    .map_err(|err| GatewayError::Internal(err.to_string()))?;
                let api_key_id = row
                    .try_get::<Option<String>, _>("api_key_id")
                    .map_err(|err| GatewayError::Internal(err.to_string()))?;
                let owner = wallet_owner_summary_from_fields(
                    user_id.as_deref(),
                    row.try_get::<Option<String>, _>("wallet_user_name")
                        .map_err(|err| GatewayError::Internal(err.to_string()))?,
                    api_key_id.as_deref(),
                    row.try_get::<Option<String>, _>("api_key_name")
                        .map_err(|err| GatewayError::Internal(err.to_string()))?,
                );
                Ok(json!({
                    "id": row.try_get::<String, _>("id").map_err(|err| GatewayError::Internal(err.to_string()))?,
                    "wallet_id": row.try_get::<String, _>("wallet_id").map_err(|err| GatewayError::Internal(err.to_string()))?,
                    "owner_type": owner.owner_type,
                    "owner_name": owner.owner_name,
                    "wallet_status": row.try_get::<String, _>("wallet_status").map_err(|err| GatewayError::Internal(err.to_string()))?,
                    "category": row.try_get::<String, _>("category").map_err(|err| GatewayError::Internal(err.to_string()))?,
                    "reason_code": row.try_get::<String, _>("reason_code").map_err(|err| GatewayError::Internal(err.to_string()))?,
                    "amount": row.try_get::<f64, _>("amount").map_err(|err| GatewayError::Internal(err.to_string()))?,
                    "balance_before": row.try_get::<f64, _>("balance_before").map_err(|err| GatewayError::Internal(err.to_string()))?,
                    "balance_after": row.try_get::<f64, _>("balance_after").map_err(|err| GatewayError::Internal(err.to_string()))?,
                    "recharge_balance_before": row.try_get::<f64, _>("recharge_balance_before").map_err(|err| GatewayError::Internal(err.to_string()))?,
                    "recharge_balance_after": row.try_get::<f64, _>("recharge_balance_after").map_err(|err| GatewayError::Internal(err.to_string()))?,
                    "gift_balance_before": row.try_get::<f64, _>("gift_balance_before").map_err(|err| GatewayError::Internal(err.to_string()))?,
                    "gift_balance_after": row.try_get::<f64, _>("gift_balance_after").map_err(|err| GatewayError::Internal(err.to_string()))?,
                    "link_type": row.try_get::<Option<String>, _>("link_type").map_err(|err| GatewayError::Internal(err.to_string()))?,
                    "link_id": row.try_get::<Option<String>, _>("link_id").map_err(|err| GatewayError::Internal(err.to_string()))?,
                    "operator_id": row.try_get::<Option<String>, _>("operator_id").map_err(|err| GatewayError::Internal(err.to_string()))?,
                    "operator_name": row.try_get::<Option<String>, _>("operator_name").map_err(|err| GatewayError::Internal(err.to_string()))?,
                    "operator_email": row.try_get::<Option<String>, _>("operator_email").map_err(|err| GatewayError::Internal(err.to_string()))?,
                    "description": row.try_get::<Option<String>, _>("description").map_err(|err| GatewayError::Internal(err.to_string()))?,
                    "created_at": optional_epoch_value(&row, "created_at_unix_secs")?,
                }))
            })
            .collect::<Result<Vec<_>, GatewayError>>()?;
    }

    Ok(Json(json!({
        "items": items,
        "total": total,
        "limit": limit,
        "offset": offset,
    }))
    .into_response())
}

pub(super) async fn build_admin_wallet_refund_requests_response(
    state: &AppState,
    request_context: &GatewayPublicRequestContext,
) -> Result<Response<Body>, GatewayError> {
    let query = request_context.request_query_string.as_deref();
    let limit = match parse_admin_wallets_limit(query) {
        Ok(value) => value,
        Err(detail) => return Ok(build_admin_wallets_bad_request_response(detail)),
    };
    let offset = match parse_admin_wallets_offset(query) {
        Ok(value) => value,
        Err(detail) => return Ok(build_admin_wallets_bad_request_response(detail)),
    };
    let status = query_param_value(query, "status");
    let owner_type = parse_admin_wallets_owner_type_filter(query);
    if owner_type.as_deref() == Some("api_key") {
        return Ok(build_admin_wallets_bad_request_response(
            ADMIN_WALLETS_API_KEY_REFUND_DETAIL,
        ));
    }

    let mut total = 0_u64;
    let mut items = Vec::new();
    if let Some((refunds, refund_total)) = state
        .list_admin_wallet_refund_requests(status.as_deref(), limit, offset)
        .await?
    {
        total = refund_total;
        let mut local_items = Vec::with_capacity(refunds.len());
        for refund in refunds {
            let Some(wallet) = state
                .find_wallet(aether_data::repository::wallet::WalletLookupKey::WalletId(
                    &refund.wallet_id,
                ))
                .await?
            else {
                continue;
            };
            let owner = resolve_admin_wallet_owner_summary(state, &wallet).await?;
            local_items.push(build_admin_wallet_refund_payload(&wallet, &owner, &refund));
        }
        items = local_items;
    } else if let Some(pool) = state.postgres_pool() {
        let count_row = sqlx::query(
            r#"
SELECT COUNT(*) AS total
FROM refund_requests rr
JOIN wallets w ON w.id = rr.wallet_id
WHERE ($1::TEXT IS NULL OR rr.status = $1)
  AND w.user_id IS NOT NULL
            "#,
        )
        .bind(status.as_deref())
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
  rr.id,
  rr.refund_no,
  rr.wallet_id,
  rr.user_id,
  rr.payment_order_id,
  rr.source_type,
  rr.source_id,
  rr.refund_mode,
  CAST(rr.amount_usd AS DOUBLE PRECISION) AS amount_usd,
  rr.status,
  rr.reason,
  rr.failure_reason,
  rr.gateway_refund_id,
  rr.payout_method,
  rr.payout_reference,
  rr.payout_proof,
  rr.requested_by,
  rr.approved_by,
  rr.processed_by,
  w.user_id AS wallet_user_id,
  w.api_key_id AS wallet_api_key_id,
  w.status AS wallet_status,
  wallet_users.username AS wallet_user_name,
  api_keys.name AS api_key_name,
  CAST(EXTRACT(EPOCH FROM rr.created_at) AS BIGINT) AS created_at_unix_secs,
  CAST(EXTRACT(EPOCH FROM rr.updated_at) AS BIGINT) AS updated_at_unix_secs,
  CAST(EXTRACT(EPOCH FROM rr.processed_at) AS BIGINT) AS processed_at_unix_secs,
  CAST(EXTRACT(EPOCH FROM rr.completed_at) AS BIGINT) AS completed_at_unix_secs
FROM refund_requests rr
JOIN wallets w ON w.id = rr.wallet_id
LEFT JOIN users wallet_users ON wallet_users.id = w.user_id
LEFT JOIN api_keys ON api_keys.id = w.api_key_id
WHERE ($1::TEXT IS NULL OR rr.status = $1)
  AND w.user_id IS NOT NULL
ORDER BY rr.created_at DESC
OFFSET $2
LIMIT $3
            "#,
        )
        .bind(status.as_deref())
        .bind(i64::try_from(offset).map_err(|err| GatewayError::Internal(err.to_string()))?)
        .bind(i64::try_from(limit).map_err(|err| GatewayError::Internal(err.to_string()))?)
        .fetch_all(&pool)
        .await
        .map_err(|err| GatewayError::Internal(err.to_string()))?;

        items = rows
            .into_iter()
            .map(|row| {
                let wallet_user_id = row
                    .try_get::<Option<String>, _>("wallet_user_id")
                    .map_err(|err| GatewayError::Internal(err.to_string()))?;
                let wallet_api_key_id = row
                    .try_get::<Option<String>, _>("wallet_api_key_id")
                    .map_err(|err| GatewayError::Internal(err.to_string()))?;
                let owner = wallet_owner_summary_from_fields(
                    wallet_user_id.as_deref(),
                    row.try_get::<Option<String>, _>("wallet_user_name")
                        .map_err(|err| GatewayError::Internal(err.to_string()))?,
                    wallet_api_key_id.as_deref(),
                    row.try_get::<Option<String>, _>("api_key_name")
                        .map_err(|err| GatewayError::Internal(err.to_string()))?,
                );
                Ok(json!({
                    "id": row.try_get::<String, _>("id").map_err(|err| GatewayError::Internal(err.to_string()))?,
                    "refund_no": row.try_get::<String, _>("refund_no").map_err(|err| GatewayError::Internal(err.to_string()))?,
                    "wallet_id": row.try_get::<String, _>("wallet_id").map_err(|err| GatewayError::Internal(err.to_string()))?,
                    "owner_type": owner.owner_type,
                    "owner_name": owner.owner_name,
                    "wallet_status": row.try_get::<String, _>("wallet_status").map_err(|err| GatewayError::Internal(err.to_string()))?,
                    "user_id": row.try_get::<Option<String>, _>("user_id").map_err(|err| GatewayError::Internal(err.to_string()))?,
                    "payment_order_id": row.try_get::<Option<String>, _>("payment_order_id").map_err(|err| GatewayError::Internal(err.to_string()))?,
                    "source_type": row.try_get::<String, _>("source_type").map_err(|err| GatewayError::Internal(err.to_string()))?,
                    "source_id": row.try_get::<Option<String>, _>("source_id").map_err(|err| GatewayError::Internal(err.to_string()))?,
                    "refund_mode": row.try_get::<String, _>("refund_mode").map_err(|err| GatewayError::Internal(err.to_string()))?,
                    "amount_usd": row.try_get::<f64, _>("amount_usd").map_err(|err| GatewayError::Internal(err.to_string()))?,
                    "status": row.try_get::<String, _>("status").map_err(|err| GatewayError::Internal(err.to_string()))?,
                    "reason": row.try_get::<Option<String>, _>("reason").map_err(|err| GatewayError::Internal(err.to_string()))?,
                    "failure_reason": row.try_get::<Option<String>, _>("failure_reason").map_err(|err| GatewayError::Internal(err.to_string()))?,
                    "gateway_refund_id": row.try_get::<Option<String>, _>("gateway_refund_id").map_err(|err| GatewayError::Internal(err.to_string()))?,
                    "payout_method": row.try_get::<Option<String>, _>("payout_method").map_err(|err| GatewayError::Internal(err.to_string()))?,
                    "payout_reference": row.try_get::<Option<String>, _>("payout_reference").map_err(|err| GatewayError::Internal(err.to_string()))?,
                    "payout_proof": row.try_get::<Option<serde_json::Value>, _>("payout_proof").map_err(|err| GatewayError::Internal(err.to_string()))?,
                    "requested_by": row.try_get::<Option<String>, _>("requested_by").map_err(|err| GatewayError::Internal(err.to_string()))?,
                    "approved_by": row.try_get::<Option<String>, _>("approved_by").map_err(|err| GatewayError::Internal(err.to_string()))?,
                    "processed_by": row.try_get::<Option<String>, _>("processed_by").map_err(|err| GatewayError::Internal(err.to_string()))?,
                    "created_at": optional_epoch_value(&row, "created_at_unix_secs")?,
                    "updated_at": optional_epoch_value(&row, "updated_at_unix_secs")?,
                    "processed_at": optional_epoch_value(&row, "processed_at_unix_secs")?,
                    "completed_at": optional_epoch_value(&row, "completed_at_unix_secs")?,
                }))
            })
            .collect::<Result<Vec<_>, GatewayError>>()?;
    }

    Ok(Json(json!({
        "items": items,
        "total": total,
        "limit": limit,
        "offset": offset,
    }))
    .into_response())
}

pub(super) async fn build_admin_wallet_transactions_response(
    state: &AppState,
    request_context: &GatewayPublicRequestContext,
) -> Result<Response<Body>, GatewayError> {
    let Some(wallet_id) =
        admin_wallet_id_from_suffix_path(&request_context.request_path, "/transactions")
    else {
        return Ok(build_admin_wallets_bad_request_response("wallet_id 无效"));
    };
    let limit = match parse_admin_wallets_limit(request_context.request_query_string.as_deref()) {
        Ok(value) => value,
        Err(detail) => return Ok(build_admin_wallets_bad_request_response(detail)),
    };
    let offset = match parse_admin_wallets_offset(request_context.request_query_string.as_deref()) {
        Ok(value) => value,
        Err(detail) => return Ok(build_admin_wallets_bad_request_response(detail)),
    };

    let Some(wallet) = state
        .find_wallet(aether_data::repository::wallet::WalletLookupKey::WalletId(
            &wallet_id,
        ))
        .await?
    else {
        return Ok(build_admin_wallet_not_found_response());
    };
    let owner = resolve_admin_wallet_owner_summary(state, &wallet).await?;
    let wallet_payload = build_admin_wallet_summary_payload(&wallet, &owner);

    let mut total = 0_u64;
    let mut items = Vec::new();
    if let Some((transactions, transaction_total)) = state
        .list_admin_wallet_transactions(&wallet.id, limit, offset)
        .await?
    {
        total = transaction_total;
        let mut local_items = Vec::with_capacity(transactions.len());
        for transaction in transactions {
            let (operator_name, operator_email) = match transaction.operator_id.as_deref() {
                Some(operator_id) => state
                    .find_user_auth_by_id(operator_id)
                    .await?
                    .map(|user| (Some(user.username), user.email))
                    .unwrap_or((None, None)),
                None => (None, None),
            };
            local_items.push(json!({
                "id": transaction.id,
                "wallet_id": transaction.wallet_id,
                "owner_type": owner.owner_type,
                "owner_name": owner.owner_name.clone(),
                "wallet_status": wallet.status.clone(),
                "category": transaction.category,
                "reason_code": transaction.reason_code,
                "amount": transaction.amount,
                "balance_before": transaction.balance_before,
                "balance_after": transaction.balance_after,
                "recharge_balance_before": transaction.recharge_balance_before,
                "recharge_balance_after": transaction.recharge_balance_after,
                "gift_balance_before": transaction.gift_balance_before,
                "gift_balance_after": transaction.gift_balance_after,
                "link_type": transaction.link_type,
                "link_id": transaction.link_id,
                "operator_id": transaction.operator_id,
                "operator_name": operator_name,
                "operator_email": operator_email,
                "description": transaction.description,
                "created_at": unix_secs_to_rfc3339(transaction.created_at_unix_secs),
            }));
        }
        items = local_items;
    } else if let Some(pool) = state.postgres_pool() {
        let count_row = sqlx::query(
            r#"
SELECT COUNT(*) AS total
FROM wallet_transactions
WHERE wallet_id = $1
            "#,
        )
        .bind(&wallet.id)
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
  tx.id,
  tx.wallet_id,
  tx.category,
  tx.reason_code,
  CAST(tx.amount AS DOUBLE PRECISION) AS amount,
  CAST(tx.balance_before AS DOUBLE PRECISION) AS balance_before,
  CAST(tx.balance_after AS DOUBLE PRECISION) AS balance_after,
  CAST(tx.recharge_balance_before AS DOUBLE PRECISION) AS recharge_balance_before,
  CAST(tx.recharge_balance_after AS DOUBLE PRECISION) AS recharge_balance_after,
  CAST(tx.gift_balance_before AS DOUBLE PRECISION) AS gift_balance_before,
  CAST(tx.gift_balance_after AS DOUBLE PRECISION) AS gift_balance_after,
  tx.link_type,
  tx.link_id,
  tx.operator_id,
  tx.description,
  operator_users.username AS operator_name,
  operator_users.email AS operator_email,
  CAST(EXTRACT(EPOCH FROM tx.created_at) AS BIGINT) AS created_at_unix_secs
FROM wallet_transactions tx
LEFT JOIN users operator_users
  ON operator_users.id = tx.operator_id
WHERE tx.wallet_id = $1
ORDER BY tx.created_at DESC
OFFSET $2
LIMIT $3
            "#,
        )
        .bind(&wallet.id)
        .bind(i64::try_from(offset).map_err(|err| GatewayError::Internal(err.to_string()))?)
        .bind(i64::try_from(limit).map_err(|err| GatewayError::Internal(err.to_string()))?)
        .fetch_all(&pool)
        .await
        .map_err(|err| GatewayError::Internal(err.to_string()))?;

        items = rows
            .into_iter()
            .map(|row| {
                let created_at_unix_secs = row
                    .try_get::<Option<i64>, _>("created_at_unix_secs")
                    .map_err(|err| GatewayError::Internal(err.to_string()))?
                    .and_then(|value| u64::try_from(value).ok())
                    .and_then(unix_secs_to_rfc3339);
                Ok(json!({
                    "id": row.try_get::<String, _>("id").map_err(|err| GatewayError::Internal(err.to_string()))?,
                    "wallet_id": row.try_get::<String, _>("wallet_id").map_err(|err| GatewayError::Internal(err.to_string()))?,
                    "owner_type": owner.owner_type,
                    "owner_name": owner.owner_name.clone(),
                    "wallet_status": wallet.status.clone(),
                    "category": row.try_get::<String, _>("category").map_err(|err| GatewayError::Internal(err.to_string()))?,
                    "reason_code": row.try_get::<String, _>("reason_code").map_err(|err| GatewayError::Internal(err.to_string()))?,
                    "amount": row.try_get::<f64, _>("amount").map_err(|err| GatewayError::Internal(err.to_string()))?,
                    "balance_before": row.try_get::<f64, _>("balance_before").map_err(|err| GatewayError::Internal(err.to_string()))?,
                    "balance_after": row.try_get::<f64, _>("balance_after").map_err(|err| GatewayError::Internal(err.to_string()))?,
                    "recharge_balance_before": row.try_get::<f64, _>("recharge_balance_before").map_err(|err| GatewayError::Internal(err.to_string()))?,
                    "recharge_balance_after": row.try_get::<f64, _>("recharge_balance_after").map_err(|err| GatewayError::Internal(err.to_string()))?,
                    "gift_balance_before": row.try_get::<f64, _>("gift_balance_before").map_err(|err| GatewayError::Internal(err.to_string()))?,
                    "gift_balance_after": row.try_get::<f64, _>("gift_balance_after").map_err(|err| GatewayError::Internal(err.to_string()))?,
                    "link_type": row.try_get::<Option<String>, _>("link_type").map_err(|err| GatewayError::Internal(err.to_string()))?,
                    "link_id": row.try_get::<Option<String>, _>("link_id").map_err(|err| GatewayError::Internal(err.to_string()))?,
                    "operator_id": row.try_get::<Option<String>, _>("operator_id").map_err(|err| GatewayError::Internal(err.to_string()))?,
                    "operator_name": row.try_get::<Option<String>, _>("operator_name").map_err(|err| GatewayError::Internal(err.to_string()))?,
                    "operator_email": row.try_get::<Option<String>, _>("operator_email").map_err(|err| GatewayError::Internal(err.to_string()))?,
                    "description": row.try_get::<Option<String>, _>("description").map_err(|err| GatewayError::Internal(err.to_string()))?,
                    "created_at": created_at_unix_secs,
                }))
            })
            .collect::<Result<Vec<_>, GatewayError>>()?;
    }

    Ok(Json(json!({
        "wallet": wallet_payload,
        "items": items,
        "total": total,
        "limit": limit,
        "offset": offset,
    }))
    .into_response())
}

pub(super) async fn build_admin_wallet_refunds_response(
    state: &AppState,
    request_context: &GatewayPublicRequestContext,
) -> Result<Response<Body>, GatewayError> {
    let Some(wallet_id) =
        admin_wallet_id_from_suffix_path(&request_context.request_path, "/refunds")
    else {
        return Ok(build_admin_wallets_bad_request_response("wallet_id 无效"));
    };
    let limit = match parse_admin_wallets_limit(request_context.request_query_string.as_deref()) {
        Ok(value) => value,
        Err(detail) => return Ok(build_admin_wallets_bad_request_response(detail)),
    };
    let offset = match parse_admin_wallets_offset(request_context.request_query_string.as_deref()) {
        Ok(value) => value,
        Err(detail) => return Ok(build_admin_wallets_bad_request_response(detail)),
    };

    let Some(wallet) = state
        .find_wallet(aether_data::repository::wallet::WalletLookupKey::WalletId(
            &wallet_id,
        ))
        .await?
    else {
        return Ok(build_admin_wallet_not_found_response());
    };
    if wallet.api_key_id.is_some() {
        return Ok(build_admin_wallets_bad_request_response(
            ADMIN_WALLETS_API_KEY_REFUND_DETAIL,
        ));
    }

    let owner = resolve_admin_wallet_owner_summary(state, &wallet).await?;
    let wallet_payload = build_admin_wallet_summary_payload(&wallet, &owner);
    let mut total = 0_u64;
    let mut items = Vec::new();

    if let Some((refunds, refund_total)) = state
        .list_admin_wallet_refunds(&wallet.id, limit, offset)
        .await?
    {
        total = refund_total;
        items = refunds
            .into_iter()
            .map(|refund| build_admin_wallet_refund_payload(&wallet, &owner, &refund))
            .collect();
    } else if let Some(pool) = state.postgres_pool() {
        let count_row = sqlx::query(
            r#"
SELECT COUNT(*) AS total
FROM refund_requests
WHERE wallet_id = $1
            "#,
        )
        .bind(&wallet.id)
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
  refund_no,
  wallet_id,
  user_id,
  payment_order_id,
  source_type,
  source_id,
  refund_mode,
  CAST(amount_usd AS DOUBLE PRECISION) AS amount_usd,
  status,
  reason,
  failure_reason,
  gateway_refund_id,
  payout_method,
  payout_reference,
  payout_proof,
  requested_by,
  approved_by,
  processed_by,
  CAST(EXTRACT(EPOCH FROM created_at) AS BIGINT) AS created_at_unix_secs,
  CAST(EXTRACT(EPOCH FROM updated_at) AS BIGINT) AS updated_at_unix_secs,
  CAST(EXTRACT(EPOCH FROM processed_at) AS BIGINT) AS processed_at_unix_secs,
  CAST(EXTRACT(EPOCH FROM completed_at) AS BIGINT) AS completed_at_unix_secs
FROM refund_requests
WHERE wallet_id = $1
ORDER BY created_at DESC
OFFSET $2
LIMIT $3
            "#,
        )
        .bind(&wallet.id)
        .bind(i64::try_from(offset).map_err(|err| GatewayError::Internal(err.to_string()))?)
        .bind(i64::try_from(limit).map_err(|err| GatewayError::Internal(err.to_string()))?)
        .fetch_all(&pool)
        .await
        .map_err(|err| GatewayError::Internal(err.to_string()))?;

        items = rows
            .into_iter()
            .map(|row| {
                let created_at = row
                    .try_get::<Option<i64>, _>("created_at_unix_secs")
                    .map_err(|err| GatewayError::Internal(err.to_string()))?
                    .and_then(|value| u64::try_from(value).ok())
                    .and_then(unix_secs_to_rfc3339);
                let updated_at = row
                    .try_get::<Option<i64>, _>("updated_at_unix_secs")
                    .map_err(|err| GatewayError::Internal(err.to_string()))?
                    .and_then(|value| u64::try_from(value).ok())
                    .and_then(unix_secs_to_rfc3339);
                let processed_at = row
                    .try_get::<Option<i64>, _>("processed_at_unix_secs")
                    .map_err(|err| GatewayError::Internal(err.to_string()))?
                    .and_then(|value| u64::try_from(value).ok())
                    .and_then(unix_secs_to_rfc3339);
                let completed_at = row
                    .try_get::<Option<i64>, _>("completed_at_unix_secs")
                    .map_err(|err| GatewayError::Internal(err.to_string()))?
                    .and_then(|value| u64::try_from(value).ok())
                    .and_then(unix_secs_to_rfc3339);
                Ok(json!({
                    "id": row.try_get::<String, _>("id").map_err(|err| GatewayError::Internal(err.to_string()))?,
                    "refund_no": row.try_get::<String, _>("refund_no").map_err(|err| GatewayError::Internal(err.to_string()))?,
                    "wallet_id": row.try_get::<String, _>("wallet_id").map_err(|err| GatewayError::Internal(err.to_string()))?,
                    "owner_type": owner.owner_type,
                    "owner_name": owner.owner_name.clone(),
                    "wallet_status": wallet.status.clone(),
                    "user_id": row.try_get::<Option<String>, _>("user_id").map_err(|err| GatewayError::Internal(err.to_string()))?,
                    "payment_order_id": row.try_get::<Option<String>, _>("payment_order_id").map_err(|err| GatewayError::Internal(err.to_string()))?,
                    "source_type": row.try_get::<String, _>("source_type").map_err(|err| GatewayError::Internal(err.to_string()))?,
                    "source_id": row.try_get::<Option<String>, _>("source_id").map_err(|err| GatewayError::Internal(err.to_string()))?,
                    "refund_mode": row.try_get::<String, _>("refund_mode").map_err(|err| GatewayError::Internal(err.to_string()))?,
                    "amount_usd": row.try_get::<f64, _>("amount_usd").map_err(|err| GatewayError::Internal(err.to_string()))?,
                    "status": row.try_get::<String, _>("status").map_err(|err| GatewayError::Internal(err.to_string()))?,
                    "reason": row.try_get::<Option<String>, _>("reason").map_err(|err| GatewayError::Internal(err.to_string()))?,
                    "failure_reason": row.try_get::<Option<String>, _>("failure_reason").map_err(|err| GatewayError::Internal(err.to_string()))?,
                    "gateway_refund_id": row.try_get::<Option<String>, _>("gateway_refund_id").map_err(|err| GatewayError::Internal(err.to_string()))?,
                    "payout_method": row.try_get::<Option<String>, _>("payout_method").map_err(|err| GatewayError::Internal(err.to_string()))?,
                    "payout_reference": row.try_get::<Option<String>, _>("payout_reference").map_err(|err| GatewayError::Internal(err.to_string()))?,
                    "payout_proof": row.try_get::<Option<serde_json::Value>, _>("payout_proof").map_err(|err| GatewayError::Internal(err.to_string()))?,
                    "requested_by": row.try_get::<Option<String>, _>("requested_by").map_err(|err| GatewayError::Internal(err.to_string()))?,
                    "approved_by": row.try_get::<Option<String>, _>("approved_by").map_err(|err| GatewayError::Internal(err.to_string()))?,
                    "processed_by": row.try_get::<Option<String>, _>("processed_by").map_err(|err| GatewayError::Internal(err.to_string()))?,
                    "created_at": created_at,
                    "updated_at": updated_at,
                    "processed_at": processed_at,
                    "completed_at": completed_at,
                }))
            })
            .collect::<Result<Vec<_>, GatewayError>>()?;
    }

    Ok(Json(json!({
        "wallet": wallet_payload,
        "items": items,
        "total": total,
        "limit": limit,
        "offset": offset,
    }))
    .into_response())
}
