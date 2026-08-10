mod error;
mod memory;
pub mod redis;

use std::collections::{BTreeMap, BTreeSet};
use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

pub use crate::redis::{
    RedisClientConfig, RedisConsumerGroup, RedisConsumerName, RedisKeyspace, RedisKvRunner,
    RedisKvRunnerConfig, RedisLaneDiagnostics, RedisLockLease, RedisLockRunner,
    RedisLockRunnerConfig, RedisRuntimeDiagnostics, RedisStreamEntry, RedisStreamName,
    RedisStreamReclaimConfig, RedisStreamRunner, RedisStreamRunnerConfig,
};
use async_trait::async_trait;
pub use error::DataLayerError;
use memory::MemoryRuntimeBackend;
pub use memory::MemoryRuntimeStateConfig;
use tokio::task::JoinHandle;
use tracing::warn;
use uuid::Uuid;

const DEFAULT_KV_TTL_SECONDS: u64 = 300;
// Runtime coordination must tolerate brief executor scheduling pauses under large streaming
// fan-in. This remains comfortably below the gateway's default 10s lease-renew interval and 30s
// lease TTL, so fail-closed fencing still has ample time to stop a stale holder.
const DEFAULT_COMMAND_TIMEOUT_MS: u64 = 2_000;
const DEFAULT_STREAM_BLOCK_TIMEOUT_GRACE_MS: u64 = 1_000;
const EXECUTION_RESERVATION_LEASE_TTL_MS: u64 = 30_000;
const EXECUTION_RESERVATION_RENEW_INTERVAL_MS: u64 = 10_000;
const EXECUTION_RESERVATION_RPM_WINDOW_SECS: u64 = 60;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RuntimeStateBackendMode {
    Auto,
    Memory,
    Redis,
}

impl RuntimeStateBackendMode {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Auto => "auto",
            Self::Memory => "memory",
            Self::Redis => "redis",
        }
    }
}

impl std::str::FromStr for RuntimeStateBackendMode {
    type Err = DataLayerError;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        match value.trim().to_ascii_lowercase().as_str() {
            "" | "auto" => Ok(Self::Auto),
            "memory" => Ok(Self::Memory),
            "redis" => Ok(Self::Redis),
            other => Err(DataLayerError::InvalidConfiguration(format!(
                "unsupported runtime backend {other}; expected auto, memory, or redis"
            ))),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RuntimeStateConfig {
    pub backend: RuntimeStateBackendMode,
    pub redis: Option<RedisClientConfig>,
    pub memory: MemoryRuntimeStateConfig,
    pub command_timeout_ms: Option<u64>,
    pub blocking_stream_lanes: Option<usize>,
}

impl Default for RuntimeStateConfig {
    fn default() -> Self {
        Self {
            backend: RuntimeStateBackendMode::Auto,
            redis: None,
            memory: MemoryRuntimeStateConfig::default(),
            command_timeout_ms: Some(DEFAULT_COMMAND_TIMEOUT_MS),
            blocking_stream_lanes: None,
        }
    }
}

impl RuntimeStateConfig {
    pub fn memory() -> Self {
        Self {
            backend: RuntimeStateBackendMode::Memory,
            redis: None,
            ..Self::default()
        }
    }

    pub fn redis(redis: RedisClientConfig) -> Self {
        Self {
            backend: RuntimeStateBackendMode::Redis,
            redis: Some(redis),
            ..Self::default()
        }
    }

    pub fn redis_url_from_env() -> Option<String> {
        env_value("AETHER_RUNTIME_REDIS_URL")
            .or_else(|| env_value("AETHER_GATEWAY_DATA_REDIS_URL"))
            .or_else(|| env_value("REDIS_URL"))
    }

    pub fn redis_key_prefix_from_env() -> Option<String> {
        env_value("AETHER_RUNTIME_REDIS_KEY_PREFIX")
            .or_else(|| env_value("AETHER_GATEWAY_DATA_REDIS_KEY_PREFIX"))
    }

    pub fn from_env_with_backend(backend: RuntimeStateBackendMode) -> Self {
        let redis = if matches!(backend, RuntimeStateBackendMode::Redis) {
            Self::redis_url_from_env().map(|url| RedisClientConfig {
                url,
                key_prefix: Self::redis_key_prefix_from_env(),
            })
        } else {
            None
        };
        Self {
            backend,
            redis,
            ..Self::default()
        }
    }

    pub fn validate(&self) -> Result<(), DataLayerError> {
        if matches!(self.backend, RuntimeStateBackendMode::Redis) && self.redis.is_none() {
            return Err(DataLayerError::InvalidConfiguration(
                "AETHER_RUNTIME_BACKEND=redis requires AETHER_RUNTIME_REDIS_URL, AETHER_GATEWAY_DATA_REDIS_URL, or REDIS_URL".to_string(),
            ));
        }
        if let Some(redis) = &self.redis {
            redis.validate()?;
        }
        if self.memory.max_kv_entries == 0 {
            return Err(DataLayerError::InvalidConfiguration(
                "runtime memory max_kv_entries must be positive".to_string(),
            ));
        }
        if matches!(self.command_timeout_ms, Some(0)) {
            return Err(DataLayerError::InvalidConfiguration(
                "runtime state command_timeout_ms must be positive".to_string(),
            ));
        }
        if matches!(self.blocking_stream_lanes, Some(0)) {
            return Err(DataLayerError::InvalidConfiguration(
                "runtime state blocking_stream_lanes must be positive".to_string(),
            ));
        }
        Ok(())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RuntimeStateBackendKind {
    Memory,
    Redis,
}

impl RuntimeStateBackendKind {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Memory => "memory",
            Self::Redis => "redis",
        }
    }
}

#[derive(Debug, Clone)]
pub struct RuntimeState {
    backend: Arc<RuntimeStateBackend>,
}

#[derive(Debug)]
enum RuntimeStateBackend {
    Memory(Box<MemoryRuntimeBackend>),
    Redis(Box<RedisRuntimeBackend>),
}

#[derive(Debug, Clone)]
struct RedisRuntimeBackend {
    keyspace: RedisKeyspace,
    kv: RedisKvRunner,
    lock: RedisLockRunner,
    stream: RedisStreamRunner,
    runtime: redis::RedisRuntimeRunner,
    command_timeout_ms: Option<u64>,
}

impl RuntimeState {
    pub async fn from_config(mut config: RuntimeStateConfig) -> Result<Self, DataLayerError> {
        if matches!(config.backend, RuntimeStateBackendMode::Auto) {
            config.backend = if config.redis.is_some() {
                RuntimeStateBackendMode::Redis
            } else {
                RuntimeStateBackendMode::Memory
            };
        }
        config.validate()?;
        match config.backend {
            RuntimeStateBackendMode::Memory => Ok(Self::memory(config.memory)),
            RuntimeStateBackendMode::Redis => {
                let redis = config.redis.clone().ok_or_else(|| {
                    DataLayerError::InvalidConfiguration("runtime redis config missing".to_string())
                })?;
                Self::redis_with_blocking_stream_lanes(
                    redis,
                    config.command_timeout_ms,
                    config.blocking_stream_lanes,
                )
                .await
            }
            RuntimeStateBackendMode::Auto => unreachable!("auto resolved above"),
        }
    }

    pub fn memory(config: MemoryRuntimeStateConfig) -> Self {
        Self {
            backend: Arc::new(RuntimeStateBackend::Memory(Box::new(
                MemoryRuntimeBackend::new(config),
            ))),
        }
    }

    pub async fn redis(
        config: RedisClientConfig,
        command_timeout_ms: Option<u64>,
    ) -> Result<Self, DataLayerError> {
        Self::redis_with_blocking_stream_lanes(config, command_timeout_ms, None).await
    }

    pub async fn redis_with_blocking_stream_lanes(
        config: RedisClientConfig,
        command_timeout_ms: Option<u64>,
        blocking_stream_lanes: Option<usize>,
    ) -> Result<Self, DataLayerError> {
        let factory = redis::RedisClientFactory::new(config)?;
        let keyspace = factory.config().keyspace();
        let connections = factory
            .connect_router_with_blocking_stream_lanes(command_timeout_ms, blocking_stream_lanes)
            .await?;
        let runtime = redis::RedisRuntimeRunner::new(
            connections.clone(),
            keyspace.clone(),
            command_timeout_ms,
        );
        runtime.ping().await?;
        let kv = RedisKvRunner::new(
            connections.clone(),
            keyspace.clone(),
            RedisKvRunnerConfig {
                command_timeout_ms,
                default_ttl_seconds: DEFAULT_KV_TTL_SECONDS,
            },
        )?;
        let lock = RedisLockRunner::new(
            connections.clone(),
            keyspace.clone(),
            RedisLockRunnerConfig {
                command_timeout_ms,
                ..RedisLockRunnerConfig::default()
            },
        )?;
        let stream = RedisStreamRunner::new(
            connections,
            keyspace.clone(),
            RedisStreamRunnerConfig {
                command_timeout_ms,
                read_block_ms: None,
                ..RedisStreamRunnerConfig::default()
            },
        )?;
        Ok(Self {
            backend: Arc::new(RuntimeStateBackend::Redis(Box::new(RedisRuntimeBackend {
                keyspace,
                kv,
                lock,
                stream,
                runtime,
                command_timeout_ms,
            }))),
        })
    }

    pub fn backend_kind(&self) -> RuntimeStateBackendKind {
        match self.backend.as_ref() {
            RuntimeStateBackend::Memory(_) => RuntimeStateBackendKind::Memory,
            RuntimeStateBackend::Redis(_) => RuntimeStateBackendKind::Redis,
        }
    }

    pub fn is_memory(&self) -> bool {
        matches!(self.backend_kind(), RuntimeStateBackendKind::Memory)
    }

    pub fn is_redis(&self) -> bool {
        matches!(self.backend_kind(), RuntimeStateBackendKind::Redis)
    }

    pub async fn redis_diagnostics(
        &self,
    ) -> Result<Option<RedisRuntimeDiagnostics>, DataLayerError> {
        match self.backend.as_ref() {
            RuntimeStateBackend::Memory(_) => Ok(None),
            RuntimeStateBackend::Redis(redis) => Ok(Some(redis.runtime.diagnostics().await?)),
        }
    }

    pub fn kv_set_local_nowait(&self, key: &str, value: String, ttl: Option<Duration>) -> bool {
        match self.backend.as_ref() {
            RuntimeStateBackend::Memory(memory) => memory.kv_set_nowait(key, value, ttl),
            RuntimeStateBackend::Redis(_) => false,
        }
    }

    pub fn set_add_local_nowait(&self, key: &str, member: &str) -> bool {
        match self.backend.as_ref() {
            RuntimeStateBackend::Memory(memory) => memory.set_add_nowait(key, member),
            RuntimeStateBackend::Redis(_) => false,
        }
    }

    pub fn namespace_key(&self, raw_key: &str) -> String {
        match self.backend.as_ref() {
            RuntimeStateBackend::Memory(_) => raw_key.to_string(),
            RuntimeStateBackend::Redis(redis) => redis.keyspace.key(raw_key),
        }
    }

    pub fn strip_namespace<'a>(&self, namespaced_key: &'a str) -> &'a str {
        match self.backend.as_ref() {
            RuntimeStateBackend::Memory(_) => namespaced_key,
            RuntimeStateBackend::Redis(redis) => {
                let probe = redis.keyspace.key("");
                let prefix = probe.trim_end_matches(':');
                namespaced_key
                    .strip_prefix(prefix)
                    .and_then(|value| value.strip_prefix(':'))
                    .unwrap_or(namespaced_key)
            }
        }
    }

    pub async fn kv_set(
        &self,
        key: &str,
        value: impl Into<String> + Send,
        ttl: Option<Duration>,
    ) -> Result<(), DataLayerError> {
        match self.backend.as_ref() {
            RuntimeStateBackend::Memory(memory) => {
                memory.kv_set(key, value.into(), ttl).await;
                Ok(())
            }
            RuntimeStateBackend::Redis(redis) => {
                let value = value.into();
                if let Some(ttl) = ttl {
                    redis.runtime.kv_set_with_ttl(key, value, ttl).await?;
                } else {
                    redis.runtime.kv_set_plain(key, value).await?;
                }
                Ok(())
            }
        }
    }

    pub async fn kv_get(&self, key: &str) -> Result<Option<String>, DataLayerError> {
        match self.backend.as_ref() {
            RuntimeStateBackend::Memory(memory) => Ok(memory.kv_get(key).await),
            RuntimeStateBackend::Redis(redis) => redis.kv.get(key).await,
        }
    }

    pub async fn kv_get_many(
        &self,
        keys: &[String],
    ) -> Result<Vec<Option<String>>, DataLayerError> {
        if keys.is_empty() {
            return Ok(Vec::new());
        }
        match self.backend.as_ref() {
            RuntimeStateBackend::Memory(memory) => {
                let mut values = Vec::with_capacity(keys.len());
                for key in keys {
                    values.push(memory.kv_get(key).await);
                }
                Ok(values)
            }
            RuntimeStateBackend::Redis(redis) => redis.runtime.kv_get_many(keys).await,
        }
    }

    pub async fn kv_take(&self, key: &str) -> Result<Option<String>, DataLayerError> {
        match self.backend.as_ref() {
            RuntimeStateBackend::Memory(memory) => Ok(memory.kv_take(key).await),
            RuntimeStateBackend::Redis(redis) => redis.kv.getdel(key).await,
        }
    }

    pub async fn kv_delete(&self, key: &str) -> Result<bool, DataLayerError> {
        match self.backend.as_ref() {
            RuntimeStateBackend::Memory(memory) => Ok(memory.kv_delete(key).await),
            RuntimeStateBackend::Redis(redis) => Ok(redis.kv.del(key).await? > 0),
        }
    }

    pub async fn kv_delete_many(&self, keys: &[String]) -> Result<usize, DataLayerError> {
        if keys.is_empty() {
            return Ok(0);
        }
        match self.backend.as_ref() {
            RuntimeStateBackend::Memory(memory) => Ok(memory.kv_delete_many(keys).await),
            RuntimeStateBackend::Redis(redis) => redis.runtime.kv_delete_many(keys).await,
        }
    }

    pub async fn kv_exists(&self, key: &str) -> Result<bool, DataLayerError> {
        match self.backend.as_ref() {
            RuntimeStateBackend::Memory(memory) => Ok(memory.kv_exists(key).await),
            RuntimeStateBackend::Redis(redis) => redis.kv.exists(key).await,
        }
    }

    pub async fn kv_ttl_seconds(&self, key: &str) -> Result<Option<i64>, DataLayerError> {
        match self.backend.as_ref() {
            RuntimeStateBackend::Memory(memory) => Ok(memory.kv_ttl_seconds(key).await),
            RuntimeStateBackend::Redis(redis) => redis.runtime.kv_ttl_seconds(key).await,
        }
    }

    pub async fn scan_keys(
        &self,
        pattern: &str,
        count: usize,
    ) -> Result<Vec<String>, DataLayerError> {
        match self.backend.as_ref() {
            RuntimeStateBackend::Memory(memory) => Ok(memory.kv_scan(pattern).await),
            RuntimeStateBackend::Redis(redis) => redis.runtime.scan_keys(pattern, count).await,
        }
    }

    pub async fn check_and_consume_rate_limit(
        &self,
        input: RateLimitInput<'_>,
    ) -> Result<RateLimitCheck, DataLayerError> {
        match self.backend.as_ref() {
            RuntimeStateBackend::Memory(memory) => {
                memory
                    .check_and_consume_rate_limit(
                        input.user_key,
                        input.key_key,
                        input.bucket,
                        input.user_limit,
                        input.key_limit,
                        Duration::from_secs(input.ttl_seconds.max(1)),
                    )
                    .await
            }
            RuntimeStateBackend::Redis(redis) => {
                redis.runtime.check_and_consume_rate_limit(input).await
            }
        }
    }

    pub async fn rate_limit_count(&self, key: &str, bucket: u64) -> Result<u32, DataLayerError> {
        match self.backend.as_ref() {
            RuntimeStateBackend::Memory(memory) => memory.rate_limit_count(key, bucket),
            RuntimeStateBackend::Redis(redis) => Ok(redis
                .kv
                .get(key)
                .await?
                .and_then(|value| value.parse::<u32>().ok())
                .unwrap_or_default()),
        }
    }

    pub async fn set_add(&self, key: &str, member: &str) -> Result<bool, DataLayerError> {
        match self.backend.as_ref() {
            RuntimeStateBackend::Memory(memory) => Ok(memory.set_add(key, member).await),
            RuntimeStateBackend::Redis(redis) => redis.runtime.set_add(key, member).await,
        }
    }

    pub async fn set_remove(&self, key: &str, member: &str) -> Result<bool, DataLayerError> {
        match self.backend.as_ref() {
            RuntimeStateBackend::Memory(memory) => Ok(memory.set_remove(key, member).await),
            RuntimeStateBackend::Redis(redis) => redis.runtime.set_remove(key, member).await,
        }
    }

    pub async fn set_members(&self, key: &str) -> Result<Vec<String>, DataLayerError> {
        match self.backend.as_ref() {
            RuntimeStateBackend::Memory(memory) => Ok(memory.set_members(key).await),
            RuntimeStateBackend::Redis(redis) => redis.runtime.set_members(key).await,
        }
    }

    pub async fn set_len(&self, key: &str) -> Result<usize, DataLayerError> {
        match self.backend.as_ref() {
            RuntimeStateBackend::Memory(memory) => Ok(memory.set_len(key).await),
            RuntimeStateBackend::Redis(redis) => redis.runtime.set_len(key).await,
        }
    }

    pub async fn score_set(
        &self,
        key: &str,
        member: &str,
        score: f64,
    ) -> Result<(), DataLayerError> {
        if !score.is_finite() {
            return Err(DataLayerError::InvalidInput(
                "runtime score must be finite".to_string(),
            ));
        }
        match self.backend.as_ref() {
            RuntimeStateBackend::Memory(memory) => {
                memory.score_set(key, member, score).await;
                Ok(())
            }
            RuntimeStateBackend::Redis(redis) => redis.runtime.score_set(key, member, score).await,
        }
    }

    pub async fn score_many(
        &self,
        key: &str,
        members: &[String],
    ) -> Result<Vec<Option<f64>>, DataLayerError> {
        if members.is_empty() {
            return Ok(Vec::new());
        }
        match self.backend.as_ref() {
            RuntimeStateBackend::Memory(memory) => Ok(memory.score_many(key, members).await),
            RuntimeStateBackend::Redis(redis) => redis.runtime.score_many(key, members).await,
        }
    }

    pub async fn score_range_by_min(
        &self,
        key: &str,
        min_score: f64,
    ) -> Result<Vec<String>, DataLayerError> {
        match self.backend.as_ref() {
            RuntimeStateBackend::Memory(memory) => {
                Ok(memory.score_range_by_min(key, min_score).await)
            }
            RuntimeStateBackend::Redis(redis) => {
                redis.runtime.score_range_by_min(key, min_score).await
            }
        }
    }

    pub async fn score_remove_by_score(
        &self,
        key: &str,
        max_score: f64,
    ) -> Result<usize, DataLayerError> {
        match self.backend.as_ref() {
            RuntimeStateBackend::Memory(memory) => {
                Ok(memory.score_remove_by_score(key, max_score).await)
            }
            RuntimeStateBackend::Redis(redis) => {
                redis.runtime.score_remove_by_score(key, max_score).await
            }
        }
    }

    pub async fn score_remove(&self, key: &str, member: &str) -> Result<bool, DataLayerError> {
        match self.backend.as_ref() {
            RuntimeStateBackend::Memory(memory) => Ok(memory.score_remove(key, member).await),
            RuntimeStateBackend::Redis(redis) => redis.runtime.score_remove(key, member).await,
        }
    }

    pub async fn score_remove_by_rank(
        &self,
        key: &str,
        start: i64,
        stop: i64,
    ) -> Result<usize, DataLayerError> {
        match self.backend.as_ref() {
            RuntimeStateBackend::Memory(memory) => {
                Ok(memory.score_remove_by_rank(key, start, stop).await)
            }
            RuntimeStateBackend::Redis(redis) => {
                redis.runtime.score_remove_by_rank(key, start, stop).await
            }
        }
    }

    pub async fn score_len(&self, key: &str) -> Result<usize, DataLayerError> {
        match self.backend.as_ref() {
            RuntimeStateBackend::Memory(memory) => Ok(memory.score_len(key).await),
            RuntimeStateBackend::Redis(redis) => redis.runtime.score_len(key).await,
        }
    }

    pub async fn key_expire(&self, key: &str, ttl: Duration) -> Result<bool, DataLayerError> {
        match self.backend.as_ref() {
            RuntimeStateBackend::Memory(memory) => Ok(memory.key_expire(key, ttl).await),
            RuntimeStateBackend::Redis(redis) => redis.runtime.key_expire(key, ttl).await,
        }
    }

    pub async fn lock_try_acquire(
        &self,
        key: &str,
        owner: &str,
        ttl: Duration,
    ) -> Result<Option<RuntimeLockLease>, DataLayerError> {
        if owner.trim().is_empty() || key.trim().is_empty() {
            return Err(DataLayerError::InvalidInput(
                "runtime lock key and owner cannot be empty".to_string(),
            ));
        }
        if ttl.is_zero() {
            return Err(DataLayerError::InvalidInput(
                "runtime lock ttl must be positive".to_string(),
            ));
        }
        let token = format!("{owner}:{}", Uuid::new_v4());
        match self.backend.as_ref() {
            RuntimeStateBackend::Memory(memory) => {
                if let Some(fencing_token) = memory
                    .lock_try_acquire(key, owner, token.clone(), ttl)
                    .await
                {
                    Ok(Some(RuntimeLockLease {
                        key: key.to_string(),
                        owner: owner.to_string(),
                        token,
                        fencing_token,
                        ttl_ms: ttl.as_millis().try_into().unwrap_or(u64::MAX),
                    }))
                } else {
                    Ok(None)
                }
            }
            RuntimeStateBackend::Redis(redis) => {
                let lease = redis
                    .lock
                    .try_acquire(
                        &redis.keyspace.lock_key(key),
                        owner,
                        Some(ttl.as_millis().try_into().unwrap_or(u64::MAX)),
                    )
                    .await?;
                Ok(lease.map(|lease| RuntimeLockLease {
                    key: key.to_string(),
                    owner: lease.owner,
                    token: lease.token,
                    fencing_token: lease.fencing_token,
                    ttl_ms: lease.ttl_ms,
                }))
            }
        }
    }

    pub async fn lock_release(&self, lease: &RuntimeLockLease) -> Result<bool, DataLayerError> {
        match self.backend.as_ref() {
            RuntimeStateBackend::Memory(memory) => {
                Ok(memory.lock_release(&lease.key, &lease.token).await)
            }
            RuntimeStateBackend::Redis(redis) => {
                redis
                    .lock
                    .release(&RedisLockLease {
                        key: redis.keyspace.lock_key(&lease.key),
                        owner: lease.owner.clone(),
                        token: lease.token.clone(),
                        fencing_token: lease.fencing_token,
                        ttl_ms: lease.ttl_ms,
                    })
                    .await
            }
        }
    }

    pub async fn lock_renew(
        &self,
        lease: &RuntimeLockLease,
        ttl: Duration,
    ) -> Result<bool, DataLayerError> {
        if ttl.is_zero() {
            return Err(DataLayerError::InvalidInput(
                "runtime lock ttl must be positive".to_string(),
            ));
        }
        match self.backend.as_ref() {
            RuntimeStateBackend::Memory(memory) => {
                Ok(memory.lock_renew(&lease.key, &lease.token, ttl).await)
            }
            RuntimeStateBackend::Redis(redis) => {
                redis
                    .lock
                    .renew(
                        &RedisLockLease {
                            key: redis.keyspace.lock_key(&lease.key),
                            owner: lease.owner.clone(),
                            token: lease.token.clone(),
                            fencing_token: lease.fencing_token,
                            ttl_ms: lease.ttl_ms,
                        },
                        Some(ttl.as_millis().try_into().unwrap_or(u64::MAX)),
                    )
                    .await
            }
        }
    }

    pub fn semaphore(
        &self,
        gate: &'static str,
        limit: usize,
        config: RuntimeSemaphoreConfig,
    ) -> Result<RuntimeSemaphore, RuntimeSemaphoreError> {
        RuntimeSemaphore::new(self.clone(), gate, limit, config)
    }

    /// Atomically reserves all configured execution concurrency and RPM scopes.
    ///
    /// Persistent observations and live runtime reservations are de-duplicated by candidate ID.
    /// Dropping the returned permit releases concurrency reservations; an RPM consumption remains
    /// until it leaves the rolling window or is hidden by the supplied reset watermark.
    pub async fn try_acquire_execution_reservation(
        &self,
        input: ExecutionReservationInput,
    ) -> Result<ExecutionReservationPermit, ExecutionReservationError> {
        input.validate()?;

        let concurrency_keys = input.concurrency_runtime_keys();
        self.try_acquire_execution_reservation_input(&input).await?;
        let provider_key_rpm = input.provider_key_rpm.map(|mut reservation| {
            let observed_candidate_ids = reservation
                .observed_candidate_ids
                .iter()
                .map(String::as_str)
                .collect::<BTreeSet<_>>();
            reservation.observed_count_floor = reservation
                .observed_count_floor
                .max(observed_candidate_ids.len())
                .saturating_add(usize::from(
                    !observed_candidate_ids.contains(input.candidate_id.as_str()),
                ));
            reservation
        });

        let healthy = Arc::new(std::sync::atomic::AtomicBool::new(true));
        let renew_task = if concurrency_keys.is_empty() {
            None
        } else {
            let runtime = self.clone();
            let candidate_id = input.candidate_id.clone();
            let renew_keys = concurrency_keys.clone();
            let renew_health = Arc::clone(&healthy);
            Some(tokio::spawn(async move {
                let interval = Duration::from_millis(EXECUTION_RESERVATION_RENEW_INTERVAL_MS);
                loop {
                    tokio::time::sleep(interval).await;
                    if let Err(err) = runtime
                        .renew_execution_reservation(&renew_keys, &candidate_id)
                        .await
                    {
                        renew_health.store(false, Ordering::Release);
                        warn!(
                            candidate_id,
                            error = %err,
                            "failed to renew execution reservation"
                        );
                        break;
                    }
                }
            }))
        };

        Ok(ExecutionReservationPermit {
            runtime: self.clone(),
            candidate_id: input.candidate_id,
            concurrency_keys,
            provider_key_rpm,
            additional_rpm_attempt_sequence: 0,
            renew_task,
            healthy,
        })
    }

    async fn try_acquire_execution_reservation_input(
        &self,
        input: &ExecutionReservationInput,
    ) -> Result<(), ExecutionReservationError> {
        let result = match self.backend.as_ref() {
            RuntimeStateBackend::Memory(memory) => {
                memory
                    .execution_reservation_try_acquire(
                        input,
                        EXECUTION_RESERVATION_LEASE_TTL_MS,
                        EXECUTION_RESERVATION_RPM_WINDOW_SECS,
                    )
                    .await
            }
            RuntimeStateBackend::Redis(redis) => redis
                .runtime
                .execution_reservation_try_acquire(
                    input,
                    EXECUTION_RESERVATION_LEASE_TTL_MS,
                    EXECUTION_RESERVATION_RPM_WINDOW_SECS,
                )
                .await
                .map_err(|err| ExecutionReservationError::Unavailable {
                    message: format!("acquire failed: {err}"),
                })?,
        };
        if let Some((scope, limit)) = result {
            return Err(ExecutionReservationError::Rejected { scope, limit });
        }
        Ok(())
    }

    async fn renew_execution_reservation(
        &self,
        concurrency_keys: &[String],
        candidate_id: &str,
    ) -> Result<(), ExecutionReservationError> {
        let renewed = match self.backend.as_ref() {
            RuntimeStateBackend::Memory(memory) => {
                memory
                    .execution_reservation_renew(
                        concurrency_keys,
                        candidate_id,
                        EXECUTION_RESERVATION_LEASE_TTL_MS,
                    )
                    .await
            }
            RuntimeStateBackend::Redis(redis) => redis
                .runtime
                .execution_reservation_renew(
                    concurrency_keys,
                    candidate_id,
                    EXECUTION_RESERVATION_LEASE_TTL_MS,
                )
                .await
                .map_err(|err| ExecutionReservationError::Unavailable {
                    message: format!("renew failed: {err}"),
                })?,
        };
        if renewed {
            Ok(())
        } else {
            Err(ExecutionReservationError::Unavailable {
                message: "one or more execution reservation leases expired".to_string(),
            })
        }
    }

    async fn release_execution_reservation(
        &self,
        concurrency_keys: &[String],
        candidate_id: &str,
    ) -> Result<(), ExecutionReservationError> {
        match self.backend.as_ref() {
            RuntimeStateBackend::Memory(memory) => {
                memory
                    .execution_reservation_release(concurrency_keys, candidate_id)
                    .await;
                Ok(())
            }
            RuntimeStateBackend::Redis(redis) => redis
                .runtime
                .execution_reservation_release(concurrency_keys, candidate_id)
                .await
                .map_err(|err| ExecutionReservationError::Unavailable {
                    message: format!("release failed: {err}"),
                }),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ExecutionReservationScope {
    Provider,
    ProviderKey,
    ApiKey,
    ProviderKeyRpm,
}

impl ExecutionReservationScope {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Provider => "provider",
            Self::ProviderKey => "provider_key",
            Self::ApiKey => "api_key",
            Self::ProviderKeyRpm => "provider_key_rpm",
        }
    }

    const fn redis_code(self) -> i64 {
        match self {
            Self::Provider => 1,
            Self::ProviderKey => 2,
            Self::ApiKey => 3,
            Self::ProviderKeyRpm => 4,
        }
    }

    fn from_redis_code(value: i64) -> Self {
        match value {
            2 => Self::ProviderKey,
            3 => Self::ApiKey,
            4 => Self::ProviderKeyRpm,
            _ => Self::Provider,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExecutionConcurrencyReservation {
    pub key: String,
    pub limit: usize,
    pub observed_candidate_ids: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExecutionRpmReservation {
    pub key: String,
    pub limit: usize,
    pub observed_candidate_ids: Vec<String>,
    pub observed_count_floor: usize,
    pub reset_after_unix_secs: Option<u64>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExecutionReservationInput {
    pub candidate_id: String,
    pub provider: Option<ExecutionConcurrencyReservation>,
    pub provider_key: Option<ExecutionConcurrencyReservation>,
    pub api_key: Option<ExecutionConcurrencyReservation>,
    pub provider_key_rpm: Option<ExecutionRpmReservation>,
}

impl ExecutionReservationInput {
    fn validate(&self) -> Result<(), ExecutionReservationError> {
        if self.candidate_id.trim().is_empty() {
            return Err(ExecutionReservationError::InvalidConfiguration(
                "execution reservation candidate_id cannot be empty".to_string(),
            ));
        }
        if self.provider.is_none()
            && self.provider_key.is_none()
            && self.api_key.is_none()
            && self.provider_key_rpm.is_none()
        {
            return Err(ExecutionReservationError::InvalidConfiguration(
                "execution reservation requires at least one scope".to_string(),
            ));
        }
        for (scope, reservation) in self.configured_concurrency_scopes() {
            if reservation.limit == 0 {
                return Err(ExecutionReservationError::InvalidConfiguration(format!(
                    "execution reservation {} limit must be positive",
                    scope.as_str()
                )));
            }
        }
        for (scope, key) in self
            .concurrency_scopes()
            .map(|(scope, reservation)| (scope, reservation.key.as_str()))
            .chain(self.provider_key_rpm.iter().map(|reservation| {
                (
                    ExecutionReservationScope::ProviderKeyRpm,
                    reservation.key.as_str(),
                )
            }))
        {
            if key.trim().is_empty() {
                return Err(ExecutionReservationError::InvalidConfiguration(format!(
                    "execution reservation {} key cannot be empty",
                    scope.as_str()
                )));
            }
        }
        Ok(())
    }

    fn concurrency_scopes(
        &self,
    ) -> impl Iterator<Item = (ExecutionReservationScope, &ExecutionConcurrencyReservation)> {
        self.configured_concurrency_scopes()
            .filter(|(_, reservation)| reservation.limit > 0)
    }

    fn configured_concurrency_scopes(
        &self,
    ) -> impl Iterator<Item = (ExecutionReservationScope, &ExecutionConcurrencyReservation)> {
        [
            (ExecutionReservationScope::Provider, self.provider.as_ref()),
            (
                ExecutionReservationScope::ProviderKey,
                self.provider_key.as_ref(),
            ),
            (ExecutionReservationScope::ApiKey, self.api_key.as_ref()),
        ]
        .into_iter()
        .filter_map(|(scope, reservation)| reservation.map(|reservation| (scope, reservation)))
    }

    fn concurrency_runtime_keys(&self) -> Vec<String> {
        self.concurrency_scopes()
            .map(|(scope, reservation)| execution_reservation_runtime_key(scope, &reservation.key))
            .collect()
    }
}

fn execution_reservation_runtime_key(scope: ExecutionReservationScope, key: &str) -> String {
    format!("execution-reservation:{}:{key}", scope.as_str())
}

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum ExecutionReservationError {
    #[error("execution reservation {scope:?} is saturated at {limit}")]
    Rejected {
        scope: ExecutionReservationScope,
        limit: usize,
    },
    #[error("execution reservation is unavailable: {message}")]
    Unavailable { message: String },
    #[error("{0}")]
    InvalidConfiguration(String),
}

#[derive(Debug)]
pub struct ExecutionReservationPermit {
    runtime: RuntimeState,
    candidate_id: String,
    concurrency_keys: Vec<String>,
    provider_key_rpm: Option<ExecutionRpmReservation>,
    additional_rpm_attempt_sequence: u64,
    renew_task: Option<JoinHandle<()>>,
    healthy: Arc<std::sync::atomic::AtomicBool>,
}

#[derive(Debug, Clone)]
pub struct ExecutionReservationHealth {
    healthy: Arc<std::sync::atomic::AtomicBool>,
}

impl ExecutionReservationHealth {
    pub fn is_healthy(&self) -> bool {
        self.healthy.load(Ordering::Acquire)
    }
}

impl aether_runtime::AdmissionPermitHealth for ExecutionReservationHealth {
    fn is_healthy(&self) -> bool {
        self.is_healthy()
    }
}

impl ExecutionReservationPermit {
    pub fn health_handle(&self) -> ExecutionReservationHealth {
        ExecutionReservationHealth {
            healthy: Arc::clone(&self.healthy),
        }
    }

    /// Atomically consumes provider-key RPM for another upstream attempt made by this permit.
    ///
    /// The additional attempt gets its own runtime member and does not acquire the concurrency
    /// scopes again. A rejected consumption has no runtime side effects and does not advance the
    /// attempt sequence, so the caller may retry after the rolling window changes.
    pub async fn consume_additional_rpm_attempt(
        &mut self,
    ) -> Result<(), ExecutionReservationError> {
        let Some(provider_key_rpm) = self.provider_key_rpm.clone() else {
            return Ok(());
        };
        let sequence = self
            .additional_rpm_attempt_sequence
            .checked_add(1)
            .ok_or_else(|| ExecutionReservationError::Unavailable {
                message: "additional RPM attempt sequence exhausted".to_string(),
            })?;
        let input = ExecutionReservationInput {
            candidate_id: format!("{}:attempt:{sequence}", self.candidate_id),
            provider: None,
            provider_key: None,
            api_key: None,
            provider_key_rpm: Some(provider_key_rpm),
        };
        input.validate()?;
        self.runtime
            .try_acquire_execution_reservation_input(&input)
            .await?;
        self.additional_rpm_attempt_sequence = sequence;
        if let Some(provider_key_rpm) = self.provider_key_rpm.as_mut() {
            provider_key_rpm.observed_count_floor =
                provider_key_rpm.observed_count_floor.saturating_add(1);
        }
        Ok(())
    }

    /// Releases concurrency scopes synchronously. RPM consumption is intentionally retained.
    pub async fn release(mut self) -> Result<(), ExecutionReservationError> {
        if let Some(renew_task) = self.renew_task.take() {
            renew_task.abort();
        }
        if self.concurrency_keys.is_empty() {
            return Ok(());
        }
        self.runtime
            .release_execution_reservation(&self.concurrency_keys, &self.candidate_id)
            .await?;
        self.concurrency_keys.clear();
        Ok(())
    }
}

impl aether_runtime::AdmissionPermitHealth for ExecutionReservationPermit {
    fn is_healthy(&self) -> bool {
        self.health_handle().is_healthy()
    }
}

impl Drop for ExecutionReservationPermit {
    fn drop(&mut self) {
        if let Some(renew_task) = self.renew_task.take() {
            renew_task.abort();
        }
        if self.concurrency_keys.is_empty() {
            return;
        }
        let runtime = self.runtime.clone();
        let candidate_id = self.candidate_id.clone();
        let concurrency_keys = self.concurrency_keys.clone();
        if let Ok(handle) = tokio::runtime::Handle::try_current() {
            handle.spawn(async move {
                if let Err(err) = runtime
                    .release_execution_reservation(&concurrency_keys, &candidate_id)
                    .await
                {
                    warn!(
                        candidate_id,
                        error = %err,
                        "failed to release execution reservation"
                    );
                }
            });
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RuntimeLockLease {
    pub key: String,
    pub owner: String,
    pub token: String,
    pub fencing_token: u64,
    pub ttl_ms: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RateLimitScope {
    User,
    Key,
}

impl RateLimitScope {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::User => "user",
            Self::Key => "key",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RateLimitCheck {
    Allowed { remaining: u32 },
    Rejected { scope: RateLimitScope, limit: u32 },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RateLimitInput<'a> {
    pub user_key: &'a str,
    pub key_key: &'a str,
    pub bucket: u64,
    pub user_limit: u32,
    pub key_limit: u32,
    pub ttl_seconds: u64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RuntimeQueueEntry {
    pub id: String,
    pub fields: BTreeMap<String, String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct RuntimeQueueStats {
    pub stream_length: u64,
    pub group_pending: u64,
    pub group_lag: Option<u64>,
    pub oldest_pending_idle_ms: Option<u64>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RuntimeQueueReclaimConfig {
    pub min_idle_ms: u64,
    pub count: usize,
}

fn validate_runtime_queue_name(value: &str, field: &str) -> Result<(), DataLayerError> {
    if value.trim().is_empty() {
        return Err(DataLayerError::InvalidInput(format!(
            "{field} cannot be empty"
        )));
    }
    Ok(())
}

fn validate_runtime_queue_reclaim_config(
    config: RuntimeQueueReclaimConfig,
) -> Result<(), DataLayerError> {
    if config.min_idle_ms == 0 {
        return Err(DataLayerError::InvalidInput(
            "runtime queue reclaim min_idle_ms must be positive".to_string(),
        ));
    }
    if config.count == 0 {
        return Err(DataLayerError::InvalidInput(
            "runtime queue reclaim count must be positive".to_string(),
        ));
    }
    Ok(())
}

#[async_trait]
pub trait RuntimeQueueStore: Send + Sync {
    async fn ensure_consumer_group(
        &self,
        stream: &str,
        group: &str,
        start_id: &str,
    ) -> Result<(), DataLayerError>;

    async fn append_fields_with_maxlen(
        &self,
        stream: &str,
        fields: &BTreeMap<String, String>,
        maxlen: Option<usize>,
    ) -> Result<String, DataLayerError>;

    async fn read_group(
        &self,
        stream: &str,
        group: &str,
        consumer: &str,
        count: usize,
        block_ms: Option<u64>,
    ) -> Result<Vec<RuntimeQueueEntry>, DataLayerError>;

    async fn claim_stale(
        &self,
        stream: &str,
        group: &str,
        consumer: &str,
        start_id: &str,
        config: RuntimeQueueReclaimConfig,
    ) -> Result<Vec<RuntimeQueueEntry>, DataLayerError>;

    async fn ack(&self, stream: &str, group: &str, ids: &[String])
        -> Result<usize, DataLayerError>;

    async fn delete(&self, stream: &str, ids: &[String]) -> Result<usize, DataLayerError>;

    async fn stats(
        &self,
        stream: &str,
        group: Option<&str>,
    ) -> Result<RuntimeQueueStats, DataLayerError>;
}

#[async_trait]
impl RuntimeQueueStore for RuntimeState {
    async fn ensure_consumer_group(
        &self,
        stream: &str,
        group: &str,
        start_id: &str,
    ) -> Result<(), DataLayerError> {
        validate_runtime_queue_name(stream, "runtime queue stream")?;
        validate_runtime_queue_name(group, "runtime queue group")?;
        validate_runtime_queue_name(start_id, "runtime queue start id")?;
        match self.backend.as_ref() {
            RuntimeStateBackend::Memory(memory) => {
                memory
                    .queue_ensure_consumer_group(stream, group, start_id)
                    .await
            }
            RuntimeStateBackend::Redis(redis) => {
                redis
                    .stream
                    .ensure_consumer_group(
                        &RedisStreamName(stream.to_string()),
                        &RedisConsumerGroup(group.to_string()),
                        start_id,
                    )
                    .await
            }
        }
    }

    async fn append_fields_with_maxlen(
        &self,
        stream: &str,
        fields: &BTreeMap<String, String>,
        maxlen: Option<usize>,
    ) -> Result<String, DataLayerError> {
        validate_runtime_queue_name(stream, "runtime queue stream")?;
        if fields.is_empty() {
            return Err(DataLayerError::InvalidInput(
                "runtime queue fields cannot be empty".to_string(),
            ));
        }
        match self.backend.as_ref() {
            RuntimeStateBackend::Memory(memory) => {
                Ok(memory.queue_append(stream, fields.clone(), maxlen).await)
            }
            RuntimeStateBackend::Redis(redis) => {
                redis
                    .stream
                    .append_fields_with_maxlen(&RedisStreamName(stream.to_string()), fields, maxlen)
                    .await
            }
        }
    }

    async fn read_group(
        &self,
        stream: &str,
        group: &str,
        consumer: &str,
        count: usize,
        block_ms: Option<u64>,
    ) -> Result<Vec<RuntimeQueueEntry>, DataLayerError> {
        validate_runtime_queue_name(stream, "runtime queue stream")?;
        validate_runtime_queue_name(group, "runtime queue group")?;
        validate_runtime_queue_name(consumer, "runtime queue consumer")?;
        if matches!(block_ms, Some(0)) {
            return Err(DataLayerError::InvalidInput(
                "runtime queue block_ms must be positive".to_string(),
            ));
        }
        match self.backend.as_ref() {
            RuntimeStateBackend::Memory(memory) => {
                memory
                    .queue_read(stream, group, consumer, count, block_ms)
                    .await
            }
            RuntimeStateBackend::Redis(redis) => {
                let runner = redis.stream.with_config(RedisStreamRunnerConfig {
                    command_timeout_ms: redis_stream_command_timeout_for_block(
                        redis.command_timeout_ms,
                        block_ms,
                    ),
                    read_block_ms: block_ms,
                    read_count: count.max(1),
                })?;
                Ok(runner
                    .read_group(
                        &RedisStreamName(stream.to_string()),
                        &RedisConsumerGroup(group.to_string()),
                        &RedisConsumerName(consumer.to_string()),
                    )
                    .await?
                    .into_iter()
                    .map(|entry| RuntimeQueueEntry {
                        id: entry.id,
                        fields: entry.fields,
                    })
                    .collect())
            }
        }
    }

    async fn claim_stale(
        &self,
        stream: &str,
        group: &str,
        consumer: &str,
        start_id: &str,
        config: RuntimeQueueReclaimConfig,
    ) -> Result<Vec<RuntimeQueueEntry>, DataLayerError> {
        validate_runtime_queue_name(stream, "runtime queue stream")?;
        validate_runtime_queue_name(group, "runtime queue group")?;
        validate_runtime_queue_name(consumer, "runtime queue consumer")?;
        validate_runtime_queue_name(start_id, "runtime queue start id")?;
        validate_runtime_queue_reclaim_config(config)?;
        match self.backend.as_ref() {
            RuntimeStateBackend::Memory(memory) => {
                memory
                    .queue_claim_stale(stream, group, consumer, start_id, config)
                    .await
            }
            RuntimeStateBackend::Redis(redis) => Ok(redis
                .stream
                .claim_stale(
                    &RedisStreamName(stream.to_string()),
                    &RedisConsumerGroup(group.to_string()),
                    &RedisConsumerName(consumer.to_string()),
                    start_id,
                    RedisStreamReclaimConfig {
                        min_idle_ms: config.min_idle_ms,
                        count: config.count,
                    },
                )
                .await?
                .entries
                .into_iter()
                .map(|entry| RuntimeQueueEntry {
                    id: entry.id,
                    fields: entry.fields,
                })
                .collect()),
        }
    }

    async fn ack(
        &self,
        stream: &str,
        group: &str,
        ids: &[String],
    ) -> Result<usize, DataLayerError> {
        validate_runtime_queue_name(stream, "runtime queue stream")?;
        validate_runtime_queue_name(group, "runtime queue group")?;
        match self.backend.as_ref() {
            RuntimeStateBackend::Memory(memory) => memory.queue_ack(stream, group, ids).await,
            RuntimeStateBackend::Redis(redis) => {
                redis
                    .stream
                    .ack(
                        &RedisStreamName(stream.to_string()),
                        &RedisConsumerGroup(group.to_string()),
                        ids,
                    )
                    .await
            }
        }
    }

    async fn delete(&self, stream: &str, ids: &[String]) -> Result<usize, DataLayerError> {
        validate_runtime_queue_name(stream, "runtime queue stream")?;
        match self.backend.as_ref() {
            RuntimeStateBackend::Memory(memory) => Ok(memory.queue_delete(stream, ids).await),
            RuntimeStateBackend::Redis(redis) => {
                redis
                    .stream
                    .delete(&RedisStreamName(stream.to_string()), ids)
                    .await
            }
        }
    }

    async fn stats(
        &self,
        stream: &str,
        group: Option<&str>,
    ) -> Result<RuntimeQueueStats, DataLayerError> {
        validate_runtime_queue_name(stream, "runtime queue stream")?;
        if let Some(group) = group {
            validate_runtime_queue_name(group, "runtime queue group")?;
        }
        match self.backend.as_ref() {
            RuntimeStateBackend::Memory(memory) => Ok(memory.queue_stats(stream, group).await),
            RuntimeStateBackend::Redis(redis) => {
                redis
                    .stream
                    .stats(
                        &RedisStreamName(stream.to_string()),
                        group
                            .map(|value| RedisConsumerGroup(value.to_string()))
                            .as_ref(),
                    )
                    .await
            }
        }
    }
}

#[async_trait]
pub trait ExpiringKvStore: Send + Sync {
    async fn set(
        &self,
        key: &str,
        value: String,
        ttl: Option<Duration>,
    ) -> Result<(), DataLayerError>;
    async fn get(&self, key: &str) -> Result<Option<String>, DataLayerError>;
    async fn get_many(&self, keys: &[String]) -> Result<Vec<Option<String>>, DataLayerError>;
    async fn take(&self, key: &str) -> Result<Option<String>, DataLayerError>;
    async fn delete(&self, key: &str) -> Result<bool, DataLayerError>;
    async fn exists(&self, key: &str) -> Result<bool, DataLayerError>;
}

#[async_trait]
impl ExpiringKvStore for RuntimeState {
    async fn set(
        &self,
        key: &str,
        value: String,
        ttl: Option<Duration>,
    ) -> Result<(), DataLayerError> {
        self.kv_set(key, value, ttl).await
    }

    async fn get(&self, key: &str) -> Result<Option<String>, DataLayerError> {
        self.kv_get(key).await
    }

    async fn get_many(&self, keys: &[String]) -> Result<Vec<Option<String>>, DataLayerError> {
        self.kv_get_many(keys).await
    }

    async fn take(&self, key: &str) -> Result<Option<String>, DataLayerError> {
        self.kv_take(key).await
    }

    async fn delete(&self, key: &str) -> Result<bool, DataLayerError> {
        self.kv_delete(key).await
    }

    async fn exists(&self, key: &str) -> Result<bool, DataLayerError> {
        self.kv_exists(key).await
    }
}

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum RuntimeSemaphoreError {
    #[error("runtime semaphore {gate} is saturated at {limit}")]
    Saturated { gate: &'static str, limit: usize },
    #[error("runtime semaphore {gate} is unavailable: {message}")]
    Unavailable {
        gate: &'static str,
        limit: usize,
        message: String,
    },
    #[error("{0}")]
    InvalidConfiguration(String),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RuntimeSemaphoreSnapshot {
    pub limit: usize,
    pub in_flight: usize,
    pub available_permits: usize,
    pub high_watermark: usize,
    pub rejected: u64,
}

impl RuntimeSemaphoreSnapshot {
    pub fn to_metric_samples(&self, gate: &'static str) -> Vec<aether_runtime::MetricSample> {
        let labels = vec![aether_runtime::MetricLabel::new("gate", gate)];
        vec![
            aether_runtime::MetricSample::new(
                "concurrency_in_flight",
                "Current number of in-flight operations guarded by the concurrency gate.",
                aether_runtime::MetricKind::Gauge,
                self.in_flight as u64,
            )
            .with_labels(labels.clone()),
            aether_runtime::MetricSample::new(
                "concurrency_available_permits",
                "Currently available permits for the concurrency gate.",
                aether_runtime::MetricKind::Gauge,
                self.available_permits as u64,
            )
            .with_labels(labels.clone()),
            aether_runtime::MetricSample::new(
                "concurrency_high_watermark",
                "Highest observed in-flight count for the concurrency gate.",
                aether_runtime::MetricKind::Gauge,
                self.high_watermark as u64,
            )
            .with_labels(labels.clone()),
            aether_runtime::MetricSample::new(
                "concurrency_rejected_total",
                "Number of operations rejected by the concurrency gate.",
                aether_runtime::MetricKind::Counter,
                self.rejected,
            )
            .with_labels(labels),
        ]
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RuntimeSemaphoreConfig {
    pub lease_ttl_ms: u64,
    pub renew_interval_ms: u64,
    pub command_timeout_ms: Option<u64>,
}

impl Default for RuntimeSemaphoreConfig {
    fn default() -> Self {
        Self {
            lease_ttl_ms: 30_000,
            renew_interval_ms: 10_000,
            command_timeout_ms: Some(DEFAULT_COMMAND_TIMEOUT_MS),
        }
    }
}

#[derive(Debug, Clone)]
pub struct RuntimeSemaphore {
    state: Arc<RuntimeSemaphoreState>,
}

#[derive(Debug)]
struct RuntimeSemaphoreState {
    runtime: RuntimeState,
    gate: &'static str,
    limit: usize,
    key: String,
    config: RuntimeSemaphoreConfig,
    high_watermark: AtomicUsize,
    rejected: AtomicU64,
}

impl RuntimeSemaphore {
    fn new(
        runtime: RuntimeState,
        gate: &'static str,
        limit: usize,
        config: RuntimeSemaphoreConfig,
    ) -> Result<Self, RuntimeSemaphoreError> {
        if limit == 0 {
            return Err(RuntimeSemaphoreError::InvalidConfiguration(
                "runtime semaphore limit must be positive".to_string(),
            ));
        }
        if config.lease_ttl_ms == 0 || config.renew_interval_ms == 0 {
            return Err(RuntimeSemaphoreError::InvalidConfiguration(
                "runtime semaphore lease and renew intervals must be positive".to_string(),
            ));
        }
        if config.renew_interval_ms >= config.lease_ttl_ms {
            return Err(RuntimeSemaphoreError::InvalidConfiguration(
                "runtime semaphore renew_interval_ms must be smaller than lease_ttl_ms".to_string(),
            ));
        }
        Ok(Self {
            state: Arc::new(RuntimeSemaphoreState {
                key: format!("admission:{gate}"),
                runtime,
                gate,
                limit,
                config,
                high_watermark: AtomicUsize::new(0),
                rejected: AtomicU64::new(0),
            }),
        })
    }

    pub fn gate(&self) -> &'static str {
        self.state.gate
    }

    pub fn limit(&self) -> usize {
        self.state.limit
    }

    pub async fn try_acquire(&self) -> Result<RuntimeSemaphorePermit, RuntimeSemaphoreError> {
        self.state.try_acquire().await
    }

    pub async fn snapshot(&self) -> Result<RuntimeSemaphoreSnapshot, RuntimeSemaphoreError> {
        self.state.snapshot().await
    }
}

#[derive(Debug)]
pub struct RuntimeSemaphorePermit {
    state: Arc<RuntimeSemaphoreState>,
    token: String,
    renew_task: JoinHandle<()>,
    healthy: Arc<std::sync::atomic::AtomicBool>,
}

impl aether_runtime::AdmissionPermitHealth for RuntimeSemaphorePermit {
    fn is_healthy(&self) -> bool {
        self.healthy.load(Ordering::Acquire)
    }
}

impl Drop for RuntimeSemaphorePermit {
    fn drop(&mut self) {
        self.renew_task.abort();
        let state = Arc::clone(&self.state);
        let token = self.token.clone();
        tokio::spawn(async move {
            if let Err(err) = state.release(&token).await {
                warn!(
                    gate = state.gate,
                    error = %err,
                    "failed to release runtime semaphore permit"
                );
            }
        });
    }
}

impl RuntimeSemaphoreState {
    async fn try_acquire(
        self: &Arc<Self>,
    ) -> Result<RuntimeSemaphorePermit, RuntimeSemaphoreError> {
        let token = format!("{}:{}", self.gate, Uuid::new_v4());
        let in_flight = match self.runtime.backend.as_ref() {
            RuntimeStateBackend::Memory(memory) => memory
                .semaphore_try_acquire(
                    &self.key,
                    token.clone(),
                    self.limit,
                    self.config.lease_ttl_ms,
                )
                .await
                .map_err(|count| {
                    self.rejected.fetch_add(1, Ordering::Relaxed);
                    self.observe_in_flight(count);
                    RuntimeSemaphoreError::Saturated {
                        gate: self.gate,
                        limit: self.limit,
                    }
                })?,
            RuntimeStateBackend::Redis(redis) => self.redis_try_acquire(redis, &token).await?,
        };
        self.observe_in_flight(in_flight);

        let renew_state = Arc::clone(self);
        let renew_token = token.clone();
        let healthy = Arc::new(std::sync::atomic::AtomicBool::new(true));
        let renew_health = Arc::clone(&healthy);
        let renew_task = tokio::spawn(async move {
            let interval = Duration::from_millis(renew_state.config.renew_interval_ms);
            loop {
                tokio::time::sleep(interval).await;
                if let Err(err) = renew_state.renew(&renew_token).await {
                    renew_health.store(false, Ordering::Release);
                    warn!(
                        gate = renew_state.gate,
                        error = %err,
                        "failed to renew runtime semaphore permit"
                    );
                    break;
                }
            }
        });
        Ok(RuntimeSemaphorePermit {
            state: Arc::clone(self),
            token,
            renew_task,
            healthy,
        })
    }

    async fn snapshot(&self) -> Result<RuntimeSemaphoreSnapshot, RuntimeSemaphoreError> {
        let in_flight = self.live_count().await?;
        Ok(RuntimeSemaphoreSnapshot {
            limit: self.limit,
            in_flight,
            available_permits: self.limit.saturating_sub(in_flight),
            high_watermark: self.high_watermark.load(Ordering::Relaxed),
            rejected: self.rejected.load(Ordering::Relaxed),
        })
    }

    async fn redis_try_acquire(
        &self,
        redis: &RedisRuntimeBackend,
        token: &str,
    ) -> Result<usize, RuntimeSemaphoreError> {
        let result = redis
            .runtime
            .semaphore_try_acquire(
                self.gate,
                self.limit,
                &self.key,
                token,
                self.config.lease_ttl_ms,
                self.config.command_timeout_ms,
            )
            .await?;
        let acquired = result.0 > 0;
        let in_flight = result.1.max(0) as usize;
        if !acquired {
            self.rejected.fetch_add(1, Ordering::Relaxed);
            self.observe_in_flight(in_flight);
            return Err(RuntimeSemaphoreError::Saturated {
                gate: self.gate,
                limit: self.limit,
            });
        }
        Ok(in_flight)
    }

    async fn renew(&self, token: &str) -> Result<(), RuntimeSemaphoreError> {
        match self.runtime.backend.as_ref() {
            RuntimeStateBackend::Memory(memory) => {
                if memory
                    .semaphore_renew(&self.key, token, self.config.lease_ttl_ms)
                    .await
                {
                    Ok(())
                } else {
                    Err(self.unavailable("lease token expired".to_string()))
                }
            }
            RuntimeStateBackend::Redis(redis) => {
                let renewed = redis
                    .runtime
                    .semaphore_renew(
                        self.gate,
                        self.limit,
                        &self.key,
                        token,
                        self.config.lease_ttl_ms,
                        self.config.command_timeout_ms,
                    )
                    .await?;
                if renewed == 0 {
                    return Err(self.unavailable("lease token expired".to_string()));
                }
                Ok(())
            }
        }
    }

    async fn release(&self, token: &str) -> Result<(), RuntimeSemaphoreError> {
        match self.runtime.backend.as_ref() {
            RuntimeStateBackend::Memory(memory) => {
                memory.semaphore_release(&self.key, token).await;
                Ok(())
            }
            RuntimeStateBackend::Redis(redis) => {
                redis
                    .runtime
                    .semaphore_release(
                        self.gate,
                        self.limit,
                        &self.key,
                        token,
                        self.config.command_timeout_ms,
                    )
                    .await
            }
        }
    }

    async fn live_count(&self) -> Result<usize, RuntimeSemaphoreError> {
        let count = match self.runtime.backend.as_ref() {
            RuntimeStateBackend::Memory(memory) => memory.semaphore_live_count(&self.key).await,
            RuntimeStateBackend::Redis(redis) => {
                redis
                    .runtime
                    .semaphore_live_count(
                        self.gate,
                        self.limit,
                        &self.key,
                        self.config.command_timeout_ms,
                    )
                    .await?
            }
        };
        self.observe_in_flight(count);
        Ok(count)
    }

    fn unavailable(&self, message: String) -> RuntimeSemaphoreError {
        RuntimeSemaphoreError::Unavailable {
            gate: self.gate,
            limit: self.limit,
            message,
        }
    }

    fn observe_in_flight(&self, in_flight: usize) {
        let mut observed = self.high_watermark.load(Ordering::Acquire);
        while in_flight > observed {
            match self.high_watermark.compare_exchange_weak(
                observed,
                in_flight,
                Ordering::AcqRel,
                Ordering::Acquire,
            ) {
                Ok(_) => break,
                Err(next) => observed = next,
            }
        }
    }
}

fn env_value(name: &str) -> Option<String> {
    std::env::var(name)
        .ok()
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
}

fn unix_time_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64
}

fn redis_stream_command_timeout_for_block(
    command_timeout_ms: Option<u64>,
    read_block_ms: Option<u64>,
) -> Option<u64> {
    match (command_timeout_ms, read_block_ms) {
        (Some(timeout_ms), Some(block_ms)) => {
            Some(timeout_ms.max(block_ms.saturating_add(DEFAULT_STREAM_BLOCK_TIMEOUT_GRACE_MS)))
        }
        (Some(timeout_ms), None) => Some(timeout_ms),
        (None, _) => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;
    use std::process::{Child, Command, Stdio};
    use tokio::io::{AsyncReadExt, AsyncWriteExt};

    #[tokio::test]
    async fn memory_kv_expires_entries() {
        let runtime = RuntimeState::memory(MemoryRuntimeStateConfig::default());
        runtime
            .kv_set("hello", "world", Some(Duration::from_millis(5)))
            .await
            .expect("set should succeed");
        tokio::time::sleep(Duration::from_millis(10)).await;
        assert_eq!(runtime.kv_get("hello").await.expect("get"), None);
        assert!(!runtime.kv_exists("hello").await.expect("exists"));
    }

    #[tokio::test]
    async fn memory_kv_take_consumes_entry_once() {
        let runtime = RuntimeState::memory(MemoryRuntimeStateConfig::default());
        runtime
            .kv_set("nonce", "payload", Some(Duration::from_secs(60)))
            .await
            .expect("set should succeed");
        assert_eq!(
            runtime.kv_take("nonce").await.expect("take").as_deref(),
            Some("payload")
        );
        assert_eq!(runtime.kv_take("nonce").await.expect("take"), None);
    }

    #[tokio::test]
    async fn memory_rate_limit_rejects_after_limit() {
        let runtime = RuntimeState::memory(MemoryRuntimeStateConfig::default());
        let input = RateLimitInput {
            user_key: "rpm:user:1:1",
            key_key: "rpm:key:1:1",
            bucket: 1,
            user_limit: 1,
            key_limit: 0,
            ttl_seconds: 60,
        };
        assert!(matches!(
            runtime
                .check_and_consume_rate_limit(input)
                .await
                .expect("first"),
            RateLimitCheck::Allowed { .. }
        ));
        assert_eq!(
            runtime
                .rate_limit_count(input.user_key, input.bucket)
                .await
                .expect("count after first"),
            1
        );
        assert_eq!(
            runtime
                .check_and_consume_rate_limit(input)
                .await
                .expect("second"),
            RateLimitCheck::Rejected {
                scope: RateLimitScope::User,
                limit: 1
            }
        );
        assert_eq!(
            runtime
                .rate_limit_count(input.user_key, input.bucket)
                .await
                .expect("count after reject"),
            1
        );
    }

    fn execution_reservation_input(candidate_id: &str) -> ExecutionReservationInput {
        ExecutionReservationInput {
            candidate_id: candidate_id.to_string(),
            provider: Some(ExecutionConcurrencyReservation {
                key: "provider-1".to_string(),
                limit: 1,
                observed_candidate_ids: Vec::new(),
            }),
            provider_key: None,
            api_key: None,
            provider_key_rpm: None,
        }
    }

    #[tokio::test]
    async fn memory_execution_reservation_serializes_concurrent_limit_one() {
        let runtime = RuntimeState::memory(MemoryRuntimeStateConfig::default());
        let (first, second) = tokio::join!(
            runtime.try_acquire_execution_reservation(execution_reservation_input("candidate-1")),
            runtime.try_acquire_execution_reservation(execution_reservation_input("candidate-2")),
        );
        let acquired = usize::from(first.is_ok()) + usize::from(second.is_ok());
        assert_eq!(acquired, 1);
        let rejected = first.err().or_else(|| second.err()).expect("one rejection");
        assert_eq!(
            rejected,
            ExecutionReservationError::Rejected {
                scope: ExecutionReservationScope::Provider,
                limit: 1,
            }
        );
    }

    #[tokio::test]
    async fn execution_reservation_health_handle_tracks_permit_health() {
        let runtime = RuntimeState::memory(MemoryRuntimeStateConfig::default());
        let permit = runtime
            .try_acquire_execution_reservation(execution_reservation_input("candidate-health"))
            .await
            .expect("execution reservation");
        let health = permit.health_handle();

        assert!(health.is_healthy());
        assert!(aether_runtime::AdmissionPermitHealth::is_healthy(&permit));
        permit.healthy.store(false, Ordering::Release);
        assert!(!health.is_healthy());
        assert!(!aether_runtime::AdmissionPermitHealth::is_healthy(&permit));
    }

    #[tokio::test]
    async fn memory_execution_reservation_unions_observed_and_live_without_double_counting() {
        let runtime = RuntimeState::memory(MemoryRuntimeStateConfig::default());
        let permit = runtime
            .try_acquire_execution_reservation(ExecutionReservationInput {
                provider: Some(ExecutionConcurrencyReservation {
                    key: "provider-1".to_string(),
                    limit: 1,
                    observed_candidate_ids: vec!["candidate-1".to_string()],
                }),
                ..execution_reservation_input("candidate-1")
            })
            .await
            .expect("the same observed candidate must not consume twice");
        drop(permit);
    }

    #[tokio::test]
    async fn memory_execution_reservation_rejection_has_no_partial_side_effects() {
        let runtime = RuntimeState::memory(MemoryRuntimeStateConfig::default());
        let blocker = runtime
            .try_acquire_execution_reservation(ExecutionReservationInput {
                provider: None,
                provider_key: Some(ExecutionConcurrencyReservation {
                    key: "provider-key-1".to_string(),
                    limit: 1,
                    observed_candidate_ids: Vec::new(),
                }),
                ..execution_reservation_input("blocker")
            })
            .await
            .expect("blocker");

        let rejected = runtime
            .try_acquire_execution_reservation(ExecutionReservationInput {
                provider: Some(ExecutionConcurrencyReservation {
                    key: "provider-1".to_string(),
                    limit: 1,
                    observed_candidate_ids: Vec::new(),
                }),
                provider_key: Some(ExecutionConcurrencyReservation {
                    key: "provider-key-1".to_string(),
                    limit: 1,
                    observed_candidate_ids: Vec::new(),
                }),
                ..execution_reservation_input("rejected")
            })
            .await;
        assert!(matches!(
            rejected,
            Err(ExecutionReservationError::Rejected {
                scope: ExecutionReservationScope::ProviderKey,
                ..
            })
        ));

        let provider_permit = runtime
            .try_acquire_execution_reservation(execution_reservation_input("accepted"))
            .await
            .expect("provider scope must remain untouched after later-scope rejection");
        drop(provider_permit);
        drop(blocker);
    }

    #[tokio::test]
    async fn memory_execution_rpm_consumption_survives_permit_drop() {
        let runtime = RuntimeState::memory(MemoryRuntimeStateConfig::default());
        let rpm_input = |candidate_id: &str| ExecutionReservationInput {
            candidate_id: candidate_id.to_string(),
            provider: None,
            provider_key: None,
            api_key: None,
            provider_key_rpm: Some(ExecutionRpmReservation {
                key: "provider-key-1".to_string(),
                limit: 1,
                observed_candidate_ids: Vec::new(),
                observed_count_floor: 0,
                reset_after_unix_secs: None,
            }),
        };
        let permit = runtime
            .try_acquire_execution_reservation(rpm_input("candidate-1"))
            .await
            .expect("first RPM consumption");
        drop(permit);
        tokio::task::yield_now().await;

        assert_eq!(
            runtime
                .try_acquire_execution_reservation(rpm_input("candidate-2"))
                .await
                .expect_err("dropping permit must not refund RPM"),
            ExecutionReservationError::Rejected {
                scope: ExecutionReservationScope::ProviderKeyRpm,
                limit: 1,
            }
        );
    }

    #[tokio::test]
    async fn memory_execution_reservation_consumes_additional_rpm_without_reacquiring_concurrency()
    {
        let runtime = RuntimeState::memory(MemoryRuntimeStateConfig::default());
        let mut permit = runtime
            .try_acquire_execution_reservation(ExecutionReservationInput {
                candidate_id: "candidate-retry".to_string(),
                provider: Some(ExecutionConcurrencyReservation {
                    key: "provider-retry".to_string(),
                    limit: 1,
                    observed_candidate_ids: Vec::new(),
                }),
                provider_key: None,
                api_key: None,
                provider_key_rpm: Some(ExecutionRpmReservation {
                    key: "provider-key-retry".to_string(),
                    limit: 2,
                    observed_candidate_ids: Vec::new(),
                    observed_count_floor: 0,
                    reset_after_unix_secs: None,
                }),
            })
            .await
            .expect("first upstream attempt");

        permit
            .consume_additional_rpm_attempt()
            .await
            .expect("second upstream attempt");
        assert_eq!(
            permit
                .consume_additional_rpm_attempt()
                .await
                .expect_err("third upstream attempt must exceed RPM"),
            ExecutionReservationError::Rejected {
                scope: ExecutionReservationScope::ProviderKeyRpm,
                limit: 2,
            }
        );
        assert_eq!(
            permit.additional_rpm_attempt_sequence, 1,
            "a rejected attempt must not advance the member sequence"
        );

        permit.release().await.expect("release concurrency");
        let concurrency_only = runtime
            .try_acquire_execution_reservation(ExecutionReservationInput {
                candidate_id: "candidate-after-release".to_string(),
                provider: Some(ExecutionConcurrencyReservation {
                    key: "provider-retry".to_string(),
                    limit: 1,
                    observed_candidate_ids: Vec::new(),
                }),
                provider_key: None,
                api_key: None,
                provider_key_rpm: None,
            })
            .await
            .expect("additional RPM attempts must not leave another concurrency holder");
        concurrency_only
            .release()
            .await
            .expect("release concurrency-only permit");
    }

    #[tokio::test]
    async fn memory_additional_rpm_attempt_respects_observed_floor_and_is_a_noop_without_rpm() {
        let runtime = RuntimeState::memory(MemoryRuntimeStateConfig::default());
        let mut floor_permit = runtime
            .try_acquire_execution_reservation(ExecutionReservationInput {
                candidate_id: "candidate-floor-initial".to_string(),
                provider: None,
                provider_key: None,
                api_key: None,
                provider_key_rpm: Some(ExecutionRpmReservation {
                    key: "provider-key-additional-floor".to_string(),
                    limit: 3,
                    observed_candidate_ids: Vec::new(),
                    observed_count_floor: 2,
                    reset_after_unix_secs: None,
                }),
            })
            .await
            .expect("initial attempt should fill the observed floor");
        assert_eq!(
            floor_permit
                .consume_additional_rpm_attempt()
                .await
                .expect_err("additional attempt must count beyond the observed floor"),
            ExecutionReservationError::Rejected {
                scope: ExecutionReservationScope::ProviderKeyRpm,
                limit: 3,
            }
        );

        let mut no_rpm = runtime
            .try_acquire_execution_reservation(execution_reservation_input("candidate-no-rpm"))
            .await
            .expect("concurrency-only permit");
        no_rpm
            .consume_additional_rpm_attempt()
            .await
            .expect("an unconfigured RPM scope is a no-op");
    }

    #[tokio::test]
    async fn memory_execution_rpm_floor_counts_a_new_candidate() {
        let runtime = RuntimeState::memory(MemoryRuntimeStateConfig::default());
        let error = runtime
            .try_acquire_execution_reservation(ExecutionReservationInput {
                candidate_id: "new-candidate".to_string(),
                provider: None,
                provider_key: None,
                api_key: None,
                provider_key_rpm: Some(ExecutionRpmReservation {
                    key: "provider-key-floor".to_string(),
                    limit: 10,
                    observed_candidate_ids: Vec::new(),
                    observed_count_floor: 10,
                    reset_after_unix_secs: None,
                }),
            })
            .await
            .expect_err("a new candidate must consume capacity beyond the observed floor");

        assert_eq!(
            error,
            ExecutionReservationError::Rejected {
                scope: ExecutionReservationScope::ProviderKeyRpm,
                limit: 10,
            }
        );
    }

    #[tokio::test]
    async fn memory_execution_rpm_applies_same_second_reset_only_once() {
        let runtime = RuntimeState::memory(MemoryRuntimeStateConfig::default());
        let reset_at = unix_time_ms() / 1_000;
        let input = |candidate_id: &str| ExecutionReservationInput {
            candidate_id: candidate_id.to_string(),
            provider: None,
            provider_key: None,
            api_key: None,
            provider_key_rpm: Some(ExecutionRpmReservation {
                key: "provider-key-reset".to_string(),
                limit: 1,
                observed_candidate_ids: Vec::new(),
                observed_count_floor: 0,
                reset_after_unix_secs: Some(reset_at),
            }),
        };

        runtime
            .try_acquire_execution_reservation(input("candidate-1"))
            .await
            .expect("first candidate after reset");
        assert_eq!(
            runtime
                .try_acquire_execution_reservation(input("candidate-2"))
                .await
                .expect_err("the same reset generation must not erase candidate-1"),
            ExecutionReservationError::Rejected {
                scope: ExecutionReservationScope::ProviderKeyRpm,
                limit: 1,
            }
        );
    }

    #[tokio::test]
    async fn memory_execution_rpm_rejection_does_not_apply_reset_generation() {
        let runtime = RuntimeState::memory(MemoryRuntimeStateConfig::default());
        let rpm_only =
            |candidate_id: &str, reset_after_unix_secs: Option<u64>| ExecutionReservationInput {
                candidate_id: candidate_id.to_string(),
                provider: None,
                provider_key: None,
                api_key: None,
                provider_key_rpm: Some(ExecutionRpmReservation {
                    key: "provider-key-reset-reject".to_string(),
                    limit: 1,
                    observed_candidate_ids: Vec::new(),
                    observed_count_floor: 0,
                    reset_after_unix_secs,
                }),
            };
        runtime
            .try_acquire_execution_reservation(rpm_only("old-candidate", None))
            .await
            .expect("seed RPM observation");
        let blocker = runtime
            .try_acquire_execution_reservation(execution_reservation_input("blocker"))
            .await
            .expect("concurrency blocker");
        let reset_at = unix_time_ms() / 1_000;
        let mut rejected = rpm_only("rejected-candidate", Some(reset_at));
        rejected.provider = execution_reservation_input("rejected-candidate").provider;
        assert!(matches!(
            runtime.try_acquire_execution_reservation(rejected).await,
            Err(ExecutionReservationError::Rejected {
                scope: ExecutionReservationScope::Provider,
                ..
            })
        ));
        blocker.release().await.expect("release blocker");

        runtime
            .try_acquire_execution_reservation(rpm_only("after-reset", Some(reset_at)))
            .await
            .expect("rejected admission must not consume the reset generation");
        assert!(matches!(
            runtime
                .try_acquire_execution_reservation(rpm_only("after-reset-2", Some(reset_at)))
                .await,
            Err(ExecutionReservationError::Rejected {
                scope: ExecutionReservationScope::ProviderKeyRpm,
                limit: 1,
            })
        ));
    }

    #[tokio::test]
    async fn memory_execution_reservation_rejects_zero_concurrency_limit() {
        let runtime = RuntimeState::memory(MemoryRuntimeStateConfig::default());
        let mut input = execution_reservation_input("candidate-zero");
        input.provider.as_mut().expect("provider scope").limit = 0;
        assert!(matches!(
            runtime.try_acquire_execution_reservation(input).await,
            Err(ExecutionReservationError::InvalidConfiguration(_))
        ));
    }

    #[tokio::test]
    async fn memory_rate_limit_keeps_user_limit_atomic_across_api_keys() {
        let runtime = RuntimeState::memory(MemoryRuntimeStateConfig::default());
        let first_key = RateLimitInput {
            user_key: "rpm:user:shared:1",
            key_key: "rpm:key:first:1",
            bucket: 1,
            user_limit: 2,
            key_limit: 10,
            ttl_seconds: 60,
        };
        let second_key = RateLimitInput {
            key_key: "rpm:key:second:1",
            ..first_key
        };

        assert!(matches!(
            runtime
                .check_and_consume_rate_limit(first_key)
                .await
                .expect("first key"),
            RateLimitCheck::Allowed { .. }
        ));
        assert!(matches!(
            runtime
                .check_and_consume_rate_limit(second_key)
                .await
                .expect("second key"),
            RateLimitCheck::Allowed { .. }
        ));
        assert_eq!(
            runtime
                .check_and_consume_rate_limit(first_key)
                .await
                .expect("user limit"),
            RateLimitCheck::Rejected {
                scope: RateLimitScope::User,
                limit: 2,
            }
        );
        assert_eq!(
            runtime
                .rate_limit_count(first_key.user_key, first_key.bucket)
                .await
                .expect("user count"),
            2
        );
        assert_eq!(
            runtime
                .rate_limit_count(first_key.key_key, first_key.bucket)
                .await
                .expect("first key count"),
            1
        );
        assert_eq!(
            runtime
                .rate_limit_count(second_key.key_key, second_key.bucket)
                .await
                .expect("second key count"),
            1
        );
    }

    #[tokio::test]
    async fn memory_rate_limit_concurrent_checks_do_not_exceed_limit() {
        let runtime =
            std::sync::Arc::new(RuntimeState::memory(MemoryRuntimeStateConfig::default()));
        let input = RateLimitInput {
            user_key: "rpm:user:concurrent:1",
            key_key: "rpm:key:concurrent:1",
            bucket: 1,
            user_limit: 32,
            key_limit: 64,
            ttl_seconds: 60,
        };
        let mut tasks = Vec::new();
        for _ in 0..128 {
            let runtime = std::sync::Arc::clone(&runtime);
            tasks.push(tokio::spawn(async move {
                runtime
                    .check_and_consume_rate_limit(input)
                    .await
                    .expect("concurrent rate-limit check")
            }));
        }

        let mut allowed = 0;
        let mut rejected = 0;
        for task in tasks {
            match task.await.expect("rate-limit task") {
                RateLimitCheck::Allowed { .. } => allowed += 1,
                RateLimitCheck::Rejected {
                    scope: RateLimitScope::User,
                    limit: 32,
                } => rejected += 1,
                other => panic!("unexpected rate-limit result: {other:?}"),
            }
        }
        assert_eq!(allowed, 32);
        assert_eq!(rejected, 96);
    }

    #[tokio::test]
    async fn memory_lock_fencing_tokens_increase_after_release() {
        let runtime = RuntimeState::memory(MemoryRuntimeStateConfig::default());
        let first = runtime
            .lock_try_acquire("fencing", "node-a", Duration::from_secs(1))
            .await
            .expect("first acquire")
            .expect("first lease");
        assert!(first.fencing_token > 0);
        assert!(runtime.lock_release(&first).await.expect("first release"));

        let second = runtime
            .lock_try_acquire("fencing", "node-b", Duration::from_secs(1))
            .await
            .expect("second acquire")
            .expect("second lease");
        assert!(second.fencing_token > first.fencing_token);
    }

    #[tokio::test]
    async fn memory_expired_lock_cannot_be_renewed() {
        let runtime = RuntimeState::memory(MemoryRuntimeStateConfig::default());
        let expired = runtime
            .lock_try_acquire("expired-fencing", "node-a", Duration::from_millis(10))
            .await
            .expect("acquire")
            .expect("lease");
        tokio::time::sleep(Duration::from_millis(30)).await;

        assert!(!runtime
            .lock_renew(&expired, Duration::from_secs(1))
            .await
            .expect("expired renew should be rejected"));
        let replacement = runtime
            .lock_try_acquire("expired-fencing", "node-b", Duration::from_secs(1))
            .await
            .expect("replacement acquire")
            .expect("replacement lease");
        assert!(replacement.fencing_token > expired.fencing_token);
    }

    #[tokio::test]
    async fn memory_semaphore_holds_until_permit_drop() {
        let runtime = RuntimeState::memory(MemoryRuntimeStateConfig::default());
        let gate = runtime
            .semaphore("test", 1, RuntimeSemaphoreConfig::default())
            .expect("gate should build");
        let permit = gate.try_acquire().await.expect("first permit");
        assert!(matches!(
            gate.try_acquire().await.expect_err("second rejected"),
            RuntimeSemaphoreError::Saturated { .. }
        ));
        drop(permit);
        tokio::time::sleep(Duration::from_millis(5)).await;
        assert_eq!(gate.snapshot().await.expect("snapshot").in_flight, 0);
    }

    #[tokio::test(flavor = "current_thread")]
    async fn memory_semaphore_marks_permit_unhealthy_after_lease_loss() {
        let runtime = RuntimeState::memory(MemoryRuntimeStateConfig::default());
        let gate = runtime
            .semaphore(
                "lease-health",
                1,
                RuntimeSemaphoreConfig {
                    lease_ttl_ms: 20,
                    renew_interval_ms: 5,
                    command_timeout_ms: Some(50),
                },
            )
            .expect("gate should build");
        let permit = gate.try_acquire().await.expect("permit should acquire");
        assert!(aether_runtime::AdmissionPermitHealth::is_healthy(&permit));

        std::thread::sleep(Duration::from_millis(30));
        tokio::time::sleep(Duration::from_millis(10)).await;

        assert!(!aether_runtime::AdmissionPermitHealth::is_healthy(&permit));
    }

    #[tokio::test]
    async fn redis_runtime_reuses_fixed_connections_for_repeated_operations() {
        let Some(redis) = TestRedisServer::start().await else {
            return;
        };
        let runtime = RuntimeState::redis(
            RedisClientConfig {
                url: redis.redis_url.clone(),
                key_prefix: Some(format!("aether-runtime-test-{}", std::process::id())),
            },
            Some(1_000),
        )
        .await
        .expect("runtime should connect");
        let before = runtime
            .redis_diagnostics()
            .await
            .expect("diagnostics")
            .expect("redis diagnostics")
            .total_connections_received
            .expect("total connections");

        for index in 0..200 {
            let key = format!("kv:{index}");
            runtime
                .kv_set(
                    &key,
                    format!("value-{index}"),
                    Some(Duration::from_secs(30)),
                )
                .await
                .expect("set");
            assert_eq!(
                runtime.kv_get(&key).await.expect("get").as_deref(),
                Some(format!("value-{index}").as_str())
            );
        }

        let after = runtime
            .redis_diagnostics()
            .await
            .expect("diagnostics")
            .expect("redis diagnostics")
            .total_connections_received
            .expect("total connections");
        assert_eq!(
            after, before,
            "runtime Redis operations should reuse initialized lanes"
        );
    }

    #[tokio::test]
    async fn redis_execution_reservation_is_atomic_and_does_not_refund_rpm() {
        let Some((_redis, runtime)) = redis_runtime_for_test("execution-reservation").await else {
            return;
        };
        let input = |candidate_id: &str| ExecutionReservationInput {
            candidate_id: candidate_id.to_string(),
            provider: Some(ExecutionConcurrencyReservation {
                key: "provider-1".to_string(),
                limit: 1,
                observed_candidate_ids: Vec::new(),
            }),
            provider_key: None,
            api_key: None,
            provider_key_rpm: Some(ExecutionRpmReservation {
                key: "provider-key-1".to_string(),
                limit: 2,
                observed_candidate_ids: Vec::new(),
                observed_count_floor: 0,
                reset_after_unix_secs: None,
            }),
        };

        let first = runtime
            .try_acquire_execution_reservation(input("candidate-1"))
            .await
            .expect("first reservation");
        assert_eq!(
            runtime
                .try_acquire_execution_reservation(input("candidate-2"))
                .await
                .expect_err("provider concurrency should reject"),
            ExecutionReservationError::Rejected {
                scope: ExecutionReservationScope::Provider,
                limit: 1,
            }
        );
        first.release().await.expect("explicit release");

        let third = runtime
            .try_acquire_execution_reservation(input("candidate-3"))
            .await
            .expect("rejected candidate must not consume RPM");
        third.release().await.expect("third release");

        let mut fourth = input("candidate-4");
        fourth.provider = None;
        assert_eq!(
            runtime
                .try_acquire_execution_reservation(fourth)
                .await
                .expect_err("RPM must survive concurrency release"),
            ExecutionReservationError::Rejected {
                scope: ExecutionReservationScope::ProviderKeyRpm,
                limit: 2,
            }
        );

        let mut floor = input("candidate-floor");
        floor.provider = None;
        floor.provider_key_rpm = Some(ExecutionRpmReservation {
            key: "provider-key-floor".to_string(),
            limit: 10,
            observed_candidate_ids: Vec::new(),
            observed_count_floor: 10,
            reset_after_unix_secs: None,
        });
        assert_eq!(
            runtime
                .try_acquire_execution_reservation(floor)
                .await
                .expect_err("a new candidate must consume capacity beyond the observed floor"),
            ExecutionReservationError::Rejected {
                scope: ExecutionReservationScope::ProviderKeyRpm,
                limit: 10,
            }
        );

        let mut old = input("reset-old");
        old.provider = None;
        old.provider_key_rpm = Some(ExecutionRpmReservation {
            key: "provider-key-reset-reject".to_string(),
            limit: 1,
            observed_candidate_ids: Vec::new(),
            observed_count_floor: 0,
            reset_after_unix_secs: None,
        });
        runtime
            .try_acquire_execution_reservation(old)
            .await
            .expect("seed RPM observation");
        let blocker = runtime
            .try_acquire_execution_reservation(execution_reservation_input("reset-blocker"))
            .await
            .expect("concurrency blocker");
        let reset_at = unix_time_ms() / 1_000;
        let reset_input = |candidate_id: &str, with_provider: bool| ExecutionReservationInput {
            candidate_id: candidate_id.to_string(),
            provider: with_provider.then(|| ExecutionConcurrencyReservation {
                key: "provider-1".to_string(),
                limit: 1,
                observed_candidate_ids: Vec::new(),
            }),
            provider_key: None,
            api_key: None,
            provider_key_rpm: Some(ExecutionRpmReservation {
                key: "provider-key-reset-reject".to_string(),
                limit: 1,
                observed_candidate_ids: Vec::new(),
                observed_count_floor: 0,
                reset_after_unix_secs: Some(reset_at),
            }),
        };
        assert!(matches!(
            runtime
                .try_acquire_execution_reservation(reset_input("reset-rejected", true))
                .await,
            Err(ExecutionReservationError::Rejected {
                scope: ExecutionReservationScope::Provider,
                ..
            })
        ));
        blocker.release().await.expect("release blocker");
        runtime
            .try_acquire_execution_reservation(reset_input("reset-after", false))
            .await
            .expect("rejected admission must not consume reset generation");
        assert!(matches!(
            runtime
                .try_acquire_execution_reservation(reset_input("reset-after-2", false))
                .await,
            Err(ExecutionReservationError::Rejected {
                scope: ExecutionReservationScope::ProviderKeyRpm,
                limit: 1,
            })
        ));

        let reset_at = unix_time_ms() / 1_000;
        let reset_input = |candidate_id: &str| ExecutionReservationInput {
            candidate_id: candidate_id.to_string(),
            provider: None,
            provider_key: None,
            api_key: None,
            provider_key_rpm: Some(ExecutionRpmReservation {
                key: "provider-key-reset".to_string(),
                limit: 1,
                observed_candidate_ids: Vec::new(),
                observed_count_floor: 0,
                reset_after_unix_secs: Some(reset_at),
            }),
        };
        runtime
            .try_acquire_execution_reservation(reset_input("reset-candidate-1"))
            .await
            .expect("first candidate after reset");
        assert_eq!(
            runtime
                .try_acquire_execution_reservation(reset_input("reset-candidate-2"))
                .await
                .expect_err("the same reset generation must not erase the first candidate"),
            ExecutionReservationError::Rejected {
                scope: ExecutionReservationScope::ProviderKeyRpm,
                limit: 1,
            }
        );
    }

    #[tokio::test]
    async fn redis_execution_reservation_consumes_additional_rpm_without_reacquiring_concurrency() {
        let Some((_redis, runtime)) =
            redis_runtime_for_test("execution-reservation-retry-rpm").await
        else {
            return;
        };
        let mut permit = runtime
            .try_acquire_execution_reservation(ExecutionReservationInput {
                candidate_id: "redis-candidate-retry".to_string(),
                provider: Some(ExecutionConcurrencyReservation {
                    key: "redis-provider-retry".to_string(),
                    limit: 1,
                    observed_candidate_ids: Vec::new(),
                }),
                provider_key: None,
                api_key: None,
                provider_key_rpm: Some(ExecutionRpmReservation {
                    key: "redis-provider-key-retry".to_string(),
                    limit: 2,
                    observed_candidate_ids: Vec::new(),
                    observed_count_floor: 0,
                    reset_after_unix_secs: None,
                }),
            })
            .await
            .expect("first Redis upstream attempt");

        permit
            .consume_additional_rpm_attempt()
            .await
            .expect("second Redis upstream attempt");
        assert_eq!(
            permit
                .consume_additional_rpm_attempt()
                .await
                .expect_err("third Redis upstream attempt must exceed RPM"),
            ExecutionReservationError::Rejected {
                scope: ExecutionReservationScope::ProviderKeyRpm,
                limit: 2,
            }
        );
        assert_eq!(permit.additional_rpm_attempt_sequence, 1);

        permit.release().await.expect("release Redis concurrency");
        let concurrency_only = runtime
            .try_acquire_execution_reservation(ExecutionReservationInput {
                candidate_id: "redis-candidate-after-release".to_string(),
                provider: Some(ExecutionConcurrencyReservation {
                    key: "redis-provider-retry".to_string(),
                    limit: 1,
                    observed_candidate_ids: Vec::new(),
                }),
                provider_key: None,
                api_key: None,
                provider_key_rpm: None,
            })
            .await
            .expect("additional Redis RPM attempts must not occupy concurrency");
        concurrency_only
            .release()
            .await
            .expect("release Redis concurrency-only permit");
    }

    #[tokio::test]
    async fn redis_execution_reservation_is_atomic_across_runtime_instances() {
        let Some(redis) = TestRedisServer::start().await else {
            return;
        };
        let key_prefix = format!(
            "aether-runtime-test-execution-reservation-multi-{}",
            std::process::id()
        );
        let config = || RedisClientConfig {
            url: redis.redis_url.clone(),
            key_prefix: Some(key_prefix.clone()),
        };
        let first_runtime = RuntimeState::redis(config(), Some(1_000))
            .await
            .expect("first runtime");
        let second_runtime = RuntimeState::redis(config(), Some(1_000))
            .await
            .expect("second runtime");

        let (first, second) = tokio::join!(
            first_runtime.try_acquire_execution_reservation(execution_reservation_input(
                "multi-candidate-1"
            )),
            second_runtime.try_acquire_execution_reservation(execution_reservation_input(
                "multi-candidate-2"
            )),
        );
        assert_eq!(usize::from(first.is_ok()) + usize::from(second.is_ok()), 1);
        let rejection = first.err().or_else(|| second.err()).expect("one rejection");
        assert_eq!(
            rejection,
            ExecutionReservationError::Rejected {
                scope: ExecutionReservationScope::Provider,
                limit: 1,
            }
        );
    }

    #[tokio::test]
    async fn redis_runtime_instances_share_ttl_kv_across_reinitialization() {
        let external_redis_url = std::env::var("AETHER_TEST_REDIS_URL")
            .ok()
            .filter(|value| !value.trim().is_empty());
        let redis = if external_redis_url.is_none() {
            TestRedisServer::start().await
        } else {
            None
        };
        let Some(redis_url) =
            external_redis_url.or_else(|| redis.as_ref().map(|redis| redis.redis_url.clone()))
        else {
            return;
        };
        let key_prefix = format!("aether-history-test-{}", std::process::id());
        let runtime_config = || RedisClientConfig {
            url: redis_url.clone(),
            key_prefix: Some(key_prefix.clone()),
        };
        let writer = RuntimeState::redis(runtime_config(), Some(1_000))
            .await
            .expect("writer runtime should connect");
        let reader = RuntimeState::redis(runtime_config(), Some(1_000))
            .await
            .expect("reader runtime should connect");
        let history_key = "ai:responses:history:v1:shared-record";

        writer
            .kv_set(
                history_key,
                "persisted-history",
                Some(Duration::from_secs(30)),
            )
            .await
            .expect("writer should persist history");
        assert_eq!(
            reader
                .kv_get(history_key)
                .await
                .expect("reader get")
                .as_deref(),
            Some("persisted-history")
        );

        drop(writer);
        drop(reader);
        let restarted = RuntimeState::redis(runtime_config(), Some(1_000))
            .await
            .expect("restarted runtime should connect");
        assert_eq!(
            restarted
                .kv_get(history_key)
                .await
                .expect("restarted get")
                .as_deref(),
            Some("persisted-history")
        );
        assert!(matches!(
            restarted
                .kv_ttl_seconds(history_key)
                .await
                .expect("history ttl"),
            Some(1..=30)
        ));
    }

    #[tokio::test]
    async fn redis_lock_fencing_tokens_increase_and_expired_lease_cannot_renew() {
        let Some((_redis, runtime)) = redis_runtime_for_test("lock-fencing").await else {
            return;
        };
        let first = runtime
            .lock_try_acquire("fencing", "node-a", Duration::from_millis(20))
            .await
            .expect("first acquire")
            .expect("first lease");
        tokio::time::sleep(Duration::from_millis(60)).await;
        assert!(!runtime
            .lock_renew(&first, Duration::from_secs(1))
            .await
            .expect("expired renew should be rejected"));

        let second = runtime
            .lock_try_acquire("fencing", "node-b", Duration::from_secs(1))
            .await
            .expect("second acquire")
            .expect("second lease");
        assert!(second.fencing_token > first.fencing_token);
        assert!(runtime.lock_release(&second).await.expect("second release"));

        let third = runtime
            .lock_try_acquire("fencing", "node-c", Duration::from_secs(1))
            .await
            .expect("third acquire")
            .expect("third lease");
        assert!(third.fencing_token > second.fencing_token);
    }

    #[tokio::test]
    async fn redis_blocking_stream_read_does_not_block_fast_lane() {
        let Some(redis) = TestRedisServer::start().await else {
            return;
        };
        let runtime = RuntimeState::redis(
            RedisClientConfig {
                url: redis.redis_url.clone(),
                key_prefix: Some(format!("aether-block-test-{}", std::process::id())),
            },
            Some(1_000),
        )
        .await
        .expect("runtime should connect");
        RuntimeQueueStore::ensure_consumer_group(&runtime, "blocking-stream", "workers", "0-0")
            .await
            .expect("consumer group");

        let blocking_runtime = runtime.clone();
        let blocking = tokio::spawn(async move {
            RuntimeQueueStore::read_group(
                &blocking_runtime,
                "blocking-stream",
                "workers",
                "consumer-a",
                1,
                Some(500),
            )
            .await
        });
        tokio::time::sleep(Duration::from_millis(50)).await;

        runtime
            .kv_set("fast-lane", "ok", Some(Duration::from_secs(30)))
            .await
            .expect("fast lane set should complete while stream read blocks");
        assert_eq!(
            runtime
                .kv_get("fast-lane")
                .await
                .expect("fast lane get")
                .as_deref(),
            Some("ok")
        );
        let _ = blocking.await.expect("blocking task join");
    }

    #[tokio::test]
    async fn redis_concurrent_blocking_stream_reads_do_not_share_single_connection() {
        let Some(redis) = TestRedisServer::start().await else {
            return;
        };
        let runtime = RuntimeState::redis(
            RedisClientConfig {
                url: redis.redis_url.clone(),
                key_prefix: Some(format!("aether-block-pool-test-{}", std::process::id())),
            },
            Some(1_000),
        )
        .await
        .expect("runtime should connect");
        RuntimeQueueStore::ensure_consumer_group(&runtime, "blocking-stream", "workers", "0-0")
            .await
            .expect("consumer group");

        let mut handles = Vec::new();
        for index in 0..4 {
            let blocking_runtime = runtime.clone();
            handles.push(tokio::spawn(async move {
                let consumer = format!("consumer-{index}");
                RuntimeQueueStore::read_group(
                    &blocking_runtime,
                    "blocking-stream",
                    "workers",
                    &consumer,
                    1,
                    Some(600),
                )
                .await
            }));
        }

        for handle in handles {
            let result = handle.await.expect("blocking task join");
            assert!(
                !matches!(result, Err(DataLayerError::TimedOut(_))),
                "concurrent blocking stream reads should not queue behind one connection"
            );
            assert!(result.expect("blocking read should succeed").is_empty());
        }
    }

    #[tokio::test]
    async fn redis_connection_manager_recovers_after_restart() {
        let Some(mut redis) = TestRedisServer::start().await else {
            return;
        };
        let runtime = RuntimeState::redis(
            RedisClientConfig {
                url: redis.redis_url.clone(),
                key_prefix: Some(format!("aether-restart-test-{}", std::process::id())),
            },
            Some(500),
        )
        .await
        .expect("runtime should connect");
        runtime
            .kv_set("before-restart", "ok", Some(Duration::from_secs(30)))
            .await
            .expect("initial set");

        redis.stop();
        let _ = runtime
            .kv_set("during-restart", "may-fail", Some(Duration::from_secs(30)))
            .await;
        redis.restart().await.expect("redis restart");

        let mut recovered = false;
        for _ in 0..20 {
            if runtime
                .kv_set("after-restart", "ok", Some(Duration::from_secs(30)))
                .await
                .is_ok()
            {
                recovered = true;
                break;
            }
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
        assert!(
            recovered,
            "connection manager should reconnect after restart"
        );
    }

    #[tokio::test]
    async fn runtime_backends_share_kv_score_and_queue_contracts() {
        let memory = RuntimeState::memory(MemoryRuntimeStateConfig::default());
        assert_kv_score_and_queue_contract(&memory).await;

        let Some((_redis, redis_runtime)) = redis_runtime_for_test("shared-contract").await else {
            return;
        };
        assert_kv_score_and_queue_contract(&redis_runtime).await;
    }

    #[tokio::test]
    async fn runtime_backends_reject_invalid_shared_inputs() {
        let memory = RuntimeState::memory(MemoryRuntimeStateConfig::default());
        assert_invalid_shared_inputs(&memory).await;

        let Some((_redis, redis_runtime)) = redis_runtime_for_test("invalid-contract").await else {
            return;
        };
        assert_invalid_shared_inputs(&redis_runtime).await;
    }

    #[tokio::test]
    async fn memory_blocking_queue_read_does_not_block_kv_operations() {
        let runtime = RuntimeState::memory(MemoryRuntimeStateConfig::default());
        RuntimeQueueStore::ensure_consumer_group(&runtime, "memory-blocking", "workers", "0-0")
            .await
            .expect("consumer group");

        let blocking_runtime = runtime.clone();
        let blocking = tokio::spawn(async move {
            RuntimeQueueStore::read_group(
                &blocking_runtime,
                "memory-blocking",
                "workers",
                "consumer-a",
                1,
                Some(100),
            )
            .await
        });
        tokio::time::sleep(Duration::from_millis(10)).await;

        runtime
            .kv_set("memory-fast-lane", "ok", Some(Duration::from_millis(100)))
            .await
            .expect("set should complete while memory stream read blocks");
        assert_eq!(
            runtime
                .kv_get("memory-fast-lane")
                .await
                .expect("get")
                .as_deref(),
            Some("ok")
        );
        assert!(blocking
            .await
            .expect("blocking task join")
            .expect("read should complete")
            .is_empty());
    }

    #[test]
    fn redis_stream_timeout_expands_past_blocking_read() {
        assert_eq!(
            redis_stream_command_timeout_for_block(Some(1_000), Some(1_000)),
            Some(2_000)
        );
        assert_eq!(
            redis_stream_command_timeout_for_block(Some(5_000), Some(500)),
            Some(5_000)
        );
        assert_eq!(
            redis_stream_command_timeout_for_block(None, Some(500)),
            None
        );
    }

    async fn assert_kv_score_and_queue_contract(runtime: &RuntimeState) {
        runtime
            .kv_set("contract:ttl:set", "value", Some(Duration::from_millis(30)))
            .await
            .expect("set with ttl");
        tokio::time::sleep(Duration::from_millis(80)).await;
        assert_eq!(
            runtime.kv_get("contract:ttl:set").await.expect("ttl get"),
            None
        );

        runtime
            .kv_set("contract:ttl:expire", "value", None)
            .await
            .expect("set without ttl");
        assert!(runtime
            .key_expire("contract:ttl:expire", Duration::from_millis(30))
            .await
            .expect("expire existing key"));
        tokio::time::sleep(Duration::from_millis(80)).await;
        assert_eq!(
            runtime
                .kv_get("contract:ttl:expire")
                .await
                .expect("expired get"),
            None
        );

        runtime
            .kv_set("contract:ttl:zero", "value", None)
            .await
            .expect("set zero ttl key");
        assert!(runtime
            .key_expire("contract:ttl:zero", Duration::ZERO)
            .await
            .expect("zero expire existing key"));
        assert_eq!(
            runtime
                .kv_get("contract:ttl:zero")
                .await
                .expect("zero expired get"),
            None
        );

        runtime
            .set_add("contract:set", "member")
            .await
            .expect("set add");
        assert!(runtime
            .key_expire("contract:set", Duration::from_millis(30))
            .await
            .expect("expire set"));
        tokio::time::sleep(Duration::from_millis(80)).await;
        assert_eq!(runtime.set_len("contract:set").await.expect("set len"), 0);

        for (member, score) in [("a", 1.0), ("b", 2.0), ("c", 3.0), ("d", 4.0)] {
            runtime
                .score_set("contract:zset", member, score)
                .await
                .expect("score set");
        }
        assert_eq!(
            runtime
                .score_range_by_min("contract:zset", 0.0)
                .await
                .expect("score range"),
            vec!["a", "b", "c", "d"]
        );
        assert_eq!(
            runtime
                .score_remove_by_rank("contract:zset", 0, -3)
                .await
                .expect("rank trim"),
            2
        );
        assert_eq!(
            runtime
                .score_many(
                    "contract:zset",
                    &["a".to_string(), "b".to_string(), "c".to_string()]
                )
                .await
                .expect("score many"),
            vec![None, None, Some(3.0)]
        );
        assert!(runtime
            .key_expire("contract:zset", Duration::from_millis(30))
            .await
            .expect("expire zset"));
        tokio::time::sleep(Duration::from_millis(80)).await;
        assert_eq!(
            runtime
                .score_len("contract:zset")
                .await
                .expect("expired zset len"),
            0
        );

        RuntimeQueueStore::ensure_consumer_group(runtime, "contract:stream", "workers", "0-0")
            .await
            .expect("consumer group");
        let fields = BTreeMap::from([("payload".to_string(), "one".to_string())]);
        let first = RuntimeQueueStore::append_fields_with_maxlen(
            runtime,
            "contract:stream",
            &fields,
            Some(100),
        )
        .await
        .expect("append first");
        let fields = BTreeMap::from([("payload".to_string(), "two".to_string())]);
        let second = RuntimeQueueStore::append_fields_with_maxlen(
            runtime,
            "contract:stream",
            &fields,
            Some(100),
        )
        .await
        .expect("append second");
        let delivered = RuntimeQueueStore::read_group(
            runtime,
            "contract:stream",
            "workers",
            "consumer-a",
            10,
            None,
        )
        .await
        .expect("read group");
        assert_eq!(
            delivered
                .iter()
                .map(|entry| entry.id.clone())
                .collect::<Vec<_>>(),
            vec![first.clone(), second.clone()]
        );
        let stats = RuntimeQueueStore::stats(runtime, "contract:stream", Some("workers"))
            .await
            .expect("queue stats");
        assert_eq!(stats.stream_length, 2);
        assert_eq!(stats.group_pending, 2);
        assert_eq!(stats.group_lag, Some(0));
        assert!(RuntimeQueueStore::read_group(
            runtime,
            "contract:stream",
            "workers",
            "consumer-a",
            10,
            None
        )
        .await
        .expect("second read")
        .is_empty());
        tokio::time::sleep(Duration::from_millis(20)).await;
        let claimed = RuntimeQueueStore::claim_stale(
            runtime,
            "contract:stream",
            "workers",
            "consumer-b",
            "0-0",
            RuntimeQueueReclaimConfig {
                min_idle_ms: 1,
                count: 10,
            },
        )
        .await
        .expect("claim stale");
        let ids = claimed
            .iter()
            .map(|entry| entry.id.clone())
            .collect::<Vec<_>>();
        assert_eq!(ids, vec![first.clone(), second.clone()]);
        let stats = RuntimeQueueStore::stats(runtime, "contract:stream", Some("workers"))
            .await
            .expect("queue stats after claim");
        assert_eq!(stats.group_pending, 2);
        assert!(stats.oldest_pending_idle_ms.unwrap_or_default() <= 5000);
        assert_eq!(
            RuntimeQueueStore::ack(runtime, "contract:stream", "workers", &ids)
                .await
                .expect("ack"),
            2
        );
        assert_eq!(
            RuntimeQueueStore::delete(runtime, "contract:stream", &ids)
                .await
                .expect("delete"),
            2
        );
        let stats = RuntimeQueueStore::stats(runtime, "contract:stream", Some("workers"))
            .await
            .expect("queue stats after delete");
        assert_eq!(stats.stream_length, 0);
        assert_eq!(stats.group_pending, 0);
        assert_eq!(stats.group_lag, Some(0));
        assert!(RuntimeQueueStore::read_group(
            runtime,
            "contract:stream",
            "workers",
            "consumer-b",
            10,
            None
        )
        .await
        .expect("read after delete")
        .is_empty());
    }

    async fn assert_invalid_shared_inputs(runtime: &RuntimeState) {
        assert!(matches!(
            runtime
                .score_set("contract:invalid-score", "nan", f64::NAN)
                .await,
            Err(DataLayerError::InvalidInput(_))
        ));
        assert!(matches!(
            RuntimeQueueStore::read_group(runtime, "", "workers", "consumer-a", 1, None).await,
            Err(DataLayerError::InvalidInput(_))
        ));
        assert!(matches!(
            RuntimeQueueStore::claim_stale(
                runtime,
                "contract:stream",
                "workers",
                "consumer-a",
                "0-0",
                RuntimeQueueReclaimConfig {
                    min_idle_ms: 0,
                    count: 1,
                },
            )
            .await,
            Err(DataLayerError::InvalidInput(_))
        ));
    }

    async fn redis_runtime_for_test(prefix: &str) -> Option<(TestRedisServer, RuntimeState)> {
        let redis = TestRedisServer::start().await?;
        let runtime = RuntimeState::redis(
            RedisClientConfig {
                url: redis.redis_url.clone(),
                key_prefix: Some(format!(
                    "aether-runtime-test-{prefix}-{}",
                    std::process::id()
                )),
            },
            Some(1_000),
        )
        .await
        .ok()?;
        Some((redis, runtime))
    }

    struct TestRedisServer {
        child: Option<Child>,
        binary: String,
        port: u16,
        workdir: PathBuf,
        redis_url: String,
    }

    impl TestRedisServer {
        async fn start() -> Option<Self> {
            let port = reserve_local_port().ok()?;
            let workdir = std::env::temp_dir().join(format!(
                "aether-runtime-state-redis-{}-{port}",
                std::process::id()
            ));
            std::fs::create_dir_all(&workdir).ok()?;
            let binary = std::env::var("AETHER_REDIS_SERVER_BIN")
                .ok()
                .filter(|value| !value.trim().is_empty())
                .unwrap_or_else(|| "redis-server".to_string());
            let mut server = Self {
                child: None,
                binary,
                port,
                workdir,
                redis_url: format!("redis://127.0.0.1:{port}/0"),
            };
            server.restart().await.ok()?;
            Some(server)
        }

        fn stop(&mut self) {
            if let Some(mut child) = self.child.take() {
                let _ = child.kill();
                let _ = child.wait();
            }
        }

        async fn restart(&mut self) -> Result<(), Box<dyn std::error::Error>> {
            self.stop();
            let child = Command::new(&self.binary)
                .arg("--save")
                .arg("")
                .arg("--appendonly")
                .arg("no")
                .arg("--port")
                .arg(self.port.to_string())
                .arg("--dir")
                .arg(&self.workdir)
                .arg("--bind")
                .arg("127.0.0.1")
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .spawn()?;
            self.child = Some(child);
            let deadline = tokio::time::Instant::now() + Duration::from_secs(5);
            while tokio::time::Instant::now() < deadline {
                if redis_ping(self.port).await.unwrap_or(false) {
                    return Ok(());
                }
                tokio::time::sleep(Duration::from_millis(50)).await;
            }
            self.stop();
            Err(std::io::Error::new(
                std::io::ErrorKind::TimedOut,
                "timed out waiting for test redis-server",
            )
            .into())
        }
    }

    impl Drop for TestRedisServer {
        fn drop(&mut self) {
            self.stop();
            let _ = std::fs::remove_dir_all(&self.workdir);
        }
    }

    async fn redis_ping(port: u16) -> Result<bool, std::io::Error> {
        let mut stream = tokio::net::TcpStream::connect(("127.0.0.1", port)).await?;
        stream.write_all(b"*1\r\n$4\r\nPING\r\n").await?;
        let mut buffer = [0_u8; 16];
        let len = stream.read(&mut buffer).await?;
        Ok(buffer[..len].starts_with(b"+PONG"))
    }

    fn reserve_local_port() -> Result<u16, std::io::Error> {
        let listener = std::net::TcpListener::bind("127.0.0.1:0")?;
        let port = listener.local_addr()?.port();
        drop(listener);
        Ok(port)
    }
}
