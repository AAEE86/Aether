use axum::body::{Body, Bytes};
use axum::http::Response;

use crate::gateway::{AppState, GatewayControlDecision, GatewayError};

mod execution;

pub(crate) use execution::execute_execution_runtime_sync;

#[allow(unused_imports)]
pub(crate) use execution::{
    maybe_build_local_sync_finalize_response, maybe_build_local_video_error_response,
    maybe_build_local_video_success_outcome, resolve_local_sync_error_background_report_kind,
    resolve_local_sync_success_background_report_kind, LocalVideoSyncSuccessOutcome,
};

pub(crate) async fn maybe_execute_via_execution_runtime_sync(
    state: &AppState,
    parts: &http::request::Parts,
    body_bytes: &Bytes,
    trace_id: &str,
    decision: Option<&GatewayControlDecision>,
) -> Result<Option<Response<Body>>, GatewayError> {
    let Some(decision) = decision else {
        return Ok(None);
    };
    #[cfg(not(test))]
    {
        let _ = state;
        if parts.method != http::Method::POST {
            return Ok(None);
        }
        return crate::gateway::executor::maybe_execute_sync_local_path(
            state, parts, body_bytes, trace_id, decision,
        )
        .await;
    }
    #[cfg(test)]
    {
        if state
            .execution_runtime_override_base_url()
            .unwrap_or_default()
            .is_empty()
            && parts.method != http::Method::POST
        {
            return Ok(None);
        }
        crate::gateway::executor::maybe_execute_sync_local_path(
            state, parts, body_bytes, trace_id, decision,
        )
        .await
    }
}
