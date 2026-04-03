use std::collections::{HashSet, VecDeque};
use std::sync::{Arc, Mutex};

use chrono::{DateTime, Utc};
use chrono_tz::Tz;
use serde_json::json;

use super::{
    cleanup_audit_logs_with, next_daily_run_after, next_db_maintenance_run_after,
    next_stats_aggregation_run_after, next_stats_hourly_aggregation_run_after,
    pending_cleanup_batch_size, pending_cleanup_timeout_minutes, plan_pending_cleanup_batch,
    provider_checkin_schedule, run_db_maintenance_with, spawn_audit_cleanup_worker,
    spawn_db_maintenance_worker, spawn_pending_cleanup_worker, spawn_pool_monitor_worker,
    spawn_provider_checkin_worker, spawn_stats_aggregation_worker,
    spawn_stats_hourly_aggregation_worker, spawn_usage_cleanup_worker,
    spawn_wallet_daily_usage_aggregation_worker, stats_aggregation_target_day,
    stats_hourly_aggregation_target_hour, summarize_postgres_pool, usage_cleanup_settings,
    usage_cleanup_window, wallet_daily_usage_aggregation_target, AppState, DbMaintenanceRunSummary,
    FailedPendingUsageRow, GatewayDataState, StalePendingUsageRow, UsageCleanupSettings,
    USAGE_CLEANUP_HOUR, USAGE_CLEANUP_MINUTE, WALLET_DAILY_USAGE_AGGREGATION_HOUR,
    WALLET_DAILY_USAGE_AGGREGATION_MINUTE,
};

#[tokio::test]
async fn spawn_audit_cleanup_worker_skips_when_postgres_unavailable() {
    assert!(spawn_audit_cleanup_worker(Arc::new(GatewayDataState::disabled())).is_none());
}

#[tokio::test]
async fn spawn_db_maintenance_worker_skips_when_postgres_unavailable() {
    assert!(spawn_db_maintenance_worker(Arc::new(GatewayDataState::disabled())).is_none());
}

#[tokio::test]
async fn spawn_pending_cleanup_worker_skips_when_postgres_unavailable() {
    assert!(spawn_pending_cleanup_worker(Arc::new(GatewayDataState::disabled())).is_none());
}

#[tokio::test]
async fn spawn_pool_monitor_worker_skips_when_postgres_unavailable() {
    assert!(spawn_pool_monitor_worker(Arc::new(GatewayDataState::disabled())).is_none());
}

#[tokio::test]
async fn spawn_stats_aggregation_worker_skips_when_postgres_unavailable() {
    assert!(spawn_stats_aggregation_worker(Arc::new(GatewayDataState::disabled())).is_none());
}

#[tokio::test]
async fn spawn_stats_hourly_aggregation_worker_skips_when_postgres_unavailable() {
    assert!(
        spawn_stats_hourly_aggregation_worker(Arc::new(GatewayDataState::disabled())).is_none()
    );
}

#[tokio::test]
async fn spawn_usage_cleanup_worker_skips_when_postgres_unavailable() {
    assert!(spawn_usage_cleanup_worker(Arc::new(GatewayDataState::disabled())).is_none());
}

#[tokio::test]
async fn spawn_wallet_daily_usage_aggregation_worker_skips_when_postgres_unavailable() {
    assert!(
        spawn_wallet_daily_usage_aggregation_worker(Arc::new(GatewayDataState::disabled()))
            .is_none()
    );
}

#[tokio::test]
async fn spawn_provider_checkin_worker_skips_when_provider_catalog_unavailable() {
    let state = AppState::new()
        .expect("gateway state should build")
        .with_data_state_for_tests(GatewayDataState::disabled());

    assert!(spawn_provider_checkin_worker(state).is_none());
}

#[tokio::test]
async fn cleanup_audit_logs_respects_auto_cleanup_toggle() {
    let data = GatewayDataState::disabled()
        .with_system_config_values_for_tests([("enable_auto_cleanup".to_string(), json!(false))]);

    let deleted = cleanup_audit_logs_with(&data, |_cutoff_time, _delete_limit| async move {
        panic!("audit cleanup should not run when auto cleanup is disabled");
        #[allow(unreachable_code)]
        Ok(0)
    })
    .await
    .expect("audit cleanup should short-circuit");

    assert_eq!(deleted, 0);
}

#[tokio::test]
async fn cleanup_audit_logs_uses_retention_and_batch_settings() {
    let data = GatewayDataState::disabled().with_system_config_values_for_tests([
        ("enable_auto_cleanup".to_string(), json!(true)),
        ("audit_log_retention_days".to_string(), json!(21)),
        ("cleanup_batch_size".to_string(), json!(2)),
    ]);
    let observed_limits = Arc::new(Mutex::new(Vec::new()));
    let observed_cutoffs = Arc::new(Mutex::new(Vec::new()));
    let batch_results = Arc::new(Mutex::new(VecDeque::from([2usize, 1usize])));
    let started_at = Utc::now();

    let deleted = cleanup_audit_logs_with(&data, {
        let observed_limits = Arc::clone(&observed_limits);
        let observed_cutoffs = Arc::clone(&observed_cutoffs);
        let batch_results = Arc::clone(&batch_results);
        move |cutoff_time, delete_limit| {
            observed_limits
                .lock()
                .expect("observed limits lock")
                .push(delete_limit);
            observed_cutoffs
                .lock()
                .expect("observed cutoffs lock")
                .push(cutoff_time);
            let next = batch_results
                .lock()
                .expect("batch results lock")
                .pop_front()
                .unwrap_or_default();
            async move { Ok(next) }
        }
    })
    .await
    .expect("audit cleanup should succeed");
    let finished_at = Utc::now();

    assert_eq!(deleted, 3);
    assert_eq!(
        *observed_limits.lock().expect("observed limits lock"),
        vec![2, 2]
    );
    let observed_cutoffs = observed_cutoffs.lock().expect("observed cutoffs lock");
    assert_eq!(observed_cutoffs.len(), 2);
    let earliest_expected = started_at - chrono::Duration::days(21);
    let latest_expected = finished_at - chrono::Duration::days(21);
    for cutoff_time in observed_cutoffs.iter() {
        assert!(*cutoff_time >= earliest_expected);
        assert!(*cutoff_time <= latest_expected);
    }
}

#[tokio::test]
async fn pending_cleanup_settings_use_timeout_and_cap_batch_size() {
    let data = GatewayDataState::disabled().with_system_config_values_for_tests([
        ("pending_request_timeout_minutes".to_string(), json!(25)),
        ("cleanup_batch_size".to_string(), json!(500)),
    ]);

    let timeout_minutes = pending_cleanup_timeout_minutes(&data)
        .await
        .expect("timeout should resolve");
    let batch_size = pending_cleanup_batch_size(&data)
        .await
        .expect("batch size should resolve");

    assert_eq!(timeout_minutes, 25);
    assert_eq!(batch_size, 200);
}

#[test]
fn pending_cleanup_plan_recovers_completed_requests_and_voids_failed_pending_billing() {
    let plan = plan_pending_cleanup_batch(
        vec![
            StalePendingUsageRow {
                id: "usage-1".to_string(),
                request_id: "req-1".to_string(),
                status: "streaming".to_string(),
                billing_status: "pending".to_string(),
            },
            StalePendingUsageRow {
                id: "usage-2".to_string(),
                request_id: "req-2".to_string(),
                status: "pending".to_string(),
                billing_status: "pending".to_string(),
            },
            StalePendingUsageRow {
                id: "usage-3".to_string(),
                request_id: "req-3".to_string(),
                status: "streaming".to_string(),
                billing_status: "settled".to_string(),
            },
        ],
        &HashSet::from(["req-1".to_string()]),
        10,
    );

    assert_eq!(plan.recovered_usage_ids, vec!["usage-1".to_string()]);
    assert_eq!(plan.recovered_request_ids, vec!["req-1".to_string()]);
    assert_eq!(
        plan.failed_request_ids,
        vec!["req-2".to_string(), "req-3".to_string()]
    );
    assert_eq!(
        plan.failed_usage_rows,
        vec![
            FailedPendingUsageRow {
                id: "usage-2".to_string(),
                error_message: "请求超时: 状态 'pending' 超过 10 分钟未完成".to_string(),
                should_void_billing: true,
            },
            FailedPendingUsageRow {
                id: "usage-3".to_string(),
                error_message: "请求超时: 状态 'streaming' 超过 10 分钟未完成".to_string(),
                should_void_billing: false,
            },
        ]
    );
}

#[tokio::test]
async fn usage_cleanup_settings_resolve_batch_and_delete_toggle() {
    let data = GatewayDataState::disabled().with_system_config_values_for_tests([
        ("detail_log_retention_days".to_string(), json!(7)),
        ("compressed_log_retention_days".to_string(), json!(30)),
        ("header_retention_days".to_string(), json!(90)),
        ("log_retention_days".to_string(), json!(365)),
        ("cleanup_batch_size".to_string(), json!(0)),
        ("auto_delete_expired_keys".to_string(), json!(true)),
    ]);

    let settings = usage_cleanup_settings(&data)
        .await
        .expect("usage cleanup settings should resolve");

    assert_eq!(
        settings,
        UsageCleanupSettings {
            detail_retention_days: 7,
            compressed_retention_days: 30,
            header_retention_days: 90,
            log_retention_days: 365,
            batch_size: 1,
            auto_delete_expired_keys: true,
        }
    );
}

#[test]
fn usage_cleanup_window_uses_non_overlapping_ranges() {
    let now_utc = "2026-03-18T03:00:00Z"
        .parse::<DateTime<Utc>>()
        .expect("timestamp should parse");
    let window = usage_cleanup_window(
        now_utc,
        UsageCleanupSettings {
            detail_retention_days: 7,
            compressed_retention_days: 30,
            header_retention_days: 90,
            log_retention_days: 365,
            batch_size: 123,
            auto_delete_expired_keys: false,
        },
    );

    assert_eq!(
        window.detail_cutoff.to_rfc3339(),
        "2026-03-11T03:00:00+00:00"
    );
    assert_eq!(
        window.compressed_cutoff.to_rfc3339(),
        "2026-02-16T03:00:00+00:00"
    );
    assert_eq!(
        window.header_cutoff.to_rfc3339(),
        "2025-12-18T03:00:00+00:00"
    );
    assert_eq!(window.log_cutoff.to_rfc3339(), "2025-03-18T03:00:00+00:00");
    assert!(window.detail_cutoff > window.compressed_cutoff);
    assert!(window.compressed_cutoff > window.log_cutoff);
}

#[tokio::test]
async fn summarize_postgres_pool_uses_busy_connections_for_usage_rate() {
    let data = GatewayDataState::from_config(
        crate::gateway::gateway_data::GatewayDataConfig::from_postgres_config(
            aether_data::postgres::PostgresPoolConfig {
                database_url: "postgres://localhost/aether".to_string(),
                min_connections: 1,
                max_connections: 8,
                acquire_timeout_ms: 1_000,
                idle_timeout_ms: 5_000,
                max_lifetime_ms: 30_000,
                statement_cache_capacity: 64,
                require_ssl: false,
            },
        ),
    )
    .expect("gateway data state should build");

    let summary = summarize_postgres_pool(&data).expect("pool summary should exist");

    assert_eq!(summary.checked_out, 0);
    assert_eq!(summary.pool_size, 0);
    assert_eq!(summary.idle, 0);
    assert_eq!(summary.max_connections, 8);
    assert_eq!(summary.usage_rate, 0.0);
}

#[test]
fn stats_aggregation_target_uses_previous_utc_day() {
    let now_utc = "2026-04-05T10:20:30Z"
        .parse::<DateTime<Utc>>()
        .expect("timestamp should parse");

    let target = stats_aggregation_target_day(now_utc);

    assert_eq!(target.to_rfc3339(), "2026-04-04T00:00:00+00:00");
}

#[test]
fn next_stats_aggregation_run_aligns_to_same_day_when_before_slot() {
    let now_utc = "2026-04-05T00:04:59Z"
        .parse::<DateTime<Utc>>()
        .expect("timestamp should parse");

    let next = next_stats_aggregation_run_after(now_utc);

    assert_eq!(next.to_rfc3339(), "2026-04-05T00:05:00+00:00");
}

#[test]
fn next_stats_aggregation_run_rolls_to_next_day_after_slot() {
    let now_utc = "2026-04-05T00:05:00Z"
        .parse::<DateTime<Utc>>()
        .expect("timestamp should parse");

    let next = next_stats_aggregation_run_after(now_utc);

    assert_eq!(next.to_rfc3339(), "2026-04-06T00:05:00+00:00");
}

#[test]
fn stats_hourly_aggregation_target_uses_previous_utc_hour() {
    let now_utc = "2026-04-05T10:20:30Z"
        .parse::<DateTime<Utc>>()
        .expect("timestamp should parse");

    let target = stats_hourly_aggregation_target_hour(now_utc);

    assert_eq!(target.to_rfc3339(), "2026-04-05T09:00:00+00:00");
}

#[test]
fn next_stats_hourly_aggregation_run_aligns_to_same_hour_when_before_slot() {
    let now_utc = "2026-04-05T10:04:59Z"
        .parse::<DateTime<Utc>>()
        .expect("timestamp should parse");

    let next = next_stats_hourly_aggregation_run_after(now_utc);

    assert_eq!(next.to_rfc3339(), "2026-04-05T10:05:00+00:00");
}

#[test]
fn next_stats_hourly_aggregation_run_rolls_to_next_hour_after_slot() {
    let now_utc = "2026-04-05T10:05:00Z"
        .parse::<DateTime<Utc>>()
        .expect("timestamp should parse");

    let next = next_stats_hourly_aggregation_run_after(now_utc);

    assert_eq!(next.to_rfc3339(), "2026-04-05T11:05:00+00:00");
}

#[tokio::test]
async fn db_maintenance_respects_enable_toggle() {
    let data = GatewayDataState::disabled()
        .with_system_config_values_for_tests([("enable_db_maintenance".to_string(), json!(false))]);

    let summary = run_db_maintenance_with(&data, |_table_name| async move {
        panic!("db maintenance should not run when disabled");
        #[allow(unreachable_code)]
        Ok(())
    })
    .await
    .expect("db maintenance should short-circuit");

    assert_eq!(
        summary,
        DbMaintenanceRunSummary {
            attempted: 0,
            succeeded: 0,
        }
    );
}

#[tokio::test]
async fn db_maintenance_continues_across_table_failures() {
    let data = GatewayDataState::disabled()
        .with_system_config_values_for_tests([("enable_db_maintenance".to_string(), json!(true))]);
    let seen_tables = Arc::new(Mutex::new(Vec::new()));

    let summary = run_db_maintenance_with(&data, {
        let seen_tables = Arc::clone(&seen_tables);
        move |table_name| {
            seen_tables
                .lock()
                .expect("seen tables lock")
                .push(table_name.to_string());
            async move {
                if table_name == "request_candidates" {
                    Err(aether_data::DataLayerError::InvalidInput(
                        "boom".to_string(),
                    ))
                } else {
                    Ok(())
                }
            }
        }
    })
    .await
    .expect("db maintenance should continue after failures");

    assert_eq!(
        summary,
        DbMaintenanceRunSummary {
            attempted: 3,
            succeeded: 2,
        }
    );
    assert_eq!(
        *seen_tables.lock().expect("seen tables lock"),
        vec![
            "usage".to_string(),
            "request_candidates".to_string(),
            "audit_logs".to_string(),
        ]
    );
}

#[test]
fn next_db_maintenance_run_aligns_to_same_week_when_before_slot() {
    let timezone: Tz = "Asia/Shanghai".parse().expect("timezone should parse");
    let now_utc = "2026-04-03T20:59:00Z"
        .parse::<DateTime<Utc>>()
        .expect("timestamp should parse");

    let next = next_db_maintenance_run_after(now_utc, timezone);

    assert_eq!(next.to_rfc3339(), "2026-04-04T21:00:00+00:00");
}

#[test]
fn next_db_maintenance_run_rolls_to_next_week_after_slot() {
    let timezone: Tz = "Asia/Shanghai".parse().expect("timezone should parse");
    let now_utc = "2026-04-04T21:00:01Z"
        .parse::<DateTime<Utc>>()
        .expect("timestamp should parse");

    let next = next_db_maintenance_run_after(now_utc, timezone);

    assert_eq!(next.to_rfc3339(), "2026-04-11T21:00:00+00:00");
}

#[test]
fn wallet_daily_usage_aggregation_target_uses_previous_local_day_window() {
    let timezone: Tz = "Asia/Shanghai".parse().expect("timezone should parse");
    let now_utc = "2026-03-31T16:15:00Z"
        .parse::<DateTime<Utc>>()
        .expect("timestamp should parse");

    let target = wallet_daily_usage_aggregation_target(now_utc, timezone);

    assert_eq!(target.billing_date.to_string(), "2026-03-31");
    assert_eq!(target.billing_timezone, "Asia/Shanghai");
    assert_eq!(
        target.window_start_utc.to_rfc3339(),
        "2026-03-30T16:00:00+00:00"
    );
    assert_eq!(
        target.window_end_utc.to_rfc3339(),
        "2026-03-31T16:00:00+00:00"
    );
}

#[tokio::test]
async fn provider_checkin_schedule_uses_default_for_invalid_value() {
    let data = GatewayDataState::disabled().with_system_config_values_for_tests([(
        "provider_checkin_time".to_string(),
        json!("25:99"),
    )]);

    let schedule = provider_checkin_schedule(&data)
        .await
        .expect("provider checkin schedule should resolve");

    assert_eq!(schedule, (1, 5));
}

#[test]
fn next_provider_checkin_run_aligns_to_same_day_when_before_slot() {
    let timezone: Tz = "Asia/Shanghai".parse().expect("timezone should parse");
    let now_utc = "2026-03-31T16:59:00Z"
        .parse::<DateTime<Utc>>()
        .expect("timestamp should parse");

    let next = next_daily_run_after(now_utc, timezone, 1, 5);

    assert_eq!(next.to_rfc3339(), "2026-03-31T17:05:00+00:00");
}

#[test]
fn next_provider_checkin_run_rolls_to_next_day_after_slot() {
    let timezone: Tz = "Asia/Shanghai".parse().expect("timezone should parse");
    let now_utc = "2026-03-31T17:05:01Z"
        .parse::<DateTime<Utc>>()
        .expect("timestamp should parse");

    let next = next_daily_run_after(now_utc, timezone, 1, 5);

    assert_eq!(next.to_rfc3339(), "2026-04-01T17:05:00+00:00");
}

#[test]
fn next_usage_cleanup_run_aligns_to_same_day_when_before_slot() {
    let timezone: Tz = "Asia/Shanghai".parse().expect("timezone should parse");
    let now_utc = "2026-03-17T18:59:00Z"
        .parse::<DateTime<Utc>>()
        .expect("timestamp should parse");

    let next = next_daily_run_after(now_utc, timezone, USAGE_CLEANUP_HOUR, USAGE_CLEANUP_MINUTE);

    assert_eq!(next.to_rfc3339(), "2026-03-17T19:00:00+00:00");
}

#[test]
fn next_usage_cleanup_run_rolls_to_next_day_after_slot() {
    let timezone: Tz = "Asia/Shanghai".parse().expect("timezone should parse");
    let now_utc = "2026-03-17T19:00:01Z"
        .parse::<DateTime<Utc>>()
        .expect("timestamp should parse");

    let next = next_daily_run_after(now_utc, timezone, USAGE_CLEANUP_HOUR, USAGE_CLEANUP_MINUTE);

    assert_eq!(next.to_rfc3339(), "2026-03-18T19:00:00+00:00");
}

#[test]
fn next_wallet_daily_usage_aggregation_run_aligns_to_same_day_when_before_slot() {
    let timezone: Tz = "Asia/Shanghai".parse().expect("timezone should parse");
    let now_utc = "2026-03-31T16:09:00Z"
        .parse::<DateTime<Utc>>()
        .expect("timestamp should parse");

    let next = next_daily_run_after(
        now_utc,
        timezone,
        WALLET_DAILY_USAGE_AGGREGATION_HOUR,
        WALLET_DAILY_USAGE_AGGREGATION_MINUTE,
    );

    assert_eq!(next.to_rfc3339(), "2026-03-31T16:10:00+00:00");
}

#[test]
fn next_wallet_daily_usage_aggregation_run_rolls_to_next_day_after_slot() {
    let timezone: Tz = "Asia/Shanghai".parse().expect("timezone should parse");
    let now_utc = "2026-03-31T16:10:01Z"
        .parse::<DateTime<Utc>>()
        .expect("timestamp should parse");

    let next = next_daily_run_after(
        now_utc,
        timezone,
        WALLET_DAILY_USAGE_AGGREGATION_HOUR,
        WALLET_DAILY_USAGE_AGGREGATION_MINUTE,
    );

    assert_eq!(next.to_rfc3339(), "2026-04-01T16:10:00+00:00");
}
