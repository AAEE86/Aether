use sha2::{Digest, Sha256};

use crate::gateway::gateway_cache::SchedulerAffinityTarget;
use crate::gateway::gateway_data::StoredGatewayAuthApiKeySnapshot;
use crate::gateway::AppState;

use super::{
    normalize_api_format, GatewayMinimalCandidateSelectionCandidate,
    SCHEDULER_AFFINITY_MAX_ENTRIES, SCHEDULER_AFFINITY_TTL,
};

pub(super) fn build_scheduler_affinity_cache_key(
    auth_snapshot: Option<&StoredGatewayAuthApiKeySnapshot>,
    api_format: &str,
    global_model_name: &str,
) -> Option<String> {
    let api_key_id = auth_snapshot
        .map(|snapshot| snapshot.api_key_id.trim())
        .filter(|value| !value.is_empty())?;
    build_scheduler_affinity_cache_key_for_api_key_id(api_key_id, api_format, global_model_name)
}

pub(super) fn build_scheduler_affinity_cache_key_for_api_key_id(
    api_key_id: &str,
    api_format: &str,
    global_model_name: &str,
) -> Option<String> {
    let api_key_id = api_key_id.trim();
    if api_key_id.is_empty() {
        return None;
    }
    let api_format = normalize_api_format(api_format);
    let global_model_name = global_model_name.trim();
    if api_format.is_empty() || global_model_name.is_empty() {
        return None;
    }

    Some(format!(
        "scheduler_affinity:{api_key_id}:{api_format}:{global_model_name}"
    ))
}

pub(super) fn compare_affinity_order(
    left: &GatewayMinimalCandidateSelectionCandidate,
    right: &GatewayMinimalCandidateSelectionCandidate,
    affinity_key: Option<&str>,
) -> std::cmp::Ordering {
    let Some(affinity_key) = affinity_key else {
        return std::cmp::Ordering::Equal;
    };

    candidate_affinity_hash(affinity_key, left).cmp(&candidate_affinity_hash(affinity_key, right))
}

pub(super) fn candidate_affinity_hash(
    affinity_key: &str,
    candidate: &GatewayMinimalCandidateSelectionCandidate,
) -> u64 {
    let mut hasher = Sha256::new();
    hasher.update(affinity_key.as_bytes());
    hasher.update(b":");
    hasher.update(candidate.provider_id.as_bytes());
    hasher.update(b":");
    hasher.update(candidate.endpoint_id.as_bytes());
    hasher.update(b":");
    hasher.update(candidate.key_id.as_bytes());
    let digest = hasher.finalize();
    u64::from_be_bytes([
        digest[0], digest[1], digest[2], digest[3], digest[4], digest[5], digest[6], digest[7],
    ])
}

pub(super) fn matches_affinity_target(
    candidate: &GatewayMinimalCandidateSelectionCandidate,
    target: &SchedulerAffinityTarget,
) -> bool {
    candidate.provider_id == target.provider_id
        && candidate.endpoint_id == target.endpoint_id
        && candidate.key_id == target.key_id
}

pub(super) fn candidate_key(
    candidate: &GatewayMinimalCandidateSelectionCandidate,
) -> (String, String, String) {
    (
        candidate.provider_id.clone(),
        candidate.endpoint_id.clone(),
        candidate.key_id.clone(),
    )
}

#[cfg_attr(not(test), allow(dead_code))]
pub(super) fn remember_scheduler_affinity(
    affinity_cache_key: Option<&str>,
    state: &AppState,
    candidate: &GatewayMinimalCandidateSelectionCandidate,
) {
    let Some(cache_key) = affinity_cache_key else {
        return;
    };

    state.scheduler_affinity_cache.insert(
        cache_key.to_string(),
        SchedulerAffinityTarget {
            provider_id: candidate.provider_id.clone(),
            endpoint_id: candidate.endpoint_id.clone(),
            key_id: candidate.key_id.clone(),
        },
        SCHEDULER_AFFINITY_TTL,
        SCHEDULER_AFFINITY_MAX_ENTRIES,
    );
}
