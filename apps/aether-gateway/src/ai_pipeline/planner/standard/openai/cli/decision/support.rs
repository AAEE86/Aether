use std::collections::BTreeSet;

use aether_scheduler_core::SchedulerMinimalCandidateSelectionCandidate;
use tracing::warn;

use crate::ai_pipeline::contracts::ExecutionRuntimeAuthContext;
use crate::ai_pipeline::conversion::{request_candidate_api_formats, request_conversion_kind};
use crate::ai_pipeline::planner::candidate_eligibility::filter_and_rank_local_execution_candidates;
use crate::ai_pipeline::planner::candidate_materialization::{
    mark_skipped_local_execution_candidate,
    persist_available_local_execution_candidates_with_context,
    persist_skipped_local_execution_candidates_with_context,
    remember_first_local_candidate_affinity,
};
use crate::ai_pipeline::planner::candidate_metadata::{
    build_local_execution_candidate_contract_metadata, LocalExecutionCandidateMetadataParts,
};
use crate::ai_pipeline::planner::candidate_source::auth_snapshot_allows_cross_format_candidate;
use crate::ai_pipeline::planner::common::extract_standard_requested_model;
use crate::ai_pipeline::planner::decision_input::{
    build_local_requested_model_decision_input, resolve_local_authenticated_decision_input,
};
use crate::ai_pipeline::planner::materialization_policy::{
    build_local_candidate_persistence_policy, LocalCandidatePersistencePolicyKind,
};
use crate::ai_pipeline::planner::spec_metadata::local_openai_cli_spec_metadata;
use crate::ai_pipeline::PlannerAppState;
use crate::ai_pipeline::{
    resolve_local_decision_execution_runtime_auth_context, ConversionMode, ExecutionStrategy,
    GatewayControlDecision,
};
use crate::clock::current_unix_secs;
use crate::{AppState, GatewayError};

use super::LocalOpenAiCliSpec;

pub(crate) use crate::ai_pipeline::planner::candidate_materialization::LocalExecutionCandidateAttempt as LocalOpenAiCliCandidateAttempt;
pub(crate) use crate::ai_pipeline::planner::decision_input::LocalRequestedModelDecisionInput as LocalOpenAiCliDecisionInput;

pub(crate) async fn resolve_local_openai_cli_decision_input(
    state: &AppState,
    trace_id: &str,
    decision: &GatewayControlDecision,
    body_json: &serde_json::Value,
) -> Option<LocalOpenAiCliDecisionInput> {
    let auth_context: ExecutionRuntimeAuthContext =
        resolve_local_decision_execution_runtime_auth_context(decision)?;

    let requested_model = extract_standard_requested_model(body_json)?;

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
                error = ?err,
                "gateway local openai cli decision auth snapshot read failed"
            );
            return None;
        }
    };

    Some(build_local_requested_model_decision_input(
        resolved_input,
        requested_model,
    ))
}

pub(crate) async fn materialize_local_openai_cli_candidate_attempts(
    state: &AppState,
    trace_id: &str,
    input: &LocalOpenAiCliDecisionInput,
    spec: LocalOpenAiCliSpec,
) -> Result<Vec<LocalOpenAiCliCandidateAttempt>, GatewayError> {
    let spec_metadata = local_openai_cli_spec_metadata(spec);
    let client_api_format = spec_metadata.api_format.trim().to_ascii_lowercase();
    let planner_state = PlannerAppState::new(state);
    let auth_context: &ExecutionRuntimeAuthContext = &input.auth_context;
    let persistence_policy = build_local_candidate_persistence_policy(
        auth_context,
        input.required_capabilities.as_ref(),
        LocalCandidatePersistencePolicyKind::OpenAiCliDecision,
    );
    let mut seen_candidates = BTreeSet::new();
    let mut candidates = Vec::new();
    for candidate_api_format in
        request_candidate_api_formats(spec_metadata.api_format, spec_metadata.require_streaming)
    {
        let auth_snapshot = if candidate_api_format == spec_metadata.api_format {
            Some(&input.auth_snapshot)
        } else {
            None
        };
        let mut selected_candidates = planner_state
            .list_selectable_candidates(
                candidate_api_format,
                &input.requested_model,
                spec_metadata.require_streaming,
                input.required_capabilities.as_ref(),
                auth_snapshot,
                current_unix_secs(),
            )
            .await?;
        if auth_snapshot.is_none() {
            selected_candidates.retain(|candidate| {
                auth_snapshot_allows_cross_format_candidate(
                    &input.auth_snapshot,
                    &input.requested_model,
                    candidate,
                )
            });
        }
        for candidate in selected_candidates {
            let candidate_key = format!(
                "{}:{}:{}:{}:{}:{}",
                candidate.provider_id,
                candidate.endpoint_id,
                candidate.key_id,
                candidate.model_id,
                candidate.selected_provider_model_name,
                candidate.endpoint_api_format,
            );
            if seen_candidates.insert(candidate_key) {
                candidates.push(candidate);
            }
        }
    }
    let (candidates, skipped_candidates) = filter_and_rank_local_execution_candidates(
        planner_state,
        candidates,
        spec_metadata.api_format,
        &input.requested_model,
        input.required_capabilities.as_ref(),
    )
    .await;

    remember_first_local_candidate_affinity(
        planner_state,
        Some(&input.auth_snapshot),
        spec_metadata.api_format,
        Some(&input.requested_model),
        &candidates,
    );
    let attempts = persist_available_local_execution_candidates_with_context(
        planner_state,
        trace_id,
        persistence_policy.available,
        candidates,
        |eligible| {
            let provider_api_format = eligible.provider_api_format.clone();
            let execution_strategy = if provider_api_format == client_api_format {
                ExecutionStrategy::LocalSameFormat
            } else {
                ExecutionStrategy::LocalCrossFormat
            };
            let conversion_mode =
                if request_conversion_kind(spec_metadata.api_format, provider_api_format.as_str())
                    .is_some()
                {
                    ConversionMode::Bidirectional
                } else {
                    ConversionMode::None
                };
            Some(build_local_execution_candidate_contract_metadata(
                LocalExecutionCandidateMetadataParts {
                    eligible,
                    provider_api_format: provider_api_format.as_str(),
                    client_api_format: spec_metadata.api_format,
                    extra_fields: serde_json::Map::new(),
                },
                execution_strategy,
                conversion_mode,
                eligible.candidate.endpoint_api_format.as_str(),
            ))
        },
    )
    .await;

    persist_skipped_local_execution_candidates_with_context(
        state,
        trace_id,
        persistence_policy.skipped,
        attempts.len() as u32,
        skipped_candidates,
    )
    .await;

    Ok(attempts)
}
pub(crate) async fn mark_skipped_local_openai_cli_candidate(
    state: &AppState,
    input: &LocalOpenAiCliDecisionInput,
    trace_id: &str,
    candidate: &SchedulerMinimalCandidateSelectionCandidate,
    candidate_index: u32,
    candidate_id: &str,
    skip_reason: &'static str,
) {
    let auth_context: &ExecutionRuntimeAuthContext = &input.auth_context;
    let persistence_policy = build_local_candidate_persistence_policy(
        auth_context,
        input.required_capabilities.as_ref(),
        LocalCandidatePersistencePolicyKind::OpenAiCliDecision,
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
