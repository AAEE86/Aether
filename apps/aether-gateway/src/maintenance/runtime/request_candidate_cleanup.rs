use tracing::info;

use crate::gateway::gateway_data::GatewayDataState;

use super::{now_unix_secs, system_config_bool, system_config_u64, system_config_usize};

pub(crate) async fn cleanup_request_candidates_once(
    data: &GatewayDataState,
) -> Result<usize, aether_data::DataLayerError> {
    if !system_config_bool(data, "enable_auto_cleanup", true).await? {
        return Ok(0);
    }

    let detail_log_retention_days = system_config_u64(data, "detail_log_retention_days", 7).await?;
    let retention_days = system_config_u64(
        data,
        "request_candidates_retention_days",
        detail_log_retention_days,
    )
    .await?
    .max(3);
    let cleanup_batch_size = system_config_usize(data, "cleanup_batch_size", 1_000).await?;
    let delete_limit = system_config_usize(
        data,
        "request_candidates_cleanup_batch_size",
        cleanup_batch_size.max(1),
    )
    .await?
    .max(1);
    let cutoff_unix_secs = now_unix_secs().saturating_sub(retention_days.saturating_mul(86_400));

    let mut total_deleted = 0usize;
    loop {
        let deleted = data
            .delete_request_candidates_created_before(cutoff_unix_secs, delete_limit)
            .await?;
        total_deleted += deleted;
        if deleted < delete_limit {
            break;
        }
    }

    Ok(total_deleted)
}

pub(super) async fn run_request_candidate_cleanup_once(
    data: &GatewayDataState,
) -> Result<(), aether_data::DataLayerError> {
    let deleted = cleanup_request_candidates_once(data).await?;
    if deleted > 0 {
        info!(deleted, "gateway deleted expired request candidates");
    }
    Ok(())
}
