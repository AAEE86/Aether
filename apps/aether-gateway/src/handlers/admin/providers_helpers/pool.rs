use crate::gateway::handlers::{AdminProviderPoolConfig, AdminProviderPoolRuntimeState};
use crate::gateway::{AppState, GatewayError};
use aether_data::redis::{RedisKeyspace, RedisKvRunner};
use aether_data::repository::provider_catalog::StoredProviderCatalogProvider;
use serde_json::json;
use std::time::{SystemTime, UNIX_EPOCH};
use tracing::warn;

use super::super::super::ADMIN_PROVIDER_POOL_SCAN_BATCH;

fn json_u64(value: &serde_json::Value) -> Option<u64> {
    value
        .as_u64()
        .or_else(|| value.as_i64().and_then(|raw| u64::try_from(raw).ok()))
}

fn admin_provider_pool_lru_enabled(
    raw_pool_advanced: &serde_json::Map<String, serde_json::Value>,
) -> bool {
    if let Some(explicit) = raw_pool_advanced
        .get("lru_enabled")
        .and_then(serde_json::Value::as_bool)
    {
        return explicit;
    }

    let Some(presets) = raw_pool_advanced
        .get("scheduling_presets")
        .and_then(serde_json::Value::as_array)
    else {
        return false;
    };

    let Some(first) = presets.first() else {
        return false;
    };

    if first.is_string() {
        return raw_pool_advanced
            .get("lru_enabled")
            .and_then(serde_json::Value::as_bool)
            .unwrap_or(true);
    }

    presets
        .iter()
        .filter_map(serde_json::Value::as_object)
        .any(|item| {
            item.get("preset")
                .and_then(serde_json::Value::as_str)
                .is_some_and(|preset| preset.eq_ignore_ascii_case("lru"))
                && item
                    .get("enabled")
                    .and_then(serde_json::Value::as_bool)
                    .unwrap_or(true)
        })
}

pub(crate) fn admin_provider_pool_config(
    provider: &StoredProviderCatalogProvider,
) -> Option<AdminProviderPoolConfig> {
    let raw_pool_advanced = provider
        .config
        .as_ref()
        .and_then(serde_json::Value::as_object)
        .and_then(|config| config.get("pool_advanced"))?;

    let Some(pool_advanced) = raw_pool_advanced.as_object() else {
        return Some(AdminProviderPoolConfig {
            lru_enabled: false,
            cost_window_seconds: 18_000,
            cost_limit_per_key_tokens: None,
        });
    };

    Some(AdminProviderPoolConfig {
        lru_enabled: admin_provider_pool_lru_enabled(pool_advanced),
        cost_window_seconds: pool_advanced
            .get("cost_window_seconds")
            .and_then(json_u64)
            .filter(|value| *value > 0)
            .unwrap_or(18_000),
        cost_limit_per_key_tokens: pool_advanced
            .get("cost_limit_per_key_tokens")
            .and_then(json_u64),
    })
}

fn pool_sticky_pattern(keyspace: &RedisKeyspace, provider_id: &str) -> String {
    keyspace.key(&format!("ap:{provider_id}:sticky:*"))
}

fn pool_lru_key(keyspace: &RedisKeyspace, provider_id: &str) -> String {
    keyspace.key(&format!("ap:{provider_id}:lru"))
}

fn pool_cooldown_key(keyspace: &RedisKeyspace, provider_id: &str, key_id: &str) -> String {
    keyspace.key(&format!("ap:{provider_id}:cooldown:{key_id}"))
}

fn pool_cooldown_index_key(keyspace: &RedisKeyspace, provider_id: &str) -> String {
    keyspace.key(&format!("ap:{provider_id}:cooldown_idx"))
}

fn pool_cost_key(keyspace: &RedisKeyspace, provider_id: &str, key_id: &str) -> String {
    keyspace.key(&format!("ap:{provider_id}:cost:{key_id}"))
}

fn parse_pool_cost_member(member: &str) -> u64 {
    member
        .rsplit_once(':')
        .and_then(|(_, suffix)| suffix.parse::<u64>().ok())
        .unwrap_or(0)
}

async fn scan_redis_keys(
    connection: &mut redis::aio::MultiplexedConnection,
    pattern: &str,
) -> Result<Vec<String>, GatewayError> {
    let mut cursor = 0u64;
    let mut keys = Vec::new();
    loop {
        let (next_cursor, batch): (u64, Vec<String>) = redis::cmd("SCAN")
            .arg(cursor)
            .arg("MATCH")
            .arg(pattern)
            .arg("COUNT")
            .arg(ADMIN_PROVIDER_POOL_SCAN_BATCH)
            .query_async(connection)
            .await
            .map_err(|err| GatewayError::Internal(err.to_string()))?;
        keys.extend(batch);
        if next_cursor == 0 {
            break;
        }
        cursor = next_cursor;
    }
    Ok(keys)
}

pub(crate) async fn read_admin_provider_pool_runtime_state(
    runner: &RedisKvRunner,
    provider_id: &str,
    key_ids: &[String],
    pool_config: AdminProviderPoolConfig,
) -> AdminProviderPoolRuntimeState {
    let mut runtime = AdminProviderPoolRuntimeState::default();
    let Ok(mut connection) = runner.client().get_multiplexed_async_connection().await else {
        warn!("gateway admin provider pool: failed to connect redis for provider {provider_id}");
        return runtime;
    };
    let keyspace = runner.keyspace().clone();

    let sticky_keys = match scan_redis_keys(
        &mut connection,
        &pool_sticky_pattern(&keyspace, provider_id),
    )
    .await
    {
        Ok(keys) => keys,
        Err(err) => {
            warn!(
                "gateway admin provider pool: failed to scan sticky keys for provider {provider_id}: {:?}",
                err
            );
            Vec::new()
        }
    };
    runtime.total_sticky_sessions = sticky_keys.len();
    if !sticky_keys.is_empty() {
        for chunk in sticky_keys.chunks(ADMIN_PROVIDER_POOL_SCAN_BATCH as usize) {
            let values = redis::cmd("MGET")
                .arg(chunk)
                .query_async::<Vec<Option<String>>>(&mut connection)
                .await;
            let Ok(values) = values else {
                warn!("gateway admin provider pool: failed to read sticky bindings for provider {provider_id}");
                break;
            };
            for bound_key_id in values.into_iter().flatten() {
                *runtime
                    .sticky_sessions_by_key
                    .entry(bound_key_id)
                    .or_insert(0) += 1;
            }
        }
    }

    for key_id in key_ids {
        let cooldown_key = pool_cooldown_key(&keyspace, provider_id, key_id);
        let reason = redis::cmd("GET")
            .arg(&cooldown_key)
            .query_async::<Option<String>>(&mut connection)
            .await;
        if let Ok(Some(reason)) = reason {
            runtime
                .cooldown_reason_by_key
                .insert(key_id.clone(), reason);
            let ttl = redis::cmd("TTL")
                .arg(&cooldown_key)
                .query_async::<i64>(&mut connection)
                .await
                .ok()
                .and_then(|value| u64::try_from(value).ok());
            if let Some(ttl_seconds) = ttl.filter(|value| *value > 0) {
                runtime
                    .cooldown_ttl_by_key
                    .insert(key_id.clone(), ttl_seconds);
            }
        }

        let window_start = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs()
            .saturating_sub(pool_config.cost_window_seconds);
        let members = redis::cmd("ZRANGEBYSCORE")
            .arg(pool_cost_key(&keyspace, provider_id, key_id))
            .arg(window_start)
            .arg("+inf")
            .query_async::<Vec<String>>(&mut connection)
            .await
            .unwrap_or_default();
        let total = members
            .iter()
            .map(|member| parse_pool_cost_member(member))
            .sum::<u64>();
        runtime
            .cost_window_usage_by_key
            .insert(key_id.clone(), total);
    }

    if pool_config.lru_enabled && !key_ids.is_empty() {
        let mut command = redis::cmd("ZMSCORE");
        command.arg(pool_lru_key(&keyspace, provider_id));
        for key_id in key_ids {
            command.arg(key_id);
        }
        if let Ok(scores) = command
            .query_async::<Vec<Option<f64>>>(&mut connection)
            .await
        {
            for (key_id, score) in key_ids.iter().zip(scores.into_iter()) {
                if let Some(score) = score {
                    runtime.lru_score_by_key.insert(key_id.clone(), score);
                }
            }
        }
    }

    runtime
}

pub(crate) async fn read_admin_provider_pool_cooldown_count(
    runner: &RedisKvRunner,
    provider_id: &str,
) -> usize {
    let Ok(mut connection) = runner.client().get_multiplexed_async_connection().await else {
        warn!("gateway admin provider pool: failed to connect redis for provider {provider_id}");
        return 0;
    };
    let keyspace = runner.keyspace().clone();
    redis::cmd("SCARD")
        .arg(pool_cooldown_index_key(&keyspace, provider_id))
        .query_async::<u64>(&mut connection)
        .await
        .map(|value| value as usize)
        .unwrap_or(0)
}

pub(crate) async fn read_admin_provider_pool_cooldown_key_ids(
    runner: &RedisKvRunner,
    provider_id: &str,
) -> Vec<String> {
    let Ok(mut connection) = runner.client().get_multiplexed_async_connection().await else {
        warn!("gateway admin provider pool: failed to connect redis for provider {provider_id}");
        return Vec::new();
    };
    let keyspace = runner.keyspace().clone();
    redis::cmd("SMEMBERS")
        .arg(pool_cooldown_index_key(&keyspace, provider_id))
        .query_async::<Vec<String>>(&mut connection)
        .await
        .unwrap_or_default()
}

pub(crate) async fn build_admin_provider_pool_status_payload(
    state: &AppState,
    provider_id: &str,
) -> Option<serde_json::Value> {
    if !state.has_provider_catalog_data_reader() {
        return None;
    }

    let provider = state
        .read_provider_catalog_providers_by_ids(&[provider_id.to_string()])
        .await
        .ok()
        .and_then(|mut providers| providers.drain(..).next())?;
    let Some(pool_config) = admin_provider_pool_config(&provider) else {
        return Some(json!({
            "provider_id": provider.id,
            "provider_name": provider.name,
            "pool_enabled": false,
            "total_keys": 0,
            "total_sticky_sessions": 0,
            "keys": [],
        }));
    };

    let keys = state
        .list_provider_catalog_keys_by_provider_ids(std::slice::from_ref(&provider.id))
        .await
        .ok()
        .unwrap_or_default();
    let key_ids = keys.iter().map(|key| key.id.clone()).collect::<Vec<_>>();
    let runtime = match state.redis_kv_runner() {
        Some(runner) => {
            read_admin_provider_pool_runtime_state(&runner, &provider.id, &key_ids, pool_config)
                .await
        }
        None => AdminProviderPoolRuntimeState::default(),
    };
    let key_payloads = keys
        .into_iter()
        .map(|key| {
            let cooldown_reason = runtime.cooldown_reason_by_key.get(&key.id).cloned();
            json!({
                "key_id": key.id,
                "key_name": key.name,
                "is_active": key.is_active,
                "cooldown_reason": cooldown_reason,
                "cooldown_ttl_seconds": cooldown_reason
                    .as_ref()
                    .and_then(|_| runtime.cooldown_ttl_by_key.get(&key.id).copied()),
                "cost_window_usage": runtime.cost_window_usage_by_key.get(&key.id).copied().unwrap_or(0),
                "cost_limit": pool_config.cost_limit_per_key_tokens,
                "sticky_sessions": runtime.sticky_sessions_by_key.get(&key.id).copied().unwrap_or(0),
                "lru_score": runtime.lru_score_by_key.get(&key.id).copied(),
            })
        })
        .collect::<Vec<_>>();

    Some(json!({
        "provider_id": provider.id,
        "provider_name": provider.name,
        "pool_enabled": true,
        "total_keys": key_payloads.len(),
        "total_sticky_sessions": runtime.total_sticky_sessions,
        "keys": key_payloads,
    }))
}

pub(crate) async fn clear_admin_provider_pool_cooldown(
    state: &AppState,
    provider_id: &str,
    key_id: &str,
) {
    let Some(runner) = state.redis_kv_runner() else {
        return;
    };
    let Ok(mut connection) = runner.client().get_multiplexed_async_connection().await else {
        warn!("gateway admin provider pool: failed to connect redis to clear cooldown for key {key_id}");
        return;
    };
    let keyspace = runner.keyspace().clone();
    let _: Result<(), _> = redis::pipe()
        .cmd("DEL")
        .arg(pool_cooldown_key(&keyspace, provider_id, key_id))
        .ignore()
        .cmd("SREM")
        .arg(pool_cooldown_index_key(&keyspace, provider_id))
        .arg(key_id)
        .ignore()
        .query_async(&mut connection)
        .await;
}

pub(crate) async fn reset_admin_provider_pool_cost(
    state: &AppState,
    provider_id: &str,
    key_id: &str,
) {
    let Some(runner) = state.redis_kv_runner() else {
        return;
    };
    let _ = runner.del(&format!("ap:{provider_id}:cost:{key_id}")).await;
}
