use super::keys::{
    parse_pool_cost_member, parse_pool_latency_member, pool_cooldown_index_key, pool_cooldown_key,
    pool_cooldown_keys, pool_cost_key, pool_cost_keys, pool_latency_keys, pool_lru_key,
    pool_sticky_key, pool_sticky_pattern,
};
use crate::handlers::admin::provider::pool::config::admin_provider_pool_cache_affinity_enabled;
use crate::handlers::admin::provider::shared::support::{
    admin_provider_pool_quota_probe_active_members_key, AdminProviderPoolConfig,
    AdminProviderPoolRuntimeState,
};
use crate::maintenance::PoolQuotaProbeWorkerConfig;
use crate::provider_pool_demand::{
    provider_pool_burst_pending, read_provider_pool_demand_snapshot,
};
use aether_runtime_state::{DataLayerError, RuntimeState};
use futures_util::future::join_all;
use std::collections::{BTreeMap, BTreeSet};
use std::time::{SystemTime, UNIX_EPOCH};
use tracing::{info, warn};

const DEFAULT_POOL_RUNTIME_WINDOW_METRIC_KEY_LIMIT: usize = 512;
const MAX_POOL_RUNTIME_WINDOW_METRIC_KEY_LIMIT: usize = 10_000;
const POOL_RUNTIME_WINDOW_METRIC_KEY_LIMIT_ENV: &str =
    "AETHER_GATEWAY_ADMIN_POOL_RUNTIME_WINDOW_METRIC_KEY_LIMIT";

fn current_unix_secs() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}

fn should_load_active_probe_members(pool_config: &AdminProviderPoolConfig) -> bool {
    pool_config.probing_enabled
}

fn pool_runtime_window_metric_key_limit() -> usize {
    std::env::var(POOL_RUNTIME_WINDOW_METRIC_KEY_LIMIT_ENV)
        .ok()
        .and_then(|value| value.trim().parse::<usize>().ok())
        .filter(|value| *value > 0)
        .unwrap_or(DEFAULT_POOL_RUNTIME_WINDOW_METRIC_KEY_LIMIT)
        .clamp(1, MAX_POOL_RUNTIME_WINDOW_METRIC_KEY_LIMIT)
}

fn bounded_runtime_window_metric_key_ids(key_ids: &[String], limit: usize) -> &[String] {
    let end = key_ids.len().min(limit.max(1));
    &key_ids[..end]
}

fn normalized_active_probe_member_ids(values: Vec<String>) -> BTreeSet<String> {
    values
        .into_iter()
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
        .collect()
}

pub(crate) async fn read_admin_provider_pool_cooldown_counts(
    runtime: &RuntimeState,
    provider_ids: &[String],
) -> BTreeMap<String, usize> {
    join_all(provider_ids.iter().map(|provider_id| async move {
        let count = runtime
            .set_len(&pool_cooldown_index_key(provider_id))
            .await
            .unwrap_or(0);
        (provider_id.clone(), count)
    }))
    .await
    .into_iter()
    .collect()
}

pub(crate) async fn read_admin_provider_pool_runtime_state(
    runtime: &RuntimeState,
    provider_id: &str,
    key_ids: &[String],
    pool_config: &AdminProviderPoolConfig,
    sticky_session_token: Option<&str>,
) -> AdminProviderPoolRuntimeState {
    let mut state = AdminProviderPoolRuntimeState::default();
    let cooldown_keys = pool_cooldown_keys(provider_id, key_ids);
    let metric_key_limit = pool_runtime_window_metric_key_limit();
    let metric_key_ids = bounded_runtime_window_metric_key_ids(key_ids, metric_key_limit);
    if metric_key_ids.len() < key_ids.len() {
        info!(
            event_name = "admin_pool_runtime_window_metrics_truncated",
            log_type = "event",
            provider_id,
            total_key_count = key_ids.len(),
            scanned_key_count = metric_key_ids.len(),
            metric_key_limit,
            "gateway limited admin pool runtime cost/latency window reads"
        );
    }
    let cost_keys = pool_cost_keys(provider_id, metric_key_ids);
    let latency_keys = pool_latency_keys(provider_id, metric_key_ids);
    let sticky_sessions_enabled = pool_config.sticky_session_ttl_seconds > 0
        && admin_provider_pool_cache_affinity_enabled(pool_config);

    if let Some(sticky_session_token) = sticky_session_token
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .filter(|_| sticky_sessions_enabled)
    {
        let sticky_key = pool_sticky_key(provider_id, sticky_session_token);
        if let Ok(Some(bound_key_id)) = runtime.kv_get(&sticky_key).await {
            let cooldown_key = pool_cooldown_key(provider_id, &bound_key_id);
            match runtime.kv_exists(&cooldown_key).await {
                Ok(false) => {
                    let _ = runtime
                        .key_expire(
                            &sticky_key,
                            std::time::Duration::from_secs(pool_config.sticky_session_ttl_seconds),
                        )
                        .await;
                    state.sticky_bound_key_id = Some(bound_key_id);
                }
                Ok(true) => {
                    let _ = runtime.kv_delete(&sticky_key).await;
                }
                Err(err) => {
                    warn!(
                        "gateway admin provider pool: failed to validate sticky cooldown for provider {provider_id}: {:?}",
                        err
                    );
                    state.sticky_bound_key_id = Some(bound_key_id);
                }
            }
        }
    }

    if sticky_sessions_enabled {
        let sticky_keys = runtime
            .scan_keys(&pool_sticky_pattern(provider_id), 200)
            .await
            .unwrap_or_default();
        state.total_sticky_sessions = sticky_keys.len();
        if !sticky_keys.is_empty() {
            let raw_keys = sticky_keys
                .iter()
                .map(|key| runtime.strip_namespace(key).to_string())
                .collect::<Vec<_>>();
            if let Ok(values) = runtime.kv_get_many(&raw_keys).await {
                for bound_key_id in values.into_iter().flatten() {
                    *state
                        .sticky_sessions_by_key
                        .entry(bound_key_id)
                        .or_insert(0) += 1;
                }
            }
        }
    }

    if should_load_active_probe_members(pool_config) {
        state.active_probe_member_ids =
            read_admin_provider_pool_active_probe_member_ids(runtime, provider_id)
                .await
                .unwrap_or_default();
    }

    let probe_config = PoolQuotaProbeWorkerConfig::from_env();
    let demand_snapshot = read_provider_pool_demand_snapshot(
        runtime,
        provider_id,
        key_ids.len(),
        probe_config.max_keys_per_provider,
    )
    .await;
    state.provider_in_flight = demand_snapshot.in_flight;
    state.provider_ema_in_flight = demand_snapshot.ema_in_flight;
    state.provider_desired_hot = if pool_config.probing_enabled {
        demand_snapshot.desired_hot
    } else {
        0
    };
    state.provider_burst_pending =
        pool_config.probing_enabled && provider_pool_burst_pending(runtime, provider_id).await;

    if !cooldown_keys.is_empty() {
        let cooldown_reasons = runtime
            .kv_get_many(&cooldown_keys)
            .await
            .unwrap_or_else(|_| vec![None; cooldown_keys.len()]);
        for (key_id, (cooldown_key, reason)) in key_ids
            .iter()
            .zip(cooldown_keys.iter().zip(cooldown_reasons))
        {
            if let Some(reason) = reason {
                state.cooldown_reason_by_key.insert(key_id.clone(), reason);
                if let Ok(Some(ttl)) = runtime.kv_ttl_seconds(cooldown_key).await {
                    if let Ok(ttl_seconds) = u64::try_from(ttl) {
                        if ttl_seconds > 0 {
                            state
                                .cooldown_ttl_by_key
                                .insert(key_id.clone(), ttl_seconds);
                        }
                    }
                }
            }
        }
    }

    let now = current_unix_secs();
    let cost_window_start = now.saturating_sub(pool_config.cost_window_seconds) as f64;
    let cost_results = join_all(
        cost_keys
            .iter()
            .map(|cost_key| runtime.score_range_by_min(cost_key, cost_window_start)),
    )
    .await;
    for (key_id, members) in metric_key_ids.iter().zip(cost_results) {
        let total = members
            .unwrap_or_default()
            .iter()
            .map(|member| parse_pool_cost_member(member))
            .sum::<u64>();
        if total > 0 {
            state.cost_window_usage_by_key.insert(key_id.clone(), total);
        }
    }

    let latency_window_start = now.saturating_sub(pool_config.latency_window_seconds) as f64;
    let latency_results = join_all(
        latency_keys
            .iter()
            .map(|latency_key| runtime.score_range_by_min(latency_key, latency_window_start)),
    )
    .await;
    for (key_id, members) in metric_key_ids.iter().zip(latency_results) {
        let samples = members
            .unwrap_or_default()
            .iter()
            .map(|member| parse_pool_latency_member(member))
            .filter(|value| *value > 0)
            .collect::<Vec<_>>();
        if samples.is_empty() {
            continue;
        }
        let total = samples.iter().sum::<u64>() as f64;
        let average = total / samples.len() as f64;
        if average.is_finite() && average >= 0.0 {
            state.latency_avg_ms_by_key.insert(key_id.clone(), average);
        }
    }

    if (pool_config.lru_enabled
        || pool_config
            .scheduling_presets
            .iter()
            .any(|item| item.enabled))
        && !key_ids.is_empty()
    {
        if let Ok(scores) = runtime
            .score_many(&pool_lru_key(provider_id), key_ids)
            .await
        {
            for (key_id, score) in key_ids.iter().zip(scores) {
                if let Some(score) = score {
                    state.lru_score_by_key.insert(key_id.clone(), score);
                }
            }
        }
    }

    state
}

pub(crate) async fn read_admin_provider_pool_cooldown_count(
    runtime: &RuntimeState,
    provider_id: &str,
) -> usize {
    runtime
        .set_len(&pool_cooldown_index_key(provider_id))
        .await
        .unwrap_or(0)
}

pub(crate) async fn read_admin_provider_pool_cooldown_key_ids(
    runtime: &RuntimeState,
    provider_id: &str,
) -> Vec<String> {
    runtime
        .set_members(&pool_cooldown_index_key(provider_id))
        .await
        .unwrap_or_default()
}

pub(crate) async fn read_admin_provider_pool_key_cooldown_reason(
    runtime: &RuntimeState,
    provider_id: &str,
    key_id: &str,
) -> Result<Option<String>, DataLayerError> {
    runtime
        .kv_get(&pool_cooldown_key(provider_id, key_id))
        .await
}

pub(crate) async fn read_admin_provider_pool_key_cost_window_usage(
    runtime: &RuntimeState,
    provider_id: &str,
    key_id: &str,
    cost_window_seconds: u64,
) -> Result<u64, DataLayerError> {
    let window_start = current_unix_secs().saturating_sub(cost_window_seconds) as f64;
    let members = runtime
        .score_range_by_min(&pool_cost_key(provider_id, key_id), window_start)
        .await?;
    Ok(members
        .iter()
        .map(|member| parse_pool_cost_member(member))
        .sum())
}

pub(crate) async fn read_admin_provider_pool_active_probe_member_ids(
    runtime: &RuntimeState,
    provider_id: &str,
) -> Result<BTreeSet<String>, DataLayerError> {
    runtime
        .set_members(&admin_provider_pool_quota_probe_active_members_key(
            provider_id,
        ))
        .await
        .map(normalized_active_probe_member_ids)
}

#[cfg(test)]
mod tests {
    use super::{
        bounded_runtime_window_metric_key_ids, normalized_active_probe_member_ids,
        read_admin_provider_pool_active_probe_member_ids,
        read_admin_provider_pool_key_cost_window_usage,
    };
    use crate::handlers::admin::provider::pool::runtime::keys::pool_cost_key;
    use crate::handlers::admin::provider::shared::support::admin_provider_pool_quota_probe_active_members_key;
    use aether_runtime_state::{MemoryRuntimeStateConfig, RuntimeState};
    use std::collections::BTreeSet;
    use std::time::{SystemTime, UNIX_EPOCH};

    #[test]
    fn runtime_window_metric_key_ids_are_bounded() {
        let key_ids = vec![
            "key-1".to_string(),
            "key-2".to_string(),
            "key-3".to_string(),
        ];

        let bounded = bounded_runtime_window_metric_key_ids(&key_ids, 2);

        assert_eq!(bounded, &key_ids[..2]);
    }

    #[test]
    fn runtime_window_metric_key_ids_keep_at_least_one_key() {
        let key_ids = vec!["key-1".to_string(), "key-2".to_string()];

        let bounded = bounded_runtime_window_metric_key_ids(&key_ids, 0);

        assert_eq!(bounded, &key_ids[..1]);
    }

    #[test]
    fn active_probe_member_ids_are_normalized() {
        let values = vec![
            " key-2 ".to_string(),
            String::new(),
            "key-1".to_string(),
            "key-2".to_string(),
            "   ".to_string(),
        ];

        assert_eq!(
            normalized_active_probe_member_ids(values),
            BTreeSet::from(["key-1".to_string(), "key-2".to_string()])
        );
    }

    #[tokio::test]
    async fn key_cost_window_usage_only_sums_members_in_window() {
        let runtime = RuntimeState::memory(MemoryRuntimeStateConfig::default());
        let cost_key = pool_cost_key("provider-1", "key-1");
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("system time should follow unix epoch")
            .as_secs() as f64;
        runtime
            .score_set(&cost_key, "recent-a:40", now)
            .await
            .expect("recent cost should write");
        runtime
            .score_set(&cost_key, "recent-b:80", now)
            .await
            .expect("recent cost should write");
        runtime
            .score_set(&cost_key, "expired:500", now - 120.0)
            .await
            .expect("expired cost should write");

        let usage =
            read_admin_provider_pool_key_cost_window_usage(&runtime, "provider-1", "key-1", 60)
                .await
                .expect("cost window should read");

        assert_eq!(usage, 120);
    }

    #[tokio::test]
    async fn active_probe_member_read_normalizes_runtime_values() {
        let runtime = RuntimeState::memory(MemoryRuntimeStateConfig::default());
        let set_key = admin_provider_pool_quota_probe_active_members_key("provider-1");
        runtime
            .set_add(&set_key, " key-2 ")
            .await
            .expect("probe member should write");
        runtime
            .set_add(&set_key, "key-1")
            .await
            .expect("probe member should write");
        runtime
            .set_add(&set_key, "   ")
            .await
            .expect("blank probe member should write");

        let members = read_admin_provider_pool_active_probe_member_ids(&runtime, "provider-1")
            .await
            .expect("active probe members should read");

        assert_eq!(
            members,
            BTreeSet::from(["key-1".to_string(), "key-2".to_string()])
        );
    }
}
