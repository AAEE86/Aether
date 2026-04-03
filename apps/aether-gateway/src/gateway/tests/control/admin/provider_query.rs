use std::sync::{Arc, Mutex};

use aether_data::repository::global_models::InMemoryGlobalModelReadRepository;
use aether_data::repository::provider_catalog::{
    InMemoryProviderCatalogReadRepository, StoredProviderCatalogEndpoint,
};
use axum::body::Body;
use axum::routing::any;
use axum::{extract::Request, Router};
use http::StatusCode;
use serde_json::json;

use super::super::{
    build_router_with_state, sample_admin_global_model, sample_admin_provider_model, sample_key,
    sample_provider, start_server, AppState,
};
use crate::gateway::constants::{
    GATEWAY_HEADER, TRUSTED_ADMIN_SESSION_ID_HEADER, TRUSTED_ADMIN_USER_ID_HEADER,
    TRUSTED_ADMIN_USER_ROLE_HEADER,
};
use crate::gateway::gateway_data::GatewayDataState;

async fn assert_admin_provider_query_route(
    path: &str,
    request_payload: serde_json::Value,
    expected_status: StatusCode,
    expected_payload_assertions: impl FnOnce(&serde_json::Value),
) {
    let upstream_hits = Arc::new(Mutex::new(0usize));
    let upstream_hits_clone = Arc::clone(&upstream_hits);
    let upstream = Router::new().route(
        path,
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
        .post(format!("{gateway_url}{path}"))
        .header(crate::gateway::constants::GATEWAY_HEADER, "rust-phase3b")
        .header(TRUSTED_ADMIN_USER_ID_HEADER, "admin-user-123")
        .header(TRUSTED_ADMIN_USER_ROLE_HEADER, "admin")
        .header(TRUSTED_ADMIN_SESSION_ID_HEADER, "session-123")
        .json(&request_payload)
        .send()
        .await
        .expect("request should succeed");

    assert_eq!(response.status(), expected_status);
    let payload: serde_json::Value = response.json().await.expect("json body should parse");
    expected_payload_assertions(&payload);
    assert_eq!(*upstream_hits.lock().expect("mutex should lock"), 0);

    gateway_handle.abort();
    upstream_handle.abort();
}

#[tokio::test]
async fn gateway_handles_admin_provider_query_models_locally_with_trusted_admin_principal() {
    let upstream_hits = Arc::new(Mutex::new(0usize));
    let upstream_hits_clone = Arc::clone(&upstream_hits);
    let upstream = Router::new().route(
        "/api/admin/provider-query/models",
        any(move |_request: Request| {
            let upstream_hits_inner = Arc::clone(&upstream_hits_clone);
            async move {
                *upstream_hits_inner.lock().expect("mutex should lock") += 1;
                (StatusCode::OK, Body::from("unexpected upstream hit"))
            }
        }),
    );

    let provider_catalog_repository = Arc::new(InMemoryProviderCatalogReadRepository::seed(
        vec![sample_provider("provider-openai", "OpenAI", 10)],
        vec![StoredProviderCatalogEndpoint::new(
            "endpoint-openai-chat".to_string(),
            "provider-openai".to_string(),
            "openai:chat".to_string(),
            Some("chat".to_string()),
            Some("primary".to_string()),
            true,
        )
        .expect("endpoint should build")
        .with_transport_fields(
            "https://api.openai.com/v1".to_string(),
            None,
            None,
            None,
            None,
            None,
            None,
            None,
        )
        .expect("endpoint transport should build")],
        vec![
            {
                let mut key = sample_key(
                    "key-openai-allowed",
                    "provider-openai",
                    "openai:chat",
                    "sk-test",
                );
                key.allowed_models = Some(json!(["gpt-5"]));
                key
            },
            sample_key(
                "key-openai-all",
                "provider-openai",
                "openai:chat",
                "sk-test-2",
            ),
        ],
    ));
    let global_model_repository = Arc::new(
        InMemoryGlobalModelReadRepository::seed(Vec::new())
            .with_admin_global_models(vec![
                sample_admin_global_model("global-gpt-5", "gpt-5", "GPT 5"),
                sample_admin_global_model("global-gpt-4.1", "gpt-4.1", "GPT 4.1"),
            ])
            .with_admin_provider_models(vec![
                {
                    let mut model = sample_admin_provider_model(
                        "provider-model-gpt-5",
                        "provider-openai",
                        "global-gpt-5",
                        "gpt-5",
                    );
                    model.global_model_name = Some("gpt-5".to_string());
                    model.global_model_display_name = Some("GPT 5".to_string());
                    model
                },
                {
                    let mut model = sample_admin_provider_model(
                        "provider-model-gpt-4.1",
                        "provider-openai",
                        "global-gpt-4.1",
                        "gpt-4.1",
                    );
                    model.global_model_name = Some("gpt-4.1".to_string());
                    model.global_model_display_name = Some("GPT 4.1".to_string());
                    model
                },
            ]),
    );

    let (upstream_url, upstream_handle) = start_server(upstream).await;
    let gateway = build_router_with_state(
        AppState::new()
            .expect("gateway should build")
            .with_data_state_for_tests(
                GatewayDataState::with_provider_catalog_reader_for_tests(
                    provider_catalog_repository,
                )
                .with_global_model_repository_for_tests(global_model_repository),
            ),
    );
    let (gateway_url, gateway_handle) = start_server(gateway).await;

    let response = reqwest::Client::new()
        .post(format!("{gateway_url}/api/admin/provider-query/models"))
        .header(crate::gateway::constants::GATEWAY_HEADER, "rust-phase3b")
        .header(TRUSTED_ADMIN_USER_ID_HEADER, "admin-user-123")
        .header(TRUSTED_ADMIN_USER_ROLE_HEADER, "admin")
        .header(TRUSTED_ADMIN_SESSION_ID_HEADER, "session-123")
        .json(&json!({
            "provider_id": "provider-openai",
            "api_key_id": "key-openai-allowed"
        }))
        .send()
        .await
        .expect("request should succeed");

    assert_eq!(response.status(), StatusCode::OK);
    let payload: serde_json::Value = response.json().await.expect("json body should parse");
    assert_eq!(payload["success"], json!(true));
    assert_eq!(payload["provider"]["id"], "provider-openai");
    assert_eq!(payload["provider"]["name"], "OpenAI");
    assert_eq!(payload["provider"]["display_name"], "OpenAI");
    assert_eq!(payload["data"]["error"], serde_json::Value::Null);
    assert_eq!(payload["data"]["from_cache"], json!(true));
    assert_eq!(payload["data"]["keys_total"], json!(2));
    let models = payload["data"]["models"]
        .as_array()
        .expect("models should be an array");
    assert_eq!(models.len(), 2);
    let model_ids: Vec<_> = models
        .iter()
        .map(|model| {
            (
                model["id"].as_str().expect("id should be present"),
                model["display_name"]
                    .as_str()
                    .expect("display_name should be present"),
            )
        })
        .collect();
    assert_eq!(model_ids, vec![("gpt-4.1", "GPT 4.1"), ("gpt-5", "GPT 5")]);
    for model in models {
        assert_eq!(model["owned_by"], "OpenAI");
        assert_eq!(model["api_format"], "openai:chat");
        assert_eq!(model["api_formats"], json!(["openai:chat"]));
    }
    assert_eq!(*upstream_hits.lock().expect("mutex should lock"), 0);

    gateway_handle.abort();
    upstream_handle.abort();
}

#[tokio::test]
async fn gateway_handles_admin_provider_query_test_model_locally_with_trusted_admin_principal() {
    assert_admin_provider_query_route(
        "/api/admin/provider-query/test-model",
        json!({ "provider_id": "provider-openai", "model": "gpt-4.1" }),
        StatusCode::OK,
        |payload| {
            assert_eq!(payload["success"], json!(false));
            assert_eq!(payload["tested"], json!(false));
            assert!(payload["provider_id"].as_str().is_some());
        },
    )
    .await;
}

#[tokio::test]
async fn gateway_handles_admin_provider_query_test_model_failover_locally_with_trusted_admin_principal(
) {
    assert_admin_provider_query_route(
        "/api/admin/provider-query/test-model-failover",
        json!({
            "provider_id": "provider-openai",
            "failover_models": ["gpt-4.1", "gpt-4o-mini"]
        }),
        StatusCode::OK,
        |payload| {
            assert_eq!(payload["success"], json!(false));
            assert_eq!(payload["tested"], json!(false));
            assert!(payload["provider_id"].as_str().is_some());
        },
    )
    .await;
}

#[tokio::test]
async fn gateway_rejects_admin_provider_query_invalid_json_body() {
    let upstream_hits = Arc::new(Mutex::new(0usize));
    let upstream_hits_clone = Arc::clone(&upstream_hits);
    let upstream = Router::new().route(
        "/api/admin/provider-query/models",
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
        .post(format!("{gateway_url}/api/admin/provider-query/models"))
        .header(crate::gateway::constants::GATEWAY_HEADER, "rust-phase3b")
        .header(TRUSTED_ADMIN_USER_ID_HEADER, "admin-user-123")
        .header(TRUSTED_ADMIN_USER_ROLE_HEADER, "admin")
        .header(TRUSTED_ADMIN_SESSION_ID_HEADER, "session-123")
        .header(reqwest::header::CONTENT_TYPE, "application/json")
        .body("{")
        .send()
        .await
        .expect("request should succeed");

    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    let payload: serde_json::Value = response.json().await.expect("json body should parse");
    assert_eq!(payload["detail"], json!("Invalid JSON request body"));
    assert_eq!(*upstream_hits.lock().expect("mutex should lock"), 0);

    gateway_handle.abort();
    upstream_handle.abort();
}

#[tokio::test]
async fn gateway_rejects_admin_provider_query_test_model_without_provider_id() {
    assert_admin_provider_query_route(
        "/api/admin/provider-query/test-model",
        json!({ "model": "gpt-4.1" }),
        StatusCode::BAD_REQUEST,
        |payload| {
            assert_eq!(payload["detail"], json!("provider_id is required"));
        },
    )
    .await;
}

#[tokio::test]
async fn gateway_rejects_admin_provider_query_test_model_without_model() {
    assert_admin_provider_query_route(
        "/api/admin/provider-query/test-model",
        json!({ "provider_id": "provider-openai" }),
        StatusCode::BAD_REQUEST,
        |payload| {
            assert_eq!(payload["detail"], json!("model is required"));
        },
    )
    .await;
}

#[tokio::test]
async fn gateway_rejects_admin_provider_query_test_model_failover_without_provider_id() {
    assert_admin_provider_query_route(
        "/api/admin/provider-query/test-model-failover",
        json!({ "failover_models": ["gpt-4.1"] }),
        StatusCode::BAD_REQUEST,
        |payload| {
            assert_eq!(payload["detail"], json!("provider_id is required"));
        },
    )
    .await;
}

#[tokio::test]
async fn gateway_rejects_admin_provider_query_test_model_failover_without_models() {
    assert_admin_provider_query_route(
        "/api/admin/provider-query/test-model-failover",
        json!({ "provider_id": "provider-openai", "failover_models": [] }),
        StatusCode::BAD_REQUEST,
        |payload| {
            assert_eq!(
                payload["detail"],
                json!("failover_models should not be empty")
            );
        },
    )
    .await;
}
