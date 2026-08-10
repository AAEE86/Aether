//! Shared admission helpers for local upstream execution.
//!
//! The stream candidate loop and long-lived WebSocket turns both need to
//! participate in the same gateway-wide upstream execution gate.  Keep the
//! provider abstraction here so tests can supply an isolated gate while
//! production callers use `AppState` directly.

use std::time::Duration;

use aether_ai_formats::api::{build_core_error_body_for_client_format, LocalCoreSyncErrorKind};
use aether_contracts::ExecutionPlan;
use aether_data_contracts::repository::candidates::RequestCandidateStatus;
use aether_runtime::{ConcurrencyGate, ConcurrencyPermit};
use aether_scheduler_core::SchedulerRequestCandidateStatusUpdate;
use axum::body::Body;
use axum::http::{Response, StatusCode};
use serde_json::json;
use tokio::time::timeout;

use crate::api::response::{attach_control_metadata_headers, build_client_response_from_parts};
use crate::clock::current_unix_ms;
use crate::control::GatewayControlDecision;
use crate::orchestration::{release_local_pool_key_lease, LocalExecutionEffectContext};
use crate::request_candidate_runtime::{
    record_local_request_candidate_status, ExecutionRequestReservationError,
};
use crate::stage_metrics::observe_gateway_stage_ms;
use crate::{AppState, GatewayError};

pub(crate) const UPSTREAM_EXECUTION_GATE_NAME: &str = "gateway_upstream_execution";
pub(crate) trait UpstreamExecutionGateProvider {
    fn upstream_execution_gate(&self) -> Option<&ConcurrencyGate>;
    fn upstream_execution_gate_queue_budget(&self) -> Duration;
}

impl UpstreamExecutionGateProvider for AppState {
    fn upstream_execution_gate(&self) -> Option<&ConcurrencyGate> {
        self.upstream_execution_gate.as_deref()
    }

    fn upstream_execution_gate_queue_budget(&self) -> Duration {
        self.frontdoor_runtime_guards.internal_gate_queue_budget
    }
}

/// Acquires the shared gateway-wide upstream execution permit.
///
/// A missing gate is an intentional configuration (unlimited), so callers
/// receive `Ok(None)`.  Saturation keeps the existing candidate-level
/// `AdmissionTimeout` contract used by the HTTP stream path.
pub(crate) async fn acquire_upstream_execution_gate(
    state: &(impl UpstreamExecutionGateProvider + ?Sized),
    trace_id: &str,
) -> Result<Option<ConcurrencyPermit>, GatewayError> {
    let Some(gate) = state.upstream_execution_gate() else {
        return Ok(None);
    };
    let budget = state.upstream_execution_gate_queue_budget();
    let gate_wait_started_at = std::time::Instant::now();
    match timeout(budget, gate.acquire()).await {
        Ok(Ok(permit)) => {
            observe_gateway_stage_ms(
                "upstream_execution_gate_wait",
                gate_wait_started_at.elapsed().as_millis() as u64,
            );
            Ok(Some(permit))
        }
        Ok(Err(err)) => Err(GatewayError::Internal(err.to_string())),
        Err(_) => Err(GatewayError::AdmissionTimeout {
            trace_id: trace_id.to_string(),
            gate: UPSTREAM_EXECUTION_GATE_NAME,
            queue_budget_ms: budget.as_millis() as u64,
        }),
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct ExecutionReservationRejection {
    status: StatusCode,
    error_type: &'static str,
    message: &'static str,
    kind: LocalCoreSyncErrorKind,
}

fn execution_reservation_rejection(
    error: &ExecutionRequestReservationError,
) -> ExecutionReservationRejection {
    match error {
        ExecutionRequestReservationError::Saturated { .. } => ExecutionReservationRejection {
            status: StatusCode::TOO_MANY_REQUESTS,
            error_type: "execution_reservation_saturated",
            message: "Gateway execution capacity is busy; retry this request",
            kind: LocalCoreSyncErrorKind::RateLimit,
        },
        ExecutionRequestReservationError::Unavailable { .. } => ExecutionReservationRejection {
            status: StatusCode::SERVICE_UNAVAILABLE,
            error_type: "execution_reservation_unavailable",
            message: "Gateway execution capacity is unavailable; retry this request",
            kind: LocalCoreSyncErrorKind::Overloaded,
        },
    }
}

/// Finalizes an attempt that could not reserve its selected execution scopes.
/// No provider request has started at this point, so only candidate state and
/// the scheduler's pool-key lease need terminal cleanup.
pub(crate) async fn build_execution_reservation_rejection_response(
    state: &AppState,
    plan: &ExecutionPlan,
    report_context: Option<&serde_json::Value>,
    trace_id: &str,
    decision: &GatewayControlDecision,
    error: ExecutionRequestReservationError,
) -> Result<Response<Body>, GatewayError> {
    let rejection = execution_reservation_rejection(&error);
    match &error {
        ExecutionRequestReservationError::Saturated { scope, limit } => {
            tracing::debug!(
                scope = scope.as_str(),
                limit,
                request_id = %plan.request_id,
                candidate_id = ?plan.candidate_id,
                "gateway rejected an HTTP execution reservation"
            );
        }
        ExecutionRequestReservationError::Unavailable { message } => {
            tracing::warn!(
                error = %message,
                request_id = %plan.request_id,
                candidate_id = ?plan.candidate_id,
                "gateway could not acquire an HTTP execution reservation"
            );
        }
    }

    record_local_request_candidate_status(
        state,
        plan,
        report_context,
        SchedulerRequestCandidateStatusUpdate {
            status: RequestCandidateStatus::Skipped,
            status_code: Some(rejection.status.as_u16()),
            error_type: Some(rejection.error_type.to_string()),
            error_message: Some(rejection.message.to_string()),
            latency_ms: None,
            started_at_unix_ms: None,
            finished_at_unix_ms: Some(current_unix_ms()),
        },
    )
    .await;
    release_local_pool_key_lease(
        state,
        LocalExecutionEffectContext {
            plan,
            report_context,
        },
    )
    .await;

    let fallback_body = json!({
        "error": {
            "type": rejection.error_type,
            "message": rejection.message,
            "code": rejection.error_type,
        }
    });
    let body = build_core_error_body_for_client_format(
        &plan.client_api_format,
        rejection.message,
        Some(rejection.error_type),
        rejection.kind,
    )
    .unwrap_or(fallback_body);
    let body = serde_json::to_vec(&body).map_err(|err| GatewayError::Internal(err.to_string()))?;
    let headers = std::collections::BTreeMap::from([
        ("content-type".to_string(), "application/json".to_string()),
        ("content-length".to_string(), body.len().to_string()),
    ]);
    let response = build_client_response_from_parts(
        rejection.status.as_u16(),
        &headers,
        Body::from(body),
        trace_id,
        Some(decision),
    )?;
    attach_control_metadata_headers(
        response,
        Some(plan.request_id.as_str()),
        plan.candidate_id.as_deref(),
    )
}

#[cfg(test)]
mod tests {
    use aether_runtime_state::ExecutionReservationScope;

    use super::{execution_reservation_rejection, ExecutionRequestReservationError};

    #[test]
    fn saturated_execution_reservation_maps_to_stable_429() {
        let rejection =
            execution_reservation_rejection(&ExecutionRequestReservationError::Saturated {
                scope: ExecutionReservationScope::ProviderKey,
                limit: 2,
            });

        assert_eq!(rejection.status, http::StatusCode::TOO_MANY_REQUESTS);
        assert_eq!(rejection.error_type, "execution_reservation_saturated");
    }

    #[test]
    fn unavailable_execution_reservation_maps_to_stable_503() {
        let rejection =
            execution_reservation_rejection(&ExecutionRequestReservationError::Unavailable {
                message: "strong read failed".to_string(),
            });

        assert_eq!(rejection.status, http::StatusCode::SERVICE_UNAVAILABLE);
        assert_eq!(rejection.error_type, "execution_reservation_unavailable");
    }
}
