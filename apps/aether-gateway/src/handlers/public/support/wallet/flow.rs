use super::{
    build_auth_error_response, build_auth_json_response, build_wallet_daily_usage_payload,
    build_wallet_payload, build_wallet_zero_today_entry, http, parse_wallet_limit,
    parse_wallet_offset, resolve_authenticated_local_user, unix_secs_to_rfc3339,
    wallet_fixed_offset, wallet_today_billing_date_string, wallet_transaction_payload_from_row,
    AppState, Body, GatewayPublicRequestContext, Response, WALLET_LEGACY_TIMEZONE,
};
use serde_json::json;
use sqlx::Row;

fn wallet_flow_sort_key(item_type: &str, payload: &serde_json::Value) -> (String, u8, String) {
    match item_type {
        "daily_usage" => {
            let data = payload.get("data").unwrap_or(payload);
            let date = data
                .get("date")
                .and_then(serde_json::Value::as_str)
                .unwrap_or("");
            let sort_dt = data
                .get("last_finalized_at")
                .and_then(serde_json::Value::as_str)
                .or_else(|| {
                    data.get("aggregated_at")
                        .and_then(serde_json::Value::as_str)
                })
                .unwrap_or("");
            (date.to_string(), 1, sort_dt.to_string())
        }
        _ => {
            let data = payload.get("data").unwrap_or(payload);
            let created_at = data
                .get("created_at")
                .and_then(serde_json::Value::as_str)
                .unwrap_or("");
            let local_date = chrono::DateTime::parse_from_rfc3339(created_at)
                .ok()
                .map(|value| {
                    value
                        .with_timezone(&wallet_fixed_offset())
                        .date_naive()
                        .to_string()
                })
                .unwrap_or_default();
            (local_date, 0, created_at.to_string())
        }
    }
}

pub(super) async fn handle_wallet_flow(
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
            "today_entry": serde_json::Value::Null,
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

    let mut today_entry = build_wallet_zero_today_entry();
    let mut items = Vec::new();
    let mut total = 0_u64;
    if let Some(pool) = state.postgres_pool() {
        let today_row = sqlx::query(
            r#"
SELECT
  id,
  billing_date::text AS billing_date,
  billing_timezone,
  CAST(total_cost_usd AS DOUBLE PRECISION) AS total_cost_usd,
  total_requests,
  input_tokens,
  output_tokens,
  cache_creation_tokens,
  cache_read_tokens,
  CAST(EXTRACT(EPOCH FROM first_finalized_at) AS BIGINT) AS first_finalized_at_unix_secs,
  CAST(EXTRACT(EPOCH FROM last_finalized_at) AS BIGINT) AS last_finalized_at_unix_secs,
  CAST(EXTRACT(EPOCH FROM aggregated_at) AS BIGINT) AS aggregated_at_unix_secs
FROM wallet_daily_usage_ledgers
WHERE wallet_id = $1
  AND billing_timezone = $2
  AND billing_date = (timezone($2, now()))::date
LIMIT 1
            "#,
        )
        .bind(&wallet.id)
        .bind(WALLET_LEGACY_TIMEZONE)
        .fetch_optional(&pool)
        .await;
        if let Ok(Some(row)) = today_row {
            today_entry = build_wallet_daily_usage_payload(
                row.try_get::<Option<String>, _>("id").ok().flatten(),
                row.try_get::<String, _>("billing_date")
                    .ok()
                    .unwrap_or_else(wallet_today_billing_date_string),
                row.try_get::<String, _>("billing_timezone")
                    .ok()
                    .unwrap_or_else(|| WALLET_LEGACY_TIMEZONE.to_string()),
                row.try_get::<f64, _>("total_cost_usd")
                    .ok()
                    .unwrap_or_default(),
                row.try_get::<i64, _>("total_requests")
                    .ok()
                    .unwrap_or_default()
                    .max(0) as u64,
                row.try_get::<i64, _>("input_tokens")
                    .ok()
                    .unwrap_or_default()
                    .max(0) as u64,
                row.try_get::<i64, _>("output_tokens")
                    .ok()
                    .unwrap_or_default()
                    .max(0) as u64,
                row.try_get::<i64, _>("cache_creation_tokens")
                    .ok()
                    .unwrap_or_default()
                    .max(0) as u64,
                row.try_get::<i64, _>("cache_read_tokens")
                    .ok()
                    .unwrap_or_default()
                    .max(0) as u64,
                row.try_get::<Option<i64>, _>("first_finalized_at_unix_secs")
                    .ok()
                    .flatten()
                    .and_then(|value| u64::try_from(value).ok())
                    .and_then(unix_secs_to_rfc3339),
                row.try_get::<Option<i64>, _>("last_finalized_at_unix_secs")
                    .ok()
                    .flatten()
                    .and_then(|value| u64::try_from(value).ok())
                    .and_then(unix_secs_to_rfc3339),
                row.try_get::<Option<i64>, _>("aggregated_at_unix_secs")
                    .ok()
                    .flatten()
                    .and_then(|value| u64::try_from(value).ok())
                    .and_then(unix_secs_to_rfc3339),
                true,
            );
        }

        let fetch_size = offset.saturating_add(limit).min(5200);
        let tx_count_row = sqlx::query(
            r#"
SELECT COUNT(*) AS total
FROM wallet_transactions
WHERE wallet_id = $1
            "#,
        )
        .bind(&wallet.id)
        .fetch_one(&pool)
        .await;
        let tx_total = tx_count_row
            .ok()
            .and_then(|row| row.try_get::<i64, _>("total").ok())
            .unwrap_or_default()
            .max(0) as u64;
        let tx_rows = sqlx::query(
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
LIMIT $2
            "#,
        )
        .bind(&wallet.id)
        .bind(i64::try_from(fetch_size).ok().unwrap_or(50))
        .fetch_all(&pool)
        .await
        .unwrap_or_default();
        let daily_count_row = sqlx::query(
            r#"
SELECT COUNT(*) AS total
FROM wallet_daily_usage_ledgers
WHERE wallet_id = $1
  AND billing_timezone = $2
  AND billing_date < (timezone($2, now()))::date
            "#,
        )
        .bind(&wallet.id)
        .bind(WALLET_LEGACY_TIMEZONE)
        .fetch_one(&pool)
        .await;
        let daily_total = daily_count_row
            .ok()
            .and_then(|row| row.try_get::<i64, _>("total").ok())
            .unwrap_or_default()
            .max(0) as u64;
        let daily_rows = sqlx::query(
            r#"
SELECT
  id,
  billing_date::text AS billing_date,
  billing_timezone,
  CAST(total_cost_usd AS DOUBLE PRECISION) AS total_cost_usd,
  total_requests,
  input_tokens,
  output_tokens,
  cache_creation_tokens,
  cache_read_tokens,
  CAST(EXTRACT(EPOCH FROM first_finalized_at) AS BIGINT) AS first_finalized_at_unix_secs,
  CAST(EXTRACT(EPOCH FROM last_finalized_at) AS BIGINT) AS last_finalized_at_unix_secs,
  CAST(EXTRACT(EPOCH FROM aggregated_at) AS BIGINT) AS aggregated_at_unix_secs
FROM wallet_daily_usage_ledgers
WHERE wallet_id = $1
  AND billing_timezone = $2
  AND billing_date < (timezone($2, now()))::date
ORDER BY billing_date DESC
LIMIT $3
            "#,
        )
        .bind(&wallet.id)
        .bind(WALLET_LEGACY_TIMEZONE)
        .bind(i64::try_from(fetch_size).ok().unwrap_or(50))
        .fetch_all(&pool)
        .await
        .unwrap_or_default();

        let mut merged = tx_rows
            .iter()
            .filter_map(|row| wallet_transaction_payload_from_row(row).ok())
            .map(|data| json!({ "type": "transaction", "data": data }))
            .collect::<Vec<_>>();
        merged.extend(daily_rows.iter().map(|row| {
            json!({
                "type": "daily_usage",
                "data": build_wallet_daily_usage_payload(
                    row.try_get::<Option<String>, _>("id").ok().flatten(),
                    row.try_get::<String, _>("billing_date").ok().unwrap_or_default(),
                    row.try_get::<String, _>("billing_timezone").ok().unwrap_or_else(|| WALLET_LEGACY_TIMEZONE.to_string()),
                    row.try_get::<f64, _>("total_cost_usd").ok().unwrap_or_default(),
                    row.try_get::<i64, _>("total_requests").ok().unwrap_or_default().max(0) as u64,
                    row.try_get::<i64, _>("input_tokens").ok().unwrap_or_default().max(0) as u64,
                    row.try_get::<i64, _>("output_tokens").ok().unwrap_or_default().max(0) as u64,
                    row.try_get::<i64, _>("cache_creation_tokens").ok().unwrap_or_default().max(0) as u64,
                    row.try_get::<i64, _>("cache_read_tokens").ok().unwrap_or_default().max(0) as u64,
                    row.try_get::<Option<i64>, _>("first_finalized_at_unix_secs").ok().flatten().and_then(|value| u64::try_from(value).ok()).and_then(unix_secs_to_rfc3339),
                    row.try_get::<Option<i64>, _>("last_finalized_at_unix_secs").ok().flatten().and_then(|value| u64::try_from(value).ok()).and_then(unix_secs_to_rfc3339),
                    row.try_get::<Option<i64>, _>("aggregated_at_unix_secs").ok().flatten().and_then(|value| u64::try_from(value).ok()).and_then(unix_secs_to_rfc3339),
                    false,
                )
            })
        }));
        merged.sort_by(|left, right| {
            let left_type = left
                .get("type")
                .and_then(serde_json::Value::as_str)
                .unwrap_or("");
            let right_type = right
                .get("type")
                .and_then(serde_json::Value::as_str)
                .unwrap_or("");
            wallet_flow_sort_key(right_type, right).cmp(&wallet_flow_sort_key(left_type, left))
        });
        items = merged
            .into_iter()
            .skip(offset)
            .take(limit)
            .collect::<Vec<_>>();
        total = tx_total.saturating_add(daily_total);
    }

    let mut payload = json!({
        "today_entry": today_entry,
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
