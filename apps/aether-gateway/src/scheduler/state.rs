use std::collections::BTreeSet;
use std::time::Duration;

use aether_data_contracts::repository::candidates::StoredRequestCandidate;
use aether_data_contracts::repository::provider_catalog::{
    StoredProviderCatalogKey, StoredProviderCatalogProvider,
};
use aether_data_contracts::repository::quota::StoredProviderQuotaSnapshot;
use aether_scheduler_core::SchedulerAffinityTarget;
use async_trait::async_trait;

use crate::GatewayError;

use super::config::SchedulerOrderingConfig;

#[async_trait]
pub(crate) trait SchedulerRuntimeState {
    async fn read_provider_quota_snapshot(
        &self,
        provider_id: &str,
    ) -> Result<Option<StoredProviderQuotaSnapshot>, GatewayError>;

    async fn read_provider_catalog_providers_by_ids(
        &self,
        provider_ids: &[String],
    ) -> Result<Vec<StoredProviderCatalogProvider>, GatewayError>;

    async fn read_provider_catalog_keys_by_ids(
        &self,
        key_ids: &[String],
    ) -> Result<Vec<StoredProviderCatalogKey>, GatewayError>;

    async fn read_runtime_scoped_request_candidates_since(
        &self,
        provider_ids: &[String],
        key_ids: &[String],
        api_key_ids: &[String],
        since_unix_secs: u64,
    ) -> Result<Vec<StoredRequestCandidate>, GatewayError>;

    async fn read_pool_key_cooldown_reason(
        &self,
        provider_id: &str,
        key_id: &str,
    ) -> Result<Option<String>, GatewayError>;

    async fn read_pool_key_cost_window_usage(
        &self,
        provider_id: &str,
        key_id: &str,
        cost_window_seconds: u64,
    ) -> Result<u64, GatewayError>;

    async fn read_pool_active_probe_member_ids(
        &self,
        provider_id: &str,
    ) -> Result<BTreeSet<String>, GatewayError>;

    fn provider_key_rpm_reset_at(&self, key_id: &str, now_unix_secs: u64) -> Option<u64>;

    fn read_cached_scheduler_affinity_target(
        &self,
        cache_key: &str,
        ttl: Duration,
    ) -> Option<SchedulerAffinityTarget>;

    fn scheduler_affinity_epoch(&self) -> u64;

    fn remember_scheduler_affinity_target(
        &self,
        cache_key: &str,
        target: SchedulerAffinityTarget,
        ttl: Duration,
        max_entries: usize,
    );

    fn remember_scheduler_affinity_target_for_epoch(
        &self,
        cache_key: &str,
        target: SchedulerAffinityTarget,
        ttl: Duration,
        max_entries: usize,
        expected_epoch: Option<u64>,
    ) -> bool;

    async fn read_scheduler_ordering_config(&self)
        -> Result<SchedulerOrderingConfig, GatewayError>;
}
