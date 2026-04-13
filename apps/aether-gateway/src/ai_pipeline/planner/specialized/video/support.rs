use aether_scheduler_core::SchedulerMinimalCandidateSelectionCandidate;
use tracing::warn;

use super::{LocalVideoCreateFamily, LocalVideoCreateSpec};
use crate::ai_pipeline::contracts::ExecutionRuntimeAuthContext;
use crate::ai_pipeline::planner::candidate_eligibility::filter_and_rank_local_execution_candidates;
use crate::ai_pipeline::planner::candidate_materialization::{
    mark_skipped_local_execution_candidate,
    persist_available_local_execution_candidates_with_context,
    persist_skipped_local_execution_candidates_with_context,
    remember_first_local_candidate_affinity,
};
use crate::ai_pipeline::planner::candidate_metadata::{
    build_local_execution_candidate_metadata, LocalExecutionCandidateMetadataParts,
};
use crate::ai_pipeline::planner::common::extract_requested_model_from_request;
use crate::ai_pipeline::planner::decision_input::{
    build_local_requested_model_decision_input, resolve_local_authenticated_decision_input,
};
use crate::ai_pipeline::planner::materialization_policy::{
    build_local_candidate_persistence_policy, LocalCandidatePersistencePolicyKind,
};
use crate::ai_pipeline::planner::spec_metadata::local_video_create_spec_metadata;
use crate::ai_pipeline::PlannerAppState;
use crate::ai_pipeline::{
    resolve_local_decision_execution_runtime_auth_context, GatewayControlDecision,
};
use crate::clock::current_unix_secs;
use crate::AppState;

pub(super) use crate::ai_pipeline::planner::candidate_materialization::LocalExecutionCandidateAttempt as LocalVideoCreateCandidateAttempt;
pub(super) use crate::ai_pipeline::planner::decision_input::LocalRequestedModelDecisionInput as LocalVideoCreateDecisionInput;

pub(super) async fn resolve_local_video_create_decision_input(
    state: &AppState,
    parts: &http::request::Parts,
    trace_id: &str,
    decision: &GatewayControlDecision,
    body_json: &serde_json::Value,
    spec: LocalVideoCreateSpec,
) -> Option<LocalVideoCreateDecisionInput> {
    let spec_metadata = local_video_create_spec_metadata(spec);
    let Some(auth_context) = resolve_local_video_create_auth_context(decision, spec.family) else {
        return None;
    };

    let requested_model = extract_requested_model_from_request(
        parts,
        body_json,
        spec_metadata
            .requested_model_family
            .expect("video specs should declare requested-model family"),
    )?;

    let resolved_input = match resolve_local_authenticated_decision_input(
        state,
        auth_context,
        Some(requested_model.as_str()),
        None,
    )
    .await
    {
        Ok(Some(resolved_input)) => resolved_input,
        Ok(None) => return None,
        Err(err) => {
            warn!(
                trace_id = %trace_id,
                decision_kind = spec_metadata.decision_kind,
                error = ?err,
                "gateway local video decision auth snapshot read failed"
            );
            return None;
        }
    };

    Some(build_local_requested_model_decision_input(
        resolved_input,
        requested_model,
    ))
}

fn resolve_local_video_create_auth_context(
    decision: &GatewayControlDecision,
    family: LocalVideoCreateFamily,
) -> Option<ExecutionRuntimeAuthContext> {
    let auth_context = resolve_local_decision_execution_runtime_auth_context(decision)?;
    match family {
        LocalVideoCreateFamily::OpenAi | LocalVideoCreateFamily::Gemini => Some(auth_context),
    }
}

pub(super) async fn list_local_video_create_candidate_attempts(
    state: &AppState,
    trace_id: &str,
    input: &LocalVideoCreateDecisionInput,
    api_format: &str,
    decision_kind: &str,
) -> Option<Vec<LocalVideoCreateCandidateAttempt>> {
    let planner_state = PlannerAppState::new(state);
    let candidates = match planner_state
        .list_selectable_candidates(
            api_format,
            &input.requested_model,
            false,
            input.required_capabilities.as_ref(),
            Some(&input.auth_snapshot),
            current_unix_secs(),
        )
        .await
    {
        Ok(candidates) => candidates,
        Err(err) => {
            warn!(
                trace_id = %trace_id,
                decision_kind = decision_kind,
                error = ?err,
                "gateway local video decision scheduler selection failed"
            );
            return None;
        }
    };

    Some(
        materialize_local_video_create_candidate_attempts(
            planner_state,
            trace_id,
            input,
            candidates,
            api_format,
        )
        .await,
    )
}

async fn materialize_local_video_create_candidate_attempts(
    state: PlannerAppState<'_>,
    trace_id: &str,
    input: &LocalVideoCreateDecisionInput,
    candidates: Vec<SchedulerMinimalCandidateSelectionCandidate>,
    api_format: &str,
) -> Vec<LocalVideoCreateCandidateAttempt> {
    let persistence_policy = build_local_candidate_persistence_policy(
        &input.auth_context,
        input.required_capabilities.as_ref(),
        LocalCandidatePersistencePolicyKind::VideoDecision,
    );
    let (candidates, skipped_candidates) = filter_and_rank_local_execution_candidates(
        state,
        candidates,
        api_format,
        &input.requested_model,
        input.required_capabilities.as_ref(),
    )
    .await;
    remember_first_local_candidate_affinity(
        state,
        Some(&input.auth_snapshot),
        api_format,
        Some(&input.requested_model),
        &candidates,
    );
    let attempts = persist_available_local_execution_candidates_with_context(
        state,
        trace_id,
        persistence_policy.available,
        candidates,
        |eligible| {
            Some(build_local_execution_candidate_metadata(
                LocalExecutionCandidateMetadataParts {
                    eligible,
                    provider_api_format: api_format,
                    client_api_format: api_format,
                    extra_fields: serde_json::Map::new(),
                },
            ))
        },
    )
    .await;

    persist_skipped_local_execution_candidates_with_context(
        state.app(),
        trace_id,
        persistence_policy.skipped,
        attempts.len() as u32,
        skipped_candidates,
    )
    .await;

    attempts
}

pub(super) async fn mark_skipped_local_video_candidate(
    state: &AppState,
    input: &LocalVideoCreateDecisionInput,
    trace_id: &str,
    candidate: &SchedulerMinimalCandidateSelectionCandidate,
    candidate_index: u32,
    candidate_id: &str,
    skip_reason: &'static str,
) {
    let persistence_policy = build_local_candidate_persistence_policy(
        &input.auth_context,
        input.required_capabilities.as_ref(),
        LocalCandidatePersistencePolicyKind::VideoDecision,
    );
    mark_skipped_local_execution_candidate(
        state,
        trace_id,
        persistence_policy.skipped,
        candidate,
        candidate_index,
        candidate_id,
        skip_reason,
    )
    .await;
}
