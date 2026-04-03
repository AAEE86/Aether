use tracing::info;

use crate::gateway::gateway_data::GatewayDataState;
use crate::gateway::{AppState, GatewayError};

use super::{
    cleanup_audit_logs_once, cleanup_expired_gemini_file_mappings_once,
    cleanup_request_candidates_once, cleanup_stale_pending_requests_once,
    perform_db_maintenance_once, perform_provider_checkin_once, perform_stats_aggregation_once,
    perform_stats_hourly_aggregation_once, perform_usage_cleanup_once,
    perform_wallet_daily_usage_aggregation_once, summarize_postgres_pool,
};

pub(super) async fn run_audit_cleanup_once(
    data: &GatewayDataState,
) -> Result<(), aether_data::DataLayerError> {
    let deleted = cleanup_audit_logs_once(data).await?;
    if deleted > 0 {
        info!(deleted, "gateway deleted expired audit logs");
    }
    Ok(())
}

pub(super) async fn run_gemini_file_mapping_cleanup_once(
    data: &GatewayDataState,
) -> Result<(), aether_data::DataLayerError> {
    let deleted = cleanup_expired_gemini_file_mappings_once(data).await?;
    if deleted > 0 {
        info!(deleted, "gateway deleted expired gemini file mappings");
    }
    Ok(())
}

pub(super) async fn run_db_maintenance_once(
    data: &GatewayDataState,
) -> Result<(), aether_data::DataLayerError> {
    let summary = perform_db_maintenance_once(data).await?;
    if summary.attempted > 0 {
        info!(
            attempted = summary.attempted,
            succeeded = summary.succeeded,
            failed = summary.attempted.saturating_sub(summary.succeeded),
            "gateway finished db maintenance"
        );
    }
    Ok(())
}

pub(super) async fn run_wallet_daily_usage_aggregation_once(
    data: &GatewayDataState,
) -> Result<(), aether_data::DataLayerError> {
    let summary = perform_wallet_daily_usage_aggregation_once(data).await?;
    info!(
        billing_date = %summary.billing_date,
        billing_timezone = %summary.billing_timezone,
        wallets = summary.aggregated_wallets,
        stale_deleted = summary.deleted_stale_ledgers,
        "gateway aggregated wallet daily usage ledgers"
    );
    Ok(())
}

pub(super) async fn run_stats_aggregation_once(
    data: &GatewayDataState,
) -> Result<(), aether_data::DataLayerError> {
    let Some(summary) = perform_stats_aggregation_once(data).await? else {
        return Ok(());
    };

    info!(
        day_start_utc = %summary.day_start_utc,
        total_requests = summary.total_requests,
        model_rows = summary.model_rows,
        provider_rows = summary.provider_rows,
        api_key_rows = summary.api_key_rows,
        error_rows = summary.error_rows,
        user_rows = summary.user_rows,
        "gateway aggregated daily stats tables"
    );
    Ok(())
}

pub(super) async fn run_usage_cleanup_once(
    data: &GatewayDataState,
) -> Result<(), aether_data::DataLayerError> {
    let summary = perform_usage_cleanup_once(data).await?;
    if summary.body_compressed > 0
        || summary.body_cleaned > 0
        || summary.header_cleaned > 0
        || summary.keys_cleaned > 0
        || summary.records_deleted > 0
    {
        info!(
            body_compressed = summary.body_compressed,
            body_cleaned = summary.body_cleaned,
            header_cleaned = summary.header_cleaned,
            keys_cleaned = summary.keys_cleaned,
            records_deleted = summary.records_deleted,
            "gateway finished usage cleanup"
        );
    }
    Ok(())
}

pub(super) fn run_pool_monitor_once(data: &GatewayDataState) {
    let Some(summary) = summarize_postgres_pool(data) else {
        return;
    };

    info!(
        checked_out = summary.checked_out,
        pool_size = summary.pool_size,
        idle = summary.idle,
        max_connections = summary.max_connections,
        usage_rate = summary.usage_rate,
        "gateway postgres pool status"
    );
}

pub(super) async fn run_pending_cleanup_once(
    data: &GatewayDataState,
) -> Result<(), aether_data::DataLayerError> {
    let summary = cleanup_stale_pending_requests_once(data).await?;
    if summary.failed > 0 || summary.recovered > 0 {
        info!(
            failed = summary.failed,
            recovered = summary.recovered,
            "gateway cleaned stale pending and streaming requests"
        );
    }
    Ok(())
}

pub(super) async fn run_stats_hourly_aggregation_once(
    data: &GatewayDataState,
) -> Result<(), aether_data::DataLayerError> {
    let Some(summary) = perform_stats_hourly_aggregation_once(data).await? else {
        return Ok(());
    };

    info!(
        hour_utc = %summary.hour_utc,
        total_requests = summary.total_requests,
        user_rows = summary.user_rows,
        model_rows = summary.model_rows,
        provider_rows = summary.provider_rows,
        "gateway aggregated stats hourly tables"
    );
    Ok(())
}

pub(super) async fn run_provider_checkin_once(state: &AppState) -> Result<(), GatewayError> {
    let summary = perform_provider_checkin_once(state).await?;
    if summary.attempted > 0 {
        info!(
            attempted = summary.attempted,
            succeeded = summary.succeeded,
            failed = summary.failed,
            skipped = summary.skipped,
            "gateway finished provider checkin"
        );
    }
    Ok(())
}
