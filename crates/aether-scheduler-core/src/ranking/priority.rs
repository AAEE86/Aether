use std::cmp::Ordering;

use crate::SchedulerPriorityMode;

use super::types::SchedulerRankableCandidate;

pub fn candidate_priority_slot(
    candidate: &SchedulerRankableCandidate,
    priority_mode: SchedulerPriorityMode,
) -> i32 {
    match priority_mode {
        SchedulerPriorityMode::Provider => candidate.provider_priority,
        SchedulerPriorityMode::GlobalKey => {
            candidate.key_global_priority_for_format.unwrap_or(i32::MAX)
        }
    }
}

pub fn compare_candidate_priority_slot(
    left: &SchedulerRankableCandidate,
    right: &SchedulerRankableCandidate,
    priority_mode: SchedulerPriorityMode,
) -> Ordering {
    match priority_mode {
        SchedulerPriorityMode::Provider => left
            .provider_priority
            .cmp(&right.provider_priority)
            .then(left.key_internal_priority.cmp(&right.key_internal_priority)),
        SchedulerPriorityMode::GlobalKey => left
            .key_global_priority_for_format
            .unwrap_or(i32::MAX)
            .cmp(&right.key_global_priority_for_format.unwrap_or(i32::MAX))
            .then(left.provider_priority.cmp(&right.provider_priority))
            .then(left.key_internal_priority.cmp(&right.key_internal_priority)),
    }
}

pub fn candidates_share_priority_group(
    left: &SchedulerRankableCandidate,
    right: &SchedulerRankableCandidate,
    priority_mode: SchedulerPriorityMode,
) -> bool {
    match priority_mode {
        SchedulerPriorityMode::Provider => {
            left.provider_priority == right.provider_priority
                && left.key_internal_priority == right.key_internal_priority
        }
        SchedulerPriorityMode::GlobalKey => {
            left.key_global_priority_for_format == right.key_global_priority_for_format
        }
    }
}
