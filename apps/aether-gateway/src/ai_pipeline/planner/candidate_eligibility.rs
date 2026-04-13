use tracing::warn;

use aether_scheduler_core::SchedulerMinimalCandidateSelectionCandidate;

use crate::ai_pipeline::{GatewayProviderTransportSnapshot, PlannerAppState};

use super::candidate_affinity::rank_eligible_local_execution_candidates;

#[derive(Debug, Clone, PartialEq)]
pub(crate) struct EligibleLocalExecutionCandidate {
    pub(crate) candidate: SchedulerMinimalCandidateSelectionCandidate,
    pub(crate) transport: GatewayProviderTransportSnapshot,
    pub(crate) provider_api_format: String,
}

#[derive(Debug, Clone, PartialEq)]
pub(crate) struct SkippedLocalExecutionCandidate {
    pub(crate) candidate: SchedulerMinimalCandidateSelectionCandidate,
    pub(crate) skip_reason: &'static str,
}

pub(crate) async fn filter_and_rank_local_execution_candidates(
    state: PlannerAppState<'_>,
    candidates: Vec<SchedulerMinimalCandidateSelectionCandidate>,
    client_api_format: &str,
    requested_model: &str,
    required_capabilities: Option<&serde_json::Value>,
) -> (
    Vec<EligibleLocalExecutionCandidate>,
    Vec<SkippedLocalExecutionCandidate>,
) {
    filter_and_rank_local_execution_candidates_with_gate(
        state,
        candidates,
        client_api_format,
        required_capabilities,
        |candidate, transport| {
            current_local_execution_candidate_skip_reason_with_transport(
                candidate,
                transport,
                client_api_format,
                requested_model,
            )
        },
    )
    .await
}

pub(crate) async fn filter_and_rank_local_execution_candidates_without_transport_pair_gate(
    state: PlannerAppState<'_>,
    candidates: Vec<SchedulerMinimalCandidateSelectionCandidate>,
    client_api_format: &str,
    requested_model: Option<&str>,
    required_capabilities: Option<&serde_json::Value>,
) -> (
    Vec<EligibleLocalExecutionCandidate>,
    Vec<SkippedLocalExecutionCandidate>,
) {
    filter_and_rank_local_execution_candidates_with_gate(
        state,
        candidates,
        client_api_format,
        required_capabilities,
        |candidate, transport| {
            current_local_execution_candidate_common_skip_reason_with_transport(
                candidate,
                transport,
                requested_model,
            )
        },
    )
    .await
}

async fn filter_and_rank_local_execution_candidates_with_gate<F>(
    state: PlannerAppState<'_>,
    candidates: Vec<SchedulerMinimalCandidateSelectionCandidate>,
    client_api_format: &str,
    required_capabilities: Option<&serde_json::Value>,
    runtime_skip_reason: F,
) -> (
    Vec<EligibleLocalExecutionCandidate>,
    Vec<SkippedLocalExecutionCandidate>,
)
where
    F: Fn(
        &SchedulerMinimalCandidateSelectionCandidate,
        &GatewayProviderTransportSnapshot,
    ) -> Option<&'static str>,
{
    let mut selectable = Vec::with_capacity(candidates.len());
    let mut skipped = Vec::new();

    for candidate in candidates {
        let Some(transport) = read_candidate_transport_snapshot(state, &candidate).await else {
            skipped.push(SkippedLocalExecutionCandidate {
                candidate,
                skip_reason: "transport_snapshot_missing",
            });
            continue;
        };
        match runtime_skip_reason(&candidate, &transport) {
            Some(skip_reason) => skipped.push(SkippedLocalExecutionCandidate {
                candidate,
                skip_reason,
            }),
            None => selectable.push(EligibleLocalExecutionCandidate {
                provider_api_format: transport.endpoint.api_format.trim().to_ascii_lowercase(),
                candidate,
                transport,
            }),
        }
    }

    let ranked = rank_eligible_local_execution_candidates(
        state,
        selectable,
        client_api_format,
        required_capabilities,
    )
    .await;

    (ranked, skipped)
}

fn current_local_execution_candidate_common_skip_reason_with_transport(
    candidate: &SchedulerMinimalCandidateSelectionCandidate,
    transport: &GatewayProviderTransportSnapshot,
    requested_model: Option<&str>,
) -> Option<&'static str> {
    let requested_model = requested_model.unwrap_or_default();

    if !transport.provider.is_active {
        return Some("provider_inactive");
    }
    if !transport.endpoint.is_active {
        return Some("endpoint_inactive");
    }
    if !transport.key.is_active {
        return Some("key_inactive");
    }

    let candidate_api_format = candidate.endpoint_api_format.trim().to_ascii_lowercase();
    let endpoint_api_format = transport.endpoint.api_format.trim().to_ascii_lowercase();
    if endpoint_api_format != candidate_api_format {
        return Some("endpoint_api_format_changed");
    }

    if !transport_key_supports_api_format(transport, endpoint_api_format.as_str()) {
        return Some("key_api_format_disabled");
    }
    if !transport_key_allows_candidate_model(transport, requested_model, candidate) {
        return Some("key_model_disabled");
    }

    None
}

fn current_local_execution_candidate_skip_reason_with_transport(
    candidate: &SchedulerMinimalCandidateSelectionCandidate,
    transport: &GatewayProviderTransportSnapshot,
    client_api_format: &str,
    requested_model: &str,
) -> Option<&'static str> {
    if let Some(skip_reason) = current_local_execution_candidate_common_skip_reason_with_transport(
        candidate,
        transport,
        Some(requested_model),
    ) {
        return Some(skip_reason);
    }

    let endpoint_api_format = transport.endpoint.api_format.trim().to_ascii_lowercase();
    let client_api_format = client_api_format.trim().to_ascii_lowercase();
    if client_api_format == endpoint_api_format {
        return None;
    }

    if !crate::ai_pipeline::conversion::request_pair_allowed_for_transport(
        transport,
        client_api_format.as_str(),
        endpoint_api_format.as_str(),
    ) {
        let skip_reason = if crate::ai_pipeline::conversion::request_conversion_kind(
            client_api_format.as_str(),
            endpoint_api_format.as_str(),
        )
        .is_some()
            && crate::ai_pipeline::conversion::request_conversion_requires_enable_flag(
                client_api_format.as_str(),
                endpoint_api_format.as_str(),
            )
            && !transport.provider.enable_format_conversion
        {
            "format_conversion_disabled"
        } else {
            "transport_unsupported"
        };
        return Some(skip_reason);
    }

    None
}

fn transport_key_supports_api_format(
    transport: &GatewayProviderTransportSnapshot,
    endpoint_api_format: &str,
) -> bool {
    match transport.key.api_formats.as_deref() {
        None => true,
        Some(formats) => formats
            .iter()
            .any(|value| value.trim().eq_ignore_ascii_case(endpoint_api_format)),
    }
}

fn transport_key_allows_candidate_model(
    transport: &GatewayProviderTransportSnapshot,
    requested_model: &str,
    candidate: &SchedulerMinimalCandidateSelectionCandidate,
) -> bool {
    let Some(allowed_models) = transport.key.allowed_models.as_deref() else {
        return true;
    };

    let allowed_models = allowed_models
        .iter()
        .map(|value| value.trim())
        .filter(|value| !value.is_empty())
        .collect::<Vec<_>>();
    if allowed_models.is_empty() {
        return false;
    }

    let requested_model = requested_model.trim();
    let global_model_name = candidate.global_model_name.trim();
    let selected_provider_model_name = candidate.selected_provider_model_name.trim();
    let mapping_matched_model = candidate
        .mapping_matched_model
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty());

    allowed_models.iter().any(|allowed_model| {
        *allowed_model == requested_model
            || *allowed_model == global_model_name
            || *allowed_model == selected_provider_model_name
            || mapping_matched_model.is_some_and(|value| value == *allowed_model)
    })
}

pub(crate) async fn read_candidate_transport_snapshot(
    state: PlannerAppState<'_>,
    candidate: &SchedulerMinimalCandidateSelectionCandidate,
) -> Option<GatewayProviderTransportSnapshot> {
    match state
        .read_provider_transport_snapshot(
            &candidate.provider_id,
            &candidate.endpoint_id,
            &candidate.key_id,
        )
        .await
    {
        Ok(Some(transport)) => Some(transport),
        Ok(None) => None,
        Err(error) => {
            warn!(
                event_name = "candidate_eligibility_transport_load_failed",
                log_type = "event",
                provider_id = %candidate.provider_id,
                endpoint_id = %candidate.endpoint_id,
                key_id = %candidate.key_id,
                error = ?error,
                "failed to load provider transport while evaluating local candidate eligibility"
            );
            None
        }
    }
}
