use crate::ai_pipeline::contracts::ExecutionRuntimeAuthContext;
use crate::ai_pipeline::{GatewayAuthApiKeySnapshot, PlannerAppState};
use crate::clock::current_unix_secs;
use crate::{AppState, GatewayError};

#[derive(Debug, Clone)]
pub(crate) struct ResolvedLocalDecisionAuthInput {
    pub(crate) auth_context: ExecutionRuntimeAuthContext,
    pub(crate) auth_snapshot: GatewayAuthApiKeySnapshot,
    pub(crate) required_capabilities: Option<serde_json::Value>,
}

#[derive(Debug, Clone)]
pub(crate) struct LocalRequestedModelDecisionInput {
    pub(crate) auth_context: ExecutionRuntimeAuthContext,
    pub(crate) requested_model: String,
    pub(crate) auth_snapshot: GatewayAuthApiKeySnapshot,
    pub(crate) required_capabilities: Option<serde_json::Value>,
}

#[derive(Debug, Clone)]
pub(crate) struct LocalAuthenticatedDecisionInput {
    pub(crate) auth_context: ExecutionRuntimeAuthContext,
    pub(crate) auth_snapshot: GatewayAuthApiKeySnapshot,
    pub(crate) required_capabilities: Option<serde_json::Value>,
}

pub(crate) fn build_local_requested_model_decision_input(
    resolved_input: ResolvedLocalDecisionAuthInput,
    requested_model: String,
) -> LocalRequestedModelDecisionInput {
    LocalRequestedModelDecisionInput {
        auth_context: resolved_input.auth_context,
        requested_model,
        auth_snapshot: resolved_input.auth_snapshot,
        required_capabilities: resolved_input.required_capabilities,
    }
}

pub(crate) fn build_local_authenticated_decision_input(
    resolved_input: ResolvedLocalDecisionAuthInput,
) -> LocalAuthenticatedDecisionInput {
    LocalAuthenticatedDecisionInput {
        auth_context: resolved_input.auth_context,
        auth_snapshot: resolved_input.auth_snapshot,
        required_capabilities: resolved_input.required_capabilities,
    }
}

pub(crate) async fn resolve_local_authenticated_decision_input(
    state: &AppState,
    auth_context: ExecutionRuntimeAuthContext,
    requested_model: Option<&str>,
    explicit_required_capabilities: Option<&serde_json::Value>,
) -> Result<Option<ResolvedLocalDecisionAuthInput>, GatewayError> {
    let planner_state = PlannerAppState::new(state);
    let auth_snapshot = match planner_state
        .read_auth_api_key_snapshot(
            &auth_context.user_id,
            &auth_context.api_key_id,
            current_unix_secs(),
        )
        .await?
    {
        Some(snapshot) => snapshot,
        None => return Ok(None),
    };

    let required_capabilities = planner_state
        .resolve_request_candidate_required_capabilities(
            &auth_context.user_id,
            &auth_context.api_key_id,
            requested_model,
            explicit_required_capabilities,
        )
        .await;

    Ok(Some(ResolvedLocalDecisionAuthInput {
        auth_context,
        auth_snapshot,
        required_capabilities,
    }))
}
