use std::sync::atomic::{AtomicUsize, Ordering};

use super::{
    any, build_router_with_state, start_server, wait_until, AppState, Arc, Body, Bytes,
    GatewayFallbackMetricKind, GatewayFallbackReason, Infallible, Request, Response, Router,
    StatusCode, EXECUTION_PATH_DISTRIBUTED_OVERLOADED, EXECUTION_PATH_HEADER,
    EXECUTION_PATH_LOCAL_EXECUTION_RUNTIME_MISS, EXECUTION_PATH_LOCAL_OVERLOADED,
};

fn sample_decision() -> crate::gateway::GatewayControlDecision {
    crate::gateway::GatewayControlDecision {
        public_path: "/v1/chat/completions".to_string(),
        public_query_string: None,
        route_class: Some("ai_public".to_string()),
        route_family: Some("openai".to_string()),
        route_kind: Some("chat".to_string()),
        auth_endpoint_signature: None,
        execution_runtime_candidate: true,
        auth_context: None,
        admin_principal: None,
        local_auth_rejection: None,
    }
}

#[tokio::test]
async fn gateway_rejects_second_in_flight_stream_request_with_distributed_overload() {
    let upstream_hits = Arc::new(AtomicUsize::new(0));
    let upstream_hits_clone = Arc::clone(&upstream_hits);
    let upstream = Router::new().route(
        "/{*path}",
        any(move |_request: Request| {
            let upstream_hits = Arc::clone(&upstream_hits_clone);
            async move {
                upstream_hits.fetch_add(1, Ordering::SeqCst);
                let stream = async_stream::stream! {
                    yield Ok::<_, Infallible>(Bytes::from_static(b"chunk-1"));
                    futures_util::future::pending::<()>().await;
                };
                Response::builder()
                    .status(StatusCode::OK)
                    .body(Body::from_stream(stream))
                    .expect("response should build")
            }
        }),
    );

    let (upstream_url, upstream_handle) = start_server(upstream).await;
    let distributed_gate = aether_runtime::DistributedConcurrencyGate::new_in_memory(
        "gateway_requests_distributed",
        1,
    );
    let gateway_a = build_router_with_state(
        AppState::new()
            .expect("gateway state should build")
            .with_distributed_request_concurrency_gate(distributed_gate.clone()),
    );
    let gateway_b = build_router_with_state(
        AppState::new()
            .expect("gateway state should build")
            .with_distributed_request_concurrency_gate(distributed_gate),
    );
    let (gateway_a_url, gateway_a_handle) = start_server(gateway_a).await;
    let (gateway_b_url, gateway_b_handle) = start_server(gateway_b).await;

    let client = reqwest::Client::new();
    let first_response = client
        .get(format!("{gateway_a_url}/proxy-stream"))
        .send()
        .await
        .expect("first request should succeed");

    wait_until(500, || upstream_hits.load(Ordering::SeqCst) == 1).await;

    let second_response = client
        .get(format!("{gateway_b_url}/proxy-stream"))
        .send()
        .await
        .expect("second request should complete");

    assert_eq!(second_response.status(), StatusCode::SERVICE_UNAVAILABLE);
    assert_eq!(
        second_response
            .headers()
            .get(EXECUTION_PATH_HEADER)
            .and_then(|value| value.to_str().ok()),
        Some(EXECUTION_PATH_DISTRIBUTED_OVERLOADED)
    );
    assert_eq!(
        second_response
            .json::<serde_json::Value>()
            .await
            .expect("json body should decode")["error"]["details"]["gate"],
        "gateway_requests_distributed"
    );
    assert_eq!(upstream_hits.load(Ordering::SeqCst), 1);

    drop(first_response);
    gateway_a_handle.abort();
    gateway_b_handle.abort();
    upstream_handle.abort();
}

#[tokio::test]
async fn gateway_rejects_second_in_flight_stream_request_with_local_overload() {
    let upstream_hits = Arc::new(AtomicUsize::new(0));
    let upstream_hits_clone = Arc::clone(&upstream_hits);
    let upstream = Router::new().route(
        "/{*path}",
        any(move |_request: Request| {
            let upstream_hits = Arc::clone(&upstream_hits_clone);
            async move {
                upstream_hits.fetch_add(1, Ordering::SeqCst);
                let stream = async_stream::stream! {
                    yield Ok::<_, Infallible>(Bytes::from_static(b"chunk-1"));
                    futures_util::future::pending::<()>().await;
                };
                Response::builder()
                    .status(StatusCode::OK)
                    .body(Body::from_stream(stream))
                    .expect("response should build")
            }
        }),
    );

    let (upstream_url, upstream_handle) = start_server(upstream).await;
    let gateway = build_router_with_state(
        AppState::new()
            .expect("gateway state should build")
            .with_request_concurrency_limit(1),
    );
    let (gateway_url, gateway_handle) = start_server(gateway).await;

    let client = reqwest::Client::new();
    let first_response = client
        .get(format!("{gateway_url}/proxy-stream"))
        .send()
        .await
        .expect("first request should succeed");

    wait_until(500, || upstream_hits.load(Ordering::SeqCst) == 1).await;

    let second_response = client
        .get(format!("{gateway_url}/proxy-stream"))
        .send()
        .await
        .expect("second request should complete");

    assert_eq!(second_response.status(), StatusCode::SERVICE_UNAVAILABLE);
    assert_eq!(
        second_response
            .headers()
            .get(EXECUTION_PATH_HEADER)
            .and_then(|value| value.to_str().ok()),
        Some(EXECUTION_PATH_LOCAL_OVERLOADED)
    );
    assert_eq!(
        second_response
            .json::<serde_json::Value>()
            .await
            .expect("json body should decode")["error"]["type"],
        "overloaded"
    );
    assert_eq!(upstream_hits.load(Ordering::SeqCst), 1);

    drop(first_response);
    gateway_handle.abort();
    upstream_handle.abort();
}

#[tokio::test]
async fn gateway_exposes_request_concurrency_metrics() {
    let gateway = build_router_with_state(
        AppState::new()
            .expect("gateway state should build")
            .with_request_concurrency_limit(3)
            .with_distributed_request_concurrency_gate(
                aether_runtime::DistributedConcurrencyGate::new_in_memory(
                    "gateway_requests_distributed",
                    5,
                ),
            ),
    );
    let (gateway_url, gateway_handle) = start_server(gateway).await;

    let response = reqwest::Client::new()
        .get(format!("{gateway_url}/_gateway/metrics"))
        .send()
        .await
        .expect("request should succeed");

    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(
        response
            .headers()
            .get(http::header::CONTENT_TYPE)
            .and_then(|value| value.to_str().ok()),
        Some("text/plain; version=0.0.4; charset=utf-8")
    );
    let body = response.text().await.expect("body should read");
    assert!(body.contains("service_up{service=\"aether-gateway\"} 1"));
    assert!(body.contains("concurrency_in_flight{gate=\"gateway_requests\"} 0"));
    assert!(body.contains("concurrency_available_permits{gate=\"gateway_requests\"} 3"));
    assert!(body.contains("concurrency_in_flight{gate=\"gateway_requests_distributed\"} 0"));
    assert!(body.contains("concurrency_available_permits{gate=\"gateway_requests_distributed\"} 5"));
    assert!(body.contains("tunnel_proxy_connections 0"));
    assert!(body.contains("tunnel_nodes 0"));
    assert!(body.contains("tunnel_active_streams 0"));

    gateway_handle.abort();
}

#[tokio::test]
async fn gateway_exposes_fallback_metrics() {
    let state = AppState::new().expect("gateway state should build");
    let decision = sample_decision();
    state.record_fallback_metric(
        GatewayFallbackMetricKind::DecisionRemote,
        Some(&decision),
        Some("openai_chat_sync"),
        None,
        GatewayFallbackReason::LocalDecisionMiss,
    );
    state.record_fallback_metric(
        GatewayFallbackMetricKind::PlanFallback,
        Some(&decision),
        Some("openai_chat_sync"),
        None,
        GatewayFallbackReason::RemoteDecisionMiss,
    );
    state.record_fallback_metric(
        GatewayFallbackMetricKind::LocalExecutionRuntimeMiss,
        Some(&decision),
        None,
        Some(EXECUTION_PATH_LOCAL_EXECUTION_RUNTIME_MISS),
        GatewayFallbackReason::LocalExecutionPathRequired,
    );
    let gateway = build_router_with_state(state);
    let (gateway_url, gateway_handle) = start_server(gateway).await;

    let response = reqwest::Client::new()
        .get(format!("{gateway_url}/_gateway/metrics"))
        .send()
        .await
        .expect("request should succeed");

    assert_eq!(response.status(), StatusCode::OK);
    let body = response.text().await.expect("body should read");
    let decision_remote = body
        .lines()
        .find(|line| line.starts_with("decision_remote_total{"))
        .expect("decision_remote_total sample should be rendered");
    assert!(decision_remote.contains("route_class=\"ai_public\""));
    assert!(decision_remote.contains("route_family=\"openai\""));
    assert!(decision_remote.contains("route_kind=\"chat\""));
    assert!(decision_remote.contains("plan_kind=\"openai_chat_sync\""));
    assert!(decision_remote.contains("execution_path=\"none\""));
    assert!(decision_remote.contains("reason=\"local_decision_miss\""));
    assert!(decision_remote.ends_with(" 1"));

    let plan_fallback = body
        .lines()
        .find(|line| line.starts_with("plan_fallback_total{"))
        .expect("plan_fallback_total sample should be rendered");
    assert!(plan_fallback.contains("route_class=\"ai_public\""));
    assert!(plan_fallback.contains("route_family=\"openai\""));
    assert!(plan_fallback.contains("route_kind=\"chat\""));
    assert!(plan_fallback.contains("plan_kind=\"openai_chat_sync\""));
    assert!(plan_fallback.contains("execution_path=\"none\""));
    assert!(plan_fallback.contains("reason=\"remote_decision_miss\""));
    assert!(plan_fallback.ends_with(" 1"));

    let local_execution_runtime_miss = body
        .lines()
        .find(|line| line.starts_with("local_execution_runtime_miss_total{"))
        .expect("local_execution_runtime_miss_total sample should be rendered");
    assert!(local_execution_runtime_miss.contains("route_class=\"ai_public\""));
    assert!(local_execution_runtime_miss.contains("route_family=\"openai\""));
    assert!(local_execution_runtime_miss.contains("route_kind=\"chat\""));
    assert!(local_execution_runtime_miss.contains("plan_kind=\"none\""));
    assert!(local_execution_runtime_miss.contains(&format!(
        "execution_path=\"{}\"",
        EXECUTION_PATH_LOCAL_EXECUTION_RUNTIME_MISS
    )));
    assert!(local_execution_runtime_miss.contains("reason=\"local_execution_path_required\""));
    assert!(local_execution_runtime_miss.ends_with(" 1"));

    gateway_handle.abort();
}
