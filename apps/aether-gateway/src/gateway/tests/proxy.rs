use std::time::{Duration, SystemTime, UNIX_EPOCH};

use super::{
    any, build_router, build_router_with_state, start_server, to_bytes, AppState, Arc, Body,
    HeaderValue, Json, Mutex, Request, Response, Router, StatusCode, DEPENDENCY_REASON_HEADER,
    EXECUTION_PATH_HEADER, EXECUTION_PATH_LOCAL_ROUTE_NOT_FOUND, FORWARDED_FOR_HEADER,
    GATEWAY_HEADER, TRACE_ID_HEADER, TRUSTED_AUTH_ACCESS_ALLOWED_HEADER,
    TRUSTED_AUTH_API_KEY_ID_HEADER, TRUSTED_AUTH_USER_ID_HEADER,
    TUNNEL_AFFINITY_FORWARDED_BY_HEADER, TUNNEL_AFFINITY_OWNER_INSTANCE_HEADER, json,
};

use aether_data::repository::auth::{
    InMemoryAuthApiKeySnapshotRepository, StoredAuthApiKeySnapshot,
};
use aether_data::repository::provider_catalog::{
    InMemoryProviderCatalogReadRepository, StoredProviderCatalogEndpoint, StoredProviderCatalogKey,
    StoredProviderCatalogProvider,
};
use sha2::{Digest, Sha256};

fn hash_api_key(value: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(value.as_bytes());
    format!("{:x}", hasher.finalize())
}

fn sample_auth_snapshot(
    api_key_id: &str,
    user_id: &str,
    allowed_model: &str,
) -> StoredAuthApiKeySnapshot {
    StoredAuthApiKeySnapshot::new(
        user_id.to_string(),
        "alice".to_string(),
        Some("alice@example.com".to_string()),
        "user".to_string(),
        "local".to_string(),
        true,
        false,
        Some(serde_json::json!(["openai"])),
        Some(serde_json::json!(["openai:chat"])),
        Some(serde_json::json!([allowed_model])),
        api_key_id.to_string(),
        Some("default".to_string()),
        true,
        false,
        false,
        Some(60),
        Some(5),
        Some(4_102_444_800),
        Some(serde_json::json!(["openai"])),
        Some(serde_json::json!(["openai:chat"])),
        Some(serde_json::json!([allowed_model])),
    )
    .expect("auth snapshot should build")
}

fn sample_provider(provider_id: &str) -> StoredProviderCatalogProvider {
    StoredProviderCatalogProvider::new(
        provider_id.to_string(),
        provider_id.to_string(),
        Some("https://provider.example".to_string()),
        "custom".to_string(),
    )
    .expect("provider should build")
    .with_transport_fields(true, false, false, None, None, None, None, None, None)
}

fn sample_endpoint(endpoint_id: &str, provider_id: &str) -> StoredProviderCatalogEndpoint {
    StoredProviderCatalogEndpoint::new(
        endpoint_id.to_string(),
        provider_id.to_string(),
        "openai:chat".to_string(),
        Some("openai".to_string()),
        Some("chat".to_string()),
        true,
    )
    .expect("endpoint should build")
    .with_transport_fields(
        "https://api.provider.example".to_string(),
        None,
        None,
        None,
        None,
        None,
        None,
        None,
    )
    .expect("endpoint transport should build")
}

fn sample_key(key_id: &str, provider_id: &str, node_id: &str) -> StoredProviderCatalogKey {
    StoredProviderCatalogKey::new(
        key_id.to_string(),
        provider_id.to_string(),
        "default".to_string(),
        "api_key".to_string(),
        None,
        true,
    )
    .expect("key should build")
    .with_transport_fields(
        Some(json!(["openai:chat"])),
        "plain-upstream-key".to_string(),
        None,
        None,
        Some(json!({"openai:chat": 1})),
        None,
        None,
        Some(json!({
            "enabled": true,
            "mode": "tunnel",
            "node_id": node_id,
        })),
        None,
    )
    .expect("key transport should build")
}

fn tunnel_attachment_key(node_id: &str) -> String {
    format!("tunnel.attachments.{node_id}")
}

fn current_unix_secs() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}

#[tokio::test]
async fn gateway_rejects_unknown_path_locally_and_generates_trace_id() {
    let upstream_hits = Arc::new(Mutex::new(0usize));
    let upstream_hits_clone = Arc::clone(&upstream_hits);
    let upstream = Router::new().route(
        "/{*path}",
        any(move |_request: Request| {
            let upstream_hits_inner = Arc::clone(&upstream_hits_clone);
            async move {
                *upstream_hits_inner.lock().expect("mutex should lock") += 1;
                (StatusCode::CREATED, Body::from("unexpected upstream hit"))
            }
        }),
    );

    let (upstream_url, upstream_handle) = start_server(upstream).await;
    let gateway = build_router().expect("gateway should build");
    let (gateway_url, gateway_handle) = start_server(gateway).await;

    let client = reqwest::Client::new();
    let response = client
        .post(format!("{gateway_url}/does/not/exist?stream=true"))
        .header(http::header::HOST, "api.example.com")
        .header(DEPENDENCY_REASON_HEADER, "forged")
        .body("{\"hello\":\"world\"}")
        .send()
        .await
        .expect("request should succeed");

    assert_eq!(response.status(), StatusCode::NOT_FOUND);
    assert_eq!(
        response
            .headers()
            .get(GATEWAY_HEADER)
            .and_then(|value| value.to_str().ok()),
        Some("rust-phase3b")
    );
    assert_eq!(
        response
            .headers()
            .get(EXECUTION_PATH_HEADER)
            .and_then(|value| value.to_str().ok()),
        Some(EXECUTION_PATH_LOCAL_ROUTE_NOT_FOUND)
    );
    assert_eq!(
        response
            .headers()
            .get(DEPENDENCY_REASON_HEADER)
            .and_then(|value| value.to_str().ok()),
        None
    );

    let response_trace_id = response
        .headers()
        .get(TRACE_ID_HEADER)
        .and_then(|value| value.to_str().ok())
        .expect("response trace id should exist")
        .to_string();
    let payload: serde_json::Value = response.json().await.expect("body should parse");
    assert_eq!(payload["error"]["type"], "http_error");
    assert_eq!(payload["error"]["message"], "Route not found");
    assert!(!response_trace_id.is_empty());
    assert_eq!(*upstream_hits.lock().expect("mutex should lock"), 0);

    gateway_handle.abort();
    upstream_handle.abort();
}

#[tokio::test]
async fn gateway_preserves_existing_trace_id_on_unknown_local_not_found() {
    let upstream_hits = Arc::new(Mutex::new(0usize));
    let upstream_hits_clone = Arc::clone(&upstream_hits);
    let upstream = Router::new().route(
        "/{*path}",
        any(move |_request: Request| {
            let upstream_hits_inner = Arc::clone(&upstream_hits_clone);
            async move {
                *upstream_hits_inner.lock().expect("mutex should lock") += 1;
                (StatusCode::OK, Body::from("unexpected upstream hit"))
            }
        }),
    );

    let (upstream_url, upstream_handle) = start_server(upstream).await;
    let gateway = build_router().expect("gateway should build");
    let (gateway_url, gateway_handle) = start_server(gateway).await;

    let response = reqwest::Client::new()
        .get(format!("{gateway_url}/streaming-proxy"))
        .header(TRACE_ID_HEADER, "trace-123")
        .send()
        .await
        .expect("request should succeed");

    assert_eq!(response.status(), StatusCode::NOT_FOUND);
    assert_eq!(
        response
            .headers()
            .get(TRACE_ID_HEADER)
            .and_then(|value| value.to_str().ok()),
        Some("trace-123")
    );
    assert_eq!(
        response
            .headers()
            .get(EXECUTION_PATH_HEADER)
            .and_then(|value| value.to_str().ok()),
        Some(EXECUTION_PATH_LOCAL_ROUTE_NOT_FOUND)
    );
    let payload: serde_json::Value = response.json().await.expect("body should parse");
    assert_eq!(payload["error"]["type"], "http_error");
    assert_eq!(payload["error"]["message"], "Route not found");
    assert_eq!(*upstream_hits.lock().expect("mutex should lock"), 0);

    gateway_handle.abort();
    upstream_handle.abort();
}

#[tokio::test]
async fn gateway_forwards_public_request_to_remote_tunnel_owner_before_fallback_probe() {
    #[derive(Debug, Clone)]
    struct SeenOwnerRequest {
        path: String,
        body: String,
        trace_id: String,
        gateway_marker: String,
        authorization: String,
        trusted_user_id: String,
        trusted_api_key_id: String,
        trusted_access_allowed: String,
        forwarded_for: String,
        forwarded_by: String,
        owner_instance_id: String,
    }

    let fallback_probe_hits = Arc::new(Mutex::new(0usize));
    let fallback_probe_hits_clone = Arc::clone(&fallback_probe_hits);
    let fallback_probe = Router::new().route(
        "/{*path}",
        any(move |_request: Request| {
            let fallback_probe_hits_inner = Arc::clone(&fallback_probe_hits_clone);
            async move {
                *fallback_probe_hits_inner.lock().expect("mutex should lock") += 1;
                (
                    StatusCode::OK,
                    Body::from("fallback-probe-should-not-be-hit"),
                )
            }
        }),
    );

    let seen_owner = Arc::new(Mutex::new(None::<SeenOwnerRequest>));
    let seen_owner_clone = Arc::clone(&seen_owner);
    let owner = Router::new().route(
        "/v1/chat/completions",
        any(move |request: Request| {
            let seen_owner_inner = Arc::clone(&seen_owner_clone);
            async move {
                let (parts, body) = request.into_parts();
                let raw_body = to_bytes(body, usize::MAX).await.expect("body should read");
                *seen_owner_inner.lock().expect("mutex should lock") = Some(SeenOwnerRequest {
                    path: parts
                        .uri
                        .path_and_query()
                        .map(|value| value.as_str())
                        .unwrap_or("/")
                        .to_string(),
                    body: String::from_utf8(raw_body.to_vec()).expect("utf-8 body"),
                    trace_id: parts
                        .headers
                        .get(TRACE_ID_HEADER)
                        .and_then(|value| value.to_str().ok())
                        .unwrap_or_default()
                        .to_string(),
                    gateway_marker: parts
                        .headers
                        .get(GATEWAY_HEADER)
                        .and_then(|value| value.to_str().ok())
                        .unwrap_or_default()
                        .to_string(),
                    authorization: parts
                        .headers
                        .get(http::header::AUTHORIZATION)
                        .and_then(|value| value.to_str().ok())
                        .unwrap_or_default()
                        .to_string(),
                    trusted_user_id: parts
                        .headers
                        .get(TRUSTED_AUTH_USER_ID_HEADER)
                        .and_then(|value| value.to_str().ok())
                        .unwrap_or_default()
                        .to_string(),
                    trusted_api_key_id: parts
                        .headers
                        .get(TRUSTED_AUTH_API_KEY_ID_HEADER)
                        .and_then(|value| value.to_str().ok())
                        .unwrap_or_default()
                        .to_string(),
                    trusted_access_allowed: parts
                        .headers
                        .get(TRUSTED_AUTH_ACCESS_ALLOWED_HEADER)
                        .and_then(|value| value.to_str().ok())
                        .unwrap_or_default()
                        .to_string(),
                    forwarded_for: parts
                        .headers
                        .get(FORWARDED_FOR_HEADER)
                        .and_then(|value| value.to_str().ok())
                        .unwrap_or_default()
                        .to_string(),
                    forwarded_by: parts
                        .headers
                        .get(TUNNEL_AFFINITY_FORWARDED_BY_HEADER)
                        .and_then(|value| value.to_str().ok())
                        .unwrap_or_default()
                        .to_string(),
                    owner_instance_id: parts
                        .headers
                        .get(TUNNEL_AFFINITY_OWNER_INSTANCE_HEADER)
                        .and_then(|value| value.to_str().ok())
                        .unwrap_or_default()
                        .to_string(),
                });
                (
                    StatusCode::OK,
                    [(GATEWAY_HEADER, "gateway-b-owner")],
                    Body::from("owner-gateway-response"),
                )
            }
        }),
    );

    let (_unused_fallback_probe_url, fallback_probe_handle) = start_server(fallback_probe).await;
    let (owner_url, owner_handle) = start_server(owner).await;

    let provider_catalog_repository = Arc::new(InMemoryProviderCatalogReadRepository::seed(
        vec![sample_provider("provider-owner")],
        vec![sample_endpoint("endpoint-owner", "provider-owner")],
        vec![sample_key("key-owner", "provider-owner", "node-owner")],
    ));
    let auth_repository = Arc::new(InMemoryAuthApiKeySnapshotRepository::seed(vec![(
        Some(hash_api_key("sk-client-openai-affinity")),
        sample_auth_snapshot("api-key-affinity-1", "user-affinity-1", "gpt-4.1"),
    )]));
    let observed_at_unix_secs = current_unix_secs();
    let data_state =
        crate::gateway::gateway_data::GatewayDataState::with_provider_transport_reader_for_tests(
            provider_catalog_repository,
            "development-key",
        )
        .with_auth_api_key_reader(auth_repository)
        .with_system_config_values_for_tests(vec![(
            tunnel_attachment_key("node-owner"),
            serde_json::to_value(crate::gateway::tunnel::TunnelAttachmentRecord {
                gateway_instance_id: "gateway-b".to_string(),
                relay_base_url: owner_url.clone(),
                conn_count: 1,
                observed_at_unix_secs,
            })
            .expect("attachment should serialize"),
        )]);

    let mut state = AppState::new().expect("gateway state should build");
    state = state
        .with_data_state_for_tests(data_state)
        .with_tunnel_identity_for_tests("gateway-a", Some("http://gateway-a:8080"));
    state.scheduler_affinity_cache.insert(
        "scheduler_affinity:api-key-affinity-1:openai:chat:gpt-4.1".to_string(),
        crate::gateway::gateway_cache::SchedulerAffinityTarget {
            provider_id: "provider-owner".to_string(),
            endpoint_id: "endpoint-owner".to_string(),
            key_id: "key-owner".to_string(),
        },
        Duration::from_secs(300),
        100,
    );
    let gateway = build_router_with_state(state);
    let (gateway_url, gateway_handle) = start_server(gateway).await;

    let response = reqwest::Client::new()
        .post(format!("{gateway_url}/v1/chat/completions?stream=false"))
        .header(http::header::CONTENT_TYPE, "application/json")
        .header(
            http::header::AUTHORIZATION,
            "Bearer sk-client-openai-affinity",
        )
        .header(TRACE_ID_HEADER, "trace-tunnel-affinity-forward-1")
        .body("{\"model\":\"gpt-4.1\",\"messages\":[]}")
        .send()
        .await
        .expect("request should succeed");

    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(
        response
            .headers()
            .get(GATEWAY_HEADER)
            .and_then(|value| value.to_str().ok()),
        Some("gateway-b-owner")
    );
    assert_eq!(
        response
            .headers()
            .get(EXECUTION_PATH_HEADER)
            .and_then(|value| value.to_str().ok()),
        Some("tunnel_affinity_forward")
    );
    assert_eq!(
        response
            .headers()
            .get(TUNNEL_AFFINITY_OWNER_INSTANCE_HEADER)
            .and_then(|value| value.to_str().ok()),
        Some("gateway-b")
    );
    assert_eq!(
        response.text().await.expect("body should read"),
        "owner-gateway-response"
    );

    assert_eq!(*fallback_probe_hits.lock().expect("mutex should lock"), 0);
    let owner_request = seen_owner
        .lock()
        .expect("mutex should lock")
        .clone()
        .expect("owner request should be captured");
    assert_eq!(owner_request.path, "/v1/chat/completions?stream=false");
    assert_eq!(
        owner_request.body,
        "{\"model\":\"gpt-4.1\",\"messages\":[]}"
    );
    assert_eq!(owner_request.trace_id, "trace-tunnel-affinity-forward-1");
    assert_eq!(owner_request.gateway_marker, "rust-phase3b-affinity");
    assert_eq!(owner_request.authorization, "");
    assert_eq!(owner_request.trusted_user_id, "user-affinity-1");
    assert_eq!(owner_request.trusted_api_key_id, "api-key-affinity-1");
    assert_eq!(owner_request.trusted_access_allowed, "true");
    assert_eq!(owner_request.forwarded_for, "127.0.0.1");
    assert_eq!(owner_request.forwarded_by, "gateway-a");
    assert_eq!(owner_request.owner_instance_id, "gateway-b");

    gateway_handle.abort();
    owner_handle.abort();
    fallback_probe_handle.abort();
}
