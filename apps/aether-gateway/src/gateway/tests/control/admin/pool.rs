use std::sync::{Arc, Mutex};

use aether_crypto::DEVELOPMENT_ENCRYPTION_KEY;
use aether_data::repository::provider_catalog::{
    InMemoryProviderCatalogReadRepository, ProviderCatalogReadRepository,
};
use axum::body::Body;
use axum::routing::{any, get, post};
use axum::{extract::Request, Router};
use http::StatusCode;
use serde_json::json;

use super::super::{
    build_router_with_state, sample_endpoint, sample_key, sample_provider, start_server,
    AppState,
};
use crate::gateway::constants::{
    GATEWAY_HEADER, TRUSTED_ADMIN_SESSION_ID_HEADER, TRUSTED_ADMIN_USER_ID_HEADER,
    TRUSTED_ADMIN_USER_ROLE_HEADER,
};
use crate::gateway::gateway_data::GatewayDataState;

#[tokio::test]
async fn gateway_handles_admin_pool_overview_locally_with_trusted_admin_principal() {
    let upstream_hits = Arc::new(Mutex::new(0usize));
    let upstream_hits_clone = Arc::clone(&upstream_hits);
    let upstream = Router::new().route(
        "/api/admin/pool/overview",
        any(move |_request: Request| {
            let upstream_hits_inner = Arc::clone(&upstream_hits_clone);
            async move {
                *upstream_hits_inner.lock().expect("mutex should lock") += 1;
                (StatusCode::OK, Body::from("unexpected upstream hit"))
            }
        }),
    );

    let pool_provider = sample_provider("provider-openai", "openai", 10).with_transport_fields(
        true,
        false,
        true,
        None,
        None,
        None,
        None,
        None,
        Some(json!({
            "pool_advanced": {
                "lru_enabled": true,
                "cost_window_seconds": 7200,
                "cost_limit_per_key_tokens": 12000
            }
        })),
    );
    let regular_provider = sample_provider("provider-plain", "plain", 20);
    let mut inactive_key = sample_key("key-openai-b", "provider-openai", "openai:chat", "sk-b");
    inactive_key.name = "standby".to_string();
    inactive_key.is_active = false;
    let provider_catalog_repository = Arc::new(InMemoryProviderCatalogReadRepository::seed(
        vec![pool_provider, regular_provider],
        Vec::new(),
        vec![
            sample_key("key-openai-a", "provider-openai", "openai:chat", "sk-a"),
            inactive_key,
            sample_key("key-plain-a", "provider-plain", "openai:chat", "sk-c"),
        ],
    ));

    let (upstream_url, upstream_handle) = start_server(upstream).await;
    let gateway = build_router_with_state(
        AppState::new()
            .expect("gateway should build")
            .with_data_state_for_tests(GatewayDataState::with_provider_catalog_reader_for_tests(
                provider_catalog_repository,
            )),
    );
    let (gateway_url, gateway_handle) = start_server(gateway).await;

    let response = reqwest::Client::new()
        .get(format!("{gateway_url}/api/admin/pool/overview"))
        .header(crate::gateway::constants::GATEWAY_HEADER, "rust-phase3b")
        .header(TRUSTED_ADMIN_USER_ID_HEADER, "admin-user-123")
        .header(TRUSTED_ADMIN_USER_ROLE_HEADER, "admin")
        .header(TRUSTED_ADMIN_SESSION_ID_HEADER, "session-123")
        .send()
        .await
        .expect("request should succeed");

    assert_eq!(response.status(), StatusCode::OK);
    let payload: serde_json::Value = response.json().await.expect("json body should parse");
    let items = payload["items"]
        .as_array()
        .expect("items should be an array");
    assert_eq!(items.len(), 1);
    assert_eq!(items[0]["provider_id"], "provider-openai");
    assert_eq!(items[0]["provider_name"], "openai");
    assert_eq!(items[0]["provider_type"], "custom");
    assert_eq!(items[0]["total_keys"], 2);
    assert_eq!(items[0]["active_keys"], 1);
    assert_eq!(items[0]["cooldown_count"], 0);
    assert_eq!(items[0]["pool_enabled"], true);
    assert_eq!(*upstream_hits.lock().expect("mutex should lock"), 0);

    gateway_handle.abort();
    upstream_handle.abort();
}

#[tokio::test]
async fn gateway_handles_admin_pool_scheduling_presets_locally_with_trusted_admin_principal() {
    let upstream_hits = Arc::new(Mutex::new(0usize));
    let upstream_hits_clone = Arc::clone(&upstream_hits);
    let upstream = Router::new().route(
        "/api/admin/pool/scheduling-presets",
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

    let response = reqwest::Client::new()
        .get(format!("{gateway_url}/api/admin/pool/scheduling-presets"))
        .header(crate::gateway::constants::GATEWAY_HEADER, "rust-phase3b")
        .header(TRUSTED_ADMIN_USER_ID_HEADER, "admin-user-123")
        .header(TRUSTED_ADMIN_USER_ROLE_HEADER, "admin")
        .header(TRUSTED_ADMIN_SESSION_ID_HEADER, "session-123")
        .send()
        .await
        .expect("request should succeed");

    assert_eq!(response.status(), StatusCode::OK);
    let payload: serde_json::Value = response.json().await.expect("json body should parse");
    let items = payload.as_array().expect("payload should be an array");
    assert_eq!(items.len(), 13);
    assert_eq!(items[0]["name"], "lru");
    assert_eq!(items[1]["name"], "cache_affinity");
    assert_eq!(items[12]["name"], "team_first");
    assert_eq!(*upstream_hits.lock().expect("mutex should lock"), 0);

    gateway_handle.abort();
    upstream_handle.abort();
}

#[tokio::test]
async fn gateway_handles_admin_pool_batch_import_locally_with_trusted_admin_principal() {
    let upstream_hits = Arc::new(Mutex::new(0usize));
    let upstream_hits_clone = Arc::clone(&upstream_hits);
    let upstream = Router::new().route(
        "/api/admin/pool/provider-openai/keys/batch-import",
        any(move |_request: Request| {
            let upstream_hits_inner = Arc::clone(&upstream_hits_clone);
            async move {
                *upstream_hits_inner.lock().expect("mutex should lock") += 1;
                (StatusCode::OK, Body::from("unexpected upstream hit"))
            }
        }),
    );

    let provider_catalog_repository = Arc::new(InMemoryProviderCatalogReadRepository::seed(
        vec![sample_provider("provider-openai", "openai", 10)],
        vec![sample_endpoint(
            "endpoint-openai-chat",
            "provider-openai",
            "openai:chat",
            "https://api.openai.com/v1",
        )],
        vec![],
    ));

    let (upstream_url, upstream_handle) = start_server(upstream).await;
    let gateway = build_router_with_state(
        AppState::new()
            .expect("gateway should build")
            .with_data_state_for_tests(
                GatewayDataState::with_provider_catalog_repository_for_tests(
                    provider_catalog_repository.clone(),
                )
                .with_encryption_key_for_tests(DEVELOPMENT_ENCRYPTION_KEY),
            ),
    );
    let (gateway_url, gateway_handle) = start_server(gateway).await;

    let response = reqwest::Client::new()
        .post(format!(
            "{gateway_url}/api/admin/pool/provider-openai/keys/batch-import"
        ))
        .header(crate::gateway::constants::GATEWAY_HEADER, "rust-phase3b")
        .header(TRUSTED_ADMIN_USER_ID_HEADER, "admin-user-123")
        .header(TRUSTED_ADMIN_USER_ROLE_HEADER, "admin")
        .header(TRUSTED_ADMIN_SESSION_ID_HEADER, "session-123")
        .json(&json!({
            "proxy_node_id": "proxy-node-1",
            "keys": [
                {
                    "name": "batch key a",
                    "api_key": "sk-batch-a"
                },
                {
                    "name": "batch key b",
                    "api_key": ""
                }
            ]
        }))
        .send()
        .await
        .expect("request should succeed");

    assert_eq!(response.status(), StatusCode::OK);
    let payload: serde_json::Value = response.json().await.expect("json body should parse");
    assert_eq!(payload["imported"], 1);
    assert_eq!(payload["skipped"], 0);
    let errors = payload["errors"]
        .as_array()
        .expect("errors should be an array");
    assert_eq!(errors.len(), 1);
    assert_eq!(errors[0]["index"], 1);
    assert_eq!(errors[0]["reason"], "api_key is empty");
    assert_eq!(*upstream_hits.lock().expect("mutex should lock"), 0);

    let keys = provider_catalog_repository
        .list_keys_by_provider_ids(&["provider-openai".to_string()])
        .await
        .expect("keys should read");
    assert_eq!(keys.len(), 1);
    assert_eq!(keys[0].name, "batch key a");
    assert_eq!(keys[0].auth_type, "api_key");
    assert_eq!(keys[0].api_formats, Some(json!(["openai:chat"])));
    assert_eq!(
        keys[0].proxy,
        Some(json!({"node_id": "proxy-node-1", "enabled": true}))
    );

    gateway_handle.abort();
    upstream_handle.abort();
}

#[tokio::test]
async fn gateway_handles_admin_pool_trailing_slash_routes_locally_with_trusted_admin_principal() {
    let upstream_hits = Arc::new(Mutex::new(0usize));
    let upstream_hits_clone = Arc::clone(&upstream_hits);
    let upstream = Router::new()
        .route(
            "/api/admin/pool/provider-openai/keys",
            any({
                let upstream_hits_inner = Arc::clone(&upstream_hits_clone);
                move |_request: Request| {
                    let upstream_hits_inner = Arc::clone(&upstream_hits_inner);
                    async move {
                        *upstream_hits_inner.lock().expect("mutex should lock") += 1;
                        (StatusCode::OK, Body::from("unexpected upstream hit"))
                    }
                }
            }),
        )
        .route(
            "/api/admin/pool/provider-openai/keys/resolve-selection",
            any({
                let upstream_hits_inner = Arc::clone(&upstream_hits_clone);
                move |_request: Request| {
                    let upstream_hits_inner = Arc::clone(&upstream_hits_inner);
                    async move {
                        *upstream_hits_inner.lock().expect("mutex should lock") += 1;
                        (StatusCode::OK, Body::from("unexpected upstream hit"))
                    }
                }
            }),
        );

    let provider = sample_provider("provider-openai", "openai", 10).with_transport_fields(
        true,
        false,
        true,
        None,
        None,
        None,
        None,
        None,
        Some(json!({
            "pool_advanced": {
                "enabled": true
            }
        })),
    );
    let mut first_key = sample_key("key-openai-a", "provider-openai", "openai:chat", "sk-a");
    first_key.name = "alpha".to_string();
    let mut second_key = sample_key("key-openai-b", "provider-openai", "openai:chat", "sk-b");
    second_key.name = "beta".to_string();
    let provider_catalog_repository = Arc::new(InMemoryProviderCatalogReadRepository::seed(
        vec![provider],
        Vec::new(),
        vec![first_key, second_key],
    ));

    let (upstream_url, upstream_handle) = start_server(upstream).await;
    let gateway = build_router_with_state(
        AppState::new()
            .expect("gateway should build")
            .with_data_state_for_tests(
                GatewayDataState::with_provider_catalog_repository_for_tests(Arc::clone(
                    &provider_catalog_repository,
                )),
            ),
    );
    let (gateway_url, gateway_handle) = start_server(gateway).await;

    let list_response = reqwest::Client::new()
        .get(format!(
            "{gateway_url}/api/admin/pool/provider-openai/keys/?page=1&page_size=10"
        ))
        .header(crate::gateway::constants::GATEWAY_HEADER, "rust-phase3b")
        .header(TRUSTED_ADMIN_USER_ID_HEADER, "admin-user-123")
        .header(TRUSTED_ADMIN_USER_ROLE_HEADER, "admin")
        .header(TRUSTED_ADMIN_SESSION_ID_HEADER, "session-123")
        .send()
        .await
        .expect("request should succeed");
    assert_eq!(list_response.status(), StatusCode::OK);
    let list_payload: serde_json::Value =
        list_response.json().await.expect("json body should parse");
    assert_eq!(
        list_payload["keys"].as_array().map(|items| items.len()),
        Some(2)
    );

    let resolve_response = reqwest::Client::new()
        .post(format!(
            "{gateway_url}/api/admin/pool/provider-openai/keys/resolve-selection/"
        ))
        .header(crate::gateway::constants::GATEWAY_HEADER, "rust-phase3b")
        .header(TRUSTED_ADMIN_USER_ID_HEADER, "admin-user-123")
        .header(TRUSTED_ADMIN_USER_ROLE_HEADER, "admin")
        .header(TRUSTED_ADMIN_SESSION_ID_HEADER, "session-123")
        .json(&json!({
            "search": "a",
            "quick_selectors": []
        }))
        .send()
        .await
        .expect("request should succeed");
    assert_eq!(resolve_response.status(), StatusCode::OK);
    let resolve_payload: serde_json::Value = resolve_response
        .json()
        .await
        .expect("json body should parse");
    assert_eq!(resolve_payload["total"], json!(2));
    assert_eq!(*upstream_hits.lock().expect("mutex should lock"), 0);

    gateway_handle.abort();
    upstream_handle.abort();
}

#[tokio::test]
async fn gateway_handles_admin_pool_list_keys_locally_with_trusted_admin_principal() {
    let upstream_hits = Arc::new(Mutex::new(0usize));
    let upstream_hits_clone = Arc::clone(&upstream_hits);
    let upstream = Router::new().route(
        "/api/admin/pool/provider-openai/keys",
        any(move |_request: Request| {
            let upstream_hits_inner = Arc::clone(&upstream_hits_clone);
            async move {
                *upstream_hits_inner.lock().expect("mutex should lock") += 1;
                (StatusCode::OK, Body::from("unexpected upstream hit"))
            }
        }),
    );

    let provider = sample_provider("provider-openai", "openai", 10).with_transport_fields(
        true,
        false,
        true,
        None,
        None,
        None,
        None,
        None,
        Some(json!({
            "pool_advanced": {
                "enabled": true,
                "cost_limit_per_key_tokens": 12000
            }
        })),
    );
    let mut primary_key = sample_key("key-openai-a", "provider-openai", "openai:chat", "sk-a");
    primary_key.name = "alpha".to_string();
    let mut cooldown_key = sample_key("key-openai-b", "provider-openai", "openai:chat", "sk-b");
    cooldown_key.name = "beta".to_string();
    cooldown_key.status_snapshot = Some(json!({
        "account": {
            "code": "cooldown",
            "label": "冷却中",
            "blocked": false
        }
    }));
    let mut inactive_key = sample_key("key-openai-c", "provider-openai", "openai:chat", "sk-c");
    inactive_key.name = "gamma".to_string();
    inactive_key.is_active = false;
    let provider_catalog_repository = Arc::new(InMemoryProviderCatalogReadRepository::seed(
        vec![provider],
        Vec::new(),
        vec![primary_key, cooldown_key, inactive_key],
    ));

    let (upstream_url, upstream_handle) = start_server(upstream).await;
    let gateway = build_router_with_state(
        AppState::new()
            .expect("gateway should build")
            .with_data_state_for_tests(GatewayDataState::with_provider_catalog_reader_for_tests(
                provider_catalog_repository,
            )),
    );
    let (gateway_url, gateway_handle) = start_server(gateway).await;

    let response = reqwest::Client::new()
        .get(format!(
            "{gateway_url}/api/admin/pool/provider-openai/keys?page=1&page_size=2&search=a&status=all"
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
    assert_eq!(payload["total"], json!(3));
    assert_eq!(payload["page"], json!(1));
    assert_eq!(payload["page_size"], json!(2));
    let keys = payload["keys"].as_array().expect("keys should be array");
    assert_eq!(keys.len(), 2);
    assert_eq!(keys[0]["key_name"], json!("alpha"));
    assert_eq!(keys[0]["scheduling_status"], json!("available"));
    assert_eq!(keys[1]["key_name"], json!("beta"));
    assert_eq!(keys[1]["scheduling_reason"], json!("available"));

    let page_two_response = reqwest::Client::new()
        .get(format!(
            "{gateway_url}/api/admin/pool/provider-openai/keys?page=2&page_size=2&search=a&status=all"
        ))
        .header(crate::gateway::constants::GATEWAY_HEADER, "rust-phase3b")
        .header(TRUSTED_ADMIN_USER_ID_HEADER, "admin-user-123")
        .header(TRUSTED_ADMIN_USER_ROLE_HEADER, "admin")
        .header(TRUSTED_ADMIN_SESSION_ID_HEADER, "session-123")
        .send()
        .await
        .expect("request should succeed");
    assert_eq!(page_two_response.status(), StatusCode::OK);
    let page_two_payload: serde_json::Value = page_two_response
        .json()
        .await
        .expect("json body should parse");
    assert_eq!(page_two_payload["total"], json!(3));
    let page_two_keys = page_two_payload["keys"]
        .as_array()
        .expect("keys should be array");
    assert_eq!(page_two_keys.len(), 1);
    assert_eq!(page_two_keys[0]["key_name"], json!("gamma"));

    let active_response = reqwest::Client::new()
        .get(format!(
            "{gateway_url}/api/admin/pool/provider-openai/keys?page=1&page_size=10&search=a&status=active"
        ))
        .header(crate::gateway::constants::GATEWAY_HEADER, "rust-phase3b")
        .header(TRUSTED_ADMIN_USER_ID_HEADER, "admin-user-123")
        .header(TRUSTED_ADMIN_USER_ROLE_HEADER, "admin")
        .header(TRUSTED_ADMIN_SESSION_ID_HEADER, "session-123")
        .send()
        .await
        .expect("request should succeed");
    assert_eq!(active_response.status(), StatusCode::OK);
    let active_payload: serde_json::Value = active_response
        .json()
        .await
        .expect("json body should parse");
    assert_eq!(active_payload["total"], json!(2));
    assert_eq!(
        active_payload["keys"].as_array().map(|items| items.len()),
        Some(2)
    );

    let inactive_response = reqwest::Client::new()
        .get(format!(
            "{gateway_url}/api/admin/pool/provider-openai/keys?page=1&page_size=10&search=a&status=inactive"
        ))
        .header(crate::gateway::constants::GATEWAY_HEADER, "rust-phase3b")
        .header(TRUSTED_ADMIN_USER_ID_HEADER, "admin-user-123")
        .header(TRUSTED_ADMIN_USER_ROLE_HEADER, "admin")
        .header(TRUSTED_ADMIN_SESSION_ID_HEADER, "session-123")
        .send()
        .await
        .expect("request should succeed");
    assert_eq!(inactive_response.status(), StatusCode::OK);
    let inactive_payload: serde_json::Value = inactive_response
        .json()
        .await
        .expect("json body should parse");
    assert_eq!(inactive_payload["total"], json!(1));
    let inactive_keys = inactive_payload["keys"]
        .as_array()
        .expect("keys should be array");
    assert_eq!(inactive_keys.len(), 1);
    assert_eq!(inactive_keys[0]["key_name"], json!("gamma"));
    assert_eq!(*upstream_hits.lock().expect("mutex should lock"), 0);

    gateway_handle.abort();
    upstream_handle.abort();
}

#[tokio::test]
async fn gateway_handles_admin_pool_resolve_selection_locally_with_trusted_admin_principal() {
    let upstream_hits = Arc::new(Mutex::new(0usize));
    let upstream_hits_clone = Arc::clone(&upstream_hits);
    let upstream = Router::new().route(
        "/api/admin/pool/provider-openai/keys/resolve-selection",
        any(move |_request: Request| {
            let upstream_hits_inner = Arc::clone(&upstream_hits_clone);
            async move {
                *upstream_hits_inner.lock().expect("mutex should lock") += 1;
                (StatusCode::OK, Body::from("unexpected upstream hit"))
            }
        }),
    );

    let provider = sample_provider("provider-openai", "openai", 10).with_transport_fields(
        true,
        false,
        true,
        None,
        None,
        None,
        None,
        None,
        Some(json!({
            "pool_advanced": {
                "enabled": true
            }
        })),
    );
    let mut proxy_key = sample_key("key-openai-a", "provider-openai", "openai:chat", "sk-a");
    proxy_key.name = "alpha proxy".to_string();
    proxy_key.proxy = Some(json!({
        "mode": "direct",
        "url": "https://proxy.example.com"
    }));
    let mut plain_key = sample_key("key-openai-b", "provider-openai", "openai:chat", "sk-b");
    plain_key.name = "beta".to_string();
    let mut disabled_proxy_key =
        sample_key("key-openai-c", "provider-openai", "openai:chat", "sk-c");
    disabled_proxy_key.name = "alpha disabled".to_string();
    disabled_proxy_key.is_active = false;
    disabled_proxy_key.proxy = Some(json!({
        "mode": "direct",
        "url": "https://proxy-disabled.example.com"
    }));
    let provider_catalog_repository = Arc::new(InMemoryProviderCatalogReadRepository::seed(
        vec![provider],
        Vec::new(),
        vec![proxy_key, plain_key, disabled_proxy_key],
    ));

    let (upstream_url, upstream_handle) = start_server(upstream).await;
    let gateway = build_router_with_state(
        AppState::new()
            .expect("gateway should build")
            .with_data_state_for_tests(GatewayDataState::with_provider_catalog_reader_for_tests(
                provider_catalog_repository,
            )),
    );
    let (gateway_url, gateway_handle) = start_server(gateway).await;

    let response = reqwest::Client::new()
        .post(format!(
            "{gateway_url}/api/admin/pool/provider-openai/keys/resolve-selection"
        ))
        .header(crate::gateway::constants::GATEWAY_HEADER, "rust-phase3b")
        .header(TRUSTED_ADMIN_USER_ID_HEADER, "admin-user-123")
        .header(TRUSTED_ADMIN_USER_ROLE_HEADER, "admin")
        .header(TRUSTED_ADMIN_SESSION_ID_HEADER, "session-123")
        .json(&json!({
            "search": "alpha",
            "quick_selectors": ["enabled", "proxy_set"]
        }))
        .send()
        .await
        .expect("request should succeed");

    assert_eq!(response.status(), StatusCode::OK);
    let payload: serde_json::Value = response.json().await.expect("json body should parse");
    assert_eq!(payload["total"], json!(1));
    let items = payload["items"].as_array().expect("items should be array");
    assert_eq!(items.len(), 1);
    assert_eq!(items[0]["key_id"], json!("key-openai-a"));
    assert_eq!(items[0]["key_name"], json!("alpha proxy"));
    assert_eq!(items[0]["auth_type"], json!("api_key"));
    assert_eq!(*upstream_hits.lock().expect("mutex should lock"), 0);

    gateway_handle.abort();
    upstream_handle.abort();
}

#[tokio::test]
async fn gateway_handles_admin_pool_batch_action_locally_with_trusted_admin_principal() {
    let upstream_hits = Arc::new(Mutex::new(0usize));
    let upstream_hits_clone = Arc::clone(&upstream_hits);
    let upstream = Router::new().route(
        "/api/admin/pool/provider-openai/keys/batch-action",
        any(move |_request: Request| {
            let upstream_hits_inner = Arc::clone(&upstream_hits_clone);
            async move {
                *upstream_hits_inner.lock().expect("mutex should lock") += 1;
                (StatusCode::OK, Body::from("unexpected upstream hit"))
            }
        }),
    );

    let provider = sample_provider("provider-openai", "openai", 10).with_transport_fields(
        true,
        false,
        true,
        None,
        None,
        None,
        None,
        None,
        Some(json!({
            "pool_advanced": {
                "enabled": true
            }
        })),
    );
    let mut first_key = sample_key("key-openai-a", "provider-openai", "openai:chat", "sk-a");
    first_key.name = "alpha".to_string();
    let mut second_key = sample_key("key-openai-b", "provider-openai", "openai:chat", "sk-b");
    second_key.name = "beta".to_string();
    let provider_catalog_repository = Arc::new(InMemoryProviderCatalogReadRepository::seed(
        vec![provider],
        Vec::new(),
        vec![first_key, second_key],
    ));

    let (upstream_url, upstream_handle) = start_server(upstream).await;
    let gateway = build_router_with_state(
        AppState::new()
            .expect("gateway should build")
            .with_data_state_for_tests(
                GatewayDataState::with_provider_catalog_repository_for_tests(Arc::clone(
                    &provider_catalog_repository,
                )),
            ),
    );
    let (gateway_url, gateway_handle) = start_server(gateway).await;

    let response = reqwest::Client::new()
        .post(format!(
            "{gateway_url}/api/admin/pool/provider-openai/keys/batch-action"
        ))
        .header(crate::gateway::constants::GATEWAY_HEADER, "rust-phase3b")
        .header(TRUSTED_ADMIN_USER_ID_HEADER, "admin-user-123")
        .header(TRUSTED_ADMIN_USER_ROLE_HEADER, "admin")
        .header(TRUSTED_ADMIN_SESSION_ID_HEADER, "session-123")
        .json(&json!({
            "action": "disable",
            "key_ids": ["key-openai-a", "key-openai-b"]
        }))
        .send()
        .await
        .expect("request should succeed");

    assert_eq!(response.status(), StatusCode::OK);
    let payload: serde_json::Value = response.json().await.expect("json body should parse");
    assert_eq!(payload["affected"], json!(2));
    assert_eq!(payload["message"], json!("2 keys disabled"));
    let stored = provider_catalog_repository
        .list_keys_by_ids(&["key-openai-a".to_string(), "key-openai-b".to_string()])
        .await
        .expect("keys should load");
    assert_eq!(stored.len(), 2);
    assert!(stored.iter().all(|item| !item.is_active));
    assert_eq!(*upstream_hits.lock().expect("mutex should lock"), 0);

    gateway_handle.abort();
    upstream_handle.abort();
}

#[tokio::test]
async fn gateway_handles_admin_pool_batch_delete_locally_with_trusted_admin_principal() {
    let upstream_hits = Arc::new(Mutex::new(0usize));
    let upstream_hits_clone = Arc::clone(&upstream_hits);
    let upstream = Router::new().route(
        "/api/admin/pool/provider-openai/keys/batch-action",
        any(move |_request: Request| {
            let upstream_hits_inner = Arc::clone(&upstream_hits_clone);
            async move {
                *upstream_hits_inner.lock().expect("mutex should lock") += 1;
                (StatusCode::OK, Body::from("unexpected upstream hit"))
            }
        }),
    );

    let provider = sample_provider("provider-openai", "openai", 10).with_transport_fields(
        true,
        false,
        true,
        None,
        None,
        None,
        None,
        None,
        Some(json!({
            "pool_advanced": {
                "enabled": true
            }
        })),
    );
    let mut first_key = sample_key("key-openai-a", "provider-openai", "openai:chat", "sk-a");
    first_key.name = "alpha".to_string();
    let mut second_key = sample_key("key-openai-b", "provider-openai", "openai:chat", "sk-b");
    second_key.name = "beta".to_string();
    let provider_catalog_repository = Arc::new(InMemoryProviderCatalogReadRepository::seed(
        vec![provider],
        Vec::new(),
        vec![first_key, second_key],
    ));

    let (upstream_url, upstream_handle) = start_server(upstream).await;
    let gateway = build_router_with_state(
        AppState::new()
            .expect("gateway should build")
            .with_data_state_for_tests(
                GatewayDataState::with_provider_catalog_repository_for_tests(Arc::clone(
                    &provider_catalog_repository,
                )),
            ),
    );
    let (gateway_url, gateway_handle) = start_server(gateway).await;

    let response = reqwest::Client::new()
        .post(format!(
            "{gateway_url}/api/admin/pool/provider-openai/keys/batch-action"
        ))
        .header(crate::gateway::constants::GATEWAY_HEADER, "rust-phase3b")
        .header(TRUSTED_ADMIN_USER_ID_HEADER, "admin-user-123")
        .header(TRUSTED_ADMIN_USER_ROLE_HEADER, "admin")
        .header(TRUSTED_ADMIN_SESSION_ID_HEADER, "session-123")
        .json(&json!({
            "action": "delete",
            "key_ids": ["key-openai-a", "key-openai-b"]
        }))
        .send()
        .await
        .expect("request should succeed");

    assert_eq!(response.status(), StatusCode::OK);
    let payload: serde_json::Value = response.json().await.expect("json body should parse");
    assert_eq!(payload["affected"], json!(2));
    assert_eq!(payload["message"], json!("2 keys deleted"));
    let stored = provider_catalog_repository
        .list_keys_by_ids(&["key-openai-a".to_string(), "key-openai-b".to_string()])
        .await
        .expect("keys should load");
    assert!(stored.is_empty());
    assert_eq!(*upstream_hits.lock().expect("mutex should lock"), 0);

    gateway_handle.abort();
    upstream_handle.abort();
}

#[tokio::test]
async fn gateway_handles_admin_pool_batch_delete_task_status_locally_with_trusted_admin_principal()
{
    let upstream_hits = Arc::new(Mutex::new(0usize));
    let upstream_hits_clone = Arc::clone(&upstream_hits);
    let upstream = Router::new().route(
        "/api/admin/pool/provider-openai/keys/batch-delete-task/task-123",
        any(move |_request: Request| {
            let upstream_hits_inner = Arc::clone(&upstream_hits_clone);
            async move {
                *upstream_hits_inner.lock().expect("mutex should lock") += 1;
                (StatusCode::OK, Body::from("unexpected upstream hit"))
            }
        }),
    );

    let (upstream_url, upstream_handle) = start_server(upstream).await;
    let mut state = AppState::new().expect("gateway should build");
    state.put_provider_delete_task(crate::gateway::LocalProviderDeleteTaskState {
        task_id: "task-123".to_string(),
        provider_id: "provider-openai".to_string(),
        status: "running".to_string(),
        stage: "deleting_keys".to_string(),
        total_keys: 3,
        deleted_keys: 1,
        total_endpoints: 2,
        deleted_endpoints: 0,
        message: "deleted 1 / 3 keys".to_string(),
    });
    let gateway = build_router_with_state(state);
    let (gateway_url, gateway_handle) = start_server(gateway).await;

    let response = reqwest::Client::new()
        .get(format!(
            "{gateway_url}/api/admin/pool/provider-openai/keys/batch-delete-task/task-123"
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
    assert_eq!(payload["task_id"], json!("task-123"));
    assert_eq!(payload["provider_id"], json!("provider-openai"));
    assert_eq!(payload["status"], json!("running"));
    assert_eq!(payload["stage"], json!("deleting_keys"));
    assert_eq!(payload["total_keys"], json!(3));
    assert_eq!(payload["deleted_keys"], json!(1));
    assert_eq!(payload["total_endpoints"], json!(2));
    assert_eq!(payload["deleted_endpoints"], json!(0));
    assert_eq!(payload["message"], json!("deleted 1 / 3 keys"));
    assert_eq!(*upstream_hits.lock().expect("mutex should lock"), 0);

    gateway_handle.abort();
    upstream_handle.abort();
}

#[tokio::test]
async fn gateway_cleans_up_admin_pool_banned_keys_locally_with_trusted_admin_principal() {
    let upstream_hits = Arc::new(Mutex::new(0usize));
    let upstream_hits_clone = Arc::clone(&upstream_hits);
    let upstream = Router::new().route(
        "/api/admin/pool/provider-openai/keys/cleanup-banned",
        any(move |_request: Request| {
            let upstream_hits_inner = Arc::clone(&upstream_hits_clone);
            async move {
                *upstream_hits_inner.lock().expect("mutex should lock") += 1;
                (StatusCode::OK, Body::from("unexpected upstream hit"))
            }
        }),
    );

    let provider = sample_provider("provider-openai", "openai", 10).with_transport_fields(
        true,
        false,
        true,
        None,
        None,
        None,
        None,
        None,
        Some(json!({
            "pool_advanced": {
                "enabled": true
            }
        })),
    );
    let mut banned_key = sample_key(
        "key-openai-banned",
        "provider-openai",
        "openai:chat",
        "sk-banned",
    );
    banned_key.name = "banned".to_string();
    banned_key.oauth_invalid_reason = Some("account_banned".to_string());
    let mut healthy_key = sample_key(
        "key-openai-healthy",
        "provider-openai",
        "openai:chat",
        "sk-healthy",
    );
    healthy_key.name = "healthy".to_string();

    let provider_catalog_repository = Arc::new(InMemoryProviderCatalogReadRepository::seed(
        vec![provider],
        Vec::new(),
        vec![banned_key, healthy_key],
    ));

    let (upstream_url, upstream_handle) = start_server(upstream).await;
    let gateway = build_router_with_state(
        AppState::new()
            .expect("gateway should build")
            .with_data_state_for_tests(
                GatewayDataState::with_provider_catalog_repository_for_tests(
                    provider_catalog_repository.clone(),
                ),
            ),
    );
    let (gateway_url, gateway_handle) = start_server(gateway).await;

    let response = reqwest::Client::new()
        .post(format!(
            "{gateway_url}/api/admin/pool/provider-openai/keys/cleanup-banned"
        ))
        .header(crate::gateway::constants::GATEWAY_HEADER, "rust-phase3b")
        .header(TRUSTED_ADMIN_USER_ID_HEADER, "admin-user-123")
        .header(TRUSTED_ADMIN_USER_ROLE_HEADER, "admin")
        .header(TRUSTED_ADMIN_SESSION_ID_HEADER, "session-123")
        .json(&json!({}))
        .send()
        .await
        .expect("request should succeed");

    assert_eq!(response.status(), StatusCode::OK);
    let payload: serde_json::Value = response.json().await.expect("json body should parse");
    assert_eq!(payload["affected"], 1);
    assert_eq!(payload["message"], "已清理 1 个异常账号");

    let remaining_keys = provider_catalog_repository
        .list_keys_by_provider_ids(&["provider-openai".to_string()])
        .await
        .expect("remaining keys should load");
    assert_eq!(remaining_keys.len(), 1);
    assert_eq!(remaining_keys[0].id, "key-openai-healthy");
    assert_eq!(*upstream_hits.lock().expect("mutex should lock"), 0);

    gateway_handle.abort();
    upstream_handle.abort();
}

#[tokio::test]
async fn gateway_rejects_admin_pool_list_keys_with_empty_provider_id() {
    let upstream_hits = Arc::new(Mutex::new(0usize));
    let upstream_hits_clone = Arc::clone(&upstream_hits);
    let upstream = Router::new().route(
        "/api/admin/pool//keys",
        any(move |_request: Request| {
            let upstream_hits_inner = Arc::clone(&upstream_hits_clone);
            async move {
                *upstream_hits_inner.lock().expect("mutex should lock") += 1;
                (StatusCode::OK, Body::from("unexpected upstream hit"))
            }
        }),
    );

    let provider_catalog_repository = Arc::new(InMemoryProviderCatalogReadRepository::seed(
        Vec::new(),
        Vec::new(),
        Vec::new(),
    ));
    let (upstream_url, upstream_handle) = start_server(upstream).await;
    let gateway = build_router_with_state(
        AppState::new()
            .expect("gateway should build")
            .with_data_state_for_tests(GatewayDataState::with_provider_catalog_reader_for_tests(
                provider_catalog_repository,
            )),
    );
    let (gateway_url, gateway_handle) = start_server(gateway).await;

    let response = reqwest::Client::new()
        .get(format!("{gateway_url}/api/admin/pool//keys"))
        .header(crate::gateway::constants::GATEWAY_HEADER, "rust-phase3b")
        .header(TRUSTED_ADMIN_USER_ID_HEADER, "admin-user-123")
        .header(TRUSTED_ADMIN_USER_ROLE_HEADER, "admin")
        .header(TRUSTED_ADMIN_SESSION_ID_HEADER, "session-123")
        .send()
        .await
        .expect("request should succeed");

    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    let payload: serde_json::Value = response.json().await.expect("json body should parse");
    assert_eq!(payload["detail"], "provider_id 无效");
    assert_eq!(*upstream_hits.lock().expect("mutex should lock"), 0);

    gateway_handle.abort();
    upstream_handle.abort();
}

#[tokio::test]
async fn gateway_rejects_admin_pool_batch_import_with_empty_provider_id() {
    let upstream_hits = Arc::new(Mutex::new(0usize));
    let upstream_hits_clone = Arc::clone(&upstream_hits);
    let upstream = Router::new().route(
        "/api/admin/pool//keys/batch-import",
        any(move |_request: Request| {
            let upstream_hits_inner = Arc::clone(&upstream_hits_clone);
            async move {
                *upstream_hits_inner.lock().expect("mutex should lock") += 1;
                (StatusCode::OK, Body::from("unexpected upstream hit"))
            }
        }),
    );

    let provider_catalog_repository = Arc::new(InMemoryProviderCatalogReadRepository::seed(
        Vec::new(),
        Vec::new(),
        Vec::new(),
    ));
    let (upstream_url, upstream_handle) = start_server(upstream).await;
    let gateway = build_router_with_state(
        AppState::new()
            .expect("gateway should build")
            .with_data_state_for_tests(
                GatewayDataState::with_provider_catalog_repository_for_tests(
                    provider_catalog_repository,
                )
                .with_encryption_key_for_tests(DEVELOPMENT_ENCRYPTION_KEY),
            ),
    );
    let (gateway_url, gateway_handle) = start_server(gateway).await;

    let response = reqwest::Client::new()
        .post(format!("{gateway_url}/api/admin/pool//keys/batch-import"))
        .header(crate::gateway::constants::GATEWAY_HEADER, "rust-phase3b")
        .header(TRUSTED_ADMIN_USER_ID_HEADER, "admin-user-123")
        .header(TRUSTED_ADMIN_USER_ROLE_HEADER, "admin")
        .header(TRUSTED_ADMIN_SESSION_ID_HEADER, "session-123")
        .json(&json!({ "keys": [] }))
        .send()
        .await
        .expect("request should succeed");

    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    let payload: serde_json::Value = response.json().await.expect("json body should parse");
    assert_eq!(payload["detail"], "provider_id 无效");
    assert_eq!(*upstream_hits.lock().expect("mutex should lock"), 0);

    gateway_handle.abort();
    upstream_handle.abort();
}

#[tokio::test]
async fn gateway_rejects_admin_pool_cleanup_banned_with_empty_provider_id() {
    let upstream_hits = Arc::new(Mutex::new(0usize));
    let upstream_hits_clone = Arc::clone(&upstream_hits);
    let upstream = Router::new().route(
        "/api/admin/pool//keys/cleanup-banned",
        any(move |_request: Request| {
            let upstream_hits_inner = Arc::clone(&upstream_hits_clone);
            async move {
                *upstream_hits_inner.lock().expect("mutex should lock") += 1;
                (StatusCode::OK, Body::from("unexpected upstream hit"))
            }
        }),
    );

    let provider_catalog_repository = Arc::new(InMemoryProviderCatalogReadRepository::seed(
        Vec::new(),
        Vec::new(),
        Vec::new(),
    ));
    let (upstream_url, upstream_handle) = start_server(upstream).await;
    let gateway = build_router_with_state(
        AppState::new()
            .expect("gateway should build")
            .with_data_state_for_tests(
                GatewayDataState::with_provider_catalog_repository_for_tests(
                    provider_catalog_repository,
                ),
            ),
    );
    let (gateway_url, gateway_handle) = start_server(gateway).await;

    let response = reqwest::Client::new()
        .post(format!("{gateway_url}/api/admin/pool//keys/cleanup-banned"))
        .header(crate::gateway::constants::GATEWAY_HEADER, "rust-phase3b")
        .header(TRUSTED_ADMIN_USER_ID_HEADER, "admin-user-123")
        .header(TRUSTED_ADMIN_USER_ROLE_HEADER, "admin")
        .header(TRUSTED_ADMIN_SESSION_ID_HEADER, "session-123")
        .json(&json!({}))
        .send()
        .await
        .expect("request should succeed");

    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    let payload: serde_json::Value = response.json().await.expect("json body should parse");
    assert_eq!(payload["detail"], "provider_id 无效");
    assert_eq!(*upstream_hits.lock().expect("mutex should lock"), 0);

    gateway_handle.abort();
    upstream_handle.abort();
}
