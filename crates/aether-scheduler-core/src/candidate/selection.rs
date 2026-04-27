use aether_data_contracts::DataLayerError;

use super::capability::{
    enabled_required_capabilities, requested_capability_priority_for_candidate_descriptors,
};
use super::enumeration::enumerate_minimal_candidate_selection;
use super::types::{
    BuildMinimalCandidateSelectionInput, SchedulerMinimalCandidateSelectionCandidate,
};

pub fn build_ranked_minimal_candidate_selection(
    input: BuildMinimalCandidateSelectionInput<'_>,
) -> Result<Vec<SchedulerMinimalCandidateSelectionCandidate>, DataLayerError> {
    let priority_mode = input.priority_mode;
    let affinity_key = input.affinity_key.map(str::to_string);
    let required_capabilities = enabled_required_capabilities(input.required_capabilities);
    let mut candidates = enumerate_minimal_candidate_selection(input)?;
    let rankables = candidates
        .iter()
        .enumerate()
        .map(|(index, candidate)| {
            crate::SchedulerRankableCandidate::from_candidate(candidate, index)
                .with_capability_priority(requested_capability_priority_for_candidate_descriptors(
                    required_capabilities.iter().copied(),
                    candidate,
                ))
                .with_affinity_hash(
                    affinity_key
                        .as_deref()
                        .map(|key| crate::candidate_affinity_hash(key, candidate)),
                )
        })
        .collect::<Vec<_>>();
    crate::apply_scheduler_candidate_ranking(
        &mut candidates,
        &rankables,
        crate::SchedulerRankingContext {
            priority_mode,
            ranking_mode: crate::SchedulerRankingMode::CacheAffinity,
            include_health: false,
            load_balance_seed: 0,
        },
    );
    Ok(candidates)
}
