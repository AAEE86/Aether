use super::{
    build_auth_error_response, build_auth_json_response, build_wallet_payload,
    build_wallet_recharge_storage_unavailable_response, http, parse_wallet_limit,
    parse_wallet_offset, resolve_authenticated_local_user, unix_secs_to_rfc3339,
    wallet_normalize_optional_string_field, AppState, Body, GatewayError,
    GatewayPublicRequestContext, Response, WALLET_SAFE_GATEWAY_RESPONSE_KEYS,
};
#[cfg(test)]
use super::{
    record_wallet_test_recharge, wallet_test_recharge_order_by_id,
    wallet_test_recharge_orders_for_user,
};
use chrono::Utc;
use serde::Deserialize;
use serde_json::json;
use sqlx::Row;
use uuid::Uuid;

#[derive(Debug, Deserialize)]
struct WalletCreateRechargeRequest {
    amount_usd: f64,
    payment_method: String,
    #[serde(default)]
    pay_amount: Option<f64>,
    #[serde(default)]
    pay_currency: Option<String>,
    #[serde(default)]
    exchange_rate: Option<f64>,
}

#[derive(Debug, Clone)]
struct NormalizedWalletCreateRechargeRequest {
    amount_usd: f64,
    payment_method: String,
    pay_amount: Option<f64>,
    pay_currency: Option<String>,
    exchange_rate: Option<f64>,
}

fn normalize_wallet_create_recharge_request(
    payload: WalletCreateRechargeRequest,
) -> Result<NormalizedWalletCreateRechargeRequest, &'static str> {
    if !payload.amount_usd.is_finite() || payload.amount_usd <= 0.0 {
        return Err("输入验证失败");
    }
    let payment_method = payload.payment_method.trim().to_ascii_lowercase();
    if payment_method.is_empty() || payment_method.chars().count() > 30 {
        return Err("输入验证失败");
    }
    if matches!(payload.pay_amount, Some(value) if !value.is_finite() || value <= 0.0) {
        return Err("输入验证失败");
    }
    if matches!(payload.exchange_rate, Some(value) if !value.is_finite() || value <= 0.0) {
        return Err("输入验证失败");
    }
    let pay_currency = wallet_normalize_optional_string_field(payload.pay_currency, 3)?;
    if matches!(pay_currency.as_deref(), Some(value) if value.chars().count() != 3) {
        return Err("输入验证失败");
    }

    Ok(NormalizedWalletCreateRechargeRequest {
        amount_usd: payload.amount_usd,
        payment_method,
        pay_amount: payload.pay_amount,
        pay_currency,
        exchange_rate: payload.exchange_rate,
    })
}

fn wallet_build_order_no(now: chrono::DateTime<chrono::Utc>) -> String {
    format!(
        "po_{}_{}",
        now.format("%Y%m%d%H%M%S%6f"),
        &Uuid::new_v4().simple().to_string()[..12]
    )
}

fn wallet_checkout_payload(
    payment_method: &str,
    order_no: &str,
    expires_at: chrono::DateTime<chrono::Utc>,
) -> Result<(String, serde_json::Value), String> {
    let expires_at = expires_at.to_rfc3339();
    match payment_method {
        "alipay" => {
            let gateway_order_id = format!("ali_{order_no}");
            Ok((
                gateway_order_id.clone(),
                json!({
                    "gateway": "alipay",
                    "display_name": "支付宝",
                    "gateway_order_id": gateway_order_id,
                    "payment_url": format!("/pay/mock/alipay/{order_no}"),
                    "qr_code": format!("mock://alipay/{order_no}"),
                    "expires_at": expires_at,
                }),
            ))
        }
        "wechat" => {
            let gateway_order_id = format!("wx_{order_no}");
            Ok((
                gateway_order_id.clone(),
                json!({
                    "gateway": "wechat",
                    "display_name": "微信支付",
                    "gateway_order_id": gateway_order_id,
                    "payment_url": format!("/pay/mock/wechat/{order_no}"),
                    "qr_code": format!("mock://wechat/{order_no}"),
                    "expires_at": expires_at,
                }),
            ))
        }
        "manual" => {
            let gateway_order_id = format!("manual_{order_no}");
            Ok((
                gateway_order_id.clone(),
                json!({
                    "gateway": "manual",
                    "display_name": "人工打款",
                    "gateway_order_id": gateway_order_id,
                    "payment_url": serde_json::Value::Null,
                    "qr_code": serde_json::Value::Null,
                    "instructions": "请线下确认到账后由管理员处理",
                    "expires_at": expires_at,
                }),
            ))
        }
        _ => Err(format!("unsupported payment_method: {payment_method}")),
    }
}

fn wallet_order_id_from_path(request_path: &str) -> Option<String> {
    let trimmed = request_path.trim_end_matches('/');
    let order_id = trimmed.strip_prefix("/api/wallet/recharge/")?.trim();
    if order_id.is_empty() || order_id.contains('/') {
        None
    } else {
        Some(order_id.to_string())
    }
}

pub(super) fn wallet_recharge_detail_path_matches(request_path: &str) -> bool {
    wallet_order_id_from_path(request_path).is_some()
}

pub(crate) fn sanitize_wallet_gateway_response(
    value: Option<serde_json::Value>,
) -> serde_json::Value {
    let Some(value) = value else {
        return json!({});
    };
    let Some(object) = value.as_object() else {
        return json!({});
    };
    let mut sanitized = serde_json::Map::new();
    for key in WALLET_SAFE_GATEWAY_RESPONSE_KEYS {
        if let Some(item) = object.get(*key) {
            sanitized.insert((*key).to_string(), item.clone());
        }
    }
    serde_json::Value::Object(sanitized)
}

fn build_wallet_payment_order_payload(
    id: String,
    order_no: String,
    wallet_id: String,
    user_id: Option<String>,
    amount_usd: f64,
    pay_amount: Option<f64>,
    pay_currency: Option<String>,
    exchange_rate: Option<f64>,
    refunded_amount_usd: f64,
    refundable_amount_usd: f64,
    payment_method: String,
    gateway_order_id: Option<String>,
    gateway_response: Option<serde_json::Value>,
    status: String,
    created_at: Option<String>,
    paid_at: Option<String>,
    credited_at: Option<String>,
    expires_at: Option<String>,
) -> serde_json::Value {
    json!({
        "id": id,
        "order_no": order_no,
        "wallet_id": wallet_id,
        "user_id": user_id,
        "amount_usd": amount_usd,
        "pay_amount": pay_amount,
        "pay_currency": pay_currency,
        "exchange_rate": exchange_rate,
        "refunded_amount_usd": refunded_amount_usd,
        "refundable_amount_usd": refundable_amount_usd,
        "payment_method": payment_method,
        "gateway_order_id": gateway_order_id,
        "gateway_response": sanitize_wallet_gateway_response(gateway_response),
        "status": status,
        "created_at": created_at,
        "paid_at": paid_at,
        "credited_at": credited_at,
        "expires_at": expires_at,
    })
}

pub(crate) fn wallet_payment_order_payload_from_row(
    row: &sqlx::postgres::PgRow,
) -> Result<serde_json::Value, GatewayError> {
    let created_at = row
        .try_get::<Option<i64>, _>("created_at_unix_secs")
        .map_err(|err| GatewayError::Internal(err.to_string()))?
        .and_then(|value| u64::try_from(value).ok())
        .and_then(unix_secs_to_rfc3339);
    let paid_at = row
        .try_get::<Option<i64>, _>("paid_at_unix_secs")
        .map_err(|err| GatewayError::Internal(err.to_string()))?
        .and_then(|value| u64::try_from(value).ok())
        .and_then(unix_secs_to_rfc3339);
    let credited_at = row
        .try_get::<Option<i64>, _>("credited_at_unix_secs")
        .map_err(|err| GatewayError::Internal(err.to_string()))?
        .and_then(|value| u64::try_from(value).ok())
        .and_then(unix_secs_to_rfc3339);
    let expires_at = row
        .try_get::<Option<i64>, _>("expires_at_unix_secs")
        .map_err(|err| GatewayError::Internal(err.to_string()))?
        .and_then(|value| u64::try_from(value).ok())
        .and_then(unix_secs_to_rfc3339);
    Ok(build_wallet_payment_order_payload(
        row.try_get::<String, _>("id")
            .map_err(|err| GatewayError::Internal(err.to_string()))?,
        row.try_get::<String, _>("order_no")
            .map_err(|err| GatewayError::Internal(err.to_string()))?,
        row.try_get::<String, _>("wallet_id")
            .map_err(|err| GatewayError::Internal(err.to_string()))?,
        row.try_get::<Option<String>, _>("user_id")
            .map_err(|err| GatewayError::Internal(err.to_string()))?,
        row.try_get::<f64, _>("amount_usd")
            .map_err(|err| GatewayError::Internal(err.to_string()))?,
        row.try_get::<Option<f64>, _>("pay_amount")
            .map_err(|err| GatewayError::Internal(err.to_string()))?,
        row.try_get::<Option<String>, _>("pay_currency")
            .map_err(|err| GatewayError::Internal(err.to_string()))?,
        row.try_get::<Option<f64>, _>("exchange_rate")
            .map_err(|err| GatewayError::Internal(err.to_string()))?,
        row.try_get::<f64, _>("refunded_amount_usd")
            .map_err(|err| GatewayError::Internal(err.to_string()))?,
        row.try_get::<f64, _>("refundable_amount_usd")
            .map_err(|err| GatewayError::Internal(err.to_string()))?,
        row.try_get::<String, _>("payment_method")
            .map_err(|err| GatewayError::Internal(err.to_string()))?,
        row.try_get::<Option<String>, _>("gateway_order_id")
            .map_err(|err| GatewayError::Internal(err.to_string()))?,
        row.try_get::<Option<serde_json::Value>, _>("gateway_response")
            .map_err(|err| GatewayError::Internal(err.to_string()))?,
        row.try_get::<String, _>("effective_status")
            .map_err(|err| GatewayError::Internal(err.to_string()))?,
        created_at,
        paid_at,
        credited_at,
        expires_at,
    ))
}

pub(super) async fn handle_wallet_create_recharge(
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
    let payload = match serde_json::from_slice::<WalletCreateRechargeRequest>(request_body) {
        Ok(value) => value,
        Err(_) => {
            return build_auth_error_response(http::StatusCode::BAD_REQUEST, "输入验证失败", false)
        }
    };
    let payload = match normalize_wallet_create_recharge_request(payload) {
        Ok(value) => value,
        Err(detail) => {
            return build_auth_error_response(http::StatusCode::BAD_REQUEST, detail, false)
        }
    };
    if payload.payment_method == "admin_manual" {
        return build_auth_error_response(
            http::StatusCode::BAD_REQUEST,
            "admin_manual is reserved for admin recharge",
            false,
        );
    }

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

    let Some(pool) = state.postgres_pool() else {
        #[cfg(test)]
        {
            let Some(wallet) = wallet else {
                return build_auth_error_response(
                    http::StatusCode::BAD_REQUEST,
                    "wallet not available",
                    false,
                );
            };
            if wallet.status != "active" {
                return build_auth_error_response(
                    http::StatusCode::BAD_REQUEST,
                    "wallet is not active",
                    false,
                );
            }
            let now = Utc::now();
            let order_id = Uuid::new_v4().to_string();
            let order_no = wallet_build_order_no(now);
            let expires_at = now + chrono::Duration::minutes(30);
            let (gateway_order_id, gateway_response) =
                match wallet_checkout_payload(&payload.payment_method, &order_no, expires_at) {
                    Ok(value) => value,
                    Err(detail) => {
                        return build_auth_error_response(
                            http::StatusCode::BAD_REQUEST,
                            detail,
                            false,
                        );
                    }
                };
            let order_payload = build_wallet_payment_order_payload(
                order_id,
                order_no,
                wallet.id.clone(),
                Some(auth.user.id.clone()),
                payload.amount_usd,
                payload.pay_amount,
                payload.pay_currency.clone(),
                payload.exchange_rate,
                0.0,
                0.0,
                payload.payment_method,
                Some(gateway_order_id),
                Some(gateway_response.clone()),
                "pending".to_string(),
                Some(now.to_rfc3339()),
                None,
                None,
                Some(expires_at.to_rfc3339()),
            );
            record_wallet_test_recharge(auth.user.id, order_payload.clone());
            return build_auth_json_response(
                http::StatusCode::OK,
                json!({
                    "order": order_payload,
                    "payment_instructions": sanitize_wallet_gateway_response(Some(gateway_response)),
                }),
                None,
            );
        }
        #[cfg(not(test))]
        return build_wallet_recharge_storage_unavailable_response();
    };

    let mut tx = match pool.begin().await {
        Ok(value) => value,
        Err(err) => {
            return build_auth_error_response(
                http::StatusCode::INTERNAL_SERVER_ERROR,
                format!("wallet recharge transaction failed: {err}"),
                false,
            )
        }
    };

    let wallet_row = match sqlx::query(
        r#"
SELECT id, status
FROM wallets
WHERE user_id = $1
LIMIT 1
FOR UPDATE
        "#,
    )
    .bind(&auth.user.id)
    .fetch_optional(&mut *tx)
    .await
    {
        Ok(Some(value)) => Some(value),
        Ok(None) => {
            let wallet_id = wallet
                .as_ref()
                .map(|value| value.id.clone())
                .unwrap_or_else(|| Uuid::new_v4().to_string());
            match sqlx::query(
                r#"
INSERT INTO wallets (
  id,
  user_id,
  balance,
  gift_balance,
  limit_mode,
  currency,
  status,
  total_recharged,
  total_consumed,
  total_refunded,
  total_adjusted,
  created_at,
  updated_at
)
VALUES (
  $1,
  $2,
  0,
  0,
  'finite',
  'USD',
  'active',
  0,
  0,
  0,
  0,
  NOW(),
  NOW()
)
ON CONFLICT (user_id) DO UPDATE
SET updated_at = wallets.updated_at
RETURNING id, status
                "#,
            )
            .bind(&wallet_id)
            .bind(&auth.user.id)
            .fetch_one(&mut *tx)
            .await
            {
                Ok(value) => Some(value),
                Err(err) => {
                    let _ = tx.rollback().await;
                    return build_auth_error_response(
                        http::StatusCode::INTERNAL_SERVER_ERROR,
                        format!("wallet recharge wallet bootstrap failed: {err}"),
                        false,
                    );
                }
            }
        }
        Err(err) => {
            let _ = tx.rollback().await;
            return build_auth_error_response(
                http::StatusCode::INTERNAL_SERVER_ERROR,
                format!("wallet recharge wallet lookup failed: {err}"),
                false,
            );
        }
    };
    let Some(wallet_row) = wallet_row else {
        let _ = tx.rollback().await;
        return build_auth_error_response(
            http::StatusCode::BAD_REQUEST,
            "wallet not available",
            false,
        );
    };
    let wallet_id = wallet_row
        .try_get::<String, _>("id")
        .ok()
        .unwrap_or_default();
    let wallet_status = wallet_row
        .try_get::<String, _>("status")
        .ok()
        .unwrap_or_default();
    if wallet_status != "active" {
        let _ = tx.rollback().await;
        return build_auth_error_response(
            http::StatusCode::BAD_REQUEST,
            "wallet is not active",
            false,
        );
    }

    let now = Utc::now();
    let order_id = Uuid::new_v4().to_string();
    let order_no = wallet_build_order_no(now);
    let expires_at = now + chrono::Duration::minutes(30);
    let (gateway_order_id, gateway_response) =
        match wallet_checkout_payload(&payload.payment_method, &order_no, expires_at) {
            Ok(value) => value,
            Err(detail) => {
                let _ = tx.rollback().await;
                return build_auth_error_response(http::StatusCode::BAD_REQUEST, detail, false);
            }
        };

    let row = match sqlx::query(
        r#"
INSERT INTO payment_orders (
  id,
  order_no,
  wallet_id,
  user_id,
  amount_usd,
  pay_amount,
  pay_currency,
  exchange_rate,
  refunded_amount_usd,
  refundable_amount_usd,
  payment_method,
  gateway_order_id,
  gateway_response,
  status,
  created_at,
  expires_at
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
  0,
  0,
  $9,
  $10,
  $11,
  'pending',
  NOW(),
  to_timestamp($12)
)
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
    .bind(&order_no)
    .bind(&wallet_id)
    .bind(&auth.user.id)
    .bind(payload.amount_usd)
    .bind(payload.pay_amount)
    .bind(payload.pay_currency.as_deref())
    .bind(payload.exchange_rate)
    .bind(&payload.payment_method)
    .bind(&gateway_order_id)
    .bind(&gateway_response)
    .bind(expires_at.timestamp())
    .fetch_one(&mut *tx)
    .await
    {
        Ok(value) => value,
        Err(err) => {
            let _ = tx.rollback().await;
            return build_auth_error_response(
                http::StatusCode::INTERNAL_SERVER_ERROR,
                format!("wallet recharge create failed: {err}"),
                false,
            );
        }
    };

    if let Err(err) = tx.commit().await {
        return build_auth_error_response(
            http::StatusCode::INTERNAL_SERVER_ERROR,
            format!("wallet recharge commit failed: {err}"),
            false,
        );
    }

    let order_payload = match wallet_payment_order_payload_from_row(&row) {
        Ok(value) => value,
        Err(err) => {
            return build_auth_error_response(
                http::StatusCode::INTERNAL_SERVER_ERROR,
                format!("wallet recharge payload failed: {err:?}"),
                false,
            );
        }
    };
    build_auth_json_response(
        http::StatusCode::OK,
        json!({
            "order": order_payload,
            "payment_instructions": sanitize_wallet_gateway_response(Some(gateway_response)),
        }),
        None,
    )
}

pub(super) async fn handle_wallet_recharge_list(
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

    let mut total = 0_u64;
    let mut items = Vec::new();
    if let Some(pool) = state.postgres_pool() {
        let count_row = match sqlx::query(
            r#"
SELECT COUNT(*) AS total
FROM payment_orders
WHERE user_id = $1
            "#,
        )
        .bind(&auth.user.id)
        .fetch_one(&pool)
        .await
        {
            Ok(value) => value,
            Err(err) => {
                return build_auth_error_response(
                    http::StatusCode::INTERNAL_SERVER_ERROR,
                    format!("wallet recharge count failed: {err}"),
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
  CASE
    WHEN status = 'pending' AND expires_at IS NOT NULL AND expires_at < now() THEN 'expired'
    ELSE status
  END AS effective_status,
  CAST(EXTRACT(EPOCH FROM created_at) AS BIGINT) AS created_at_unix_secs,
  CAST(EXTRACT(EPOCH FROM paid_at) AS BIGINT) AS paid_at_unix_secs,
  CAST(EXTRACT(EPOCH FROM credited_at) AS BIGINT) AS credited_at_unix_secs,
  CAST(EXTRACT(EPOCH FROM expires_at) AS BIGINT) AS expires_at_unix_secs
FROM payment_orders
WHERE user_id = $1
ORDER BY created_at DESC
OFFSET $2
LIMIT $3
            "#,
        )
        .bind(&auth.user.id)
        .bind(i64::try_from(offset).ok().unwrap_or_default())
        .bind(i64::try_from(limit).ok().unwrap_or_default())
        .fetch_all(&pool)
        .await
        {
            Ok(value) => value,
            Err(err) => {
                return build_auth_error_response(
                    http::StatusCode::INTERNAL_SERVER_ERROR,
                    format!("wallet recharge query failed: {err}"),
                    false,
                )
            }
        };
        items = match rows
            .iter()
            .map(wallet_payment_order_payload_from_row)
            .collect::<Result<Vec<_>, GatewayError>>()
        {
            Ok(value) => value,
            Err(err) => {
                return build_auth_error_response(
                    http::StatusCode::INTERNAL_SERVER_ERROR,
                    format!("wallet recharge payload failed: {err:?}"),
                    false,
                )
            }
        };
    } else {
        #[cfg(test)]
        {
            let (test_items, test_total) =
                wallet_test_recharge_orders_for_user(&auth.user.id, limit, offset);
            items = test_items;
            total = test_total;
        }
    }

    let mut payload = json!({
        "items": items,
        "total": total,
        "limit": limit,
        "offset": offset,
    });
    if let Some(object) = payload.as_object_mut() {
        if let Some(wallet_payload) = build_wallet_payload(wallet.as_ref()).as_object() {
            object.extend(wallet_payload.clone());
        }
    }
    build_auth_json_response(http::StatusCode::OK, payload, None)
}

pub(super) async fn handle_wallet_recharge_detail(
    state: &AppState,
    request_context: &GatewayPublicRequestContext,
    headers: &http::HeaderMap,
) -> Response<Body> {
    let auth = match resolve_authenticated_local_user(state, request_context, headers).await {
        Ok(value) => value,
        Err(response) => return response,
    };
    let Some(order_id) = wallet_order_id_from_path(&request_context.request_path) else {
        return build_auth_error_response(
            http::StatusCode::NOT_FOUND,
            "Payment order not found",
            false,
        );
    };
    let Some(pool) = state.postgres_pool() else {
        #[cfg(test)]
        {
            if let Some(order) = wallet_test_recharge_order_by_id(&auth.user.id, &order_id) {
                return build_auth_json_response(
                    http::StatusCode::OK,
                    json!({ "order": order }),
                    None,
                );
            }
        }
        return build_auth_error_response(
            http::StatusCode::NOT_FOUND,
            "Payment order not found",
            false,
        );
    };
    let row = match sqlx::query(
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
  CASE
    WHEN status = 'pending' AND expires_at IS NOT NULL AND expires_at < now() THEN 'expired'
    ELSE status
  END AS effective_status,
  CAST(EXTRACT(EPOCH FROM created_at) AS BIGINT) AS created_at_unix_secs,
  CAST(EXTRACT(EPOCH FROM paid_at) AS BIGINT) AS paid_at_unix_secs,
  CAST(EXTRACT(EPOCH FROM credited_at) AS BIGINT) AS credited_at_unix_secs,
  CAST(EXTRACT(EPOCH FROM expires_at) AS BIGINT) AS expires_at_unix_secs
FROM payment_orders
WHERE id = $1 AND user_id = $2
LIMIT 1
        "#,
    )
    .bind(&order_id)
    .bind(&auth.user.id)
    .fetch_optional(&pool)
    .await
    {
        Ok(value) => value,
        Err(err) => {
            return build_auth_error_response(
                http::StatusCode::INTERNAL_SERVER_ERROR,
                format!("wallet recharge detail query failed: {err}"),
                false,
            )
        }
    };
    let Some(row) = row else {
        return build_auth_error_response(
            http::StatusCode::NOT_FOUND,
            "Payment order not found",
            false,
        );
    };
    let payload = match wallet_payment_order_payload_from_row(&row) {
        Ok(value) => json!({ "order": value }),
        Err(err) => {
            return build_auth_error_response(
                http::StatusCode::INTERNAL_SERVER_ERROR,
                format!("wallet recharge detail payload failed: {err:?}"),
                false,
            )
        }
    };
    build_auth_json_response(http::StatusCode::OK, payload, None)
}
