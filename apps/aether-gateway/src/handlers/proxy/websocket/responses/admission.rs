//! Per-turn resource admission for the Responses WebSocket bridge.
//!
//! A WebSocket connection may live for a long time, but each `response.create`
//! is still one active upstream execution.  Keep the resource leases attached
//! to the turn instead of the socket so idle connections do not consume
//! upstream capacity.

use std::time::Instant;

use aether_contracts::ExecutionPlan;
use aether_runtime::AdmissionPermitHealth;
use aether_runtime_state::ExecutionReservationPermit;

use crate::control::GatewayControlAuthContext;
use crate::execution_runtime::acquire_upstream_execution_gate;
use crate::provider_pool_demand::{
    acquire_provider_pool_in_flight_guard, ProviderPoolInFlightGuard,
};
use crate::request_candidate_runtime::{
    acquire_execution_request_reservation, ExecutionRequestReservationError,
};
use crate::upstream_admission::UpstreamTargetAdmissionPermit;
use crate::{AppState, GatewayError};

pub(super) struct ResponsesWebSocketTurnAdmission {
    upstream_execution: Option<aether_runtime::ConcurrencyPermit>,
    upstream_target: Option<UpstreamTargetAdmissionPermit>,
    provider_pool: Option<ProviderPoolInFlightGuard>,
    execution_reservation: Option<ExecutionReservationPermit>,
    acquired_at: Instant,
}

impl ResponsesWebSocketTurnAdmission {
    pub(super) async fn acquire(
        state: &AppState,
        plan: &ExecutionPlan,
        trace_id: &str,
        auth_context: Option<&GatewayControlAuthContext>,
    ) -> Result<Self, GatewayError> {
        let upstream_execution = acquire_upstream_execution_gate(state, trace_id).await?;
        let upstream_target = match state
            .upstream_target_admission
            .acquire(plan, trace_id)
            .await
        {
            Ok(permit) => permit,
            Err(error) => {
                drop(upstream_execution);
                return Err(error);
            }
        };
        let provider_pool = acquire_provider_pool_in_flight_guard(
            state.runtime_state.clone(),
            &plan.provider_id,
            &plan.request_id,
            plan.candidate_id.as_deref(),
            &plan.key_id,
        )
        .await;
        let execution_reservation =
            match acquire_execution_request_reservation(state, plan, auth_context).await {
                Ok(permit) => permit,
                Err(error) => {
                    drop(provider_pool);
                    drop(upstream_target);
                    drop(upstream_execution);
                    return Err(execution_reservation_gateway_error(error));
                }
            };

        Ok(Self {
            upstream_execution,
            upstream_target,
            provider_pool,
            execution_reservation,
            acquired_at: Instant::now(),
        })
    }

    pub(super) fn is_healthy(&self) -> bool {
        self.execution_reservation
            .as_ref()
            .is_none_or(AdmissionPermitHealth::is_healthy)
    }

    /// Release the distributed provider token before the turn's persistence
    /// work. The remaining permits are local RAII guards and are dropped with
    /// this value.
    pub(super) async fn release(mut self) {
        if let Some(reservation) = self.execution_reservation.take() {
            if let Err(error) = reservation.release().await {
                tracing::debug!(
                    error = ?error,
                    "gateway failed to eagerly release a Responses WebSocket execution reservation"
                );
            }
        }
        if let Some(provider_pool) = self.provider_pool.take() {
            provider_pool.release().await;
        }
        drop(self.upstream_target.take());
        drop(self.upstream_execution.take());
    }
}

fn execution_reservation_gateway_error(error: ExecutionRequestReservationError) -> GatewayError {
    match error {
        ExecutionRequestReservationError::Saturated { scope, limit } => {
            tracing::debug!(
                scope = scope.as_str(),
                limit,
                "gateway rejected a Responses WebSocket execution reservation"
            );
            GatewayError::Client {
                status: http::StatusCode::TOO_MANY_REQUESTS,
                message: "Gateway execution capacity is busy; retry this response".to_string(),
            }
        }
        ExecutionRequestReservationError::Unavailable { message } => {
            tracing::warn!(
                error = %message,
                "gateway could not acquire a Responses WebSocket execution reservation"
            );
            GatewayError::Client {
                status: http::StatusCode::SERVICE_UNAVAILABLE,
                message: "Gateway execution capacity is unavailable; retry this response"
                    .to_string(),
            }
        }
    }
}

impl Drop for ResponsesWebSocketTurnAdmission {
    fn drop(&mut self) {
        crate::stage_metrics::observe_gateway_stage_ms(
            "websocket_turn_admission_held",
            self.acquired_at.elapsed().as_millis() as u64,
        );
    }
}
