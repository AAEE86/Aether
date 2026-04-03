use chrono::{DateTime, Utc};

use crate::gateway::gateway_data::GatewayDataState;

use super::{
    system_config_bool, system_config_u64, system_config_usize, DELETE_AUDIT_LOGS_BEFORE_SQL,
};

pub(crate) async fn cleanup_audit_logs_once(
    data: &GatewayDataState,
) -> Result<usize, aether_data::DataLayerError> {
    cleanup_audit_logs_with(data, |cutoff_time, delete_limit| async move {
        let Some(pool) = data.postgres_pool() else {
            return Ok(0);
        };
        let deleted = sqlx::query(DELETE_AUDIT_LOGS_BEFORE_SQL)
            .bind(cutoff_time)
            .bind(i64::try_from(delete_limit).unwrap_or(i64::MAX))
            .execute(&pool)
            .await?
            .rows_affected();
        Ok(usize::try_from(deleted).unwrap_or(usize::MAX))
    })
    .await
}

pub(super) async fn cleanup_audit_logs_with<F, Fut>(
    data: &GatewayDataState,
    mut delete_batch: F,
) -> Result<usize, aether_data::DataLayerError>
where
    F: FnMut(DateTime<Utc>, usize) -> Fut,
    Fut: std::future::Future<Output = Result<usize, aether_data::DataLayerError>>,
{
    if !system_config_bool(data, "enable_auto_cleanup", true).await? {
        return Ok(0);
    }

    let retention_days = system_config_u64(data, "audit_log_retention_days", 30)
        .await?
        .max(7);
    let delete_limit = system_config_usize(data, "cleanup_batch_size", 1_000)
        .await?
        .max(1);
    let retention_days_i64 = i64::try_from(retention_days).unwrap_or(i64::MAX);
    let cutoff_time = Utc::now() - chrono::Duration::days(retention_days_i64);

    let mut total_deleted = 0usize;
    loop {
        let deleted = delete_batch(cutoff_time, delete_limit).await?;
        total_deleted += deleted;
        if deleted < delete_limit {
            break;
        }
    }

    Ok(total_deleted)
}
