use std::sync::{Arc, Mutex};

use aether_data::repository::proxy_nodes::{InMemoryProxyNodeRepository, StoredProxyNodeEvent};
use axum::body::Body;
use axum::routing::any;
use axum::{extract::Request, Router};
use http::StatusCode;
use serde_json::json;

use super::super::{build_router_with_state, sample_proxy_node, start_server, AppState};
use crate::gateway::constants::{
    GATEWAY_HEADER, TRUSTED_ADMIN_SESSION_ID_HEADER, TRUSTED_ADMIN_USER_ID_HEADER,
    TRUSTED_ADMIN_USER_ROLE_HEADER,
};
use crate::gateway::gateway_data::GatewayDataState;

#[tokio::test]
async fn gateway_handles_admin_proxy_nodes_locally_with_trusted_admin_principal() {
    let upstream_hits = Arc::new(Mutex::new(0usize));
    let upstream_hits_clone = Arc::clone(&upstream_hits);
    let upstream = Router::new().route(
        "/api/admin/proxy-nodes",
        any(move |_request: Request| {
            let upstream_hits_inner = Arc::clone(&upstream_hits_clone);
            async move {
                *upstream_hits_inner.lock().expect("mutex should lock") += 1;
                (StatusCode::OK, Body::from("unexpected upstream hit"))
            }
        }),
    );

    let mut manual_node = sample_proxy_node("proxy-node-manual");
    manual_node.name = "alpha-manual".to_string();
    manual_node.status = "online".to_string();
    manual_node.is_manual = true;
    manual_node.tunnel_mode = false;
    manual_node.tunnel_connected = false;
    manual_node.proxy_url = Some("http://proxy.example:8080".to_string());
    manual_node.proxy_username = Some("alice".to_string());
    manual_node.proxy_password = Some("supersecret".to_string());
    manual_node.last_heartbeat_at_unix_secs = None;
    manual_node.tunnel_connected_at_unix_secs = None;

    let mut tunnel_node = sample_proxy_node("proxy-node-tunnel");
    tunnel_node.name = "zeta-tunnel".to_string();
    tunnel_node.status = "offline".to_string();

    let proxy_node_repository = Arc::new(InMemoryProxyNodeRepository::seed(vec![
        tunnel_node,
        manual_node,
    ]));

    let (upstream_url, upstream_handle) = start_server(upstream).await;
    let gateway = build_router_with_state(
        AppState::new()
            .expect("gateway should build")
            .with_data_state_for_tests(GatewayDataState::with_proxy_node_repository_for_tests(
                proxy_node_repository,
            )),
    );
    let (gateway_url, gateway_handle) = start_server(gateway).await;

    let response = reqwest::Client::new()
        .get(format!(
            "{gateway_url}/api/admin/proxy-nodes?status=online&skip=0&limit=10"
        ))
        .header(crate::gateway::constants::GATEWAY_HEADER, "rust-phase3b")
        .header(TRUSTED_ADMIN_USER_ID_HEADER, "admin-user-123")
        .header(TRUSTED_ADMIN_USER_ROLE_HEADER, "admin")
        .header(TRUSTED_ADMIN_SESSION_ID_HEADER, "session-123")
        .send()
        .await
        .expect("request should succeed");

    assert_eq!(response.status(), StatusCode::OK);
    let payload: serde_json::Value = response.json().await.expect("json body should parse");
    assert_eq!(payload["total"], 1);
    assert_eq!(payload["skip"], 0);
    assert_eq!(payload["limit"], 10);

    let items = payload["items"].as_array().expect("items should be array");
    assert_eq!(items.len(), 1);
    assert_eq!(items[0]["id"], "proxy-node-manual");
    assert_eq!(items[0]["name"], "alpha-manual");
    assert_eq!(items[0]["status"], "online");
    assert_eq!(items[0]["is_manual"], true);
    assert_eq!(items[0]["proxy_url"], "http://proxy.example:8080");
    assert_eq!(items[0]["proxy_username"], "alice");
    assert_eq!(items[0]["proxy_password"], "su****et");
    assert!(items[0]["created_at"].is_string());
    assert_eq!(*upstream_hits.lock().expect("mutex should lock"), 0);

    gateway_handle.abort();
    upstream_handle.abort();
}

#[tokio::test]
async fn gateway_rejects_admin_proxy_nodes_unavailable_routes_locally() {
    let upstream_hits = Arc::new(Mutex::new(0usize));
    let upstream_hits_clone = Arc::clone(&upstream_hits);
    let upstream = Router::new().route(
        "/api/admin/proxy-nodes/register",
        any(move |_request: Request| {
            let upstream_hits_inner = Arc::clone(&upstream_hits_clone);
            async move {
                *upstream_hits_inner.lock().expect("mutex should lock") += 1;
                (StatusCode::OK, Body::from("unexpected upstream hit"))
            }
        }),
    );

    let (upstream_url, upstream_handle) = start_server(upstream).await;
    let gateway = build_router_with_state(AppState::new().expect("gateway should build"));
    let (gateway_url, gateway_handle) = start_server(gateway).await;
    let client = reqwest::Client::new();

    let register_response = client
        .post(format!("{gateway_url}/api/admin/proxy-nodes/register"))
        .header(crate::gateway::constants::GATEWAY_HEADER, "rust-phase3b")
        .header(TRUSTED_ADMIN_USER_ID_HEADER, "admin-user-123")
        .header(TRUSTED_ADMIN_USER_ROLE_HEADER, "admin")
        .header(TRUSTED_ADMIN_SESSION_ID_HEADER, "session-123")
        .json(&json!({
            "name": "proxy-1",
            "ip": "1.1.1.1",
            "port": 8080
        }))
        .send()
        .await
        .expect("request should succeed");
    assert_eq!(register_response.status(), StatusCode::SERVICE_UNAVAILABLE);
    let register_payload: serde_json::Value = register_response
        .json()
        .await
        .expect("json body should parse");
    assert_eq!(
        register_payload["detail"],
        "Admin proxy nodes data unavailable"
    );

    assert_eq!(*upstream_hits.lock().expect("mutex should lock"), 0);

    gateway_handle.abort();
    upstream_handle.abort();
}

#[tokio::test]
async fn gateway_handles_admin_proxy_node_events_locally_with_trusted_admin_principal() {
    let upstream_hits = Arc::new(Mutex::new(0usize));
    let upstream_hits_clone = Arc::clone(&upstream_hits);
    let upstream = Router::new().route(
        "/api/admin/proxy-nodes/node-1/events",
        any(move |_request: Request| {
            let upstream_hits_inner = Arc::clone(&upstream_hits_clone);
            async move {
                *upstream_hits_inner.lock().expect("mutex should lock") += 1;
                (StatusCode::OK, Body::from("unexpected upstream hit"))
            }
        }),
    );

    let proxy_node_repository = Arc::new(InMemoryProxyNodeRepository::seed_with_events(
        vec![sample_proxy_node("node-1")],
        vec![
            StoredProxyNodeEvent {
                id: 1,
                node_id: "node-1".to_string(),
                event_type: "connected".to_string(),
                detail: Some("older".to_string()),
                created_at_unix_secs: Some(1_710_000_000),
            },
            StoredProxyNodeEvent {
                id: 2,
                node_id: "node-1".to_string(),
                event_type: "disconnected".to_string(),
                detail: Some("newer".to_string()),
                created_at_unix_secs: Some(1_710_000_100),
            },
        ],
    ));

    let (upstream_url, upstream_handle) = start_server(upstream).await;
    let gateway = build_router_with_state(
        AppState::new()
            .expect("gateway should build")
            .with_data_state_for_tests(GatewayDataState::with_proxy_node_repository_for_tests(
                proxy_node_repository,
            )),
    );
    let (gateway_url, gateway_handle) = start_server(gateway).await;

    let response = reqwest::Client::new()
        .get(format!(
            "{gateway_url}/api/admin/proxy-nodes/node-1/events?limit=1"
        ))
        .header(crate::gateway::constants::GATEWAY_HEADER, "rust-phase3b")
        .header(TRUSTED_ADMIN_USER_ID_HEADER, "admin-user-123")
        .header(TRUSTED_ADMIN_USER_ROLE_HEADER, "admin")
        .header(TRUSTED_ADMIN_SESSION_ID_HEADER, "session-123")
        .send()
        .await
        .expect("request should succeed");

    assert_eq!(response.status(), StatusCode::OK);
    let payload: serde_json::Value = response.json().await.expect("json body should parse");
    let items = payload["items"].as_array().expect("items should be array");
    assert_eq!(items.len(), 1);
    assert_eq!(items[0]["id"], 2);
    assert_eq!(items[0]["event_type"], "disconnected");
    assert_eq!(items[0]["detail"], "newer");
    assert_eq!(items[0]["created_at"], "2024-03-09T16:01:40Z");
    assert_eq!(*upstream_hits.lock().expect("mutex should lock"), 0);

    gateway_handle.abort();
    upstream_handle.abort();
}
