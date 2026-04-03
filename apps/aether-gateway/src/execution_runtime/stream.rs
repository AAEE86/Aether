use axum::body::{Body, Bytes};
use axum::http::Response;

use crate::gateway::{AppState, GatewayControlDecision, GatewayError};

mod error;
mod execution;

pub(crate) use execution::execute_execution_runtime_stream;

pub(crate) async fn maybe_execute_via_execution_runtime_stream(
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
        return crate::gateway::executor::maybe_execute_stream_local_path(
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
        crate::gateway::executor::maybe_execute_stream_local_path(
            state, parts, body_bytes, trace_id, decision,
        )
        .await
    }
}
