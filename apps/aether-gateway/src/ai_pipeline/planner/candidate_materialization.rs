use aether_scheduler_core::SchedulerMinimalCandidateSelectionCandidate;
use serde_json::Value;
use uuid::Uuid;

use crate::ai_pipeline::planner::candidate_affinity::remember_scheduler_affinity_for_candidate;
use crate::ai_pipeline::planner::candidate_eligibility::{
    EligibleLocalExecutionCandidate, SkippedLocalExecutionCandidate,
};
use crate::ai_pipeline::planner::runtime_miss::record_local_runtime_candidate_skip_reason;
use crate::ai_pipeline::{GatewayAuthApiKeySnapshot, PlannerAppState};
use crate::clock::current_unix_ms;
use crate::AppState;

#[derive(Debug, Clone)]
pub(crate) struct LocalExecutionCandidateAttempt {
    pub(crate) eligible: EligibleLocalExecutionCandidate,
    pub(crate) candidate_index: u32,
    pub(crate) candidate_id: String,
}

#[derive(Debug, Clone, Copy)]
pub(crate) struct LocalAvailableCandidatePersistenceContext<'a> {
    pub(crate) user_id: &'a str,
    pub(crate) api_key_id: &'a str,
    pub(crate) required_capabilities: Option<&'a Value>,
    pub(crate) error_context: &'static str,
}

#[derive(Debug, Clone, Copy)]
pub(crate) struct LocalSkippedCandidatePersistenceContext<'a> {
    pub(crate) user_id: &'a str,
    pub(crate) api_key_id: &'a str,
    pub(crate) required_capabilities: Option<&'a Value>,
    pub(crate) error_context: &'static str,
    pub(crate) record_runtime_miss_diagnostic: bool,
}

pub(crate) fn remember_first_local_candidate_affinity(
    state: PlannerAppState<'_>,
    auth_snapshot: Option<&GatewayAuthApiKeySnapshot>,
    client_api_format: &str,
    requested_model: Option<&str>,
    candidates: &[EligibleLocalExecutionCandidate],
) {
    let Some(first_candidate) = candidates.first() else {
        return;
    };
    let affinity_requested_model = requested_model
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .unwrap_or(first_candidate.candidate.global_model_name.as_str());
    remember_scheduler_affinity_for_candidate(
        state,
        auth_snapshot,
        client_api_format,
        affinity_requested_model,
        &first_candidate.candidate,
    );
}

#[allow(clippy::too_many_arguments)]
pub(crate) async fn persist_available_local_execution_candidates<F>(
    state: PlannerAppState<'_>,
    trace_id: &str,
    user_id: &str,
    api_key_id: &str,
    required_capabilities: Option<&Value>,
    candidates: Vec<EligibleLocalExecutionCandidate>,
    error_context: &'static str,
    build_extra_data: F,
) -> Vec<LocalExecutionCandidateAttempt>
where
    F: Fn(&EligibleLocalExecutionCandidate) -> Option<Value>,
{
    let created_at_unix_ms = current_unix_ms();
    let mut materialized = Vec::with_capacity(candidates.len());

    for (candidate_index, eligible) in candidates.into_iter().enumerate() {
        let generated_candidate_id = Uuid::new_v4().to_string();
        let candidate_id = state
            .persist_available_local_candidate(
                trace_id,
                user_id,
                api_key_id,
                &eligible.candidate,
                candidate_index as u32,
                &generated_candidate_id,
                required_capabilities,
                build_extra_data(&eligible),
                created_at_unix_ms,
                error_context,
            )
            .await;

        materialized.push(LocalExecutionCandidateAttempt {
            eligible,
            candidate_index: candidate_index as u32,
            candidate_id,
        });
    }

    materialized
}

pub(crate) async fn persist_available_local_execution_candidates_with_context<F>(
    state: PlannerAppState<'_>,
    trace_id: &str,
    context: LocalAvailableCandidatePersistenceContext<'_>,
    candidates: Vec<EligibleLocalExecutionCandidate>,
    build_extra_data: F,
) -> Vec<LocalExecutionCandidateAttempt>
where
    F: Fn(&EligibleLocalExecutionCandidate) -> Option<Value>,
{
    persist_available_local_execution_candidates(
        state,
        trace_id,
        context.user_id,
        context.api_key_id,
        context.required_capabilities,
        candidates,
        context.error_context,
        build_extra_data,
    )
    .await
}

#[allow(clippy::too_many_arguments)]
pub(crate) async fn persist_skipped_local_execution_candidate(
    state: &AppState,
    trace_id: &str,
    user_id: &str,
    api_key_id: &str,
    candidate: &SchedulerMinimalCandidateSelectionCandidate,
    candidate_index: u32,
    candidate_id: &str,
    required_capabilities: Option<&Value>,
    skip_reason: &'static str,
    error_context: &'static str,
    record_runtime_miss_diagnostic: bool,
) {
    if record_runtime_miss_diagnostic {
        record_local_runtime_candidate_skip_reason(state, trace_id, skip_reason);
    }

    PlannerAppState::new(state)
        .persist_skipped_local_candidate(
            trace_id,
            user_id,
            api_key_id,
            candidate,
            candidate_index,
            candidate_id,
            required_capabilities,
            skip_reason,
            current_unix_ms(),
            error_context,
        )
        .await;
}

pub(crate) async fn mark_skipped_local_execution_candidate(
    state: &AppState,
    trace_id: &str,
    context: LocalSkippedCandidatePersistenceContext<'_>,
    candidate: &SchedulerMinimalCandidateSelectionCandidate,
    candidate_index: u32,
    candidate_id: &str,
    skip_reason: &'static str,
) {
    persist_skipped_local_execution_candidate(
        state,
        trace_id,
        context.user_id,
        context.api_key_id,
        candidate,
        candidate_index,
        candidate_id,
        context.required_capabilities,
        skip_reason,
        context.error_context,
        context.record_runtime_miss_diagnostic,
    )
    .await;
}

#[allow(clippy::too_many_arguments)]
pub(crate) async fn persist_skipped_local_execution_candidates(
    state: &AppState,
    trace_id: &str,
    user_id: &str,
    api_key_id: &str,
    required_capabilities: Option<&Value>,
    starting_candidate_index: u32,
    skipped_candidates: Vec<SkippedLocalExecutionCandidate>,
    error_context: &'static str,
    record_runtime_miss_diagnostic: bool,
) {
    for (skipped_offset, skipped_candidate) in skipped_candidates.into_iter().enumerate() {
        let generated_candidate_id = Uuid::new_v4().to_string();
        persist_skipped_local_execution_candidate(
            state,
            trace_id,
            user_id,
            api_key_id,
            &skipped_candidate.candidate,
            starting_candidate_index + skipped_offset as u32,
            &generated_candidate_id,
            required_capabilities,
            skipped_candidate.skip_reason,
            error_context,
            record_runtime_miss_diagnostic,
        )
        .await;
    }
}

pub(crate) async fn persist_skipped_local_execution_candidates_with_context(
    state: &AppState,
    trace_id: &str,
    context: LocalSkippedCandidatePersistenceContext<'_>,
    starting_candidate_index: u32,
    skipped_candidates: Vec<SkippedLocalExecutionCandidate>,
) {
    persist_skipped_local_execution_candidates(
        state,
        trace_id,
        context.user_id,
        context.api_key_id,
        context.required_capabilities,
        starting_candidate_index,
        skipped_candidates,
        context.error_context,
        context.record_runtime_miss_diagnostic,
    )
    .await;
}
