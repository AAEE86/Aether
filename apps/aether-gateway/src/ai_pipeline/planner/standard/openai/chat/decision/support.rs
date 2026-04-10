use aether_scheduler_core::SchedulerMinimalCandidateSelectionCandidate;
use serde_json::json;
use uuid::Uuid;

use crate::ai_pipeline::contracts::ExecutionRuntimeAuthContext;
use crate::ai_pipeline::conversion::{
    request_conversion_kind, request_conversion_requires_enable_flag,
    request_pair_allowed_for_transport,
};
use crate::ai_pipeline::planner::candidate_affinity::{
    rank_local_execution_candidates, remember_scheduler_affinity_for_candidate,
};
use crate::ai_pipeline::GatewayAuthApiKeySnapshot;
use crate::ai_pipeline::{ConversionMode, ExecutionStrategy, PlannerAppState};
use crate::clock::{current_unix_ms, current_unix_secs};
use crate::{append_execution_contract_fields_to_value, AppState};

#[derive(Debug, Clone)]
pub(crate) struct LocalOpenAiChatDecisionInput {
    pub(crate) auth_context: ExecutionRuntimeAuthContext,
    pub(crate) requested_model: String,
    pub(crate) auth_snapshot: GatewayAuthApiKeySnapshot,
    pub(crate) required_capabilities: Option<serde_json::Value>,
}

#[derive(Debug, Clone)]
pub(crate) struct LocalOpenAiChatCandidateAttempt {
    pub(crate) candidate: SchedulerMinimalCandidateSelectionCandidate,
    pub(crate) candidate_index: u32,
    pub(crate) candidate_id: String,
}

pub(crate) async fn mark_skipped_local_openai_chat_candidate(
    state: &AppState,
    input: &LocalOpenAiChatDecisionInput,
    trace_id: &str,
    candidate: &SchedulerMinimalCandidateSelectionCandidate,
    candidate_index: u32,
    candidate_id: &str,
    skip_reason: &'static str,
) {
    let planner_state = PlannerAppState::new(state);
    state.mutate_local_execution_runtime_miss_diagnostic(trace_id, |diagnostic| {
        *diagnostic
            .skip_reasons
            .entry(skip_reason.to_string())
            .or_insert(0) += 1;
        *diagnostic.skipped_candidate_count.get_or_insert(0) += 1;
    });
    planner_state
        .persist_skipped_local_candidate(
            trace_id,
            &input.auth_context.user_id,
            &input.auth_context.api_key_id,
            candidate,
            candidate_index,
            candidate_id,
            input.required_capabilities.as_ref(),
            skip_reason,
            current_unix_ms(),
            "gateway local openai chat decision failed to persist skipped candidate",
        )
        .await;
}

pub(crate) async fn materialize_local_openai_chat_candidate_attempts(
    state: &AppState,
    trace_id: &str,
    input: &LocalOpenAiChatDecisionInput,
    candidates: Vec<SchedulerMinimalCandidateSelectionCandidate>,
) -> Vec<LocalOpenAiChatCandidateAttempt> {
    let planner_state = PlannerAppState::new(state);
    let candidates = rank_local_execution_candidates(
        planner_state,
        candidates,
        "openai:chat",
        input.required_capabilities.as_ref(),
    )
    .await;
    let created_at_unix_ms = current_unix_ms();
    let mut attempts = Vec::with_capacity(candidates.len());
    let mut affinity_remembered = false;

    for (candidate_index, candidate) in candidates.into_iter().enumerate() {
        let generated_candidate_id = Uuid::new_v4().to_string();
        let provider_api_format = candidate.endpoint_api_format.trim().to_ascii_lowercase();
        if provider_api_format != "openai:chat" {
            if let Ok(Some(transport)) = planner_state
                .read_provider_transport_snapshot(
                    &candidate.provider_id,
                    &candidate.endpoint_id,
                    &candidate.key_id,
                )
                .await
            {
                if !request_pair_allowed_for_transport(
                    &transport,
                    "openai:chat",
                    provider_api_format.as_str(),
                ) {
                    let skip_reason =
                        if request_conversion_kind("openai:chat", provider_api_format.as_str())
                            .is_some()
                            && request_conversion_requires_enable_flag(
                                "openai:chat",
                                provider_api_format.as_str(),
                            )
                            && !transport.provider.enable_format_conversion
                        {
                            "format_conversion_disabled"
                        } else {
                            "transport_unsupported"
                        };
                    mark_skipped_local_openai_chat_candidate(
                        state,
                        input,
                        trace_id,
                        &candidate,
                        candidate_index as u32,
                        &generated_candidate_id,
                        skip_reason,
                    )
                    .await;
                    continue;
                }
            }
        }
        if !affinity_remembered {
            remember_scheduler_affinity_for_candidate(
                planner_state,
                Some(&input.auth_snapshot),
                "openai:chat",
                &input.requested_model,
                &candidate,
            );
            affinity_remembered = true;
        }
        let (execution_strategy, conversion_mode) = if provider_api_format == "openai:chat" {
            (ExecutionStrategy::LocalSameFormat, ConversionMode::None)
        } else {
            (
                ExecutionStrategy::LocalCrossFormat,
                ConversionMode::Bidirectional,
            )
        };
        let extra_data = append_execution_contract_fields_to_value(
            json!({
                "provider_api_format": provider_api_format,
                "client_api_format": "openai:chat",
                "global_model_id": candidate.global_model_id.clone(),
                "global_model_name": candidate.global_model_name.clone(),
                "model_id": candidate.model_id.clone(),
                "selected_provider_model_name": candidate.selected_provider_model_name.clone(),
                "mapping_matched_model": candidate.mapping_matched_model.clone(),
                "provider_name": candidate.provider_name.clone(),
                "key_name": candidate.key_name.clone(),
            }),
            execution_strategy,
            conversion_mode,
            "openai:chat",
            candidate.endpoint_api_format.trim(),
        );

        let candidate_id = planner_state
            .persist_available_local_candidate(
                trace_id,
                &input.auth_context.user_id,
                &input.auth_context.api_key_id,
                &candidate,
                candidate_index as u32,
                &generated_candidate_id,
                input.required_capabilities.as_ref(),
                Some(extra_data),
                created_at_unix_ms,
                "gateway local openai chat decision request candidate upsert failed",
            )
            .await;

        attempts.push(LocalOpenAiChatCandidateAttempt {
            candidate,
            candidate_index: candidate_index as u32,
            candidate_id,
        });
    }

    attempts
}
