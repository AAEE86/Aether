use super::types::{SchedulerMinimalCandidateSelectionCandidate, SchedulerPriorityMode};

pub fn compare_candidates_by_priority_mode(
    left: &SchedulerMinimalCandidateSelectionCandidate,
    right: &SchedulerMinimalCandidateSelectionCandidate,
    priority_mode: SchedulerPriorityMode,
    affinity_key: Option<&str>,
) -> std::cmp::Ordering {
    match priority_mode {
        SchedulerPriorityMode::Provider => left
            .provider_priority
            .cmp(&right.provider_priority)
            .then(left.key_internal_priority.cmp(&right.key_internal_priority))
            .then_with(|| crate::compare_affinity_order(left, right, affinity_key))
            .then_with(|| compare_candidate_identity(left, right)),
        SchedulerPriorityMode::GlobalKey => left
            .key_global_priority_for_format
            .unwrap_or(i32::MAX)
            .cmp(&right.key_global_priority_for_format.unwrap_or(i32::MAX))
            .then_with(|| crate::compare_affinity_order(left, right, affinity_key))
            .then(left.provider_priority.cmp(&right.provider_priority))
            .then(left.key_internal_priority.cmp(&right.key_internal_priority))
            .then_with(|| compare_candidate_identity(left, right)),
    }
}

pub(crate) fn compare_candidate_identity(
    left: &SchedulerMinimalCandidateSelectionCandidate,
    right: &SchedulerMinimalCandidateSelectionCandidate,
) -> std::cmp::Ordering {
    left.provider_id
        .cmp(&right.provider_id)
        .then(left.endpoint_id.cmp(&right.endpoint_id))
        .then(left.key_id.cmp(&right.key_id))
        .then(
            left.selected_provider_model_name
                .cmp(&right.selected_provider_model_name),
        )
}
