use sqlx::Row;

use super::{
    AdminBillingCollectorRecord, AdminBillingRuleRecord, AdminWalletPaymentOrderRecord,
    AdminWalletRefundRecord, GatewayError,
};

pub(super) fn admin_wallet_build_order_no(now: chrono::DateTime<chrono::Utc>) -> String {
    format!(
        "po_{}_{}",
        now.format("%Y%m%d%H%M%S%6f"),
        &uuid::Uuid::new_v4().simple().to_string()[..12]
    )
}

pub(super) fn admin_payment_gateway_response_map(
    value: Option<serde_json::Value>,
) -> serde_json::Map<String, serde_json::Value> {
    match value {
        Some(serde_json::Value::Object(map)) => map,
        _ => serde_json::Map::new(),
    }
}

pub(super) fn admin_wallet_snapshot_from_row(
    row: &sqlx::postgres::PgRow,
) -> Result<aether_data::repository::wallet::StoredWalletSnapshot, GatewayError> {
    aether_data::repository::wallet::StoredWalletSnapshot::new(
        row.try_get("id")
            .map_err(|err| GatewayError::Internal(err.to_string()))?,
        row.try_get("user_id")
            .map_err(|err| GatewayError::Internal(err.to_string()))?,
        row.try_get("api_key_id")
            .map_err(|err| GatewayError::Internal(err.to_string()))?,
        row.try_get("balance")
            .map_err(|err| GatewayError::Internal(err.to_string()))?,
        row.try_get("gift_balance")
            .map_err(|err| GatewayError::Internal(err.to_string()))?,
        row.try_get("limit_mode")
            .map_err(|err| GatewayError::Internal(err.to_string()))?,
        row.try_get("currency")
            .map_err(|err| GatewayError::Internal(err.to_string()))?,
        row.try_get("status")
            .map_err(|err| GatewayError::Internal(err.to_string()))?,
        row.try_get("total_recharged")
            .map_err(|err| GatewayError::Internal(err.to_string()))?,
        row.try_get("total_consumed")
            .map_err(|err| GatewayError::Internal(err.to_string()))?,
        row.try_get("total_refunded")
            .map_err(|err| GatewayError::Internal(err.to_string()))?,
        row.try_get("total_adjusted")
            .map_err(|err| GatewayError::Internal(err.to_string()))?,
        row.try_get("updated_at_unix_secs")
            .map_err(|err| GatewayError::Internal(err.to_string()))?,
    )
    .map_err(|err| GatewayError::Internal(err.to_string()))
}

pub(super) fn admin_wallet_payment_order_from_row(
    row: &sqlx::postgres::PgRow,
) -> Result<AdminWalletPaymentOrderRecord, GatewayError> {
    Ok(AdminWalletPaymentOrderRecord {
        id: row
            .try_get("id")
            .map_err(|err| GatewayError::Internal(err.to_string()))?,
        order_no: row
            .try_get("order_no")
            .map_err(|err| GatewayError::Internal(err.to_string()))?,
        wallet_id: row
            .try_get("wallet_id")
            .map_err(|err| GatewayError::Internal(err.to_string()))?,
        user_id: row
            .try_get("user_id")
            .map_err(|err| GatewayError::Internal(err.to_string()))?,
        amount_usd: row
            .try_get("amount_usd")
            .map_err(|err| GatewayError::Internal(err.to_string()))?,
        pay_amount: row
            .try_get("pay_amount")
            .map_err(|err| GatewayError::Internal(err.to_string()))?,
        pay_currency: row
            .try_get("pay_currency")
            .map_err(|err| GatewayError::Internal(err.to_string()))?,
        exchange_rate: row
            .try_get("exchange_rate")
            .map_err(|err| GatewayError::Internal(err.to_string()))?,
        refunded_amount_usd: row
            .try_get("refunded_amount_usd")
            .map_err(|err| GatewayError::Internal(err.to_string()))?,
        refundable_amount_usd: row
            .try_get("refundable_amount_usd")
            .map_err(|err| GatewayError::Internal(err.to_string()))?,
        payment_method: row
            .try_get("payment_method")
            .map_err(|err| GatewayError::Internal(err.to_string()))?,
        gateway_order_id: row
            .try_get("gateway_order_id")
            .map_err(|err| GatewayError::Internal(err.to_string()))?,
        status: row
            .try_get("status")
            .map_err(|err| GatewayError::Internal(err.to_string()))?,
        gateway_response: row
            .try_get("gateway_response")
            .map_err(|err| GatewayError::Internal(err.to_string()))?,
        created_at_unix_secs: row
            .try_get::<i64, _>("created_at_unix_secs")
            .map_err(|err| GatewayError::Internal(err.to_string()))?
            .max(0) as u64,
        paid_at_unix_secs: row
            .try_get::<Option<i64>, _>("paid_at_unix_secs")
            .map_err(|err| GatewayError::Internal(err.to_string()))?
            .map(|value| value.max(0) as u64),
        credited_at_unix_secs: row
            .try_get::<Option<i64>, _>("credited_at_unix_secs")
            .map_err(|err| GatewayError::Internal(err.to_string()))?
            .map(|value| value.max(0) as u64),
        expires_at_unix_secs: row
            .try_get::<Option<i64>, _>("expires_at_unix_secs")
            .map_err(|err| GatewayError::Internal(err.to_string()))?
            .map(|value| value.max(0) as u64),
    })
}

pub(super) fn admin_wallet_refund_from_row(
    row: &sqlx::postgres::PgRow,
) -> Result<AdminWalletRefundRecord, GatewayError> {
    Ok(AdminWalletRefundRecord {
        id: row
            .try_get("id")
            .map_err(|err| GatewayError::Internal(err.to_string()))?,
        refund_no: row
            .try_get("refund_no")
            .map_err(|err| GatewayError::Internal(err.to_string()))?,
        wallet_id: row
            .try_get("wallet_id")
            .map_err(|err| GatewayError::Internal(err.to_string()))?,
        user_id: row
            .try_get("user_id")
            .map_err(|err| GatewayError::Internal(err.to_string()))?,
        payment_order_id: row
            .try_get("payment_order_id")
            .map_err(|err| GatewayError::Internal(err.to_string()))?,
        source_type: row
            .try_get("source_type")
            .map_err(|err| GatewayError::Internal(err.to_string()))?,
        source_id: row
            .try_get("source_id")
            .map_err(|err| GatewayError::Internal(err.to_string()))?,
        refund_mode: row
            .try_get("refund_mode")
            .map_err(|err| GatewayError::Internal(err.to_string()))?,
        amount_usd: row
            .try_get("amount_usd")
            .map_err(|err| GatewayError::Internal(err.to_string()))?,
        status: row
            .try_get("status")
            .map_err(|err| GatewayError::Internal(err.to_string()))?,
        reason: row
            .try_get("reason")
            .map_err(|err| GatewayError::Internal(err.to_string()))?,
        failure_reason: row
            .try_get("failure_reason")
            .map_err(|err| GatewayError::Internal(err.to_string()))?,
        gateway_refund_id: row
            .try_get("gateway_refund_id")
            .map_err(|err| GatewayError::Internal(err.to_string()))?,
        payout_method: row
            .try_get("payout_method")
            .map_err(|err| GatewayError::Internal(err.to_string()))?,
        payout_reference: row
            .try_get("payout_reference")
            .map_err(|err| GatewayError::Internal(err.to_string()))?,
        payout_proof: row
            .try_get("payout_proof")
            .map_err(|err| GatewayError::Internal(err.to_string()))?,
        requested_by: row
            .try_get("requested_by")
            .map_err(|err| GatewayError::Internal(err.to_string()))?,
        approved_by: row
            .try_get("approved_by")
            .map_err(|err| GatewayError::Internal(err.to_string()))?,
        processed_by: row
            .try_get("processed_by")
            .map_err(|err| GatewayError::Internal(err.to_string()))?,
        created_at_unix_secs: row
            .try_get::<i64, _>("created_at_unix_secs")
            .map_err(|err| GatewayError::Internal(err.to_string()))?
            .max(0) as u64,
        updated_at_unix_secs: row
            .try_get::<i64, _>("updated_at_unix_secs")
            .map_err(|err| GatewayError::Internal(err.to_string()))?
            .max(0) as u64,
        processed_at_unix_secs: row
            .try_get::<Option<i64>, _>("processed_at_unix_secs")
            .map_err(|err| GatewayError::Internal(err.to_string()))?
            .map(|value| value.max(0) as u64),
        completed_at_unix_secs: row
            .try_get::<Option<i64>, _>("completed_at_unix_secs")
            .map_err(|err| GatewayError::Internal(err.to_string()))?
            .map(|value| value.max(0) as u64),
    })
}

pub(super) fn admin_billing_rule_from_row(
    row: &sqlx::postgres::PgRow,
) -> Result<AdminBillingRuleRecord, GatewayError> {
    Ok(AdminBillingRuleRecord {
        id: row
            .try_get("id")
            .map_err(|err| GatewayError::Internal(err.to_string()))?,
        name: row
            .try_get("name")
            .map_err(|err| GatewayError::Internal(err.to_string()))?,
        task_type: row
            .try_get("task_type")
            .map_err(|err| GatewayError::Internal(err.to_string()))?,
        global_model_id: row
            .try_get("global_model_id")
            .map_err(|err| GatewayError::Internal(err.to_string()))?,
        model_id: row
            .try_get("model_id")
            .map_err(|err| GatewayError::Internal(err.to_string()))?,
        expression: row
            .try_get("expression")
            .map_err(|err| GatewayError::Internal(err.to_string()))?,
        variables: row
            .try_get::<Option<serde_json::Value>, _>("variables")
            .map_err(|err| GatewayError::Internal(err.to_string()))?
            .unwrap_or_else(|| serde_json::json!({})),
        dimension_mappings: row
            .try_get::<Option<serde_json::Value>, _>("dimension_mappings")
            .map_err(|err| GatewayError::Internal(err.to_string()))?
            .unwrap_or_else(|| serde_json::json!({})),
        is_enabled: row
            .try_get("is_enabled")
            .map_err(|err| GatewayError::Internal(err.to_string()))?,
        created_at_unix_secs: row
            .try_get::<i64, _>("created_at_unix_secs")
            .map_err(|err| GatewayError::Internal(err.to_string()))?
            .max(0) as u64,
        updated_at_unix_secs: row
            .try_get::<i64, _>("updated_at_unix_secs")
            .map_err(|err| GatewayError::Internal(err.to_string()))?
            .max(0) as u64,
    })
}

pub(super) fn admin_billing_collector_from_row(
    row: &sqlx::postgres::PgRow,
) -> Result<AdminBillingCollectorRecord, GatewayError> {
    Ok(AdminBillingCollectorRecord {
        id: row
            .try_get("id")
            .map_err(|err| GatewayError::Internal(err.to_string()))?,
        api_format: row
            .try_get("api_format")
            .map_err(|err| GatewayError::Internal(err.to_string()))?,
        task_type: row
            .try_get("task_type")
            .map_err(|err| GatewayError::Internal(err.to_string()))?,
        dimension_name: row
            .try_get("dimension_name")
            .map_err(|err| GatewayError::Internal(err.to_string()))?,
        source_type: row
            .try_get("source_type")
            .map_err(|err| GatewayError::Internal(err.to_string()))?,
        source_path: row
            .try_get("source_path")
            .map_err(|err| GatewayError::Internal(err.to_string()))?,
        value_type: row
            .try_get("value_type")
            .map_err(|err| GatewayError::Internal(err.to_string()))?,
        transform_expression: row
            .try_get("transform_expression")
            .map_err(|err| GatewayError::Internal(err.to_string()))?,
        default_value: row
            .try_get("default_value")
            .map_err(|err| GatewayError::Internal(err.to_string()))?,
        priority: row
            .try_get("priority")
            .map_err(|err| GatewayError::Internal(err.to_string()))?,
        is_enabled: row
            .try_get("is_enabled")
            .map_err(|err| GatewayError::Internal(err.to_string()))?,
        created_at_unix_secs: row
            .try_get::<i64, _>("created_at_unix_secs")
            .map_err(|err| GatewayError::Internal(err.to_string()))?
            .max(0) as u64,
        updated_at_unix_secs: row
            .try_get::<i64, _>("updated_at_unix_secs")
            .map_err(|err| GatewayError::Internal(err.to_string()))?
            .max(0) as u64,
    })
}
