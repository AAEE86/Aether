use chrono::{DateTime, Utc};
use sqlx::Row;
use uuid::Uuid;

use crate::gateway::gateway_data::GatewayDataState;

use super::{
    maintenance_timezone, wallet_daily_usage_aggregation_target,
    DELETE_STALE_WALLET_DAILY_USAGE_LEDGERS_SQL, SELECT_WALLET_DAILY_USAGE_AGGREGATION_ROWS_SQL,
    UPSERT_WALLET_DAILY_USAGE_LEDGER_SQL,
};

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct WalletDailyUsageAggregationSummary {
    pub(crate) billing_date: chrono::NaiveDate,
    pub(crate) billing_timezone: String,
    pub(crate) aggregated_wallets: usize,
    pub(crate) deleted_stale_ledgers: usize,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct WalletDailyUsageAggregationTarget {
    pub(super) billing_date: chrono::NaiveDate,
    pub(super) billing_timezone: String,
    pub(super) window_start_utc: DateTime<Utc>,
    pub(super) window_end_utc: DateTime<Utc>,
}

pub(super) async fn perform_wallet_daily_usage_aggregation_once(
    data: &GatewayDataState,
) -> Result<WalletDailyUsageAggregationSummary, aether_data::DataLayerError> {
    let timezone = maintenance_timezone();
    let now_utc = Utc::now();
    let target = wallet_daily_usage_aggregation_target(now_utc, timezone);
    let Some(pool) = data.postgres_pool() else {
        return Ok(WalletDailyUsageAggregationSummary {
            billing_date: target.billing_date,
            billing_timezone: target.billing_timezone,
            aggregated_wallets: 0,
            deleted_stale_ledgers: 0,
        });
    };

    let mut tx = pool.begin().await?;
    let rows = sqlx::query(SELECT_WALLET_DAILY_USAGE_AGGREGATION_ROWS_SQL)
        .bind(target.window_start_utc)
        .bind(target.window_end_utc)
        .fetch_all(&mut *tx)
        .await?;
    for row in &rows {
        sqlx::query(UPSERT_WALLET_DAILY_USAGE_LEDGER_SQL)
            .bind(Uuid::new_v4().to_string())
            .bind(row.try_get::<String, _>("wallet_id")?)
            .bind(target.billing_date)
            .bind(target.billing_timezone.as_str())
            .bind(row.try_get::<f64, _>("total_cost_usd")?)
            .bind(row.try_get::<i64, _>("total_requests")?)
            .bind(row.try_get::<i64, _>("input_tokens")?)
            .bind(row.try_get::<i64, _>("output_tokens")?)
            .bind(row.try_get::<i64, _>("cache_creation_tokens")?)
            .bind(row.try_get::<i64, _>("cache_read_tokens")?)
            .bind(row.try_get::<Option<DateTime<Utc>>, _>("first_finalized_at")?)
            .bind(row.try_get::<Option<DateTime<Utc>>, _>("last_finalized_at")?)
            .bind(now_utc)
            .bind(now_utc)
            .bind(now_utc)
            .execute(&mut *tx)
            .await?;
    }

    let deleted_stale_ledgers = sqlx::query(DELETE_STALE_WALLET_DAILY_USAGE_LEDGERS_SQL)
        .bind(target.billing_date)
        .bind(target.billing_timezone.as_str())
        .bind(target.window_start_utc)
        .bind(target.window_end_utc)
        .execute(&mut *tx)
        .await?
        .rows_affected();
    tx.commit().await?;

    Ok(WalletDailyUsageAggregationSummary {
        billing_date: target.billing_date,
        billing_timezone: target.billing_timezone,
        aggregated_wallets: rows.len(),
        deleted_stale_ledgers: usize::try_from(deleted_stale_ledgers).unwrap_or(usize::MAX),
    })
}
