use crate::gateway::handlers::{query_param_value, unix_secs_to_rfc3339};
use crate::gateway::{AppState, GatewayError, GatewayPublicRequestContext};
use axum::{
    body::Body,
    http,
    response::{IntoResponse, Response},
    Json,
};
use serde_json::json;
use sqlx::Row;

pub(super) const ADMIN_WALLETS_DATA_UNAVAILABLE_DETAIL: &str = "Admin wallets data unavailable";
pub(super) const ADMIN_WALLETS_API_KEY_REFUND_DETAIL: &str = "独立密钥钱包不支持退款审批";
pub(super) const ADMIN_WALLETS_API_KEY_RECHARGE_DETAIL: &str = "独立密钥钱包不支持充值，请使用调账";
pub(super) const ADMIN_WALLETS_API_KEY_GIFT_ADJUST_DETAIL: &str = "独立密钥钱包不支持赠款调账";

#[derive(Debug, serde::Deserialize)]
pub(super) struct AdminWalletRechargeRequest {
    pub(super) amount_usd: f64,
    #[serde(default = "default_admin_wallet_payment_method")]
    pub(super) payment_method: String,
    #[serde(default)]
    pub(super) description: Option<String>,
}

#[derive(Debug, serde::Deserialize)]
pub(super) struct AdminWalletAdjustRequest {
    pub(super) amount_usd: f64,
    #[serde(default = "default_admin_wallet_balance_type")]
    pub(super) balance_type: String,
    #[serde(default)]
    pub(super) description: Option<String>,
}

#[derive(Debug, serde::Deserialize)]
pub(super) struct AdminWalletRefundFailRequest {
    pub(super) reason: String,
}

#[derive(Debug, serde::Deserialize)]
pub(super) struct AdminWalletRefundCompleteRequest {
    #[serde(default)]
    pub(super) gateway_refund_id: Option<String>,
    #[serde(default)]
    pub(super) payout_reference: Option<String>,
    #[serde(default)]
    pub(super) payout_proof: Option<serde_json::Value>,
}

fn default_admin_wallet_payment_method() -> String {
    "admin_manual".to_string()
}

fn default_admin_wallet_balance_type() -> String {
    "recharge".to_string()
}

pub(super) fn build_admin_wallets_data_unavailable_response() -> Response<Body> {
    (
        http::StatusCode::SERVICE_UNAVAILABLE,
        Json(json!({ "detail": ADMIN_WALLETS_DATA_UNAVAILABLE_DETAIL })),
    )
        .into_response()
}

pub(super) fn build_admin_wallets_bad_request_response(
    detail: impl Into<String>,
) -> Response<Body> {
    (
        http::StatusCode::BAD_REQUEST,
        Json(json!({ "detail": detail.into() })),
    )
        .into_response()
}

pub(super) fn build_admin_wallet_not_found_response() -> Response<Body> {
    (
        http::StatusCode::NOT_FOUND,
        Json(json!({ "detail": "Wallet not found" })),
    )
        .into_response()
}

pub(super) fn build_admin_wallet_refund_not_found_response() -> Response<Body> {
    (
        http::StatusCode::NOT_FOUND,
        Json(json!({ "detail": "Refund request not found" })),
    )
        .into_response()
}

pub(super) fn build_admin_wallet_payment_order_payload(
    order_id: String,
    order_no: String,
    amount_usd: f64,
    payment_method: String,
    status: String,
    created_at: Option<String>,
    credited_at: Option<String>,
) -> serde_json::Value {
    json!({
        "id": order_id,
        "order_no": order_no,
        "amount_usd": amount_usd,
        "payment_method": payment_method,
        "status": status,
        "created_at": created_at,
        "credited_at": credited_at,
    })
}

#[allow(clippy::too_many_arguments)]
pub(super) fn build_admin_wallet_transaction_payload(
    wallet: &aether_data::repository::wallet::StoredWalletSnapshot,
    owner: &AdminWalletOwnerSummary,
    transaction_id: String,
    category: &str,
    reason_code: &str,
    amount: f64,
    balance_before: f64,
    balance_after: f64,
    recharge_balance_before: f64,
    recharge_balance_after: f64,
    gift_balance_before: f64,
    gift_balance_after: f64,
    link_type: Option<&str>,
    link_id: Option<&str>,
    operator_id: Option<&str>,
    description: Option<&str>,
    created_at: Option<String>,
) -> serde_json::Value {
    json!({
        "id": transaction_id,
        "wallet_id": wallet.id,
        "owner_type": owner.owner_type,
        "owner_name": owner.owner_name.clone(),
        "wallet_status": wallet.status,
        "category": category,
        "reason_code": reason_code,
        "amount": amount,
        "balance_before": balance_before,
        "balance_after": balance_after,
        "recharge_balance_before": recharge_balance_before,
        "recharge_balance_after": recharge_balance_after,
        "gift_balance_before": gift_balance_before,
        "gift_balance_after": gift_balance_after,
        "link_type": link_type,
        "link_id": link_id,
        "operator_id": operator_id,
        "operator_name": serde_json::Value::Null,
        "operator_email": serde_json::Value::Null,
        "description": description,
        "created_at": created_at,
    })
}

pub(super) fn admin_wallet_build_order_no(now: chrono::DateTime<chrono::Utc>) -> String {
    format!(
        "po_{}_{}",
        now.format("%Y%m%d%H%M%S%6f"),
        &uuid::Uuid::new_v4().simple().to_string()[..12]
    )
}

pub(super) fn normalize_admin_wallet_description(
    value: Option<String>,
) -> Result<Option<String>, String> {
    match value {
        None => Ok(None),
        Some(value) => {
            let trimmed = value.trim();
            if trimmed.is_empty() {
                return Ok(None);
            }
            Ok(Some(trimmed.chars().take(500).collect()))
        }
    }
}

pub(super) fn normalize_admin_wallet_required_text(
    value: String,
    field_name: &str,
    max_len: usize,
) -> Result<String, String> {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        return Err(format!("{field_name} 不能为空"));
    }
    if trimmed.chars().count() > max_len {
        return Err(format!("{field_name} 长度不能超过 {max_len}"));
    }
    Ok(trimmed.to_string())
}

pub(super) fn normalize_admin_wallet_optional_text(
    value: Option<String>,
    field_name: &str,
    max_len: usize,
) -> Result<Option<String>, String> {
    match value {
        None => Ok(None),
        Some(value) => {
            let trimmed = value.trim();
            if trimmed.is_empty() {
                return Ok(None);
            }
            if trimmed.chars().count() > max_len {
                return Err(format!("{field_name} 长度不能超过 {max_len}"));
            }
            Ok(Some(trimmed.to_string()))
        }
    }
}

pub(super) fn normalize_admin_wallet_payment_method(value: String) -> Result<String, String> {
    let normalized = value.trim();
    if normalized.is_empty() {
        return Err("payment_method 不能为空".to_string());
    }
    Ok(normalized.chars().take(30).collect())
}

pub(super) fn normalize_admin_wallet_balance_type(value: String) -> Result<String, String> {
    let normalized = value.trim().to_ascii_lowercase();
    match normalized.as_str() {
        "recharge" | "gift" => Ok(normalized),
        _ => Err("balance_type 必须为 recharge 或 gift".to_string()),
    }
}

pub(super) fn normalize_admin_wallet_positive_amount(
    value: f64,
    field_name: &str,
) -> Result<f64, String> {
    if !value.is_finite() || value <= 0.0 {
        return Err(format!("{field_name} 必须为大于 0 的有限数字"));
    }
    Ok(value)
}

pub(super) fn normalize_admin_wallet_non_zero_amount(
    value: f64,
    field_name: &str,
) -> Result<f64, String> {
    if !value.is_finite() || value == 0.0 {
        return Err(format!("{field_name} 不能为 0，且必须为有限数字"));
    }
    Ok(value)
}

pub(super) fn admin_wallet_operator_id(
    request_context: &GatewayPublicRequestContext,
) -> Option<String> {
    request_context
        .control_decision
        .as_ref()
        .and_then(|decision| decision.admin_principal.as_ref())
        .map(|principal| principal.user_id.clone())
}

pub(super) fn admin_wallet_recharge_reason_code(payment_method: &str) -> &'static str {
    match payment_method {
        "card_code" | "gift_code" | "card_recharge" => "topup_card_code",
        _ => "topup_admin_manual",
    }
}

pub(super) fn admin_wallet_apply_manual_recharge_to_snapshot(
    wallet: &mut aether_data::repository::wallet::StoredWalletSnapshot,
    amount_usd: f64,
) -> (f64, f64, f64, f64, f64, f64) {
    let recharge_before = wallet.balance;
    let gift_before = wallet.gift_balance;
    let balance_before = recharge_before + gift_before;

    wallet.balance += amount_usd;
    wallet.total_recharged += amount_usd;
    wallet.updated_at_unix_secs = chrono::Utc::now().timestamp().max(0) as u64;

    let recharge_after = wallet.balance;
    let gift_after = wallet.gift_balance;
    let balance_after = recharge_after + gift_after;

    (
        balance_before,
        balance_after,
        recharge_before,
        recharge_after,
        gift_before,
        gift_after,
    )
}

pub(super) fn admin_wallet_apply_adjust_to_snapshot(
    wallet: &mut aether_data::repository::wallet::StoredWalletSnapshot,
    amount_usd: f64,
    balance_type: &str,
) -> Result<(f64, f64, f64, f64, f64, f64), String> {
    if amount_usd == 0.0 {
        return Err("adjust amount must not be zero".to_string());
    }
    if balance_type == "gift" && wallet.api_key_id.is_some() {
        return Err(ADMIN_WALLETS_API_KEY_GIFT_ADJUST_DETAIL.to_string());
    }

    let recharge_before = wallet.balance;
    let gift_before = wallet.gift_balance;
    let balance_before = recharge_before + gift_before;

    let mut recharge_after = recharge_before;
    let mut gift_after = gift_before;

    if amount_usd > 0.0 {
        if balance_type == "gift" {
            gift_after += amount_usd;
        } else {
            recharge_after += amount_usd;
        }
    } else {
        let mut remaining = -amount_usd;
        let consume_positive_bucket = |balance: &mut f64, remaining: &mut f64| {
            if *remaining <= 0.0 {
                return;
            }
            let available = balance.max(0.0);
            let consumed = available.min(*remaining);
            *balance -= consumed;
            *remaining -= consumed;
        };

        if balance_type == "gift" {
            consume_positive_bucket(&mut gift_after, &mut remaining);
            consume_positive_bucket(&mut recharge_after, &mut remaining);
        } else {
            consume_positive_bucket(&mut recharge_after, &mut remaining);
            consume_positive_bucket(&mut gift_after, &mut remaining);
        }

        if remaining > 0.0 {
            recharge_after -= remaining;
        }
        if gift_after < 0.0 {
            return Err("gift balance cannot be negative".to_string());
        }
    }

    wallet.balance = recharge_after;
    wallet.gift_balance = gift_after;
    wallet.total_adjusted += amount_usd;
    wallet.updated_at_unix_secs = chrono::Utc::now().timestamp().max(0) as u64;

    Ok((
        balance_before,
        recharge_after + gift_after,
        recharge_before,
        recharge_after,
        gift_before,
        gift_after,
    ))
}

pub(super) fn admin_wallet_id_from_detail_path(request_path: &str) -> Option<String> {
    request_path
        .strip_prefix("/api/admin/wallets/")?
        .trim()
        .trim_matches('/')
        .split('/')
        .next()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .filter(|value| !value.contains('/'))
        .map(ToOwned::to_owned)
}

pub(super) fn admin_wallet_id_from_suffix_path(request_path: &str, suffix: &str) -> Option<String> {
    request_path
        .strip_prefix("/api/admin/wallets/")?
        .strip_suffix(suffix)
        .map(|value| value.trim().trim_matches('/').to_string())
        .filter(|value| !value.is_empty() && !value.contains('/'))
}

pub(super) fn admin_wallet_refund_ids_from_suffix_path(
    request_path: &str,
    suffix: &str,
) -> Option<(String, String)> {
    let trimmed = request_path
        .strip_prefix("/api/admin/wallets/")?
        .strip_suffix(suffix)?
        .trim()
        .trim_matches('/');
    let mut segments = trimmed.split('/');
    let wallet_id = segments.next()?.trim();
    let literal = segments.next()?.trim();
    let refund_id = segments.next()?.trim();
    if literal != "refunds"
        || wallet_id.is_empty()
        || refund_id.is_empty()
        || wallet_id.contains('/')
        || refund_id.contains('/')
        || segments.next().is_some()
    {
        return None;
    }
    Some((wallet_id.to_string(), refund_id.to_string()))
}

pub(super) fn parse_admin_wallets_limit(query: Option<&str>) -> Result<usize, String> {
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

pub(super) fn parse_admin_wallets_offset(query: Option<&str>) -> Result<usize, String> {
    match query_param_value(query, "offset") {
        Some(value) => value
            .parse::<usize>()
            .map_err(|_| "offset must be a non-negative integer".to_string()),
        None => Ok(0),
    }
}

pub(super) fn parse_admin_wallets_owner_type_filter(query: Option<&str>) -> Option<String> {
    match query_param_value(query, "owner_type") {
        Some(value) if value.eq_ignore_ascii_case("user") => Some("user".to_string()),
        Some(value) if value.eq_ignore_ascii_case("api_key") => Some("api_key".to_string()),
        _ => None,
    }
}

pub(super) fn wallet_owner_summary_from_fields(
    user_id: Option<&str>,
    user_name: Option<String>,
    api_key_id: Option<&str>,
    api_key_name: Option<String>,
) -> AdminWalletOwnerSummary {
    if user_id.is_some() {
        return AdminWalletOwnerSummary {
            owner_type: "user",
            owner_name: user_name,
        };
    }
    if let Some(api_key_id) = api_key_id {
        return AdminWalletOwnerSummary {
            owner_type: "api_key",
            owner_name: api_key_name
                .filter(|value| !value.trim().is_empty())
                .or_else(|| Some(format!("Key-{}", &api_key_id[..api_key_id.len().min(8)]))),
        };
    }
    AdminWalletOwnerSummary {
        owner_type: "orphaned",
        owner_name: None,
    }
}

pub(super) fn optional_epoch_value(
    row: &sqlx::postgres::PgRow,
    key: &str,
) -> Result<Option<String>, GatewayError> {
    Ok(row
        .try_get::<Option<i64>, _>(key)
        .map_err(|err| GatewayError::Internal(err.to_string()))?
        .and_then(|value| u64::try_from(value).ok())
        .and_then(unix_secs_to_rfc3339))
}

#[derive(Clone)]
pub(super) struct AdminWalletOwnerSummary {
    pub(super) owner_type: &'static str,
    pub(super) owner_name: Option<String>,
}

pub(super) async fn resolve_admin_wallet_owner_summary(
    state: &AppState,
    wallet: &aether_data::repository::wallet::StoredWalletSnapshot,
) -> Result<AdminWalletOwnerSummary, GatewayError> {
    if let Some(user_id) = wallet.user_id.as_deref() {
        let user = state.find_user_auth_by_id(user_id).await?;
        Ok(AdminWalletOwnerSummary {
            owner_type: "user",
            owner_name: user.map(|record| record.username),
        })
    } else if let Some(api_key_id) = wallet.api_key_id.as_deref() {
        let api_key_ids = vec![api_key_id.to_string()];
        let snapshots = state
            .read_auth_api_key_snapshots_by_ids(&api_key_ids)
            .await?;
        let owner_name = snapshots
            .into_iter()
            .find(|snapshot| snapshot.api_key_id == api_key_id)
            .and_then(|snapshot| snapshot.api_key_name)
            .filter(|value| !value.trim().is_empty())
            .or_else(|| Some(format!("Key-{}", &api_key_id[..api_key_id.len().min(8)])));
        Ok(AdminWalletOwnerSummary {
            owner_type: "api_key",
            owner_name,
        })
    } else {
        Ok(AdminWalletOwnerSummary {
            owner_type: "orphaned",
            owner_name: None,
        })
    }
}

pub(super) fn build_admin_wallet_summary_payload(
    wallet: &aether_data::repository::wallet::StoredWalletSnapshot,
    owner: &AdminWalletOwnerSummary,
) -> serde_json::Value {
    json!({
        "id": wallet.id.clone(),
        "user_id": wallet.user_id.clone(),
        "api_key_id": wallet.api_key_id.clone(),
        "owner_type": owner.owner_type,
        "owner_name": owner.owner_name.clone(),
        "balance": wallet.balance + wallet.gift_balance,
        "recharge_balance": wallet.balance,
        "gift_balance": wallet.gift_balance,
        "refundable_balance": wallet.balance,
        "currency": wallet.currency.clone(),
        "status": wallet.status.clone(),
        "limit_mode": wallet.limit_mode.clone(),
        "unlimited": wallet.limit_mode.eq_ignore_ascii_case("unlimited"),
        "total_recharged": wallet.total_recharged,
        "total_consumed": wallet.total_consumed,
        "total_refunded": wallet.total_refunded,
        "total_adjusted": wallet.total_adjusted,
        "created_at": serde_json::Value::Null,
        "updated_at": unix_secs_to_rfc3339(wallet.updated_at_unix_secs),
    })
}

pub(super) fn build_admin_wallet_refund_payload(
    wallet: &aether_data::repository::wallet::StoredWalletSnapshot,
    owner: &AdminWalletOwnerSummary,
    refund: &crate::gateway::AdminWalletRefundRecord,
) -> serde_json::Value {
    json!({
        "id": refund.id.clone(),
        "refund_no": refund.refund_no.clone(),
        "wallet_id": refund.wallet_id.clone(),
        "owner_type": owner.owner_type,
        "owner_name": owner.owner_name.clone(),
        "wallet_status": wallet.status.clone(),
        "user_id": refund.user_id.clone(),
        "payment_order_id": refund.payment_order_id.clone(),
        "source_type": refund.source_type.clone(),
        "source_id": refund.source_id.clone(),
        "refund_mode": refund.refund_mode.clone(),
        "amount_usd": refund.amount_usd,
        "status": refund.status.clone(),
        "reason": refund.reason.clone(),
        "failure_reason": refund.failure_reason.clone(),
        "gateway_refund_id": refund.gateway_refund_id.clone(),
        "payout_method": refund.payout_method.clone(),
        "payout_reference": refund.payout_reference.clone(),
        "payout_proof": refund.payout_proof.clone(),
        "requested_by": refund.requested_by.clone(),
        "approved_by": refund.approved_by.clone(),
        "processed_by": refund.processed_by.clone(),
        "created_at": unix_secs_to_rfc3339(refund.created_at_unix_secs),
        "updated_at": unix_secs_to_rfc3339(refund.updated_at_unix_secs),
        "processed_at": refund.processed_at_unix_secs.and_then(unix_secs_to_rfc3339),
        "completed_at": refund.completed_at_unix_secs.and_then(unix_secs_to_rfc3339),
    })
}
