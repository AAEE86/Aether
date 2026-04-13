use aether_scheduler_core::SchedulerMinimalCandidateSelectionCandidate;
use tracing::warn;

use crate::ai_pipeline::{
    GatewayProviderTransportSnapshot, LocalResolvedOAuthRequestAuth, PlannerAppState,
};

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct PreparedHeaderAuthenticatedCandidate {
    pub(crate) auth_header: String,
    pub(crate) auth_value: String,
    pub(crate) mapped_model: String,
}

#[derive(Debug, Clone, Copy)]
pub(crate) struct OauthPreparationContext<'a> {
    pub(crate) trace_id: &'a str,
    pub(crate) api_format: &'a str,
    pub(crate) operation: &'a str,
}

pub(crate) async fn prepare_header_authenticated_candidate(
    state: PlannerAppState<'_>,
    transport: &GatewayProviderTransportSnapshot,
    candidate: &SchedulerMinimalCandidateSelectionCandidate,
    direct_auth: Option<(String, String)>,
    context: OauthPreparationContext<'_>,
) -> Result<PreparedHeaderAuthenticatedCandidate, &'static str> {
    let oauth_auth = if direct_auth.is_none() {
        match resolve_candidate_oauth_auth(state, transport, context).await {
            Some(LocalResolvedOAuthRequestAuth::Header { name, value }) => Some((name, value)),
            Some(LocalResolvedOAuthRequestAuth::Kiro(_)) => None,
            None => None,
        }
    } else {
        None
    };

    let Some((auth_header, auth_value)) = direct_auth.or(oauth_auth) else {
        return Err("transport_auth_unavailable");
    };
    let mapped_model = resolve_candidate_mapped_model(candidate)?;

    Ok(PreparedHeaderAuthenticatedCandidate {
        auth_header,
        auth_value,
        mapped_model,
    })
}

pub(crate) fn resolve_candidate_mapped_model(
    candidate: &SchedulerMinimalCandidateSelectionCandidate,
) -> Result<String, &'static str> {
    let mapped_model = candidate.selected_provider_model_name.trim().to_string();
    if mapped_model.is_empty() {
        return Err("mapped_model_missing");
    }

    Ok(mapped_model)
}

pub(crate) async fn resolve_candidate_oauth_auth(
    state: PlannerAppState<'_>,
    transport: &GatewayProviderTransportSnapshot,
    context: OauthPreparationContext<'_>,
) -> Option<LocalResolvedOAuthRequestAuth> {
    match state.resolve_local_oauth_request_auth(transport).await {
        Ok(Some(auth)) => Some(auth),
        Ok(None) => None,
        Err(err) => {
            warn!(
                event_name = "candidate_preparation_oauth_auth_resolution_failed",
                log_type = "event",
                trace_id = %context.trace_id,
                api_format = %context.api_format,
                operation = %context.operation,
                provider_type = %transport.provider.provider_type,
                error = ?err,
                "failed to resolve oauth auth while preparing local candidate"
            );
            None
        }
    }
}
