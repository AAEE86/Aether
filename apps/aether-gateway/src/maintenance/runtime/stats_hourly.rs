use chrono::{DateTime, Utc};
use sqlx::Row;
use uuid::Uuid;

use crate::gateway::gateway_data::GatewayDataState;

use super::{
    stats_hourly_aggregation_target_hour, system_config_bool, SELECT_STATS_HOURLY_AGGREGATE_SQL,
    SELECT_STATS_HOURLY_MODEL_AGGREGATES_SQL, SELECT_STATS_HOURLY_PROVIDER_AGGREGATES_SQL,
    SELECT_STATS_HOURLY_USER_AGGREGATES_SQL, UPSERT_STATS_HOURLY_MODEL_SQL,
    UPSERT_STATS_HOURLY_PROVIDER_SQL, UPSERT_STATS_HOURLY_SQL, UPSERT_STATS_HOURLY_USER_SQL,
};

#[derive(Debug, Clone, PartialEq)]
pub(super) struct StatsHourlyAggregationSummary {
    pub(super) hour_utc: DateTime<Utc>,
    pub(super) total_requests: i64,
    pub(super) user_rows: usize,
    pub(super) model_rows: usize,
    pub(super) provider_rows: usize,
}

pub(super) async fn perform_stats_hourly_aggregation_once(
    data: &GatewayDataState,
) -> Result<Option<StatsHourlyAggregationSummary>, aether_data::DataLayerError> {
    let Some(pool) = data.postgres_pool() else {
        return Ok(None);
    };
    if !system_config_bool(data, "enable_stats_aggregation", true).await? {
        return Ok(None);
    }

    let now_utc = Utc::now();
    let hour_utc = stats_hourly_aggregation_target_hour(now_utc);
    let hour_end = hour_utc + chrono::Duration::hours(1);
    let aggregated_at = now_utc;
    let mut tx = pool.begin().await?;

    let row = sqlx::query(SELECT_STATS_HOURLY_AGGREGATE_SQL)
        .bind(hour_utc)
        .bind(hour_end)
        .fetch_one(&mut *tx)
        .await?;
    let total_requests = row.try_get::<i64, _>("total_requests")?;
    let error_requests = row.try_get::<i64, _>("error_requests")?;
    let success_requests = total_requests.saturating_sub(error_requests);
    sqlx::query(UPSERT_STATS_HOURLY_SQL)
        .bind(Uuid::new_v4().to_string())
        .bind(hour_utc)
        .bind(total_requests)
        .bind(success_requests)
        .bind(error_requests)
        .bind(row.try_get::<i64, _>("input_tokens")?)
        .bind(row.try_get::<i64, _>("output_tokens")?)
        .bind(row.try_get::<i64, _>("cache_creation_tokens")?)
        .bind(row.try_get::<i64, _>("cache_read_tokens")?)
        .bind(row.try_get::<f64, _>("total_cost")?)
        .bind(row.try_get::<f64, _>("actual_total_cost")?)
        .bind(row.try_get::<f64, _>("avg_response_time_ms")?)
        .bind(true)
        .bind(aggregated_at)
        .bind(aggregated_at)
        .bind(aggregated_at)
        .execute(&mut *tx)
        .await?;

    let user_rows =
        upsert_stats_hourly_user_rows(&mut tx, hour_utc, hour_end, aggregated_at).await?;
    let model_rows =
        upsert_stats_hourly_model_rows(&mut tx, hour_utc, hour_end, aggregated_at).await?;
    let provider_rows =
        upsert_stats_hourly_provider_rows(&mut tx, hour_utc, hour_end, aggregated_at).await?;
    tx.commit().await?;

    Ok(Some(StatsHourlyAggregationSummary {
        hour_utc,
        total_requests,
        user_rows,
        model_rows,
        provider_rows,
    }))
}

async fn upsert_stats_hourly_user_rows(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    hour_utc: DateTime<Utc>,
    hour_end: DateTime<Utc>,
    now_utc: DateTime<Utc>,
) -> Result<usize, sqlx::Error> {
    let rows = sqlx::query(SELECT_STATS_HOURLY_USER_AGGREGATES_SQL)
        .bind(hour_utc)
        .bind(hour_end)
        .fetch_all(&mut **tx)
        .await?;

    for row in &rows {
        let user_id = row.try_get::<String, _>("user_id")?;
        let total_requests = row.try_get::<i64, _>("total_requests")?;
        let error_requests = row.try_get::<i64, _>("error_requests")?;
        let success_requests = total_requests.saturating_sub(error_requests);
        sqlx::query(UPSERT_STATS_HOURLY_USER_SQL)
            .bind(Uuid::new_v4().to_string())
            .bind(hour_utc)
            .bind(user_id)
            .bind(total_requests)
            .bind(success_requests)
            .bind(error_requests)
            .bind(row.try_get::<i64, _>("input_tokens")?)
            .bind(row.try_get::<i64, _>("output_tokens")?)
            .bind(row.try_get::<f64, _>("total_cost")?)
            .bind(now_utc)
            .bind(now_utc)
            .execute(&mut **tx)
            .await?;
    }

    Ok(rows.len())
}

async fn upsert_stats_hourly_model_rows(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    hour_utc: DateTime<Utc>,
    hour_end: DateTime<Utc>,
    now_utc: DateTime<Utc>,
) -> Result<usize, sqlx::Error> {
    let rows = sqlx::query(SELECT_STATS_HOURLY_MODEL_AGGREGATES_SQL)
        .bind(hour_utc)
        .bind(hour_end)
        .fetch_all(&mut **tx)
        .await?;
    let mut inserted = 0usize;

    for row in &rows {
        let model = row.try_get::<Option<String>, _>("model")?;
        let Some(model) = model.filter(|value| !value.is_empty()) else {
            continue;
        };
        sqlx::query(UPSERT_STATS_HOURLY_MODEL_SQL)
            .bind(Uuid::new_v4().to_string())
            .bind(hour_utc)
            .bind(model)
            .bind(row.try_get::<i64, _>("total_requests")?)
            .bind(row.try_get::<i64, _>("input_tokens")?)
            .bind(row.try_get::<i64, _>("output_tokens")?)
            .bind(row.try_get::<f64, _>("total_cost")?)
            .bind(row.try_get::<f64, _>("avg_response_time_ms")?)
            .bind(now_utc)
            .bind(now_utc)
            .execute(&mut **tx)
            .await?;
        inserted += 1;
    }

    Ok(inserted)
}

async fn upsert_stats_hourly_provider_rows(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    hour_utc: DateTime<Utc>,
    hour_end: DateTime<Utc>,
    now_utc: DateTime<Utc>,
) -> Result<usize, sqlx::Error> {
    let rows = sqlx::query(SELECT_STATS_HOURLY_PROVIDER_AGGREGATES_SQL)
        .bind(hour_utc)
        .bind(hour_end)
        .fetch_all(&mut **tx)
        .await?;
    let mut inserted = 0usize;

    for row in &rows {
        let provider_name = row.try_get::<Option<String>, _>("provider_name")?;
        let Some(provider_name) = provider_name.filter(|value| !value.is_empty()) else {
            continue;
        };
        sqlx::query(UPSERT_STATS_HOURLY_PROVIDER_SQL)
            .bind(Uuid::new_v4().to_string())
            .bind(hour_utc)
            .bind(provider_name)
            .bind(row.try_get::<i64, _>("total_requests")?)
            .bind(row.try_get::<i64, _>("input_tokens")?)
            .bind(row.try_get::<i64, _>("output_tokens")?)
            .bind(row.try_get::<f64, _>("total_cost")?)
            .bind(now_utc)
            .bind(now_utc)
            .execute(&mut **tx)
            .await?;
        inserted += 1;
    }

    Ok(inserted)
}
