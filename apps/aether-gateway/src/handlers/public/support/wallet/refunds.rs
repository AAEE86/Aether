use super::{
    build_auth_error_response, build_auth_json_response, build_wallet_payload,
    build_wallet_refund_storage_unavailable_response, http, parse_wallet_limit,
    parse_wallet_offset, resolve_authenticated_local_user, unix_secs_to_rfc3339,
    wallet_normalize_optional_string_field, AppState, Body, GatewayError,
    GatewayPublicRequestContext, Response,
};
#[cfg(test)]
use super::{
    record_wallet_test_refund, wallet_test_refund_by_id, wallet_test_refund_by_idempotency,
    wallet_test_refunds_for_wallet, wallet_test_reserved_refund_amount,
};
use chrono::Utc;
use serde::Deserialize;
use serde_json::json;
use sqlx::Row;
use uuid::Uuid;

#[derive(Debug, Deserialize)]
struct WalletCreateRefundRequest {
    amount_usd: f64,
    #[serde(default)]
    payment_order_id: Option<String>,
    #[serde(default)]
    source_type: Option<String>,
    #[serde(default)]
    source_id: Option<String>,
    #[serde(default)]
    refund_mode: Option<String>,
    #[serde(default)]
    reason: Option<String>,
    #[serde(default)]
    idempotency_key: Option<String>,
}

#[derive(Debug, Clone)]
struct NormalizedWalletCreateRefundRequest {
    amount_usd: f64,
    payment_order_id: Option<String>,
    source_type: Option<String>,
    source_id: Option<String>,
    refund_mode: Option<String>,
    reason: Option<String>,
    idempotency_key: Option<String>,
}

fn normalize_wallet_create_refund_request(
    payload: WalletCreateRefundRequest,
) -> Result<NormalizedWalletCreateRefundRequest, &'static str> {
    if !payload.amount_usd.is_finite() || payload.amount_usd <= 0.0 {
        return Err("输入验证失败");
    }

    Ok(NormalizedWalletCreateRefundRequest {
        amount_usd: payload.amount_usd,
        payment_order_id: wallet_normalize_optional_string_field(payload.payment_order_id, 100)?,
        source_type: wallet_normalize_optional_string_field(payload.source_type, 30)?,
        source_id: wallet_normalize_optional_string_field(payload.source_id, 100)?,
        refund_mode: wallet_normalize_optional_string_field(payload.refund_mode, 30)?,
        reason: wallet_normalize_optional_string_field(payload.reason, 500)?,
        idempotency_key: wallet_normalize_optional_string_field(payload.idempotency_key, 128)?,
    })
}

fn wallet_default_refund_mode_for_payment_method(payment_method: &str) -> &'static str {
    if matches!(
        payment_method,
        "admin_manual" | "card_recharge" | "card_code" | "gift_code"
    ) {
        return "offline_payout";
    }
    "original_channel"
}

fn wallet_build_refund_no(now: chrono::DateTime<chrono::Utc>) -> String {
    format!(
        "rf_{}_{}",
        now.format("%Y%m%d%H%M%S%6f"),
        &Uuid::new_v4().simple().to_string()[..8]
    )
}

fn wallet_refund_id_from_path(request_path: &str) -> Option<String> {
    let trimmed = request_path.trim_end_matches('/');
    let refund_id = trimmed.strip_prefix("/api/wallet/refunds/")?.trim();
    if refund_id.is_empty() || refund_id.contains('/') {
        None
    } else {
        Some(refund_id.to_string())
    }
}

pub(super) fn wallet_refund_detail_path_matches(request_path: &str) -> bool {
    wallet_refund_id_from_path(request_path).is_some()
}

fn wallet_refund_payload_from_row(
    row: &sqlx::postgres::PgRow,
) -> Result<serde_json::Value, GatewayError> {
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
        "created_at": created_at,
        "updated_at": updated_at,
        "processed_at": processed_at,
        "completed_at": completed_at,
    }))
}

pub(super) async fn handle_wallet_refunds_list(
    state: &AppState,
    request_context: &GatewayPublicRequestContext,
    headers: &http::HeaderMap,
) -> Response<Body> {
    let auth = match resolve_authenticated_local_user(state, request_context, headers).await {
        Ok(value) => value,
        Err(response) => return response,
    };
    let query = request_context.request_query_string.as_deref();
    let limit = match parse_wallet_limit(query) {
        Ok(value) => value,
        Err(detail) => {
            return build_auth_error_response(http::StatusCode::BAD_REQUEST, detail, false)
        }
    };
    let offset = match parse_wallet_offset(query) {
        Ok(value) => value,
        Err(detail) => {
            return build_auth_error_response(http::StatusCode::BAD_REQUEST, detail, false)
        }
    };
    let wallet = match state
        .find_wallet(aether_data::repository::wallet::WalletLookupKey::UserId(
            &auth.user.id,
        ))
        .await
    {
        Ok(value) => value,
        Err(err) => {
            return build_auth_error_response(
                http::StatusCode::INTERNAL_SERVER_ERROR,
                format!("wallet lookup failed: {err:?}"),
                false,
            )
        }
    };
    let Some(wallet) = wallet else {
        let mut payload = json!({
            "items": [],
            "total": 0,
            "limit": limit,
            "offset": offset,
        });
        if let Some(object) = payload.as_object_mut() {
            if let Some(wallet_payload) = build_wallet_payload(None).as_object() {
                object.extend(wallet_payload.clone());
            }
        }
        return build_auth_json_response(http::StatusCode::OK, payload, None);
    };

    let mut total = 0_u64;
    let mut items = Vec::new();
    if let Some(pool) = state.postgres_pool() {
        let count_row = match sqlx::query(
            r#"
SELECT COUNT(*) AS total
FROM refund_requests
WHERE wallet_id = $1
            "#,
        )
        .bind(&wallet.id)
        .fetch_one(&pool)
        .await
        {
            Ok(value) => value,
            Err(err) => {
                return build_auth_error_response(
                    http::StatusCode::INTERNAL_SERVER_ERROR,
                    format!("wallet refund count failed: {err}"),
                    false,
                )
            }
        };
        total = count_row
            .try_get::<i64, _>("total")
            .ok()
            .unwrap_or_default()
            .max(0) as u64;
        let rows = match sqlx::query(
            r#"
SELECT
  id,
  refund_no,
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
        .bind(i64::try_from(offset).ok().unwrap_or_default())
        .bind(i64::try_from(limit).ok().unwrap_or_default())
        .fetch_all(&pool)
        .await
        {
            Ok(value) => value,
            Err(err) => {
                return build_auth_error_response(
                    http::StatusCode::INTERNAL_SERVER_ERROR,
                    format!("wallet refund query failed: {err}"),
                    false,
                )
            }
        };
        items = match rows
            .iter()
            .map(wallet_refund_payload_from_row)
            .collect::<Result<Vec<_>, GatewayError>>()
        {
            Ok(value) => value,
            Err(err) => {
                return build_auth_error_response(
                    http::StatusCode::INTERNAL_SERVER_ERROR,
                    format!("wallet refund payload failed: {err:?}"),
                    false,
                )
            }
        };
    }
    #[cfg(test)]
    if state.postgres_pool().is_none() {
        let all_items = wallet_test_refunds_for_wallet(&wallet.id);
        total = all_items.len() as u64;
        items = all_items
            .into_iter()
            .skip(offset)
            .take(limit)
            .collect::<Vec<_>>();
    }

    let mut payload = json!({
        "items": items,
        "total": total,
        "limit": limit,
        "offset": offset,
    });
    if let Some(object) = payload.as_object_mut() {
        if let Some(wallet_payload) = build_wallet_payload(Some(&wallet)).as_object() {
            object.extend(wallet_payload.clone());
        }
    }
    build_auth_json_response(http::StatusCode::OK, payload, None)
}

pub(super) async fn handle_wallet_refund_detail(
    state: &AppState,
    request_context: &GatewayPublicRequestContext,
    headers: &http::HeaderMap,
) -> Response<Body> {
    let auth = match resolve_authenticated_local_user(state, request_context, headers).await {
        Ok(value) => value,
        Err(response) => return response,
    };
    let Some(refund_id) = wallet_refund_id_from_path(&request_context.request_path) else {
        return build_auth_error_response(
            http::StatusCode::NOT_FOUND,
            "Refund request not found",
            false,
        );
    };
    let wallet = match state
        .find_wallet(aether_data::repository::wallet::WalletLookupKey::UserId(
            &auth.user.id,
        ))
        .await
    {
        Ok(value) => value,
        Err(err) => {
            return build_auth_error_response(
                http::StatusCode::INTERNAL_SERVER_ERROR,
                format!("wallet lookup failed: {err:?}"),
                false,
            )
        }
    };
    let Some(wallet) = wallet else {
        return build_auth_error_response(
            http::StatusCode::NOT_FOUND,
            "Refund request not found",
            false,
        );
    };
    let Some(pool) = state.postgres_pool() else {
        #[cfg(test)]
        if let Some(payload) = wallet_test_refund_by_id(&wallet.id, &refund_id) {
            return build_auth_json_response(http::StatusCode::OK, payload, None);
        }
        return build_auth_error_response(
            http::StatusCode::NOT_FOUND,
            "Refund request not found",
            false,
        );
    };
    let row = match sqlx::query(
        r#"
SELECT
  id,
  refund_no,
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
  CAST(EXTRACT(EPOCH FROM created_at) AS BIGINT) AS created_at_unix_secs,
  CAST(EXTRACT(EPOCH FROM updated_at) AS BIGINT) AS updated_at_unix_secs,
  CAST(EXTRACT(EPOCH FROM processed_at) AS BIGINT) AS processed_at_unix_secs,
  CAST(EXTRACT(EPOCH FROM completed_at) AS BIGINT) AS completed_at_unix_secs
FROM refund_requests
WHERE wallet_id = $1 AND id = $2
LIMIT 1
        "#,
    )
    .bind(&wallet.id)
    .bind(&refund_id)
    .fetch_optional(&pool)
    .await
    {
        Ok(value) => value,
        Err(err) => {
            return build_auth_error_response(
                http::StatusCode::INTERNAL_SERVER_ERROR,
                format!("wallet refund detail query failed: {err}"),
                false,
            )
        }
    };
    let Some(row) = row else {
        return build_auth_error_response(
            http::StatusCode::NOT_FOUND,
            "Refund request not found",
            false,
        );
    };
    let payload = match wallet_refund_payload_from_row(&row) {
        Ok(value) => value,
        Err(err) => {
            return build_auth_error_response(
                http::StatusCode::INTERNAL_SERVER_ERROR,
                format!("wallet refund detail payload failed: {err:?}"),
                false,
            )
        }
    };
    build_auth_json_response(http::StatusCode::OK, payload, None)
}

pub(super) async fn handle_wallet_create_refund(
    state: &AppState,
    request_context: &GatewayPublicRequestContext,
    headers: &http::HeaderMap,
    request_body: Option<&axum::body::Bytes>,
) -> Response<Body> {
    let auth = match resolve_authenticated_local_user(state, request_context, headers).await {
        Ok(value) => value,
        Err(response) => return response,
    };
    let Some(request_body) = request_body else {
        return build_auth_error_response(http::StatusCode::BAD_REQUEST, "缺少请求体", false);
    };
    let payload = match serde_json::from_slice::<WalletCreateRefundRequest>(request_body) {
        Ok(value) => value,
        Err(_) => {
            return build_auth_error_response(http::StatusCode::BAD_REQUEST, "输入验证失败", false)
        }
    };
    let payload = match normalize_wallet_create_refund_request(payload) {
        Ok(value) => value,
        Err(detail) => {
            return build_auth_error_response(http::StatusCode::BAD_REQUEST, detail, false);
        }
    };

    let wallet = match state
        .find_wallet(aether_data::repository::wallet::WalletLookupKey::UserId(
            &auth.user.id,
        ))
        .await
    {
        Ok(value) => value,
        Err(err) => {
            return build_auth_error_response(
                http::StatusCode::INTERNAL_SERVER_ERROR,
                format!("wallet lookup failed: {err:?}"),
                false,
            )
        }
    };
    let Some(wallet) = wallet else {
        return build_auth_error_response(
            http::StatusCode::BAD_REQUEST,
            "当前账户尚未开通钱包，无法申请退款",
            false,
        );
    };

    let Some(pool) = state.postgres_pool() else {
        #[cfg(test)]
        {
            if let Some(idempotency_key) = payload.idempotency_key.as_deref() {
                if let Some(existing) =
                    wallet_test_refund_by_idempotency(&auth.user.id, idempotency_key)
                {
                    return build_auth_json_response(http::StatusCode::OK, existing, None);
                }
            }
            let reserved_amount = wallet_test_reserved_refund_amount(&wallet.id);
            if payload.amount_usd > (wallet.balance - reserved_amount) {
                return build_auth_error_response(
                    http::StatusCode::BAD_REQUEST,
                    "refund amount exceeds available refundable recharge balance",
                    false,
                );
            }
            let now = Utc::now();
            let created = json!({
                "id": Uuid::new_v4().to_string(),
                "refund_no": wallet_build_refund_no(now),
                "payment_order_id": serde_json::Value::Null,
                "source_type": payload.source_type.as_deref().unwrap_or("wallet_balance"),
                "source_id": payload.source_id,
                "refund_mode": payload.refund_mode.as_deref().unwrap_or("offline_payout"),
                "amount_usd": payload.amount_usd,
                "status": "pending_approval",
                "reason": payload.reason,
                "failure_reason": serde_json::Value::Null,
                "gateway_refund_id": serde_json::Value::Null,
                "payout_method": serde_json::Value::Null,
                "payout_reference": serde_json::Value::Null,
                "payout_proof": serde_json::Value::Null,
                "created_at": now.to_rfc3339(),
                "updated_at": now.to_rfc3339(),
                "processed_at": serde_json::Value::Null,
                "completed_at": serde_json::Value::Null,
            });
            record_wallet_test_refund(
                wallet.id,
                auth.user.id,
                payload.idempotency_key,
                created.clone(),
            );
            return build_auth_json_response(http::StatusCode::OK, created, None);
        }
        #[cfg(not(test))]
        return build_wallet_refund_storage_unavailable_response();
    };

    let mut tx = match pool.begin().await {
        Ok(value) => value,
        Err(err) => {
            return build_auth_error_response(
                http::StatusCode::INTERNAL_SERVER_ERROR,
                format!("wallet refund transaction failed: {err}"),
                false,
            )
        }
    };

    let locked_wallet_row = match sqlx::query(
        r#"
SELECT
  id,
  CAST(balance AS DOUBLE PRECISION) AS balance
FROM wallets
WHERE id = $1
LIMIT 1
FOR UPDATE
        "#,
    )
    .bind(&wallet.id)
    .fetch_optional(&mut *tx)
    .await
    {
        Ok(value) => value,
        Err(err) => {
            let _ = tx.rollback().await;
            return build_auth_error_response(
                http::StatusCode::INTERNAL_SERVER_ERROR,
                format!("wallet refund wallet lock failed: {err}"),
                false,
            );
        }
    };
    let Some(locked_wallet_row) = locked_wallet_row else {
        let _ = tx.rollback().await;
        return build_auth_error_response(
            http::StatusCode::BAD_REQUEST,
            "当前账户尚未开通钱包，无法申请退款",
            false,
        );
    };
    let wallet_recharge_balance = locked_wallet_row
        .try_get::<f64, _>("balance")
        .ok()
        .unwrap_or_default();
    let wallet_reserved_row = match sqlx::query(
        r#"
SELECT COALESCE(CAST(SUM(amount_usd) AS DOUBLE PRECISION), 0) AS total
FROM refund_requests
WHERE wallet_id = $1
  AND status IN ('pending_approval', 'approved')
        "#,
    )
    .bind(&wallet.id)
    .fetch_one(&mut *tx)
    .await
    {
        Ok(value) => value,
        Err(err) => {
            let _ = tx.rollback().await;
            return build_auth_error_response(
                http::StatusCode::INTERNAL_SERVER_ERROR,
                format!("wallet refund reserved amount lookup failed: {err}"),
                false,
            );
        }
    };
    let wallet_reserved_amount = wallet_reserved_row
        .try_get::<f64, _>("total")
        .ok()
        .unwrap_or_default();
    if payload.amount_usd > (wallet_recharge_balance - wallet_reserved_amount) {
        let _ = tx.rollback().await;
        return build_auth_error_response(
            http::StatusCode::BAD_REQUEST,
            "refund amount exceeds available refundable recharge balance",
            false,
        );
    }

    let mut payment_order_id = None;
    let mut source_type = payload
        .source_type
        .clone()
        .unwrap_or_else(|| "wallet_balance".to_string());
    let mut source_id = payload.source_id.clone();
    let mut refund_mode = payload
        .refund_mode
        .clone()
        .unwrap_or_else(|| "offline_payout".to_string());
    if let Some(order_id) = payload.payment_order_id.as_deref() {
        let order_row = match sqlx::query(
            r#"
SELECT
  id,
  wallet_id,
  status,
  payment_method,
  CAST(refundable_amount_usd AS DOUBLE PRECISION) AS refundable_amount_usd
FROM payment_orders
WHERE id = $1
  AND wallet_id = $2
LIMIT 1
FOR UPDATE
            "#,
        )
        .bind(order_id)
        .bind(&wallet.id)
        .fetch_optional(&mut *tx)
        .await
        {
            Ok(value) => value,
            Err(err) => {
                let _ = tx.rollback().await;
                return build_auth_error_response(
                    http::StatusCode::INTERNAL_SERVER_ERROR,
                    format!("wallet refund payment order lookup failed: {err}"),
                    false,
                );
            }
        };
        let Some(order_row) = order_row else {
            let _ = tx.rollback().await;
            return build_auth_error_response(
                http::StatusCode::NOT_FOUND,
                "Payment order not found",
                false,
            );
        };
        let status = order_row
            .try_get::<String, _>("status")
            .ok()
            .unwrap_or_default();
        if status != "credited" {
            let _ = tx.rollback().await;
            return build_auth_error_response(
                http::StatusCode::BAD_REQUEST,
                "payment order is not refundable",
                false,
            );
        }

        let order_reserved_row = match sqlx::query(
            r#"
SELECT COALESCE(CAST(SUM(amount_usd) AS DOUBLE PRECISION), 0) AS total
FROM refund_requests
WHERE payment_order_id = $1
  AND status IN ('pending_approval', 'approved')
            "#,
        )
        .bind(order_id)
        .fetch_one(&mut *tx)
        .await
        {
            Ok(value) => value,
            Err(err) => {
                let _ = tx.rollback().await;
                return build_auth_error_response(
                    http::StatusCode::INTERNAL_SERVER_ERROR,
                    format!("wallet refund payment order reserve lookup failed: {err}"),
                    false,
                );
            }
        };
        let refundable_amount = order_row
            .try_get::<f64, _>("refundable_amount_usd")
            .ok()
            .unwrap_or_default();
        let reserved_amount = order_reserved_row
            .try_get::<f64, _>("total")
            .ok()
            .unwrap_or_default();
        if payload.amount_usd > (refundable_amount - reserved_amount) {
            let _ = tx.rollback().await;
            return build_auth_error_response(
                http::StatusCode::BAD_REQUEST,
                "refund amount exceeds available refundable amount",
                false,
            );
        }

        payment_order_id = Some(order_id.to_string());
        source_type = "payment_order".to_string();
        source_id = Some(order_id.to_string());
        if payload.refund_mode.is_none() {
            let payment_method = order_row
                .try_get::<String, _>("payment_method")
                .ok()
                .unwrap_or_default();
            refund_mode =
                wallet_default_refund_mode_for_payment_method(&payment_method).to_string();
        }
    }

    let now = Utc::now();
    let refund_id = Uuid::new_v4().to_string();
    let refund_no = wallet_build_refund_no(now);
    let insert_result = sqlx::query(
        r#"
INSERT INTO refund_requests (
  id,
  refund_no,
  wallet_id,
  user_id,
  payment_order_id,
  source_type,
  source_id,
  refund_mode,
  amount_usd,
  status,
  reason,
  requested_by,
  idempotency_key,
  created_at,
  updated_at
)
VALUES (
  $1,
  $2,
  $3,
  $4,
  $5,
  $6,
  $7,
  $8,
  $9,
  'pending_approval',
  $10,
  $11,
  $12,
  NOW(),
  NOW()
)
RETURNING
  id,
  refund_no,
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
  CAST(EXTRACT(EPOCH FROM created_at) AS BIGINT) AS created_at_unix_secs,
  CAST(EXTRACT(EPOCH FROM updated_at) AS BIGINT) AS updated_at_unix_secs,
  CAST(EXTRACT(EPOCH FROM processed_at) AS BIGINT) AS processed_at_unix_secs,
  CAST(EXTRACT(EPOCH FROM completed_at) AS BIGINT) AS completed_at_unix_secs
        "#,
    )
    .bind(&refund_id)
    .bind(&refund_no)
    .bind(&wallet.id)
    .bind(&auth.user.id)
    .bind(payment_order_id.as_deref())
    .bind(&source_type)
    .bind(source_id.as_deref())
    .bind(&refund_mode)
    .bind(payload.amount_usd)
    .bind(payload.reason.as_deref())
    .bind(&auth.user.id)
    .bind(payload.idempotency_key.as_deref())
    .fetch_one(&mut *tx)
    .await;

    let row = match insert_result {
        Ok(value) => value,
        Err(err) => {
            let _ = tx.rollback().await;
            if let Some(idempotency_key) = payload.idempotency_key.as_deref() {
                let existing = match sqlx::query(
                    r#"
SELECT
  id,
  refund_no,
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
  CAST(EXTRACT(EPOCH FROM created_at) AS BIGINT) AS created_at_unix_secs,
  CAST(EXTRACT(EPOCH FROM updated_at) AS BIGINT) AS updated_at_unix_secs,
  CAST(EXTRACT(EPOCH FROM processed_at) AS BIGINT) AS processed_at_unix_secs,
  CAST(EXTRACT(EPOCH FROM completed_at) AS BIGINT) AS completed_at_unix_secs
FROM refund_requests
WHERE user_id = $1
  AND idempotency_key = $2
LIMIT 1
                    "#,
                )
                .bind(&auth.user.id)
                .bind(idempotency_key)
                .fetch_optional(&pool)
                .await
                {
                    Ok(value) => value,
                    Err(read_err) => {
                        return build_auth_error_response(
                            http::StatusCode::INTERNAL_SERVER_ERROR,
                            format!("wallet refund idempotency lookup failed: {read_err}"),
                            false,
                        );
                    }
                };
                if let Some(existing) = existing {
                    let payload = match wallet_refund_payload_from_row(&existing) {
                        Ok(value) => value,
                        Err(payload_err) => {
                            return build_auth_error_response(
                                http::StatusCode::INTERNAL_SERVER_ERROR,
                                format!("wallet refund payload failed: {payload_err:?}"),
                                false,
                            );
                        }
                    };
                    return build_auth_json_response(http::StatusCode::OK, payload, None);
                }
            }
            if err
                .as_database_error()
                .and_then(|value| value.code())
                .as_deref()
                == Some("23505")
            {
                return build_auth_error_response(
                    http::StatusCode::BAD_REQUEST,
                    "退款申请重复，请勿重复提交",
                    false,
                );
            }
            return build_auth_error_response(
                http::StatusCode::INTERNAL_SERVER_ERROR,
                format!("wallet refund create failed: {err}"),
                false,
            );
        }
    };

    if let Err(err) = tx.commit().await {
        return build_auth_error_response(
            http::StatusCode::INTERNAL_SERVER_ERROR,
            format!("wallet refund commit failed: {err}"),
            false,
        );
    }

    let payload = match wallet_refund_payload_from_row(&row) {
        Ok(value) => value,
        Err(err) => {
            return build_auth_error_response(
                http::StatusCode::INTERNAL_SERVER_ERROR,
                format!("wallet refund payload failed: {err:?}"),
                false,
            );
        }
    };
    build_auth_json_response(http::StatusCode::OK, payload, None)
}
