use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};
use std::time::{SystemTime, UNIX_EPOCH};

use aether_data::repository::shadow_results::{RecordShadowResultSample, ShadowResultSampleOrigin};
use axum::body::Body;
use axum::http::header::CONTENT_TYPE;
use axum::http::Response;
use tracing::warn;

use crate::gateway::constants::{
    CONTROL_CANDIDATE_ID_HEADER, CONTROL_REQUEST_ID_HEADER, EXECUTION_PATH_CONTROL_EXECUTE_STREAM,
    EXECUTION_PATH_CONTROL_EXECUTE_SYNC,
};
use crate::gateway::{AppState, GatewayControlDecision};

pub(crate) fn record_shadow_result_non_blocking(
    state: AppState,
    trace_id: &str,
    method: &http::Method,
    path_and_query: &str,
    control_decision: Option<&GatewayControlDecision>,
    execution_path: &'static str,
    response: &Response<Body>,
) {
    let Some(decision) =
        control_decision.filter(|decision| decision.route_class.as_deref() == Some("ai_public"))
    else {
        return;
    };
    if !state.has_shadow_result_data_writer() {
        return;
    }

    let route_family = decision.route_family.clone();
    let route_kind = decision.route_kind.clone();
    let status_code = response.status().as_u16();
    let content_type = response
        .headers()
        .get(CONTENT_TYPE)
        .and_then(|value| value.to_str().ok())
        .unwrap_or_default()
        .to_string();
    let candidate_id = response
        .headers()
        .get(CONTROL_CANDIDATE_ID_HEADER)
        .and_then(|value| value.to_str().ok())
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(ToOwned::to_owned);
    let request_id = response
        .headers()
        .get(CONTROL_REQUEST_ID_HEADER)
        .and_then(|value| value.to_str().ok())
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(ToOwned::to_owned);
    let now_unix_secs = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();

    let sample = RecordShadowResultSample {
        trace_id: trace_id.to_string(),
        request_fingerprint: build_request_fingerprint(
            method,
            path_and_query,
            route_family.as_deref(),
            route_kind.as_deref(),
        ),
        request_id,
        route_family,
        route_kind,
        candidate_id,
        origin: sample_origin_for_execution_path(execution_path),
        result_digest: build_result_digest(status_code, &content_type),
        status_code: Some(status_code),
        error_message: (status_code >= 400)
            .then(|| format!("gateway response status {status_code}")),
        recorded_at_unix_secs: now_unix_secs,
    };
    let trace_id = trace_id.to_string();

    tokio::spawn(async move {
        if let Err(err) = state.record_shadow_result_sample(sample).await {
            warn!(trace_id = %trace_id, error = ?err, "gateway failed to record shadow result");
        }
    });
}

fn build_request_fingerprint(
    method: &http::Method,
    path_and_query: &str,
    route_family: Option<&str>,
    route_kind: Option<&str>,
) -> String {
    let mut hasher = DefaultHasher::new();
    method.as_str().hash(&mut hasher);
    path_and_query.hash(&mut hasher);
    route_family.unwrap_or_default().hash(&mut hasher);
    route_kind.unwrap_or_default().hash(&mut hasher);
    format!("{:x}", hasher.finish())
}

fn build_result_digest(status_code: u16, content_type: &str) -> String {
    let mut hasher = DefaultHasher::new();
    status_code.hash(&mut hasher);
    content_type.hash(&mut hasher);
    format!("{:x}", hasher.finish())
}

fn sample_origin_for_execution_path(execution_path: &str) -> ShadowResultSampleOrigin {
    match execution_path {
        EXECUTION_PATH_CONTROL_EXECUTE_SYNC | EXECUTION_PATH_CONTROL_EXECUTE_STREAM => {
            ShadowResultSampleOrigin::Python
        }
        _ => ShadowResultSampleOrigin::Rust,
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use aether_data::repository::shadow_results::{
        InMemoryShadowResultRepository, ShadowResultMatchStatus, ShadowResultReadRepository,
    };
    use axum::body::Body;
    use axum::http::header::CONTENT_TYPE;
    use axum::http::{Method, Response, StatusCode};

    use super::record_shadow_result_non_blocking;
    use crate::gateway::constants::{
        CONTROL_REQUEST_ID_HEADER, EXECUTION_PATH_CONTROL_EXECUTE_SYNC,
        EXECUTION_PATH_EXECUTION_RUNTIME_SYNC,
    };
    use crate::gateway::{AppState, GatewayControlDecision};

    fn sample_decision() -> GatewayControlDecision {
        GatewayControlDecision {
            public_path: "/v1/chat/completions".to_string(),
            public_query_string: Some("stream=true".to_string()),
            route_class: Some("ai_public".to_string()),
            route_family: Some("openai".to_string()),
            route_kind: Some("chat".to_string()),
            auth_endpoint_signature: Some("openai:chat".to_string()),
            execution_runtime_candidate: true,
            auth_context: None,
            admin_principal: None,
            local_auth_rejection: None,
        }
    }

    #[tokio::test]
    async fn records_shadow_result_for_ai_public_response() {
        let repository = Arc::new(InMemoryShadowResultRepository::default());
        let state = AppState::new()
            .expect("app state should build")
            .with_shadow_result_data_writer_for_tests(repository.clone());
        let response = Response::builder()
            .status(StatusCode::OK)
            .header(CONTENT_TYPE, "application/json")
            .header(CONTROL_REQUEST_ID_HEADER, "req-shadow-123")
            .body(Body::from("{}"))
            .expect("response should build");

        record_shadow_result_non_blocking(
            state,
            "trace-shadow-123",
            &Method::POST,
            "/v1/chat/completions?stream=true",
            Some(&sample_decision()),
            EXECUTION_PATH_EXECUTION_RUNTIME_SYNC,
            &response,
        );

        for _ in 0..30 {
            if repository
                .list_recent(1)
                .await
                .map(|rows| !rows.is_empty())
                .unwrap_or(false)
            {
                break;
            }
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        }

        let stored = repository
            .list_recent(1)
            .await
            .expect("list should succeed")
            .into_iter()
            .next()
            .expect("stored result should exist");
        assert_eq!(stored.trace_id, "trace-shadow-123");
        assert_eq!(stored.request_id.as_deref(), Some("req-shadow-123"));
        assert_eq!(stored.route_family.as_deref(), Some("openai"));
        assert_eq!(stored.route_kind.as_deref(), Some("chat"));
        assert_eq!(stored.match_status, ShadowResultMatchStatus::Pending);
        assert_eq!(stored.status_code, Some(200));
        assert!(stored.rust_result_digest.is_some());
    }

    #[tokio::test]
    async fn merges_rust_and_python_shadow_samples_into_match() {
        let repository = Arc::new(InMemoryShadowResultRepository::default());
        let state = AppState::new()
            .expect("app state should build")
            .with_shadow_result_data_repository_for_tests(repository.clone());
        let response = Response::builder()
            .status(StatusCode::OK)
            .header(CONTENT_TYPE, "application/json")
            .header(CONTROL_REQUEST_ID_HEADER, "req-shadow-compare-123")
            .body(Body::from("{}"))
            .expect("response should build");

        record_shadow_result_non_blocking(
            state.clone(),
            "trace-shadow-compare-123",
            &Method::POST,
            "/v1/chat/completions?stream=true",
            Some(&sample_decision()),
            EXECUTION_PATH_EXECUTION_RUNTIME_SYNC,
            &response,
        );
        record_shadow_result_non_blocking(
            state,
            "trace-shadow-compare-123",
            &Method::POST,
            "/v1/chat/completions?stream=true",
            Some(&sample_decision()),
            EXECUTION_PATH_CONTROL_EXECUTE_SYNC,
            &response,
        );

        for _ in 0..30 {
            if repository
                .list_recent(1)
                .await
                .map(|rows| {
                    rows.first()
                        .map(|row| row.match_status == ShadowResultMatchStatus::Match)
                        .unwrap_or(false)
                })
                .unwrap_or(false)
            {
                break;
            }
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        }

        let stored = repository
            .list_recent(1)
            .await
            .expect("list should succeed")
            .into_iter()
            .next()
            .expect("stored result should exist");
        assert_eq!(stored.request_id.as_deref(), Some("req-shadow-compare-123"));
        assert_eq!(stored.match_status, ShadowResultMatchStatus::Match);
        assert!(stored.rust_result_digest.is_some());
        assert!(stored.python_result_digest.is_some());
    }
}
