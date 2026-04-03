use super::{
    any, build_router_with_state, build_state_with_execution_runtime_override, hash_api_key, json,
    sample_local_openai_auth_snapshot, sample_local_openai_candidate_row,
    sample_local_openai_endpoint, sample_local_openai_key, sample_local_openai_provider,
    start_server, Arc, DEVELOPMENT_ENCRYPTION_KEY, GatewayDataState,
    InMemoryAuthApiKeySnapshotRepository, InMemoryBillingReadRepository,
    InMemoryMinimalCandidateSelectionReadRepository, InMemoryProviderCatalogReadRepository,
    InMemoryRequestCandidateRepository, InMemoryUsageReadRepository, InMemoryWalletRepository,
    Json, Request, Router, StatusCode, StoredBillingModelContext, StoredWalletSnapshot,
    TRACE_ID_HEADER, UsageReadRepository, UsageRuntimeConfig, WalletLookupKey,
    WalletReadRepository,
};

#[tokio::test]
async fn gateway_settles_wallet_for_completed_execution_runtime_sync_usage() {
    let usage_repository = Arc::new(InMemoryUsageReadRepository::default());
    let request_candidate_repository = Arc::new(InMemoryRequestCandidateRepository::default());
    let billing_repository = Arc::new(InMemoryBillingReadRepository::seed(vec![
        StoredBillingModelContext::new(
            "provider-openai-usage-local-1".to_string(),
            Some("pay_as_you_go".to_string()),
            Some("key-openai-usage-local-1".to_string()),
            Some(json!({"openai:chat": 1.0})),
            Some(60),
            "global-model-openai-usage-local-1".to_string(),
            "gpt-5".to_string(),
            None,
            Some(0.02),
            Some(
                json!({"tiers":[{"up_to":null,"input_price_per_1m":3.0,"output_price_per_1m":15.0}]}),
            ),
            Some("model-openai-usage-local-1".to_string()),
            Some("gpt-5".to_string()),
            None,
            None,
            None,
        )
        .expect("billing context should build"),
    ]));
    let wallet_repository = Arc::new(InMemoryWalletRepository::seed(vec![
        StoredWalletSnapshot::new(
            "wallet-usage-sync-123".to_string(),
            Some("user-usage-sync-123".to_string()),
            None,
            10.0,
            2.0,
            "finite".to_string(),
            "USD".to_string(),
            "active".to_string(),
            0.0,
            0.0,
            0.0,
            0.0,
            100,
        )
        .expect("wallet should build"),
    ]));

    let upstream = Router::new().route(
        "/api/internal/gateway/report-sync",
        any(|_request: Request| async move { Json(json!({"ok": true})) }),
    );

    let execution_runtime = Router::new().route(
        "/v1/execute/sync",
        any(|_request: Request| async move {
            Json(json!({
                "request_id": "req-usage-wallet-sync-123",
                "status_code": 200,
                "headers": {
                    "content-type": "application/json"
                },
                "body": {
                    "json_body": {
                        "id": "chatcmpl-usage-wallet-sync-123",
                        "usage": {
                            "input_tokens": 1000,
                            "output_tokens": 500,
                            "total_tokens": 1500
                        }
                    }
                },
                "telemetry": {
                    "elapsed_ms": 45
                }
            }))
        }),
    );

    let (upstream_url, upstream_handle) = start_server(upstream).await;
    let (execution_runtime_url, execution_runtime_handle) = start_server(execution_runtime).await;
    let auth_repository = Arc::new(InMemoryAuthApiKeySnapshotRepository::seed(vec![(
        Some(hash_api_key("sk-client-openai-wallet-sync")),
        sample_local_openai_auth_snapshot("api-key-usage-sync-123", "user-usage-sync-123"),
    )]));
    let candidate_selection_repository =
        Arc::new(InMemoryMinimalCandidateSelectionReadRepository::seed(vec![
            sample_local_openai_candidate_row(),
        ]));
    let provider_catalog_repository = Arc::new(InMemoryProviderCatalogReadRepository::seed(
        vec![sample_local_openai_provider()],
        vec![sample_local_openai_endpoint()],
        vec![sample_local_openai_key()],
    ));
    let data_state = GatewayDataState::with_auth_candidate_selection_provider_catalog_request_candidates_usage_billing_and_wallet_for_tests(
        auth_repository,
        candidate_selection_repository,
        provider_catalog_repository,
        Arc::clone(&request_candidate_repository),
        usage_repository.clone(),
        billing_repository,
        wallet_repository.clone(),
        DEVELOPMENT_ENCRYPTION_KEY,
    );
    let gateway_state = build_state_with_execution_runtime_override(execution_runtime_url)
        .with_data_state_for_tests(data_state)
        .with_usage_runtime_for_tests(UsageRuntimeConfig {
            enabled: true,
            ..UsageRuntimeConfig::default()
        });
    let gateway = build_router_with_state(gateway_state);
    let (gateway_url, gateway_handle) = start_server(gateway).await;

    let response = reqwest::Client::new()
        .post(format!("{gateway_url}/v1/chat/completions"))
        .header(http::header::CONTENT_TYPE, "application/json")
        .header(
            http::header::AUTHORIZATION,
            "Bearer sk-client-openai-wallet-sync",
        )
        .header(TRACE_ID_HEADER, "req-usage-wallet-sync-123")
        .body("{\"model\":\"gpt-5\",\"messages\":[]}")
        .send()
        .await
        .expect("request should succeed");

    assert_eq!(response.status(), StatusCode::OK);

    let mut stored = None;
    for _ in 0..50 {
        stored = usage_repository
            .find_by_request_id("req-usage-wallet-sync-123")
            .await
            .expect("usage lookup should succeed");
        if stored.is_some() {
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(10)).await;
    }
    let stored = stored.expect("usage should be recorded");
    assert_eq!(stored.status, "completed");
    assert_eq!(stored.total_tokens, 1500);

    let wallet = wallet_repository
        .find(WalletLookupKey::UserId("user-usage-sync-123"))
        .await
        .expect("wallet lookup should succeed")
        .expect("wallet should exist");
    assert!(wallet.balance < 10.0 || wallet.gift_balance < 2.0);
    assert!(wallet.total_consumed > 0.0);

    gateway_handle.abort();
    execution_runtime_handle.abort();
    upstream_handle.abort();
}
