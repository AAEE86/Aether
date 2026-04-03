use super::{
    build_auth_error_response, build_auth_json_response, build_auth_wallet_summary_payload, http,
    query_param_value, resolve_authenticated_local_user, unix_secs_to_rfc3339, AppState, Body,
    GatewayError, GatewayPublicRequestContext, Response, WALLET_LEGACY_TIMEZONE,
};
use crate::gateway::handlers::admin::round_to;
use chrono::Utc;
use serde_json::json;
use sqlx::Row;

const WALLET_TODAY_COST_UNAVAILABLE_DETAIL: &str = "钱包今日费用数据暂不可用";

pub(super) fn build_wallet_payload(
    wallet: Option<&aether_data::repository::wallet::StoredWalletSnapshot>,
) -> serde_json::Value {
    let wallet_payload = build_auth_wallet_summary_payload(wallet);
    json!({
        "wallet": wallet_payload.clone(),
        "unlimited": wallet_payload.get("unlimited").cloned().unwrap_or(json!(false)),
        "limit_mode": wallet_payload
            .get("limit_mode")
            .cloned()
            .unwrap_or_else(|| json!("finite")),
        "balance": wallet_payload.get("balance").cloned().unwrap_or(json!(0.0)),
        "recharge_balance": wallet_payload
            .get("recharge_balance")
            .cloned()
            .unwrap_or(json!(0.0)),
        "gift_balance": wallet_payload
            .get("gift_balance")
            .cloned()
            .unwrap_or(json!(0.0)),
        "refundable_balance": wallet_payload
            .get("refundable_balance")
            .cloned()
            .unwrap_or(json!(0.0)),
        "currency": wallet_payload
            .get("currency")
            .cloned()
            .unwrap_or_else(|| json!("USD")),
    })
}

fn build_wallet_balance_payload(
    wallet: Option<&aether_data::repository::wallet::StoredWalletSnapshot>,
) -> serde_json::Value {
    let mut payload = build_wallet_payload(wallet);
    payload["pending_refund_count"] = json!(0);
    payload
}

pub(super) fn parse_wallet_limit(query: Option<&str>) -> Result<usize, String> {
    match query_param_value(query, "limit") {
        Some(value) => {
            let parsed = value
                .parse::<usize>()
                .map_err(|_| "limit must be an integer between 1 and 200".to_string())?;
            if (1..=200).contains(&parsed) {
                Ok(parsed)
            } else {
                Err("limit must be an integer between 1 and 200".to_string())
            }
        }
        None => Ok(50),
    }
}

pub(super) fn parse_wallet_offset(query: Option<&str>) -> Result<usize, String> {
    match query_param_value(query, "offset") {
        Some(value) => value
            .parse::<usize>()
            .map_err(|_| "offset must be a non-negative integer".to_string()),
        None => Ok(0),
    }
}

pub(super) fn wallet_fixed_offset() -> chrono::FixedOffset {
    chrono::FixedOffset::east_opt(8 * 3600).expect("Asia/Shanghai offset should be valid")
}

pub(super) fn wallet_today_billing_date_string() -> String {
    Utc::now()
        .with_timezone(&wallet_fixed_offset())
        .date_naive()
        .to_string()
}

pub(super) fn build_wallet_daily_usage_payload(
    id: Option<String>,
    date: String,
    timezone: String,
    total_cost: f64,
    total_requests: u64,
    input_tokens: u64,
    output_tokens: u64,
    cache_creation_tokens: u64,
    cache_read_tokens: u64,
    first_finalized_at: Option<String>,
    last_finalized_at: Option<String>,
    aggregated_at: Option<String>,
    is_today: bool,
) -> serde_json::Value {
    json!({
        "id": id,
        "date": date,
        "timezone": timezone,
        "total_cost": round_to(total_cost, 6),
        "total_requests": total_requests,
        "input_tokens": input_tokens,
        "output_tokens": output_tokens,
        "cache_creation_tokens": cache_creation_tokens,
        "cache_read_tokens": cache_read_tokens,
        "first_finalized_at": first_finalized_at,
        "last_finalized_at": last_finalized_at,
        "aggregated_at": aggregated_at,
        "is_today": is_today,
    })
}

pub(super) fn build_wallet_zero_today_entry() -> serde_json::Value {
    build_wallet_daily_usage_payload(
        None,
        wallet_today_billing_date_string(),
        WALLET_LEGACY_TIMEZONE.to_string(),
        0.0,
        0,
        0,
        0,
        0,
        0,
        None,
        None,
        Some(Utc::now().to_rfc3339()),
        true,
    )
}

pub(super) fn wallet_transaction_payload_from_row(
    row: &sqlx::postgres::PgRow,
) -> Result<serde_json::Value, GatewayError> {
    let created_at = row
        .try_get::<Option<i64>, _>("created_at_unix_secs")
        .map_err(|err| GatewayError::Internal(err.to_string()))?
        .and_then(|value| u64::try_from(value).ok())
        .and_then(unix_secs_to_rfc3339);
    Ok(json!({
        "id": row.try_get::<String, _>("id").map_err(|err| GatewayError::Internal(err.to_string()))?,
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
        "description": row.try_get::<Option<String>, _>("description").map_err(|err| GatewayError::Internal(err.to_string()))?,
        "created_at": created_at,
    }))
}

pub(super) async fn handle_wallet_balance(
    state: &AppState,
    request_context: &GatewayPublicRequestContext,
    headers: &http::HeaderMap,
) -> Response<Body> {
    let auth = match resolve_authenticated_local_user(state, request_context, headers).await {
        Ok(value) => value,
        Err(response) => return response,
    };
    let wallet = state
        .read_wallet_snapshot_for_auth(&auth.user.id, "", false)
        .await
        .ok()
        .flatten();
    build_auth_json_response(
        http::StatusCode::OK,
        build_wallet_balance_payload(wallet.as_ref()),
        None,
    )
}

pub(super) async fn handle_wallet_today_cost(
    state: &AppState,
    request_context: &GatewayPublicRequestContext,
    headers: &http::HeaderMap,
) -> Response<Body> {
    if !state.has_usage_data_reader() {
        return build_auth_error_response(
            http::StatusCode::SERVICE_UNAVAILABLE,
            WALLET_TODAY_COST_UNAVAILABLE_DETAIL,
            false,
        );
    }

    let auth = match resolve_authenticated_local_user(state, request_context, headers).await {
        Ok(value) => value,
        Err(response) => return response,
    };
    let today = Utc::now().date_naive();
    let Some(start_of_day) = today.and_hms_opt(0, 0, 0) else {
        return build_auth_error_response(
            http::StatusCode::INTERNAL_SERVER_ERROR,
            "wallet today start is invalid",
            false,
        );
    };
    let start_unix_secs = u64::try_from(
        chrono::DateTime::<chrono::Utc>::from_naive_utc_and_offset(start_of_day, chrono::Utc)
            .timestamp(),
    )
    .unwrap_or_default();
    let end_unix_secs = start_unix_secs.saturating_add(24 * 3600);

    let items = match state
        .list_usage_audits(&aether_data::repository::usage::UsageAuditListQuery {
            created_from_unix_secs: Some(start_unix_secs),
            created_until_unix_secs: Some(end_unix_secs),
            user_id: Some(auth.user.id.clone()),
            provider_name: None,
            model: None,
        })
        .await
    {
        Ok(value) => value,
        Err(err) => {
            return build_auth_error_response(
                http::StatusCode::INTERNAL_SERVER_ERROR,
                format!("wallet today cost lookup failed: {err:?}"),
                false,
            )
        }
    };

    let settled = items
        .into_iter()
        .filter(|item| item.billing_status == "settled" && item.total_cost_usd > 0.0)
        .collect::<Vec<_>>();
    let total_cost = settled.iter().map(|item| item.total_cost_usd).sum::<f64>();
    let total_requests = settled.len() as u64;
    let input_tokens = settled.iter().map(|item| item.input_tokens).sum::<u64>();
    let output_tokens = settled.iter().map(|item| item.output_tokens).sum::<u64>();
    let cache_creation_tokens = settled
        .iter()
        .map(|item| item.cache_creation_input_tokens)
        .sum::<u64>();
    let cache_read_tokens = settled
        .iter()
        .map(|item| item.cache_read_input_tokens)
        .sum::<u64>();
    let first_finalized_at = settled
        .iter()
        .filter_map(|item| item.finalized_at_unix_secs)
        .min()
        .and_then(unix_secs_to_rfc3339);
    let last_finalized_at = settled
        .iter()
        .filter_map(|item| item.finalized_at_unix_secs)
        .max()
        .and_then(unix_secs_to_rfc3339);

    build_auth_json_response(
        http::StatusCode::OK,
        json!({
            "id": serde_json::Value::Null,
            "date": today.to_string(),
            "timezone": "UTC",
            "total_cost": round_to(total_cost, 6),
            "total_requests": total_requests,
            "input_tokens": input_tokens,
            "output_tokens": output_tokens,
            "cache_creation_tokens": cache_creation_tokens,
            "cache_read_tokens": cache_read_tokens,
            "first_finalized_at": first_finalized_at,
            "last_finalized_at": last_finalized_at,
            "aggregated_at": Utc::now().to_rfc3339(),
            "is_today": true,
        }),
        None,
    )
}

pub(super) async fn handle_wallet_transactions(
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
        return build_auth_json_response(
            http::StatusCode::OK,
            json!({
                "items": [],
                "total": 0,
                "limit": limit,
                "offset": offset,
            })
            .as_object()
            .cloned()
            .map(|mut value| {
                if let Some(wallet_payload) = build_wallet_payload(None).as_object() {
                    value.extend(wallet_payload.clone());
                }
                serde_json::Value::Object(value)
            })
            .unwrap_or_else(|| json!({})),
            None,
        );
    };

    let mut total = 0_u64;
    let mut items = Vec::new();
    if let Some(pool) = state.postgres_pool() {
        let count_row = match sqlx::query(
            r#"
SELECT COUNT(*) AS total
FROM wallet_transactions
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
                    format!("wallet transaction count failed: {err}"),
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
  category,
  reason_code,
  CAST(amount AS DOUBLE PRECISION) AS amount,
  CAST(balance_before AS DOUBLE PRECISION) AS balance_before,
  CAST(balance_after AS DOUBLE PRECISION) AS balance_after,
  CAST(recharge_balance_before AS DOUBLE PRECISION) AS recharge_balance_before,
  CAST(recharge_balance_after AS DOUBLE PRECISION) AS recharge_balance_after,
  CAST(gift_balance_before AS DOUBLE PRECISION) AS gift_balance_before,
  CAST(gift_balance_after AS DOUBLE PRECISION) AS gift_balance_after,
  link_type,
  link_id,
  operator_id,
  description,
  CAST(EXTRACT(EPOCH FROM created_at) AS BIGINT) AS created_at_unix_secs
FROM wallet_transactions
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
                    format!("wallet transaction query failed: {err}"),
                    false,
                )
            }
        };
        items = match rows
            .iter()
            .map(wallet_transaction_payload_from_row)
            .collect::<Result<Vec<_>, GatewayError>>()
        {
            Ok(value) => value,
            Err(err) => {
                return build_auth_error_response(
                    http::StatusCode::INTERNAL_SERVER_ERROR,
                    format!("wallet transaction payload failed: {err:?}"),
                    false,
                )
            }
        };
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
