use std::collections::{BTreeMap, BTreeSet};

use aether_admin::provider::{
    pool as admin_provider_pool_pure, status as admin_provider_status_pure,
};
use aether_data_contracts::repository::candidates::StoredRequestCandidate;
use aether_data_contracts::repository::provider_catalog::StoredProviderCatalogKey;
use aether_scheduler_core::{
    auth_api_key_concurrency_limit_reached, build_provider_concurrent_limit_map,
    candidate_is_selectable_with_runtime_state, candidate_runtime_skip_reason_with_state,
    effective_provider_key_rpm_limit, CandidateRuntimeSelectabilityInput,
};
use std::time::{SystemTime, UNIX_EPOCH};

use crate::data::auth::GatewayAuthApiKeySnapshot;
use crate::GatewayError;

use super::{SchedulerMinimalCandidateSelectionCandidate, SchedulerRuntimeState};

pub(super) use aether_scheduler_core::should_skip_provider_quota;

const REQUEST_CANDIDATE_RUNTIME_WINDOW_SECS: u64 = 300;

pub(super) struct CandidateRuntimeSelectionSnapshot {
    pub(super) recent_candidates: Vec<StoredRequestCandidate>,
    pub(super) provider_concurrent_limits: BTreeMap<String, usize>,
    pub(super) provider_key_rpm_states: BTreeMap<String, StoredProviderCatalogKey>,
    pub(super) pool_provider_ids: BTreeSet<String>,
    provider_quota_blocks_requests: BTreeMap<String, bool>,
    key_account_quota_exhausted: BTreeMap<String, bool>,
    key_oauth_invalid: BTreeMap<String, bool>,
    key_pool_account_skip_reasons: BTreeMap<String, &'static str>,
    provider_key_rpm_reset_ats: BTreeMap<String, Option<u64>>,
}

pub(super) async fn read_candidate_runtime_selection_snapshot(
    state: &(impl SchedulerRuntimeState + ?Sized),
    candidates: &[SchedulerMinimalCandidateSelectionCandidate],
    auth_snapshot: Option<&GatewayAuthApiKeySnapshot>,
    now_unix_secs: u64,
) -> Result<CandidateRuntimeSelectionSnapshot, GatewayError> {
    let provider_concurrent_limits = read_provider_concurrent_limits(state, candidates).await?;
    let provider_pool_state = read_provider_pool_state_map(state, candidates).await?;
    let provider_skip_exhausted_accounts = provider_pool_state
        .iter()
        .map(|(provider_id, state)| (provider_id.clone(), state.skip_exhausted_accounts))
        .collect::<BTreeMap<_, _>>();
    let pool_provider_ids = provider_pool_state
        .iter()
        .filter_map(|(provider_id, state)| state.pool_enabled.then_some(provider_id.clone()))
        .collect::<BTreeSet<_>>();
    let provider_key_rpm_states = read_provider_key_rpm_states(state, candidates).await?;
    let runtime_scopes = runtime_request_candidate_scopes(
        candidates,
        auth_snapshot,
        &provider_concurrent_limits,
        &provider_key_rpm_states,
        now_unix_secs,
    );
    let recent_candidates = if runtime_scopes.is_empty() {
        Vec::new()
    } else {
        state
            .read_runtime_scoped_request_candidates_since(
                &runtime_scopes.provider_ids,
                &runtime_scopes.key_ids,
                &runtime_scopes.api_key_ids,
                now_unix_secs.saturating_sub(REQUEST_CANDIDATE_RUNTIME_WINDOW_SECS),
            )
            .await?
    };
    let key_account_quota_exhausted = read_key_account_quota_exhaustion_map(
        candidates,
        &provider_key_rpm_states,
        &provider_skip_exhausted_accounts,
    );
    let key_oauth_invalid =
        read_key_oauth_invalid_map(candidates, &provider_key_rpm_states, now_unix_secs);
    let key_pool_account_skip_reasons = read_key_pool_account_skip_reason_map(
        candidates,
        &provider_key_rpm_states,
        &provider_pool_state,
        now_unix_secs,
    );
    let provider_quota_blocks_requests =
        read_provider_quota_block_map(state, candidates, now_unix_secs).await?;
    let provider_key_rpm_reset_ats =
        read_provider_key_rpm_reset_at_map(state, candidates, now_unix_secs);

    Ok(CandidateRuntimeSelectionSnapshot {
        recent_candidates,
        provider_concurrent_limits,
        provider_key_rpm_states,
        pool_provider_ids,
        provider_quota_blocks_requests,
        key_account_quota_exhausted,
        key_oauth_invalid,
        key_pool_account_skip_reasons,
        provider_key_rpm_reset_ats,
    })
}

#[derive(Debug, Default, PartialEq, Eq)]
struct RuntimeRequestCandidateScopes {
    provider_ids: Vec<String>,
    key_ids: Vec<String>,
    api_key_ids: Vec<String>,
}

impl RuntimeRequestCandidateScopes {
    fn is_empty(&self) -> bool {
        self.provider_ids.is_empty() && self.key_ids.is_empty() && self.api_key_ids.is_empty()
    }
}

fn runtime_request_candidate_scopes(
    candidates: &[SchedulerMinimalCandidateSelectionCandidate],
    auth_snapshot: Option<&GatewayAuthApiKeySnapshot>,
    provider_concurrent_limits: &BTreeMap<String, usize>,
    provider_key_rpm_states: &BTreeMap<String, StoredProviderCatalogKey>,
    now_unix_secs: u64,
) -> RuntimeRequestCandidateScopes {
    let api_key_ids = auth_snapshot
        .and_then(|snapshot| snapshot.api_key_concurrent_limit)
        .filter(|limit| *limit > 0)
        .and_then(|_| auth_snapshot.map(|snapshot| snapshot.api_key_id.trim()))
        .filter(|api_key_id| !api_key_id.is_empty())
        .map(|api_key_id| vec![api_key_id.to_string()])
        .unwrap_or_default();
    let provider_ids = candidates
        .iter()
        .filter(|candidate| {
            provider_concurrent_limits
                .get(candidate.provider_id.as_str())
                .is_some_and(|limit| *limit > 0)
        })
        .map(|candidate| candidate.provider_id.clone())
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect();
    let key_ids = candidates
        .iter()
        .filter(|candidate| {
            provider_key_rpm_states
                .get(candidate.key_id.as_str())
                .is_some_and(|key| {
                    key.concurrent_limit.is_some_and(|limit| limit > 0)
                        || effective_provider_key_rpm_limit(key, now_unix_secs).is_some()
                })
        })
        .map(|candidate| candidate.key_id.clone())
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect();

    RuntimeRequestCandidateScopes {
        provider_ids,
        key_ids,
        api_key_ids,
    }
}

pub(super) fn auth_snapshot_concurrency_limit_reached(
    auth_snapshot: Option<&GatewayAuthApiKeySnapshot>,
    snapshot: &CandidateRuntimeSelectionSnapshot,
    now_unix_secs: u64,
) -> bool {
    auth_snapshot
        .and_then(|snapshot| {
            usize::try_from(snapshot.api_key_concurrent_limit?)
                .ok()
                .and_then(|limit| {
                    if limit == 0 {
                        return None;
                    }
                    Some((snapshot.api_key_id.as_str(), limit))
                })
        })
        .is_some_and(|(api_key_id, limit)| {
            auth_api_key_concurrency_limit_reached(
                &snapshot.recent_candidates,
                now_unix_secs,
                api_key_id,
                limit,
            )
        })
}

pub(super) fn is_candidate_selectable(
    candidate: &SchedulerMinimalCandidateSelectionCandidate,
    snapshot: &CandidateRuntimeSelectionSnapshot,
    now_unix_secs: u64,
) -> bool {
    let pool_group = snapshot
        .pool_provider_ids
        .contains(candidate.provider_id.as_str());
    candidate_is_selectable_with_runtime_state(CandidateRuntimeSelectabilityInput {
        candidate,
        recent_candidates: &snapshot.recent_candidates,
        provider_concurrent_limits: &snapshot.provider_concurrent_limits,
        provider_key_rpm_states: &snapshot.provider_key_rpm_states,
        now_unix_secs,
        provider_quota_blocks_requests: snapshot
            .provider_quota_blocks_requests
            .get(candidate.provider_id.as_str())
            .copied()
            .unwrap_or(false),
        account_quota_exhausted: !pool_group
            && snapshot
                .key_account_quota_exhausted
                .get(candidate.key_id.as_str())
                .copied()
                .unwrap_or(false),
        oauth_invalid: !pool_group
            && snapshot
                .key_oauth_invalid
                .get(candidate.key_id.as_str())
                .copied()
                .unwrap_or(false),
        enforce_key_circuit_breaker: !pool_group,
        rpm_reset_at: (!pool_group)
            .then(|| {
                snapshot
                    .provider_key_rpm_reset_ats
                    .get(candidate.key_id.as_str())
                    .copied()
                    .flatten()
            })
            .flatten(),
    })
}

pub(super) fn current_candidate_runtime_skip_reason(
    candidate: &SchedulerMinimalCandidateSelectionCandidate,
    snapshot: &CandidateRuntimeSelectionSnapshot,
    now_unix_secs: u64,
) -> Option<&'static str> {
    let pool_group = snapshot
        .pool_provider_ids
        .contains(candidate.provider_id.as_str());
    let provider_quota_blocks_requests = snapshot
        .provider_quota_blocks_requests
        .get(candidate.provider_id.as_str())
        .copied()
        .unwrap_or(false);
    let rpm_reset_at = (!pool_group)
        .then(|| {
            snapshot
                .provider_key_rpm_reset_ats
                .get(candidate.key_id.as_str())
                .copied()
                .flatten()
        })
        .flatten();

    candidate_runtime_skip_reason_with_state(CandidateRuntimeSelectabilityInput {
        candidate,
        recent_candidates: &snapshot.recent_candidates,
        provider_concurrent_limits: &snapshot.provider_concurrent_limits,
        provider_key_rpm_states: &snapshot.provider_key_rpm_states,
        now_unix_secs,
        provider_quota_blocks_requests,
        account_quota_exhausted: !pool_group
            && snapshot
                .key_account_quota_exhausted
                .get(candidate.key_id.as_str())
                .copied()
                .unwrap_or(false),
        oauth_invalid: !pool_group
            && snapshot
                .key_oauth_invalid
                .get(candidate.key_id.as_str())
                .copied()
                .unwrap_or(false),
        enforce_key_circuit_breaker: !pool_group,
        rpm_reset_at,
    })
}

const POOL_ACTIVE_PROBE_SEALED_SKIP_REASON: &str = "pool_active_probe_sealed";

#[derive(Debug, Clone, Copy)]
pub(crate) struct ConcretePoolRuntimePolicy {
    pub(crate) cost_window_seconds: u64,
    pub(crate) cost_limit_per_key_tokens: Option<u64>,
    pub(crate) probing_enabled: bool,
}

pub(super) async fn concrete_pool_candidate_runtime_skip_reason(
    state: &(impl SchedulerRuntimeState + ?Sized),
    candidate: &SchedulerMinimalCandidateSelectionCandidate,
    policy: ConcretePoolRuntimePolicy,
) -> Result<Option<&'static str>, GatewayError> {
    let cooldown_reason = state
        .read_pool_key_cooldown_reason(candidate.provider_id.as_str(), candidate.key_id.as_str())
        .await?;
    if let Some(reason) = classify_concrete_pool_candidate_runtime_skip_reason(
        cooldown_reason.as_deref(),
        None,
        None,
        None,
        candidate.key_id.as_str(),
    ) {
        return Ok(Some(reason));
    }

    let cost_window_usage = if policy.cost_limit_per_key_tokens.is_some() {
        Some(
            state
                .read_pool_key_cost_window_usage(
                    candidate.provider_id.as_str(),
                    candidate.key_id.as_str(),
                    policy.cost_window_seconds,
                )
                .await?,
        )
    } else {
        None
    };
    if let Some(reason) = classify_concrete_pool_candidate_runtime_skip_reason(
        None,
        cost_window_usage,
        policy.cost_limit_per_key_tokens,
        None,
        candidate.key_id.as_str(),
    ) {
        return Ok(Some(reason));
    }

    let active_member_ids = if policy.probing_enabled {
        Some(
            state
                .read_pool_active_probe_member_ids(candidate.provider_id.as_str())
                .await?,
        )
    } else {
        None
    };

    Ok(classify_concrete_pool_candidate_runtime_skip_reason(
        None,
        None,
        None,
        active_member_ids.as_ref(),
        candidate.key_id.as_str(),
    ))
}

fn classify_concrete_pool_candidate_runtime_skip_reason(
    cooldown_reason: Option<&str>,
    cost_window_usage: Option<u64>,
    cost_limit: Option<u64>,
    active_member_ids: Option<&BTreeSet<String>>,
    key_id: &str,
) -> Option<&'static str> {
    if cooldown_reason.is_some() {
        return Some(aether_pool_core::POOL_COOLDOWN_SKIP_REASON);
    }
    if cost_limit
        .zip(cost_window_usage)
        .is_some_and(|(limit, usage)| usage >= limit)
    {
        return Some(aether_pool_core::POOL_COST_LIMIT_REACHED_SKIP_REASON);
    }
    if active_member_ids.is_some_and(|members| !members.is_empty() && !members.contains(key_id)) {
        return Some(POOL_ACTIVE_PROBE_SEALED_SKIP_REASON);
    }
    None
}

pub(super) fn current_concrete_candidate_runtime_skip_reason(
    candidate: &SchedulerMinimalCandidateSelectionCandidate,
    snapshot: &CandidateRuntimeSelectionSnapshot,
    now_unix_secs: u64,
) -> Option<&'static str> {
    if let Some(reason) = snapshot
        .key_pool_account_skip_reasons
        .get(candidate.key_id.as_str())
        .copied()
    {
        return Some(reason);
    }
    let pool_group = snapshot
        .pool_provider_ids
        .contains(candidate.provider_id.as_str());
    candidate_runtime_skip_reason_with_state(CandidateRuntimeSelectabilityInput {
        candidate,
        recent_candidates: &snapshot.recent_candidates,
        provider_concurrent_limits: &snapshot.provider_concurrent_limits,
        provider_key_rpm_states: &snapshot.provider_key_rpm_states,
        now_unix_secs,
        provider_quota_blocks_requests: snapshot
            .provider_quota_blocks_requests
            .get(candidate.provider_id.as_str())
            .copied()
            .unwrap_or(false),
        account_quota_exhausted: !pool_group
            && snapshot
                .key_account_quota_exhausted
                .get(candidate.key_id.as_str())
                .copied()
                .unwrap_or(false),
        oauth_invalid: !pool_group
            && snapshot
                .key_oauth_invalid
                .get(candidate.key_id.as_str())
                .copied()
                .unwrap_or(false),
        enforce_key_circuit_breaker: true,
        rpm_reset_at: snapshot
            .provider_key_rpm_reset_ats
            .get(candidate.key_id.as_str())
            .copied()
            .flatten(),
    })
}

pub(super) async fn read_provider_concurrent_limits(
    state: &(impl SchedulerRuntimeState + ?Sized),
    candidates: &[SchedulerMinimalCandidateSelectionCandidate],
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

pub(super) async fn read_provider_key_rpm_states(
    state: &(impl SchedulerRuntimeState + ?Sized),
    candidates: &[SchedulerMinimalCandidateSelectionCandidate],
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

async fn read_provider_quota_block_map(
    state: &(impl SchedulerRuntimeState + ?Sized),
    candidates: &[SchedulerMinimalCandidateSelectionCandidate],
    now_unix_secs: u64,
) -> Result<BTreeMap<String, bool>, GatewayError> {
    let provider_ids = candidates
        .iter()
        .map(|candidate| candidate.provider_id.clone())
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect::<Vec<_>>();
    let mut quota_blocks = BTreeMap::new();

    for provider_id in provider_ids {
        let blocks_requests = state
            .read_provider_quota_snapshot(&provider_id)
            .await?
            .as_ref()
            .is_some_and(|quota| should_skip_provider_quota(quota, now_unix_secs));
        quota_blocks.insert(provider_id, blocks_requests);
    }

    Ok(quota_blocks)
}

#[derive(Debug, Clone, Copy, Default)]
struct ProviderPoolState {
    pool_enabled: bool,
    skip_exhausted_accounts: bool,
}

async fn read_provider_pool_state_map(
    state: &(impl SchedulerRuntimeState + ?Sized),
    candidates: &[SchedulerMinimalCandidateSelectionCandidate],
) -> Result<BTreeMap<String, ProviderPoolState>, GatewayError> {
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
    Ok(providers
        .into_iter()
        .map(|provider| {
            let pool_advanced = provider
                .config
                .as_ref()
                .and_then(|value| value.get("pool_advanced"));
            let skip_exhausted_accounts = pool_advanced
                .and_then(serde_json::Value::as_object)
                .and_then(|value| value.get("skip_exhausted_accounts"))
                .and_then(serde_json::Value::as_bool)
                .unwrap_or(false);
            (
                provider.id,
                ProviderPoolState {
                    pool_enabled: pool_advanced.is_some(),
                    skip_exhausted_accounts,
                },
            )
        })
        .collect())
}

fn read_key_account_quota_exhaustion_map(
    candidates: &[SchedulerMinimalCandidateSelectionCandidate],
    provider_key_rpm_states: &BTreeMap<String, StoredProviderCatalogKey>,
    provider_skip_exhausted_accounts: &BTreeMap<String, bool>,
) -> BTreeMap<String, bool> {
    candidates
        .iter()
        .map(|candidate| {
            let exhausted = provider_skip_exhausted_accounts
                .get(candidate.provider_id.as_str())
                .copied()
                .unwrap_or(false)
                && provider_key_rpm_states
                    .get(candidate.key_id.as_str())
                    .is_some_and(|key| {
                        admin_provider_pool_pure::admin_pool_key_account_quota_exhausted(
                            key,
                            candidate.provider_type.as_str(),
                        )
                    });
            (candidate.key_id.clone(), exhausted)
        })
        .collect()
}

fn read_key_pool_account_skip_reason_map(
    candidates: &[SchedulerMinimalCandidateSelectionCandidate],
    provider_key_rpm_states: &BTreeMap<String, StoredProviderCatalogKey>,
    provider_pool_state: &BTreeMap<String, ProviderPoolState>,
    now_unix_secs: u64,
) -> BTreeMap<String, &'static str> {
    candidates
        .iter()
        .filter_map(|candidate| {
            let pool_state = provider_pool_state.get(candidate.provider_id.as_str())?;
            let key = provider_key_rpm_states.get(candidate.key_id.as_str())?;
            concrete_pool_account_skip_reason(
                key,
                candidate.provider_type.as_str(),
                *pool_state,
                now_unix_secs,
            )
            .map(|reason| (candidate.key_id.clone(), reason))
        })
        .collect()
}

fn concrete_pool_account_skip_reason(
    key: &StoredProviderCatalogKey,
    provider_type: &str,
    pool_state: ProviderPoolState,
    now_unix_secs: u64,
) -> Option<&'static str> {
    if !pool_state.pool_enabled {
        return None;
    }
    if admin_provider_pool_pure::admin_pool_key_is_known_banned(key)
        || admin_provider_pool_pure::admin_pool_key_requires_reauth_for_scheduling(
            key,
            now_unix_secs,
        )
    {
        return Some(aether_pool_core::POOL_ACCOUNT_BLOCKED_SKIP_REASON);
    }
    if admin_provider_pool_pure::admin_pool_key_quota_hard_blocked(key, provider_type)
        || (pool_state.skip_exhausted_accounts
            && admin_provider_pool_pure::admin_pool_key_account_quota_exhausted(key, provider_type))
    {
        return Some(aether_pool_core::POOL_ACCOUNT_EXHAUSTED_SKIP_REASON);
    }
    None
}

fn read_key_oauth_invalid_map(
    candidates: &[SchedulerMinimalCandidateSelectionCandidate],
    provider_key_rpm_states: &BTreeMap<String, StoredProviderCatalogKey>,
    now_unix_secs: u64,
) -> BTreeMap<String, bool> {
    candidates
        .iter()
        .map(|candidate| {
            let oauth_invalid = provider_key_rpm_states
                .get(candidate.key_id.as_str())
                .is_some_and(|key| {
                    key_requires_oauth_reauth(key, candidate.provider_type.as_str(), now_unix_secs)
                });
            (candidate.key_id.clone(), oauth_invalid)
        })
        .collect()
}

fn key_requires_oauth_reauth(
    key: &StoredProviderCatalogKey,
    provider_type: &str,
    now_unix_secs: u64,
) -> bool {
    if !key.auth_type.trim().eq_ignore_ascii_case("oauth") {
        return false;
    }

    let invalid_reason = key
        .oauth_invalid_reason
        .as_deref()
        .map(str::trim)
        .unwrap_or_default();
    if !invalid_reason.is_empty() {
        return oauth_invalid_reason_blocks_scheduling(
            key,
            provider_type,
            invalid_reason,
            now_unix_secs,
        );
    }

    false
}

fn oauth_invalid_reason_blocks_scheduling(
    key: &StoredProviderCatalogKey,
    provider_type: &str,
    invalid_reason: &str,
    now_unix_secs: u64,
) -> bool {
    let trimmed_reason = invalid_reason.trim();

    let account_state = admin_provider_status_pure::resolve_pool_account_state(
        Some(provider_type),
        key.upstream_metadata.as_ref(),
        Some(trimmed_reason),
    );
    if account_state.blocked
        && !account_state.recoverable
        && account_state
            .code
            .as_deref()
            .is_some_and(oauth_account_state_code_is_hard_block)
    {
        return true;
    }

    if oauth_invalid_reason_has_tag(trimmed_reason, "[REFRESH_FAILED]") {
        return oauth_access_token_expired(key, now_unix_secs);
    }

    false
}

fn oauth_invalid_reason_has_tag(reason: &str, tag: &str) -> bool {
    reason
        .lines()
        .map(str::trim)
        .any(|line| line.starts_with(tag))
}

fn oauth_access_token_expired(key: &StoredProviderCatalogKey, now_unix_secs: u64) -> bool {
    let now_unix_secs = if now_unix_secs == 0 {
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .ok()
            .map(|duration| duration.as_secs())
            .unwrap_or(0)
    } else {
        now_unix_secs
    };
    key.expires_at_unix_secs
        .is_none_or(|expires_at| expires_at == 0 || expires_at <= now_unix_secs)
}

fn oauth_account_state_code_is_hard_block(code: &str) -> bool {
    matches!(
        code.trim().to_ascii_lowercase().as_str(),
        "account_banned"
            | "account_suspended"
            | "account_disabled"
            | "workspace_deactivated"
            | "account_forbidden"
            | "account_blocked"
            | "account_verification"
            | "oauth_token_invalid"
    )
}

fn read_provider_key_rpm_reset_at_map(
    state: &(impl SchedulerRuntimeState + ?Sized),
    candidates: &[SchedulerMinimalCandidateSelectionCandidate],
    now_unix_secs: u64,
) -> BTreeMap<String, Option<u64>> {
    candidates
        .iter()
        .map(|candidate| {
            (
                candidate.key_id.clone(),
                state.provider_key_rpm_reset_at(candidate.key_id.as_str(), now_unix_secs),
            )
        })
        .collect::<BTreeMap<_, _>>()
}

#[cfg(test)]
mod tests {
    use super::{
        classify_concrete_pool_candidate_runtime_skip_reason, concrete_pool_account_skip_reason,
        ProviderPoolState,
    };
    use aether_data_contracts::repository::provider_catalog::StoredProviderCatalogKey;
    use serde_json::json;
    use std::collections::BTreeSet;

    fn sample_pool_key() -> StoredProviderCatalogKey {
        StoredProviderCatalogKey::new(
            "key-1".to_string(),
            "provider-1".to_string(),
            "key-1".to_string(),
            "oauth".to_string(),
            None,
            true,
        )
        .expect("pool key should build")
    }

    fn pool_state(skip_exhausted_accounts: bool) -> ProviderPoolState {
        ProviderPoolState {
            pool_enabled: true,
            skip_exhausted_accounts,
        }
    }

    #[test]
    fn concrete_pool_runtime_prefers_cooldown_over_other_blocks() {
        let active_members = BTreeSet::from(["other-key".to_string()]);

        assert_eq!(
            classify_concrete_pool_candidate_runtime_skip_reason(
                Some("rate_limit"),
                Some(100),
                Some(100),
                Some(&active_members),
                "key-1",
            ),
            Some("pool_cooldown")
        );
    }

    #[test]
    fn concrete_pool_runtime_blocks_at_cost_limit() {
        assert_eq!(
            classify_concrete_pool_candidate_runtime_skip_reason(
                None,
                Some(100),
                Some(100),
                None,
                "key-1",
            ),
            Some("pool_cost_limit_reached")
        );
    }

    #[test]
    fn concrete_pool_runtime_enforces_nonempty_active_probe_seal() {
        let active_members = BTreeSet::from(["other-key".to_string()]);

        assert_eq!(
            classify_concrete_pool_candidate_runtime_skip_reason(
                None,
                None,
                None,
                Some(&active_members),
                "key-1",
            ),
            Some("pool_active_probe_sealed")
        );
    }

    #[test]
    fn concrete_pool_runtime_allows_empty_active_probe_set_for_cold_start() {
        assert_eq!(
            classify_concrete_pool_candidate_runtime_skip_reason(
                None,
                None,
                None,
                Some(&BTreeSet::new()),
                "key-1",
            ),
            None
        );
    }

    #[test]
    fn concrete_pool_account_known_ban_is_always_authorization_blocked() {
        let mut key = sample_pool_key();
        key.upstream_metadata = Some(json!({
            "codex": {
                "account_disabled": true,
                "reason": "deactivated_workspace"
            }
        }));

        assert_eq!(
            concrete_pool_account_skip_reason(&key, "codex", pool_state(false), 100),
            Some("pool_account_blocked")
        );
    }

    #[test]
    fn concrete_pool_account_hard_quota_is_unconditional_capacity_limit() {
        let mut key = sample_pool_key();
        key.status_snapshot = Some(json!({
            "quota": {
                "provider_type": "codex",
                "allowed": false,
                "limit_reached": true,
                "observed_at": 4_000_000_000u64
            }
        }));

        assert_eq!(
            concrete_pool_account_skip_reason(&key, "codex", pool_state(false), 100),
            Some("pool_account_exhausted")
        );
    }

    #[test]
    fn concrete_pool_account_ordinary_exhaustion_respects_provider_flag() {
        let mut key = sample_pool_key();
        key.upstream_metadata = Some(json!({
            "kiro": { "remaining": 0 }
        }));

        assert_eq!(
            concrete_pool_account_skip_reason(&key, "kiro", pool_state(false), 100),
            None
        );
        assert_eq!(
            concrete_pool_account_skip_reason(&key, "kiro", pool_state(true), 100),
            Some("pool_account_exhausted")
        );
    }

    #[test]
    fn concrete_pool_account_uses_dispatch_oauth_reauth_semantics() {
        let mut key = sample_pool_key();
        key.expires_at_unix_secs = Some(200);
        key.oauth_invalid_reason = Some("[REFRESH_FAILED] refresh token rotated".to_string());

        assert_eq!(
            concrete_pool_account_skip_reason(&key, "codex", pool_state(false), 100),
            None
        );
        assert_eq!(
            concrete_pool_account_skip_reason(&key, "codex", pool_state(false), 200),
            Some("pool_account_blocked")
        );
    }
}
