use std::collections::{BTreeMap, BTreeSet};

use aether_data::repository::candidates::StoredRequestCandidate;
use aether_data::repository::provider_catalog::{
    StoredProviderCatalogKey, StoredProviderCatalogProvider,
};
use aether_data::repository::quota::StoredProviderQuotaSnapshot;
use aether_wallet::{ProviderBillingType, ProviderQuotaSnapshot};

use crate::gateway::gateway_cache::SchedulerAffinityTarget;
use crate::gateway::gateway_data::StoredGatewayAuthApiKeySnapshot;
use crate::gateway::{AppState, GatewayError};

use super::super::health::{
    count_recent_active_requests_for_provider, effective_provider_key_health_score,
    is_candidate_in_recent_failure_cooldown, is_provider_key_circuit_open,
    provider_key_health_bucket, provider_key_health_score, provider_key_rpm_allows_request_since,
    ProviderKeyHealthBucket,
};

use super::{
    compare_affinity_order, matches_affinity_target, GatewayMinimalCandidateSelectionCandidate,
};

pub(super) fn reorder_candidates_by_scheduler_health(
    candidates: &mut [GatewayMinimalCandidateSelectionCandidate],
    provider_key_rpm_states: &BTreeMap<String, StoredProviderCatalogKey>,
    auth_snapshot: Option<&StoredGatewayAuthApiKeySnapshot>,
) {
    let affinity_key = auth_snapshot
        .map(|snapshot| snapshot.api_key_id.trim())
        .filter(|value| !value.is_empty());
    candidates.sort_by(|left, right| {
        left.key_global_priority_for_format
            .unwrap_or(i32::MAX)
            .cmp(&right.key_global_priority_for_format.unwrap_or(i32::MAX))
            .then_with(|| compare_provider_key_health_order(left, right, provider_key_rpm_states))
            .then_with(|| compare_affinity_order(left, right, affinity_key))
            .then(left.provider_priority.cmp(&right.provider_priority))
            .then(left.key_internal_priority.cmp(&right.key_internal_priority))
            .then(left.provider_id.cmp(&right.provider_id))
            .then(left.endpoint_id.cmp(&right.endpoint_id))
            .then(left.key_id.cmp(&right.key_id))
            .then(
                left.selected_provider_model_name
                    .cmp(&right.selected_provider_model_name),
            )
    });
}

fn compare_provider_key_health_order(
    left: &GatewayMinimalCandidateSelectionCandidate,
    right: &GatewayMinimalCandidateSelectionCandidate,
    provider_key_rpm_states: &BTreeMap<String, StoredProviderCatalogKey>,
) -> std::cmp::Ordering {
    let left_bucket = candidate_provider_key_health_bucket(left, provider_key_rpm_states);
    let right_bucket = candidate_provider_key_health_bucket(right, provider_key_rpm_states);
    right_bucket.cmp(&left_bucket).then_with(|| {
        let left_score = candidate_provider_key_health_score(left, provider_key_rpm_states);
        let right_score = candidate_provider_key_health_score(right, provider_key_rpm_states);
        right_score
            .partial_cmp(&left_score)
            .unwrap_or(std::cmp::Ordering::Equal)
    })
}

fn candidate_provider_key_health_bucket(
    candidate: &GatewayMinimalCandidateSelectionCandidate,
    provider_key_rpm_states: &BTreeMap<String, StoredProviderCatalogKey>,
) -> Option<ProviderKeyHealthBucket> {
    provider_key_rpm_states
        .get(&candidate.key_id)
        .and_then(|key| provider_key_health_bucket(key, candidate.endpoint_api_format.as_str()))
}

fn candidate_provider_key_health_score(
    candidate: &GatewayMinimalCandidateSelectionCandidate,
    provider_key_rpm_states: &BTreeMap<String, StoredProviderCatalogKey>,
) -> f64 {
    provider_key_rpm_states
        .get(&candidate.key_id)
        .and_then(|key| {
            effective_provider_key_health_score(key, candidate.endpoint_api_format.as_str())
        })
        .unwrap_or(1.0)
}

pub(super) fn should_skip_provider_quota(
    quota: &StoredProviderQuotaSnapshot,
    now_unix_secs: u64,
) -> bool {
    let snapshot = ProviderQuotaSnapshot {
        provider_id: quota.provider_id.clone(),
        billing_type: ProviderBillingType::parse(&quota.billing_type),
        monthly_quota_usd: quota.monthly_quota_usd,
        monthly_used_usd: quota.monthly_used_usd,
        quota_reset_day: quota.quota_reset_day,
        quota_last_reset_at_unix_secs: quota.quota_last_reset_at_unix_secs,
        quota_expires_at_unix_secs: quota.quota_expires_at_unix_secs,
        is_active: quota.is_active,
    };

    if !snapshot.is_active || snapshot.is_expired(now_unix_secs) {
        return true;
    }

    match snapshot.billing_type {
        ProviderBillingType::MonthlyQuota | ProviderBillingType::FreeTier => snapshot
            .remaining_quota_usd()
            .is_some_and(|remaining| remaining <= 0.0),
        ProviderBillingType::PayAsYouGo | ProviderBillingType::Unknown => false,
    }
}

fn is_candidate_cooled_down(
    candidate: &GatewayMinimalCandidateSelectionCandidate,
    recent_candidates: &[StoredRequestCandidate],
    now_unix_secs: u64,
) -> bool {
    is_candidate_in_recent_failure_cooldown(
        recent_candidates,
        candidate.provider_id.as_str(),
        candidate.endpoint_id.as_str(),
        candidate.key_id.as_str(),
        now_unix_secs,
    )
}

pub(super) async fn is_candidate_selectable(
    candidate: &GatewayMinimalCandidateSelectionCandidate,
    recent_candidates: &[StoredRequestCandidate],
    provider_concurrent_limits: &BTreeMap<String, usize>,
    provider_key_rpm_states: &BTreeMap<String, StoredProviderCatalogKey>,
    now_unix_secs: u64,
    cached_affinity_target: Option<&SchedulerAffinityTarget>,
    state: &AppState,
) -> Result<bool, GatewayError> {
    let quota = state
        .read_provider_quota_snapshot(&candidate.provider_id)
        .await?;
    if quota
        .as_ref()
        .is_some_and(|quota| should_skip_provider_quota(quota, now_unix_secs))
    {
        return Ok(false);
    }
    if is_candidate_cooled_down(candidate, recent_candidates, now_unix_secs) {
        return Ok(false);
    }
    if provider_concurrent_limits
        .get(&candidate.provider_id)
        .is_some_and(|limit| {
            count_recent_active_requests_for_provider(
                recent_candidates,
                candidate.provider_id.as_str(),
                now_unix_secs,
            ) >= *limit
        })
    {
        return Ok(false);
    }
    let is_cached_user =
        cached_affinity_target.is_some_and(|target| matches_affinity_target(candidate, target));
    if let Some(provider_key) = provider_key_rpm_states.get(&candidate.key_id) {
        if is_provider_key_circuit_open(provider_key, candidate.endpoint_api_format.as_str()) {
            return Ok(false);
        }
        if provider_key_health_score(provider_key, candidate.endpoint_api_format.as_str())
            .is_some_and(|score| score <= 0.0)
        {
            return Ok(false);
        }
        let rpm_reset_at =
            state.provider_key_rpm_reset_at(candidate.key_id.as_str(), now_unix_secs);
        if !provider_key_rpm_allows_request_since(
            provider_key,
            recent_candidates,
            now_unix_secs,
            is_cached_user,
            rpm_reset_at,
        ) {
            return Ok(false);
        }
    }

    Ok(true)
}

pub(super) async fn read_provider_concurrent_limits(
    state: &AppState,
    candidates: &[GatewayMinimalCandidateSelectionCandidate],
) -> Result<BTreeMap<String, usize>, GatewayError> {
    let provider_ids = candidates
        .iter()
        .map(|candidate| candidate.provider_id.clone())
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect::<Vec<_>>();
    if provider_ids.is_empty() {
        return Ok(BTreeMap::new());
    }

    let providers = state
        .read_provider_catalog_providers_by_ids(&provider_ids)
        .await?;
    Ok(build_provider_concurrent_limit_map(providers))
}

fn build_provider_concurrent_limit_map(
    providers: Vec<StoredProviderCatalogProvider>,
) -> BTreeMap<String, usize> {
    providers
        .into_iter()
        .filter_map(|provider| {
            provider
                .concurrent_limit
                .and_then(|limit| usize::try_from(limit).ok())
                .filter(|limit| *limit > 0)
                .map(|limit| (provider.id, limit))
        })
        .collect()
}

pub(super) async fn read_provider_key_rpm_states(
    state: &AppState,
    candidates: &[GatewayMinimalCandidateSelectionCandidate],
) -> Result<BTreeMap<String, StoredProviderCatalogKey>, GatewayError> {
    let key_ids = candidates
        .iter()
        .map(|candidate| candidate.key_id.clone())
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect::<Vec<_>>();
    if key_ids.is_empty() {
        return Ok(BTreeMap::new());
    }

    let keys = state.read_provider_catalog_keys_by_ids(&key_ids).await?;
    Ok(keys
        .into_iter()
        .map(|key| (key.id.clone(), key))
        .collect::<BTreeMap<_, _>>())
}
