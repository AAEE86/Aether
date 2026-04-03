use super::payment_shared::{
    payment_callback_mark_failed_response, payment_callback_payload_hash,
    NormalizedPaymentCallbackRequest,
};
use axum::{body::Body, http, response::Response};
use chrono::Utc;
use serde_json::json;
use sqlx::Row;
use uuid::Uuid;

use super::super::{build_auth_json_response, wallet_payment_order_payload_from_row};
use super::{
    build_auth_error_response, build_payment_callback_storage_unavailable_response, AppState,
    GatewayPublicRequestContext,
};

async fn update_payment_callback_failure(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    callback_id: &str,
    payload: &NormalizedPaymentCallbackRequest,
    callback_payload_hash: &str,
    signature_valid: bool,
    error: &str,
) {
    let _ = sqlx::query(
        r#"
UPDATE payment_callbacks
SET signature_valid = $2,
    status = 'failed',
    error_message = $3,
    payload_hash = $4,
    payload = $5,
    processed_at = NOW(),
    order_no = COALESCE($6, order_no),
    gateway_order_id = COALESCE($7, gateway_order_id)
WHERE id = $1
        "#,
    )
    .bind(callback_id)
    .bind(signature_valid)
    .bind(error)
    .bind(callback_payload_hash)
    .bind(&payload.payload)
    .bind(payload.order_no.as_deref())
    .bind(payload.gateway_order_id.as_deref())
    .execute(&mut **tx)
    .await;
}

pub(super) async fn handle_payment_callback_with_postgres(
    state: &AppState,
    payment_method: &str,
    request_context: &GatewayPublicRequestContext,
    payload: &NormalizedPaymentCallbackRequest,
    signature_valid: bool,
) -> Response<Body> {
    let Some(pool) = state.postgres_pool() else {
        return build_payment_callback_storage_unavailable_response();
    };
    let mut tx = match pool.begin().await {
        Ok(value) => value,
        Err(err) => {
            return build_auth_error_response(
                http::StatusCode::INTERNAL_SERVER_ERROR,
                format!("payment callback transaction failed: {err}"),
                false,
            );
        }
    };

    let existing_callback = match sqlx::query(
        r#"
SELECT id, payment_order_id, status, order_no, gateway_order_id
FROM payment_callbacks
WHERE callback_key = $1
LIMIT 1
        "#,
    )
    .bind(&payload.callback_key)
    .fetch_optional(&mut *tx)
    .await
    {
        Ok(value) => value,
        Err(err) => {
            let _ = tx.rollback().await;
            return build_auth_error_response(
                http::StatusCode::INTERNAL_SERVER_ERROR,
                format!("payment callback lookup failed: {err}"),
                false,
            );
        }
    };

    let callback_payload_hash = match payment_callback_payload_hash(&payload.payload) {
        Ok(value) => value,
        Err(err) => {
            let _ = tx.rollback().await;
            return build_auth_error_response(http::StatusCode::INTERNAL_SERVER_ERROR, err, false);
        }
    };

    let duplicate = existing_callback.is_some();
    let callback_id = if let Some(row) = existing_callback.as_ref() {
        let status = row.try_get::<String, _>("status").ok().unwrap_or_default();
        if status == "processed" {
            let order_id = row
                .try_get::<Option<String>, _>("payment_order_id")
                .ok()
                .flatten();
            let _ = tx.rollback().await;
            return build_auth_json_response(
                http::StatusCode::OK,
                json!({
                    "ok": true,
                    "duplicate": true,
                    "credited": false,
                    "order_id": order_id,
                    "payment_method": payment_method,
                    "request_path": request_context.request_path,
                }),
                None,
            );
        }
        row.try_get::<String, _>("id").ok().unwrap_or_default()
    } else {
        let callback_id = Uuid::new_v4().to_string();
        let insert_result = sqlx::query(
            r#"
INSERT INTO payment_callbacks (
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
  created_at,
  processed_at
)
VALUES (
  $1,
  NULL,
  $2,
  $3,
  $4,
  $5,
  $6,
  $7,
  'received',
  $8,
  NULL,
  NOW(),
  NULL
)
            "#,
        )
        .bind(&callback_id)
        .bind(payment_method)
        .bind(&payload.callback_key)
        .bind(payload.order_no.as_deref())
        .bind(payload.gateway_order_id.as_deref())
        .bind(&callback_payload_hash)
        .bind(signature_valid)
        .bind(&payload.payload)
        .execute(&mut *tx)
        .await;
        if let Err(err) = insert_result {
            let _ = tx.rollback().await;
            return build_auth_error_response(
                http::StatusCode::INTERNAL_SERVER_ERROR,
                format!("payment callback create failed: {err}"),
                false,
            );
        }
        callback_id
    };

    if !signature_valid {
        update_payment_callback_failure(
            &mut tx,
            &callback_id,
            payload,
            &callback_payload_hash,
            signature_valid,
            "invalid callback signature",
        )
        .await;
        let _ = tx.commit().await;
        return payment_callback_mark_failed_response(
            duplicate,
            "invalid callback signature",
            payment_method,
            &request_context.request_path,
        );
    }

    let lookup_order_no = payload.order_no.clone().or_else(|| {
        existing_callback
            .as_ref()
            .and_then(|row| row.try_get::<Option<String>, _>("order_no").ok().flatten())
    });
    let lookup_gateway_order_id = payload.gateway_order_id.clone().or_else(|| {
        existing_callback.as_ref().and_then(|row| {
            row.try_get::<Option<String>, _>("gateway_order_id")
                .ok()
                .flatten()
        })
    });

    let order_row = if let Some(order_no) = lookup_order_no.as_deref() {
        match sqlx::query(
            r#"
SELECT
  id,
  order_no,
  wallet_id,
  user_id,
  CAST(amount_usd AS DOUBLE PRECISION) AS amount_usd,
  CAST(pay_amount AS DOUBLE PRECISION) AS pay_amount,
  pay_currency,
  CAST(exchange_rate AS DOUBLE PRECISION) AS exchange_rate,
  CAST(refunded_amount_usd AS DOUBLE PRECISION) AS refunded_amount_usd,
  CAST(refundable_amount_usd AS DOUBLE PRECISION) AS refundable_amount_usd,
  payment_method,
  gateway_order_id,
  gateway_response,
  status AS effective_status,
  CAST(EXTRACT(EPOCH FROM created_at) AS BIGINT) AS created_at_unix_secs,
  CAST(EXTRACT(EPOCH FROM paid_at) AS BIGINT) AS paid_at_unix_secs,
  CAST(EXTRACT(EPOCH FROM credited_at) AS BIGINT) AS credited_at_unix_secs,
  CAST(EXTRACT(EPOCH FROM expires_at) AS BIGINT) AS expires_at_unix_secs
FROM payment_orders
WHERE order_no = $1
LIMIT 1
FOR UPDATE
            "#,
        )
        .bind(order_no)
        .fetch_optional(&mut *tx)
        .await
        {
            Ok(value) => value,
            Err(err) => {
                let _ = tx.rollback().await;
                return build_auth_error_response(
                    http::StatusCode::INTERNAL_SERVER_ERROR,
                    format!("payment callback order lookup failed: {err}"),
                    false,
                );
            }
        }
    } else if let Some(gateway_order_id) = lookup_gateway_order_id.as_deref() {
        match sqlx::query(
            r#"
SELECT
  id,
  order_no,
  wallet_id,
  user_id,
  CAST(amount_usd AS DOUBLE PRECISION) AS amount_usd,
  CAST(pay_amount AS DOUBLE PRECISION) AS pay_amount,
  pay_currency,
  CAST(exchange_rate AS DOUBLE PRECISION) AS exchange_rate,
  CAST(refunded_amount_usd AS DOUBLE PRECISION) AS refunded_amount_usd,
  CAST(refundable_amount_usd AS DOUBLE PRECISION) AS refundable_amount_usd,
  payment_method,
  gateway_order_id,
  gateway_response,
  status AS effective_status,
  CAST(EXTRACT(EPOCH FROM created_at) AS BIGINT) AS created_at_unix_secs,
  CAST(EXTRACT(EPOCH FROM paid_at) AS BIGINT) AS paid_at_unix_secs,
  CAST(EXTRACT(EPOCH FROM credited_at) AS BIGINT) AS credited_at_unix_secs,
  CAST(EXTRACT(EPOCH FROM expires_at) AS BIGINT) AS expires_at_unix_secs
FROM payment_orders
WHERE gateway_order_id = $1
LIMIT 1
FOR UPDATE
            "#,
        )
        .bind(gateway_order_id)
        .fetch_optional(&mut *tx)
        .await
        {
            Ok(value) => value,
            Err(err) => {
                let _ = tx.rollback().await;
                return build_auth_error_response(
                    http::StatusCode::INTERNAL_SERVER_ERROR,
                    format!("payment callback order lookup failed: {err}"),
                    false,
                );
            }
        }
    } else {
        None
    };

    let Some(order_row) = order_row else {
        update_payment_callback_failure(
            &mut tx,
            &callback_id,
            payload,
            &callback_payload_hash,
            signature_valid,
            "payment order not found",
        )
        .await;
        let _ = tx.commit().await;
        return payment_callback_mark_failed_response(
            duplicate,
            "payment order not found",
            payment_method,
            &request_context.request_path,
        );
    };

    let order_id = order_row
        .try_get::<String, _>("id")
        .ok()
        .unwrap_or_default();
    let order_no = order_row
        .try_get::<String, _>("order_no")
        .ok()
        .unwrap_or_default();
    let order_wallet_id = order_row
        .try_get::<String, _>("wallet_id")
        .ok()
        .unwrap_or_default();
    let order_payment_method = order_row
        .try_get::<String, _>("payment_method")
        .ok()
        .unwrap_or_default();
    let order_amount_usd = order_row
        .try_get::<f64, _>("amount_usd")
        .ok()
        .unwrap_or_default();
    let order_status = order_row
        .try_get::<String, _>("effective_status")
        .ok()
        .unwrap_or_default();
    let expires_at_unix_secs = order_row
        .try_get::<Option<i64>, _>("expires_at_unix_secs")
        .ok()
        .flatten();
    if (payload.amount_usd - order_amount_usd).abs() > f64::EPSILON {
        update_payment_callback_failure(
            &mut tx,
            &callback_id,
            payload,
            &callback_payload_hash,
            signature_valid,
            "callback amount mismatch",
        )
        .await;
        let _ = tx.commit().await;
        return payment_callback_mark_failed_response(
            duplicate,
            "callback amount mismatch",
            payment_method,
            &request_context.request_path,
        );
    }
    if !order_payment_method.eq_ignore_ascii_case(payment_method) {
        update_payment_callback_failure(
            &mut tx,
            &callback_id,
            payload,
            &callback_payload_hash,
            signature_valid,
            "payment method mismatch",
        )
        .await;
        let _ = tx.commit().await;
        return payment_callback_mark_failed_response(
            duplicate,
            "payment method mismatch",
            payment_method,
            &request_context.request_path,
        );
    }
    if order_status == "credited" {
        let _ = sqlx::query(
            r#"
UPDATE payment_callbacks
SET payment_order_id = $2,
    signature_valid = true,
    status = 'processed',
    error_message = NULL,
    payload_hash = $3,
    payload = $4,
    processed_at = NOW(),
    order_no = $5,
    gateway_order_id = COALESCE($6, gateway_order_id)
WHERE id = $1
            "#,
        )
        .bind(&callback_id)
        .bind(&order_id)
        .bind(&callback_payload_hash)
        .bind(&payload.payload)
        .bind(&order_no)
        .bind(payload.gateway_order_id.as_deref())
        .execute(&mut *tx)
        .await;
        let _ = tx.commit().await;
        return build_auth_json_response(
            http::StatusCode::OK,
            json!({
                "ok": true,
                "duplicate": duplicate,
                "credited": false,
                "order_id": order_id,
                "order_no": order_no,
                "status": "credited",
                "wallet_id": order_wallet_id,
                "payment_method": payment_method,
                "request_path": request_context.request_path,
            }),
            None,
        );
    }
    if matches!(order_status.as_str(), "failed" | "expired" | "refunded") {
        let error = format!("payment order is not creditable: {order_status}");
        update_payment_callback_failure(
            &mut tx,
            &callback_id,
            payload,
            &callback_payload_hash,
            signature_valid,
            &error,
        )
        .await;
        let _ = tx.commit().await;
        return payment_callback_mark_failed_response(
            duplicate,
            &error,
            payment_method,
            &request_context.request_path,
        );
    }
    if order_status == "pending" {
        let now = Utc::now().timestamp();
        if expires_at_unix_secs.is_some_and(|value| value < now) {
            let _ = sqlx::query("UPDATE payment_orders SET status = 'expired' WHERE id = $1")
                .bind(&order_id)
                .execute(&mut *tx)
                .await;
            update_payment_callback_failure(
                &mut tx,
                &callback_id,
                payload,
                &callback_payload_hash,
                signature_valid,
                "payment order expired",
            )
            .await;
            let _ = tx.commit().await;
            return payment_callback_mark_failed_response(
                duplicate,
                "payment order expired",
                payment_method,
                &request_context.request_path,
            );
        }
    }

    let wallet_row = match sqlx::query(
        r#"
SELECT
  id,
  status,
  CAST(balance AS DOUBLE PRECISION) AS balance,
  CAST(gift_balance AS DOUBLE PRECISION) AS gift_balance,
  CAST(total_recharged AS DOUBLE PRECISION) AS total_recharged
FROM wallets
WHERE id = $1
LIMIT 1
FOR UPDATE
        "#,
    )
    .bind(&order_wallet_id)
    .fetch_optional(&mut *tx)
    .await
    {
        Ok(value) => value,
        Err(err) => {
            let _ = tx.rollback().await;
            return build_auth_error_response(
                http::StatusCode::INTERNAL_SERVER_ERROR,
                format!("payment callback wallet lookup failed: {err}"),
                false,
            );
        }
    };
    let Some(wallet_row) = wallet_row else {
        update_payment_callback_failure(
            &mut tx,
            &callback_id,
            payload,
            &callback_payload_hash,
            signature_valid,
            "wallet not found",
        )
        .await;
        let _ = tx.commit().await;
        return payment_callback_mark_failed_response(
            duplicate,
            "wallet not found",
            payment_method,
            &request_context.request_path,
        );
    };
    let wallet_status = wallet_row
        .try_get::<String, _>("status")
        .ok()
        .unwrap_or_default();
    if wallet_status != "active" {
        update_payment_callback_failure(
            &mut tx,
            &callback_id,
            payload,
            &callback_payload_hash,
            signature_valid,
            "wallet is not active",
        )
        .await;
        let _ = tx.commit().await;
        return payment_callback_mark_failed_response(
            duplicate,
            "wallet is not active",
            payment_method,
            &request_context.request_path,
        );
    }

    let before_recharge = wallet_row
        .try_get::<f64, _>("balance")
        .ok()
        .unwrap_or_default();
    let before_gift = wallet_row
        .try_get::<f64, _>("gift_balance")
        .ok()
        .unwrap_or_default();
    let before_total = before_recharge + before_gift;
    let after_recharge = before_recharge + order_amount_usd;
    let after_total = after_recharge + before_gift;
    let wallet_tx_id = Uuid::new_v4().to_string();

    if let Err(err) = sqlx::query(
        r#"
UPDATE wallets
SET balance = $2,
    total_recharged = total_recharged + $3,
    updated_at = NOW()
WHERE id = $1
        "#,
    )
    .bind(&order_wallet_id)
    .bind(after_recharge)
    .bind(order_amount_usd)
    .execute(&mut *tx)
    .await
    {
        let _ = tx.rollback().await;
        return build_auth_error_response(
            http::StatusCode::INTERNAL_SERVER_ERROR,
            format!("payment callback wallet update failed: {err}"),
            false,
        );
    }

    if let Err(err) = sqlx::query(
        r#"
INSERT INTO wallet_transactions (
  id,
  wallet_id,
  category,
  reason_code,
  amount,
  balance_before,
  balance_after,
  recharge_balance_before,
  recharge_balance_after,
  gift_balance_before,
  gift_balance_after,
  link_type,
  link_id,
  operator_id,
  description,
  created_at
)
VALUES (
  $1,
  $2,
  'recharge',
  'topup_gateway',
  $3,
  $4,
  $5,
  $6,
  $7,
  $8,
  $9,
  'payment_order',
  $10,
  NULL,
  $11,
  NOW()
)
        "#,
    )
    .bind(&wallet_tx_id)
    .bind(&order_wallet_id)
    .bind(order_amount_usd)
    .bind(before_total)
    .bind(after_total)
    .bind(before_recharge)
    .bind(after_recharge)
    .bind(before_gift)
    .bind(before_gift)
    .bind(&order_id)
    .bind(format!("充值到账({payment_method})"))
    .execute(&mut *tx)
    .await
    {
        let _ = tx.rollback().await;
        return build_auth_error_response(
            http::StatusCode::INTERNAL_SERVER_ERROR,
            format!("payment callback wallet transaction failed: {err}"),
            false,
        );
    }

    let updated_order_row = match sqlx::query(
        r#"
UPDATE payment_orders
SET gateway_order_id = COALESCE($2, gateway_order_id),
    gateway_response = $3,
    pay_amount = COALESCE($4, pay_amount),
    pay_currency = COALESCE($5, pay_currency),
    exchange_rate = COALESCE($6, exchange_rate),
    status = 'credited',
    paid_at = COALESCE(paid_at, NOW()),
    credited_at = NOW(),
    refundable_amount_usd = amount_usd
WHERE id = $1
RETURNING
  id,
  order_no,
  wallet_id,
  user_id,
  CAST(amount_usd AS DOUBLE PRECISION) AS amount_usd,
  CAST(pay_amount AS DOUBLE PRECISION) AS pay_amount,
  pay_currency,
  CAST(exchange_rate AS DOUBLE PRECISION) AS exchange_rate,
  CAST(refunded_amount_usd AS DOUBLE PRECISION) AS refunded_amount_usd,
  CAST(refundable_amount_usd AS DOUBLE PRECISION) AS refundable_amount_usd,
  payment_method,
  gateway_order_id,
  gateway_response,
  status AS effective_status,
  CAST(EXTRACT(EPOCH FROM created_at) AS BIGINT) AS created_at_unix_secs,
  CAST(EXTRACT(EPOCH FROM paid_at) AS BIGINT) AS paid_at_unix_secs,
  CAST(EXTRACT(EPOCH FROM credited_at) AS BIGINT) AS credited_at_unix_secs,
  CAST(EXTRACT(EPOCH FROM expires_at) AS BIGINT) AS expires_at_unix_secs
        "#,
    )
    .bind(&order_id)
    .bind(payload.gateway_order_id.as_deref())
    .bind(&payload.payload)
    .bind(payload.pay_amount)
    .bind(payload.pay_currency.as_deref())
    .bind(payload.exchange_rate)
    .fetch_one(&mut *tx)
    .await
    {
        Ok(value) => value,
        Err(err) => {
            let _ = tx.rollback().await;
            return build_auth_error_response(
                http::StatusCode::INTERNAL_SERVER_ERROR,
                format!("payment callback order update failed: {err}"),
                false,
            );
        }
    };

    if let Err(err) = sqlx::query(
        r#"
UPDATE payment_callbacks
SET payment_order_id = $2,
    signature_valid = true,
    status = 'processed',
    error_message = NULL,
    payload_hash = $3,
    payload = $4,
    processed_at = NOW(),
    order_no = $5,
    gateway_order_id = COALESCE($6, gateway_order_id)
WHERE id = $1
        "#,
    )
    .bind(&callback_id)
    .bind(&order_id)
    .bind(&callback_payload_hash)
    .bind(&payload.payload)
    .bind(&order_no)
    .bind(payload.gateway_order_id.as_deref())
    .execute(&mut *tx)
    .await
    {
        let _ = tx.rollback().await;
        return build_auth_error_response(
            http::StatusCode::INTERNAL_SERVER_ERROR,
            format!("payment callback update failed: {err}"),
            false,
        );
    }

    if let Err(err) = tx.commit().await {
        return build_auth_error_response(
            http::StatusCode::INTERNAL_SERVER_ERROR,
            format!("payment callback commit failed: {err}"),
            false,
        );
    }

    let updated_order_payload = match wallet_payment_order_payload_from_row(&updated_order_row) {
        Ok(value) => value,
        Err(err) => {
            return build_auth_error_response(
                http::StatusCode::INTERNAL_SERVER_ERROR,
                format!("payment callback order payload failed: {err:?}"),
                false,
            );
        }
    };
    build_auth_json_response(
        http::StatusCode::OK,
        json!({
            "ok": true,
            "duplicate": duplicate,
            "credited": true,
            "order_id": order_id,
            "order_no": order_no,
            "status": updated_order_payload["status"],
            "wallet_id": order_wallet_id,
            "payment_method": payment_method,
            "request_path": request_context.request_path,
        }),
        None,
    )
}

#[cfg(test)]
mod tests {
    use super::{handle_payment_callback_with_postgres, AppState, NormalizedPaymentCallbackRequest};
    use crate::gateway::handlers::public::support::support_payment::PAYMENT_CALLBACK_STORAGE_UNAVAILABLE_DETAIL;
    use crate::gateway::GatewayPublicRequestContext;
    use axum::body::to_bytes;
    use axum::http::{HeaderMap, Method, Uri};
    use serde_json::json;

    #[tokio::test]
    async fn payment_callback_postgres_handler_returns_explicit_503_without_pool() {
        let state = AppState::new().expect("state should build");
        let request_context = GatewayPublicRequestContext::from_request_parts(
            "trace-payment-callback-postgres-missing",
            &Method::POST,
            &"/api/payment/callback/alipay"
                .parse::<Uri>()
                .expect("uri should parse"),
            &HeaderMap::new(),
            None,
        );
        let payload = NormalizedPaymentCallbackRequest {
            callback_key: "callback-key-1".to_string(),
            order_no: Some("order-no-1".to_string()),
            gateway_order_id: Some("gateway-order-1".to_string()),
            amount_usd: 10.0,
            pay_amount: Some(10.0),
            pay_currency: Some("USD".to_string()),
            exchange_rate: Some(1.0),
            payload: json!({ "status": "paid" }),
        };

        let response = handle_payment_callback_with_postgres(
            &state,
            "alipay",
            &request_context,
            &payload,
            true,
        )
        .await;

        assert_eq!(response.status(), http::StatusCode::SERVICE_UNAVAILABLE);
        let body = to_bytes(response.into_body(), usize::MAX)
            .await
            .expect("body should read");
        let payload: serde_json::Value =
            serde_json::from_slice(&body).expect("json body should parse");
        assert_eq!(
            payload,
            json!({ "detail": PAYMENT_CALLBACK_STORAGE_UNAVAILABLE_DETAIL })
        );
    }
}
