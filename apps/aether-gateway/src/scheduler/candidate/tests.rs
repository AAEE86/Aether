use std::sync::Arc;
use std::time::Duration;

use aether_data::repository::candidate_selection::StoredProviderModelMapping;
use aether_data::repository::candidate_selection::{
    InMemoryMinimalCandidateSelectionReadRepository, StoredMinimalCandidateSelectionRow,
};
use aether_data::repository::candidates::{
    InMemoryRequestCandidateRepository, RequestCandidateStatus, StoredRequestCandidate,
};
use aether_data::repository::provider_catalog::{
    InMemoryProviderCatalogReadRepository, StoredProviderCatalogKey, StoredProviderCatalogProvider,
};
use aether_data::repository::quota::InMemoryProviderQuotaRepository;
use aether_data::repository::quota::StoredProviderQuotaSnapshot;

use super::{
    build_scheduler_affinity_cache_key, candidate_affinity_hash, candidate_model_names,
    matches_model_mapping, read_minimal_candidate_selection, resolve_provider_model_name,
    resolve_requested_global_model_name, select_minimal_candidate, select_provider_model_name,
    should_skip_provider_quota, GatewayMinimalCandidateSelectionCandidate,
    StoredGatewayAuthApiKeySnapshot,
};
use crate::gateway::gateway_cache::SchedulerAffinityTarget;
use crate::gateway::{AppState, GatewayDataState};

fn sample_row() -> StoredMinimalCandidateSelectionRow {
    StoredMinimalCandidateSelectionRow {
        provider_id: "provider-1".to_string(),
        provider_name: "OpenAI".to_string(),
        provider_type: "custom".to_string(),
        provider_priority: 10,
        provider_is_active: true,
        endpoint_id: "endpoint-1".to_string(),
        endpoint_api_format: "openai:chat".to_string(),
        endpoint_api_family: Some("openai".to_string()),
        endpoint_kind: Some("chat".to_string()),
        endpoint_is_active: true,
        key_id: "key-1".to_string(),
        key_name: "prod".to_string(),
        key_auth_type: "api_key".to_string(),
        key_is_active: true,
        key_api_formats: Some(vec!["openai:chat".to_string()]),
        key_allowed_models: None,
        key_capabilities: Some(serde_json::json!({"cache_1h": true})),
        key_internal_priority: 50,
        key_global_priority_by_format: Some(serde_json::json!({"openai:chat": 2})),
        model_id: "model-1".to_string(),
        global_model_id: "global-model-1".to_string(),
        global_model_name: "gpt-4.1".to_string(),
        global_model_mappings: Some(vec!["gpt-4\\.1-.*".to_string()]),
        global_model_supports_streaming: Some(true),
        model_provider_model_name: "gpt-4.1-upstream".to_string(),
        model_provider_model_mappings: Some(vec![
            StoredProviderModelMapping {
                name: "gpt-4.1-canary".to_string(),
                priority: 1,
                api_formats: Some(vec!["openai:chat".to_string()]),
            },
            StoredProviderModelMapping {
                name: "gpt-4.1-responses".to_string(),
                priority: 1,
                api_formats: Some(vec!["openai:responses".to_string()]),
            },
        ]),
        model_supports_streaming: None,
        model_is_active: true,
        model_is_available: true,
    }
}

fn sample_provider(id: &str, concurrent_limit: Option<i32>) -> StoredProviderCatalogProvider {
    StoredProviderCatalogProvider::new(
        id.to_string(),
        format!("provider-{id}"),
        Some("https://example.com".to_string()),
        "custom".to_string(),
    )
    .expect("provider should build")
    .with_transport_fields(
        true,
        false,
        false,
        concurrent_limit,
        None,
        None,
        None,
        None,
        None,
    )
}

fn sample_key(id: &str, provider_id: &str, rpm_limit: Option<u32>) -> StoredProviderCatalogKey {
    StoredProviderCatalogKey::new(
        id.to_string(),
        provider_id.to_string(),
        format!("key-{id}"),
        "api_key".to_string(),
        None,
        true,
    )
    .expect("key should build")
    .with_rate_limit_fields(rpm_limit, None, None, None, None, None, Some(20), Some(20))
}

#[test]
fn selects_provider_model_name_with_api_format_scope() {
    let row = sample_row();

    assert_eq!(
        select_provider_model_name(&row, "openai:chat"),
        "gpt-4.1-canary"
    );
}

#[test]
fn candidate_model_names_keep_base_and_scoped_mappings() {
    let row = sample_row();
    let names = candidate_model_names(&row, "openai:chat");

    assert!(names.contains("gpt-4.1-upstream"));
    assert!(names.contains("gpt-4.1-canary"));
    assert!(!names.contains("gpt-4.1-responses"));
}

#[test]
fn resolves_mapping_matched_model_from_key_allowed_models() {
    let mut row = sample_row();
    row.key_allowed_models = Some(vec!["gpt-4.1-canary".to_string()]);

    let resolved = resolve_provider_model_name(&row, "gpt-4.1", "openai:chat")
        .expect("candidate should resolve");

    assert_eq!(resolved.0, "gpt-4.1-canary");
    assert_eq!(resolved.1, Some("gpt-4.1-canary".to_string()));
}

#[test]
fn resolves_mapping_matched_model_from_global_regex_mapping() {
    let mut row = sample_row();
    row.key_allowed_models = Some(vec!["gpt-4.1-variant".to_string()]);

    let resolved = resolve_provider_model_name(&row, "gpt-4.1", "openai:chat")
        .expect("candidate should resolve");

    assert_eq!(resolved.0, "gpt-4.1-variant");
    assert_eq!(resolved.1, Some("gpt-4.1-variant".to_string()));
}

#[test]
fn invalid_regex_mapping_is_treated_as_non_match() {
    assert!(!matches_model_mapping("(", "gpt-4.1-variant"));
}

#[test]
fn resolves_requested_global_model_from_provider_model_alias() {
    let mut row = sample_row();
    row.global_model_name = "gpt-5".to_string();
    row.model_provider_model_name = "gpt-5.2".to_string();
    row.model_provider_model_mappings = Some(vec![StoredProviderModelMapping {
        name: "gpt-5.2".to_string(),
        priority: 1,
        api_formats: Some(vec!["openai:chat".to_string()]),
    }]);

    let resolved = resolve_requested_global_model_name(&[row], "gpt-5.2", "openai:chat");

    assert_eq!(resolved.as_deref(), Some("gpt-5"));
}

#[test]
fn resolves_requested_global_model_from_global_regex_mapping() {
    let mut row = sample_row();
    row.global_model_name = "gpt-5".to_string();
    row.global_model_mappings = Some(vec!["gpt-5(?:\\.\\d+)?".to_string()]);

    let resolved = resolve_requested_global_model_name(&[row], "gpt-5.2", "openai:chat");

    assert_eq!(resolved.as_deref(), Some("gpt-5"));
}

#[test]
fn scheduler_candidate_is_serializable() {
    let candidate = GatewayMinimalCandidateSelectionCandidate {
        provider_id: "provider-1".to_string(),
        provider_name: "OpenAI".to_string(),
        provider_type: "custom".to_string(),
        provider_priority: 10,
        endpoint_id: "endpoint-1".to_string(),
        endpoint_api_format: "openai:chat".to_string(),
        key_id: "key-1".to_string(),
        key_name: "prod".to_string(),
        key_auth_type: "api_key".to_string(),
        key_internal_priority: 50,
        key_global_priority_for_format: Some(2),
        key_capabilities: Some(serde_json::json!({"cache_1h": true})),
        model_id: "model-1".to_string(),
        global_model_id: "global-model-1".to_string(),
        global_model_name: "gpt-4.1".to_string(),
        selected_provider_model_name: "gpt-4.1-canary".to_string(),
        mapping_matched_model: Some("gpt-4.1-canary".to_string()),
    };

    let json = serde_json::to_value(candidate).expect("candidate should serialize");
    assert_eq!(json["provider_name"], "OpenAI");
}

#[test]
fn skips_inactive_or_exhausted_monthly_quota_provider() {
    let inactive = StoredProviderQuotaSnapshot::new(
        "provider-1".to_string(),
        "monthly_quota".to_string(),
        Some(10.0),
        1.0,
        Some(30),
        Some(1_000),
        None,
        false,
    )
    .expect("quota should build");
    assert!(should_skip_provider_quota(&inactive, 2_000));

    let exhausted = StoredProviderQuotaSnapshot::new(
        "provider-1".to_string(),
        "monthly_quota".to_string(),
        Some(10.0),
        10.0,
        Some(30),
        Some(1_000),
        None,
        true,
    )
    .expect("quota should build");
    assert!(should_skip_provider_quota(&exhausted, 2_000));

    let payg = StoredProviderQuotaSnapshot::new(
        "provider-1".to_string(),
        "pay_as_you_go".to_string(),
        None,
        10.0,
        None,
        None,
        None,
        true,
    )
    .expect("quota should build");
    assert!(!should_skip_provider_quota(&payg, 2_000));
}

#[tokio::test]
async fn selects_next_candidate_when_first_provider_quota_is_exhausted() {
    let mut first = sample_row();
    first.provider_id = "provider-1".to_string();
    first.provider_name = "openai-primary".to_string();
    first.endpoint_id = "endpoint-1".to_string();
    first.key_id = "key-1".to_string();
    first.key_name = "primary".to_string();
    first.model_provider_model_name = "gpt-4.1-primary".to_string();
    first.model_provider_model_mappings = Some(vec![StoredProviderModelMapping {
        name: "gpt-4.1-primary".to_string(),
        priority: 1,
        api_formats: Some(vec!["openai:chat".to_string()]),
    }]);
    first.key_global_priority_by_format = Some(serde_json::json!({"openai:chat": 1}));

    let mut second = sample_row();
    second.provider_id = "provider-2".to_string();
    second.provider_name = "openai-secondary".to_string();
    second.endpoint_id = "endpoint-2".to_string();
    second.key_id = "key-2".to_string();
    second.key_name = "secondary".to_string();
    second.model_provider_model_name = "gpt-4.1-secondary".to_string();
    second.model_provider_model_mappings = Some(vec![StoredProviderModelMapping {
        name: "gpt-4.1-secondary".to_string(),
        priority: 1,
        api_formats: Some(vec!["openai:chat".to_string()]),
    }]);
    second.key_global_priority_by_format = Some(serde_json::json!({"openai:chat": 2}));

    let candidates = Arc::new(InMemoryMinimalCandidateSelectionReadRepository::seed(vec![
        first, second,
    ]));
    let quotas = Arc::new(InMemoryProviderQuotaRepository::seed(vec![
        StoredProviderQuotaSnapshot::new(
            "provider-1".to_string(),
            "monthly_quota".to_string(),
            Some(10.0),
            10.0,
            Some(30),
            Some(1_000),
            None,
            true,
        )
        .expect("quota should build"),
    ]));
    let state = AppState::new()
        .expect("state should build")
        .with_data_state_for_tests(
            GatewayDataState::with_candidate_selection_and_quota_for_tests(candidates, quotas),
        );

    let selected = select_minimal_candidate(&state, "openai:chat", "gpt-4.1", false, None, 2_000)
        .await
        .expect("selection should succeed")
        .expect("candidate should exist");

    assert_eq!(selected.provider_id, "provider-2");
    assert_eq!(selected.selected_provider_model_name, "gpt-4.1-secondary");
}

#[tokio::test]
async fn cooled_down_when_recent_failures_are_recorded_for_same_key() {
    let mut first = sample_row();
    first.provider_id = "provider-1".to_string();
    first.provider_name = "openai-primary".to_string();
    first.endpoint_id = "endpoint-1".to_string();
    first.key_id = "key-1".to_string();
    first.key_name = "primary".to_string();
    first.model_provider_model_name = "gpt-4.1-primary".to_string();
    first.model_provider_model_mappings = Some(vec![StoredProviderModelMapping {
        name: "gpt-4.1-primary".to_string(),
        priority: 1,
        api_formats: Some(vec!["openai:chat".to_string()]),
    }]);
    first.key_global_priority_by_format = Some(serde_json::json!({"openai:chat": 1}));

    let mut second = sample_row();
    second.provider_id = "provider-2".to_string();
    second.provider_name = "openai-secondary".to_string();
    second.endpoint_id = "endpoint-2".to_string();
    second.key_id = "key-2".to_string();
    second.key_name = "secondary".to_string();
    second.model_provider_model_name = "gpt-4.1-secondary".to_string();
    second.model_provider_model_mappings = Some(vec![StoredProviderModelMapping {
        name: "gpt-4.1-secondary".to_string(),
        priority: 1,
        api_formats: Some(vec!["openai:chat".to_string()]),
    }]);
    second.key_global_priority_by_format = Some(serde_json::json!({"openai:chat": 2}));

    let candidates = Arc::new(InMemoryMinimalCandidateSelectionReadRepository::seed(vec![
        first, second,
    ]));
    let quotas = Arc::new(InMemoryProviderQuotaRepository::seed(vec![]));
    let request_candidates = Arc::new(InMemoryRequestCandidateRepository::seed(vec![
        StoredRequestCandidate::new(
            "cand-1".to_string(),
            "req-1".to_string(),
            None,
            None,
            None,
            None,
            0,
            0,
            Some("provider-1".to_string()),
            Some("endpoint-1".to_string()),
            Some("key-1".to_string()),
            RequestCandidateStatus::Failed,
            None,
            false,
            Some(502),
            None,
            Some("upstream".to_string()),
            Some(100),
            None,
            None,
            None,
            95,
            Some(95),
            Some(95),
        )
        .expect("candidate should build"),
        StoredRequestCandidate::new(
            "cand-2".to_string(),
            "req-2".to_string(),
            None,
            None,
            None,
            None,
            0,
            0,
            Some("provider-1".to_string()),
            Some("endpoint-1".to_string()),
            Some("key-1".to_string()),
            RequestCandidateStatus::Cancelled,
            None,
            false,
            Some(499),
            None,
            Some("cancelled".to_string()),
            Some(80),
            None,
            None,
            None,
            98,
            Some(98),
            Some(98),
        )
        .expect("candidate should build"),
    ]));
    let state = AppState::new()
        .expect("state should build")
        .with_data_state_for_tests(
            GatewayDataState::with_candidate_selection_quota_and_request_candidates_for_tests(
                candidates,
                quotas,
                request_candidates,
            ),
        );

    let selected = select_minimal_candidate(&state, "openai:chat", "gpt-4.1", false, None, 100)
        .await
        .expect("selection should succeed")
        .expect("candidate should exist");

    assert_eq!(selected.provider_id, "provider-2");
    assert_eq!(selected.selected_provider_model_name, "gpt-4.1-secondary");
}

#[tokio::test]
async fn same_priority_candidates_are_distributed_by_affinity_key() {
    let mut first = sample_row();
    first.provider_id = "provider-a".to_string();
    first.provider_name = "openai-a".to_string();
    first.endpoint_id = "endpoint-a".to_string();
    first.key_id = "key-a".to_string();
    first.key_name = "alpha".to_string();
    first.provider_priority = 10;
    first.key_internal_priority = 10;
    first.key_global_priority_by_format = Some(serde_json::json!({"openai:chat": 1}));
    first.model_provider_model_name = "gpt-4.1-a".to_string();
    first.model_provider_model_mappings = Some(vec![StoredProviderModelMapping {
        name: "gpt-4.1-a".to_string(),
        priority: 1,
        api_formats: Some(vec!["openai:chat".to_string()]),
    }]);

    let mut second = sample_row();
    second.provider_id = "provider-b".to_string();
    second.provider_name = "openai-b".to_string();
    second.endpoint_id = "endpoint-b".to_string();
    second.key_id = "key-b".to_string();
    second.key_name = "beta".to_string();
    second.provider_priority = 10;
    second.key_internal_priority = 10;
    second.key_global_priority_by_format = Some(serde_json::json!({"openai:chat": 1}));
    second.model_provider_model_name = "gpt-4.1-b".to_string();
    second.model_provider_model_mappings = Some(vec![StoredProviderModelMapping {
        name: "gpt-4.1-b".to_string(),
        priority: 1,
        api_formats: Some(vec!["openai:chat".to_string()]),
    }]);

    let candidates = Arc::new(InMemoryMinimalCandidateSelectionReadRepository::seed(vec![
        first, second,
    ]));
    let quotas = Arc::new(InMemoryProviderQuotaRepository::seed(vec![]));
    let state = GatewayDataState::with_candidate_selection_and_quota_for_tests(candidates, quotas);

    let auth_snapshot = StoredGatewayAuthApiKeySnapshot {
        user_id: "user-1".to_string(),
        username: "alice".to_string(),
        email: None,
        user_role: "user".to_string(),
        user_auth_source: "local".to_string(),
        user_is_active: true,
        user_is_deleted: false,
        user_rate_limit: None,
        user_allowed_providers: None,
        user_allowed_api_formats: None,
        user_allowed_models: None,
        api_key_id: "affinity-key-1".to_string(),
        api_key_name: Some("default".to_string()),
        api_key_is_active: true,
        api_key_is_locked: false,
        api_key_is_standalone: false,
        api_key_rate_limit: None,
        api_key_concurrent_limit: None,
        api_key_expires_at_unix_secs: None,
        api_key_allowed_providers: None,
        api_key_allowed_api_formats: None,
        api_key_allowed_models: None,
        currently_usable: true,
    };

    let selection = read_minimal_candidate_selection(
        &state,
        "openai:chat",
        "gpt-4.1",
        false,
        Some(&auth_snapshot),
    )
    .await
    .expect("selection should succeed");

    assert_eq!(selection.len(), 2);

    let left_hash = candidate_affinity_hash(&auth_snapshot.api_key_id, &selection[0]);
    let right_hash = candidate_affinity_hash(&auth_snapshot.api_key_id, &selection[1]);
    assert!(
        left_hash <= right_hash,
        "same-priority candidates should be ordered by affinity hash"
    );
}

#[tokio::test]
async fn read_minimal_candidate_selection_resolves_provider_model_alias() {
    let mut row = sample_row();
    row.global_model_name = "gpt-5".to_string();
    row.model_provider_model_name = "gpt-5.2".to_string();
    row.model_provider_model_mappings = Some(vec![StoredProviderModelMapping {
        name: "gpt-5.2".to_string(),
        priority: 1,
        api_formats: Some(vec!["openai:chat".to_string()]),
    }]);

    let candidates = Arc::new(InMemoryMinimalCandidateSelectionReadRepository::seed(vec![
        row,
    ]));
    let quotas = Arc::new(InMemoryProviderQuotaRepository::seed(vec![]));
    let state = GatewayDataState::with_candidate_selection_and_quota_for_tests(candidates, quotas);

    let selection = read_minimal_candidate_selection(&state, "openai:chat", "gpt-5.2", false, None)
        .await
        .expect("selection should succeed");

    assert_eq!(selection.len(), 1);
    assert_eq!(selection[0].global_model_name, "gpt-5");
    assert_eq!(selection[0].selected_provider_model_name, "gpt-5.2");
}

#[tokio::test]
async fn read_minimal_candidate_selection_allows_resolved_global_model_in_auth_snapshot() {
    let mut row = sample_row();
    row.global_model_name = "gpt-5".to_string();
    row.global_model_mappings = Some(vec!["gpt-5(?:\\.\\d+)?".to_string()]);
    row.model_provider_model_name = "gpt-5-upstream".to_string();
    row.model_provider_model_mappings = Some(vec![StoredProviderModelMapping {
        name: "gpt-5-upstream".to_string(),
        priority: 1,
        api_formats: Some(vec!["openai:chat".to_string()]),
    }]);

    let candidates = Arc::new(InMemoryMinimalCandidateSelectionReadRepository::seed(vec![
        row,
    ]));
    let quotas = Arc::new(InMemoryProviderQuotaRepository::seed(vec![]));
    let state = GatewayDataState::with_candidate_selection_and_quota_for_tests(candidates, quotas);
    let auth_snapshot = StoredGatewayAuthApiKeySnapshot {
        user_id: "user-1".to_string(),
        username: "alice".to_string(),
        email: None,
        user_role: "user".to_string(),
        user_auth_source: "local".to_string(),
        user_is_active: true,
        user_is_deleted: false,
        user_rate_limit: None,
        user_allowed_providers: None,
        user_allowed_api_formats: None,
        user_allowed_models: Some(vec!["gpt-5".to_string()]),
        api_key_id: "api-key-1".to_string(),
        api_key_name: Some("default".to_string()),
        api_key_is_active: true,
        api_key_is_locked: false,
        api_key_is_standalone: false,
        api_key_rate_limit: None,
        api_key_concurrent_limit: None,
        api_key_expires_at_unix_secs: None,
        api_key_allowed_providers: None,
        api_key_allowed_api_formats: None,
        api_key_allowed_models: Some(vec!["gpt-5".to_string()]),
        currently_usable: true,
    };

    let selection = read_minimal_candidate_selection(
        &state,
        "openai:chat",
        "gpt-5.2",
        false,
        Some(&auth_snapshot),
    )
    .await
    .expect("selection should succeed");

    assert_eq!(selection.len(), 1);
    assert_eq!(selection[0].global_model_name, "gpt-5");
}

#[tokio::test]
async fn reuses_cached_scheduler_affinity_candidate_before_sorted_fallback() {
    let mut first = sample_row();
    first.provider_id = "provider-a".to_string();
    first.provider_name = "openai-a".to_string();
    first.endpoint_id = "endpoint-a".to_string();
    first.key_id = "key-a".to_string();
    first.key_name = "alpha".to_string();
    first.key_global_priority_by_format = Some(serde_json::json!({"openai:chat": 1}));

    let mut second = sample_row();
    second.provider_id = "provider-b".to_string();
    second.provider_name = "openai-b".to_string();
    second.endpoint_id = "endpoint-b".to_string();
    second.key_id = "key-b".to_string();
    second.key_name = "beta".to_string();
    second.key_global_priority_by_format = Some(serde_json::json!({"openai:chat": 2}));

    let candidates = Arc::new(InMemoryMinimalCandidateSelectionReadRepository::seed(vec![
        first, second,
    ]));
    let quotas = Arc::new(InMemoryProviderQuotaRepository::seed(vec![]));
    let state = AppState::new()
        .expect("state should build")
        .with_data_state_for_tests(
            GatewayDataState::with_candidate_selection_and_quota_for_tests(candidates, quotas),
        );

    let auth_snapshot = StoredGatewayAuthApiKeySnapshot {
        user_id: "user-1".to_string(),
        username: "alice".to_string(),
        email: None,
        user_role: "user".to_string(),
        user_auth_source: "local".to_string(),
        user_is_active: true,
        user_is_deleted: false,
        user_rate_limit: None,
        user_allowed_providers: None,
        user_allowed_api_formats: None,
        user_allowed_models: None,
        api_key_id: "affinity-key-1".to_string(),
        api_key_name: Some("default".to_string()),
        api_key_is_active: true,
        api_key_is_locked: false,
        api_key_is_standalone: false,
        api_key_rate_limit: None,
        api_key_concurrent_limit: None,
        api_key_expires_at_unix_secs: None,
        api_key_allowed_providers: None,
        api_key_allowed_api_formats: None,
        api_key_allowed_models: None,
        currently_usable: true,
    };
    let cache_key =
        build_scheduler_affinity_cache_key(Some(&auth_snapshot), "openai:chat", "gpt-4.1")
            .expect("cache key should build");
    state.scheduler_affinity_cache.insert(
        cache_key,
        SchedulerAffinityTarget {
            provider_id: "provider-b".to_string(),
            endpoint_id: "endpoint-b".to_string(),
            key_id: "key-b".to_string(),
        },
        Duration::from_secs(300),
        100,
    );

    let selected = select_minimal_candidate(
        &state,
        "openai:chat",
        "gpt-4.1",
        false,
        Some(&auth_snapshot),
        100,
    )
    .await
    .expect("selection should succeed")
    .expect("candidate should exist");

    assert_eq!(selected.provider_id, "provider-b");
    assert_eq!(selected.key_id, "key-b");
}

#[tokio::test]
async fn selects_next_candidate_when_first_provider_concurrent_limit_is_reached() {
    let mut first = sample_row();
    first.provider_id = "provider-a".to_string();
    first.provider_name = "openai-a".to_string();
    first.endpoint_id = "endpoint-a".to_string();
    first.key_id = "key-a".to_string();
    first.key_name = "alpha".to_string();
    first.key_global_priority_by_format = Some(serde_json::json!({"openai:chat": 1}));

    let mut second = sample_row();
    second.provider_id = "provider-b".to_string();
    second.provider_name = "openai-b".to_string();
    second.endpoint_id = "endpoint-b".to_string();
    second.key_id = "key-b".to_string();
    second.key_name = "beta".to_string();
    second.key_global_priority_by_format = Some(serde_json::json!({"openai:chat": 2}));

    let candidates = Arc::new(InMemoryMinimalCandidateSelectionReadRepository::seed(vec![
        first, second,
    ]));
    let provider_catalog = Arc::new(InMemoryProviderCatalogReadRepository::seed(
        vec![
            sample_provider("provider-a", Some(1)),
            sample_provider("provider-b", None),
        ],
        Vec::new(),
        Vec::new(),
    ));
    let quotas = Arc::new(InMemoryProviderQuotaRepository::seed(vec![]));
    let request_candidates = Arc::new(InMemoryRequestCandidateRepository::seed(vec![
        StoredRequestCandidate::new(
            "cand-1".to_string(),
            "req-1".to_string(),
            None,
            None,
            None,
            None,
            0,
            0,
            Some("provider-a".to_string()),
            Some("endpoint-a".to_string()),
            Some("key-a".to_string()),
            RequestCandidateStatus::Streaming,
            None,
            false,
            None,
            None,
            None,
            None,
            None,
            None,
            None,
            95,
            Some(95),
            None,
        )
        .expect("candidate should build"),
    ]));
    let state = AppState::new()
            .expect("state should build")
            .with_data_state_for_tests(
                GatewayDataState::with_candidate_selection_provider_catalog_quota_and_request_candidates_for_tests(
                    candidates,
                    provider_catalog,
                    quotas,
                    request_candidates,
                ),
            );

    let selected = select_minimal_candidate(&state, "openai:chat", "gpt-4.1", false, None, 100)
        .await
        .expect("selection should succeed")
        .expect("candidate should exist");

    assert_eq!(selected.provider_id, "provider-b");
    assert_eq!(selected.key_id, "key-b");
}

#[tokio::test]
async fn returns_none_when_auth_api_key_concurrent_limit_is_reached() {
    let candidates = Arc::new(InMemoryMinimalCandidateSelectionReadRepository::seed(vec![
        sample_row(),
    ]));
    let provider_catalog = Arc::new(InMemoryProviderCatalogReadRepository::seed(
        vec![sample_provider("provider-1", None)],
        Vec::new(),
        Vec::new(),
    ));
    let quotas = Arc::new(InMemoryProviderQuotaRepository::seed(vec![]));
    let request_candidates = Arc::new(InMemoryRequestCandidateRepository::seed(vec![
        StoredRequestCandidate::new(
            "cand-1".to_string(),
            "req-1".to_string(),
            Some("user-1".to_string()),
            Some("api-key-1".to_string()),
            None,
            None,
            0,
            0,
            Some("provider-1".to_string()),
            Some("endpoint-1".to_string()),
            Some("key-1".to_string()),
            RequestCandidateStatus::Pending,
            None,
            false,
            None,
            None,
            None,
            None,
            None,
            None,
            None,
            95,
            Some(95),
            None,
        )
        .expect("candidate should build"),
    ]));
    let state = AppState::new()
            .expect("state should build")
            .with_data_state_for_tests(
                GatewayDataState::with_candidate_selection_provider_catalog_quota_and_request_candidates_for_tests(
                    candidates,
                    provider_catalog,
                    quotas,
                    request_candidates,
                ),
            );

    let auth_snapshot = StoredGatewayAuthApiKeySnapshot {
        user_id: "user-1".to_string(),
        username: "alice".to_string(),
        email: None,
        user_role: "user".to_string(),
        user_auth_source: "local".to_string(),
        user_is_active: true,
        user_is_deleted: false,
        user_rate_limit: None,
        user_allowed_providers: None,
        user_allowed_api_formats: None,
        user_allowed_models: None,
        api_key_id: "api-key-1".to_string(),
        api_key_name: Some("default".to_string()),
        api_key_is_active: true,
        api_key_is_locked: false,
        api_key_is_standalone: false,
        api_key_rate_limit: None,
        api_key_concurrent_limit: Some(1),
        api_key_expires_at_unix_secs: None,
        api_key_allowed_providers: None,
        api_key_allowed_api_formats: None,
        api_key_allowed_models: None,
        currently_usable: true,
    };

    let selected = select_minimal_candidate(
        &state,
        "openai:chat",
        "gpt-4.1",
        false,
        Some(&auth_snapshot),
        100,
    )
    .await
    .expect("selection should succeed");

    assert!(selected.is_none());
}

#[tokio::test]
async fn selects_next_candidate_when_first_provider_key_rpm_slots_are_reserved_for_new_user() {
    let mut first = sample_row();
    first.provider_id = "provider-a".to_string();
    first.provider_name = "openai-a".to_string();
    first.endpoint_id = "endpoint-a".to_string();
    first.key_id = "key-a".to_string();
    first.key_name = "alpha".to_string();
    first.key_global_priority_by_format = Some(serde_json::json!({"openai:chat": 1}));

    let mut second = sample_row();
    second.provider_id = "provider-b".to_string();
    second.provider_name = "openai-b".to_string();
    second.endpoint_id = "endpoint-b".to_string();
    second.key_id = "key-b".to_string();
    second.key_name = "beta".to_string();
    second.key_global_priority_by_format = Some(serde_json::json!({"openai:chat": 2}));

    let candidates = Arc::new(InMemoryMinimalCandidateSelectionReadRepository::seed(vec![
        first, second,
    ]));
    let provider_catalog = Arc::new(InMemoryProviderCatalogReadRepository::seed(
        vec![
            sample_provider("provider-a", None),
            sample_provider("provider-b", None),
        ],
        Vec::new(),
        vec![
            sample_key("key-a", "provider-a", Some(10)),
            sample_key("key-b", "provider-b", Some(10)),
        ],
    ));
    let quotas = Arc::new(InMemoryProviderQuotaRepository::seed(vec![]));
    let request_candidates = Arc::new(InMemoryRequestCandidateRepository::seed(vec![
        StoredRequestCandidate::new(
            "cand-1".to_string(),
            "req-1".to_string(),
            None,
            Some("api-key-new-user".to_string()),
            None,
            None,
            0,
            0,
            Some("provider-a".to_string()),
            Some("endpoint-a".to_string()),
            Some("key-a".to_string()),
            RequestCandidateStatus::Success,
            None,
            false,
            Some(200),
            None,
            None,
            Some(10),
            Some(9),
            None,
            None,
            95,
            Some(95),
            Some(96),
        )
        .expect("candidate should build"),
    ]));
    let state = AppState::new()
            .expect("state should build")
            .with_data_state_for_tests(
                GatewayDataState::with_candidate_selection_provider_catalog_quota_and_request_candidates_for_tests(
                    candidates,
                    provider_catalog,
                    quotas,
                    request_candidates,
                ),
            );

    let auth_snapshot = StoredGatewayAuthApiKeySnapshot {
        user_id: "user-1".to_string(),
        username: "alice".to_string(),
        email: None,
        user_role: "user".to_string(),
        user_auth_source: "local".to_string(),
        user_is_active: true,
        user_is_deleted: false,
        user_rate_limit: None,
        user_allowed_providers: None,
        user_allowed_api_formats: None,
        user_allowed_models: None,
        api_key_id: "api-key-new-user".to_string(),
        api_key_name: Some("default".to_string()),
        api_key_is_active: true,
        api_key_is_locked: false,
        api_key_is_standalone: false,
        api_key_rate_limit: None,
        api_key_concurrent_limit: None,
        api_key_expires_at_unix_secs: None,
        api_key_allowed_providers: None,
        api_key_allowed_api_formats: None,
        api_key_allowed_models: None,
        currently_usable: true,
    };

    let selected = select_minimal_candidate(
        &state,
        "openai:chat",
        "gpt-4.1",
        false,
        Some(&auth_snapshot),
        100,
    )
    .await
    .expect("selection should succeed")
    .expect("candidate should exist");

    assert_eq!(selected.provider_id, "provider-b");
    assert_eq!(selected.key_id, "key-b");
}

#[tokio::test]
async fn cached_affinity_candidate_can_use_reserved_provider_key_rpm_capacity() {
    let mut first = sample_row();
    first.provider_id = "provider-a".to_string();
    first.provider_name = "openai-a".to_string();
    first.endpoint_id = "endpoint-a".to_string();
    first.key_id = "key-a".to_string();
    first.key_name = "alpha".to_string();
    first.key_global_priority_by_format = Some(serde_json::json!({"openai:chat": 1}));

    let mut second = sample_row();
    second.provider_id = "provider-b".to_string();
    second.provider_name = "openai-b".to_string();
    second.endpoint_id = "endpoint-b".to_string();
    second.key_id = "key-b".to_string();
    second.key_name = "beta".to_string();
    second.key_global_priority_by_format = Some(serde_json::json!({"openai:chat": 2}));

    let candidates = Arc::new(InMemoryMinimalCandidateSelectionReadRepository::seed(vec![
        first, second,
    ]));
    let provider_catalog = Arc::new(InMemoryProviderCatalogReadRepository::seed(
        vec![
            sample_provider("provider-a", None),
            sample_provider("provider-b", None),
        ],
        Vec::new(),
        vec![
            sample_key("key-a", "provider-a", Some(10)),
            sample_key("key-b", "provider-b", Some(10)),
        ],
    ));
    let quotas = Arc::new(InMemoryProviderQuotaRepository::seed(vec![]));
    let request_candidates = Arc::new(InMemoryRequestCandidateRepository::seed(vec![
        StoredRequestCandidate::new(
            "cand-1".to_string(),
            "req-1".to_string(),
            None,
            Some("api-key-cached-user".to_string()),
            None,
            None,
            0,
            0,
            Some("provider-a".to_string()),
            Some("endpoint-a".to_string()),
            Some("key-a".to_string()),
            RequestCandidateStatus::Success,
            None,
            false,
            Some(200),
            None,
            None,
            Some(10),
            Some(9),
            None,
            None,
            95,
            Some(95),
            Some(96),
        )
        .expect("candidate should build"),
    ]));
    let state = AppState::new()
            .expect("state should build")
            .with_data_state_for_tests(
                GatewayDataState::with_candidate_selection_provider_catalog_quota_and_request_candidates_for_tests(
                    candidates,
                    provider_catalog,
                    quotas,
                    request_candidates,
                ),
            );

    let auth_snapshot = StoredGatewayAuthApiKeySnapshot {
        user_id: "user-1".to_string(),
        username: "alice".to_string(),
        email: None,
        user_role: "user".to_string(),
        user_auth_source: "local".to_string(),
        user_is_active: true,
        user_is_deleted: false,
        user_rate_limit: None,
        user_allowed_providers: None,
        user_allowed_api_formats: None,
        user_allowed_models: None,
        api_key_id: "api-key-cached-user".to_string(),
        api_key_name: Some("default".to_string()),
        api_key_is_active: true,
        api_key_is_locked: false,
        api_key_is_standalone: false,
        api_key_rate_limit: None,
        api_key_concurrent_limit: None,
        api_key_expires_at_unix_secs: None,
        api_key_allowed_providers: None,
        api_key_allowed_api_formats: None,
        api_key_allowed_models: None,
        currently_usable: true,
    };
    let cache_key =
        build_scheduler_affinity_cache_key(Some(&auth_snapshot), "openai:chat", "gpt-4.1")
            .expect("cache key should build");
    state.scheduler_affinity_cache.insert(
        cache_key,
        SchedulerAffinityTarget {
            provider_id: "provider-a".to_string(),
            endpoint_id: "endpoint-a".to_string(),
            key_id: "key-a".to_string(),
        },
        Duration::from_secs(300),
        100,
    );

    let selected = select_minimal_candidate(
        &state,
        "openai:chat",
        "gpt-4.1",
        false,
        Some(&auth_snapshot),
        100,
    )
    .await
    .expect("selection should succeed")
    .expect("candidate should exist");

    assert_eq!(selected.provider_id, "provider-a");
    assert_eq!(selected.key_id, "key-a");
}

#[tokio::test]
async fn selects_next_candidate_when_first_provider_key_circuit_is_open() {
    let mut first = sample_row();
    first.provider_id = "provider-a".to_string();
    first.provider_name = "openai-a".to_string();
    first.endpoint_id = "endpoint-a".to_string();
    first.key_id = "key-a".to_string();
    first.key_name = "alpha".to_string();
    first.key_global_priority_by_format = Some(serde_json::json!({"openai:chat": 1}));

    let mut second = sample_row();
    second.provider_id = "provider-b".to_string();
    second.provider_name = "openai-b".to_string();
    second.endpoint_id = "endpoint-b".to_string();
    second.key_id = "key-b".to_string();
    second.key_name = "beta".to_string();
    second.key_global_priority_by_format = Some(serde_json::json!({"openai:chat": 2}));

    let candidates = Arc::new(InMemoryMinimalCandidateSelectionReadRepository::seed(vec![
        first, second,
    ]));
    let provider_catalog = Arc::new(InMemoryProviderCatalogReadRepository::seed(
        vec![
            sample_provider("provider-a", None),
            sample_provider("provider-b", None),
        ],
        Vec::new(),
        vec![
            sample_key("key-a", "provider-a", Some(10)).with_health_fields(
                Some(serde_json::json!({"openai:chat": {"health_score": 0.2}})),
                Some(serde_json::json!({"openai:chat": {"open": true}})),
            ),
            sample_key("key-b", "provider-b", Some(10)),
        ],
    ));
    let quotas = Arc::new(InMemoryProviderQuotaRepository::seed(vec![]));
    let request_candidates = Arc::new(InMemoryRequestCandidateRepository::seed(vec![]));
    let state = AppState::new()
            .expect("state should build")
            .with_data_state_for_tests(
                GatewayDataState::with_candidate_selection_provider_catalog_quota_and_request_candidates_for_tests(
                    candidates,
                    provider_catalog,
                    quotas,
                    request_candidates,
                ),
            );

    let selected = select_minimal_candidate(&state, "openai:chat", "gpt-4.1", false, None, 100)
        .await
        .expect("selection should succeed")
        .expect("candidate should exist");

    assert_eq!(selected.provider_id, "provider-b");
    assert_eq!(selected.key_id, "key-b");
}

#[tokio::test]
async fn same_priority_candidates_prefer_healthier_provider_key_before_id_order() {
    let mut first = sample_row();
    first.provider_id = "provider-a".to_string();
    first.provider_name = "openai-a".to_string();
    first.endpoint_id = "endpoint-a".to_string();
    first.key_id = "key-a".to_string();
    first.key_name = "alpha".to_string();
    first.key_global_priority_by_format = Some(serde_json::json!({"openai:chat": 1}));

    let mut second = sample_row();
    second.provider_id = "provider-b".to_string();
    second.provider_name = "openai-b".to_string();
    second.endpoint_id = "endpoint-b".to_string();
    second.key_id = "key-b".to_string();
    second.key_name = "beta".to_string();
    second.key_global_priority_by_format = Some(serde_json::json!({"openai:chat": 1}));

    let candidates = Arc::new(InMemoryMinimalCandidateSelectionReadRepository::seed(vec![
        first, second,
    ]));
    let provider_catalog = Arc::new(InMemoryProviderCatalogReadRepository::seed(
        vec![
            sample_provider("provider-a", None),
            sample_provider("provider-b", None),
        ],
        Vec::new(),
        vec![
            sample_key("key-a", "provider-a", Some(10)).with_health_fields(
                Some(serde_json::json!({"openai:chat": {"health_score": 0.30}})),
                None,
            ),
            sample_key("key-b", "provider-b", Some(10)).with_health_fields(
                Some(serde_json::json!({"openai:chat": {"health_score": 0.95}})),
                None,
            ),
        ],
    ));
    let quotas = Arc::new(InMemoryProviderQuotaRepository::seed(vec![]));
    let request_candidates = Arc::new(InMemoryRequestCandidateRepository::seed(vec![]));
    let state = AppState::new()
            .expect("state should build")
            .with_data_state_for_tests(
                GatewayDataState::with_candidate_selection_provider_catalog_quota_and_request_candidates_for_tests(
                    candidates,
                    provider_catalog,
                    quotas,
                    request_candidates,
                ),
            );

    let selected = select_minimal_candidate(&state, "openai:chat", "gpt-4.1", false, None, 100)
        .await
        .expect("selection should succeed")
        .expect("candidate should exist");

    assert_eq!(selected.provider_id, "provider-b");
    assert_eq!(selected.key_id, "key-b");
}

#[tokio::test]
async fn same_priority_candidates_use_aggregate_health_score_when_api_format_specific_health_is_missing(
) {
    let mut first = sample_row();
    first.provider_id = "provider-a".to_string();
    first.provider_name = "openai-a".to_string();
    first.endpoint_id = "endpoint-a".to_string();
    first.key_id = "key-a".to_string();
    first.key_name = "alpha".to_string();
    first.key_global_priority_by_format = Some(serde_json::json!({"openai:chat": 1}));

    let mut second = sample_row();
    second.provider_id = "provider-b".to_string();
    second.provider_name = "openai-b".to_string();
    second.endpoint_id = "endpoint-b".to_string();
    second.key_id = "key-b".to_string();
    second.key_name = "beta".to_string();
    second.key_global_priority_by_format = Some(serde_json::json!({"openai:chat": 1}));

    let candidates = Arc::new(InMemoryMinimalCandidateSelectionReadRepository::seed(vec![
        first, second,
    ]));
    let provider_catalog = Arc::new(InMemoryProviderCatalogReadRepository::seed(
        vec![
            sample_provider("provider-a", None),
            sample_provider("provider-b", None),
        ],
        Vec::new(),
        vec![
            sample_key("key-a", "provider-a", Some(10)).with_health_fields(
                Some(serde_json::json!({
                    "openai:responses": {"health_score": 0.40},
                    "claude:chat": {"health_score": 0.55}
                })),
                None,
            ),
            sample_key("key-b", "provider-b", Some(10)).with_health_fields(
                Some(serde_json::json!({
                    "openai:responses": {"health_score": 0.90},
                    "claude:chat": {"health_score": 0.92}
                })),
                None,
            ),
        ],
    ));
    let quotas = Arc::new(InMemoryProviderQuotaRepository::seed(vec![]));
    let request_candidates = Arc::new(InMemoryRequestCandidateRepository::seed(vec![]));
    let state = AppState::new()
            .expect("state should build")
            .with_data_state_for_tests(
                GatewayDataState::with_candidate_selection_provider_catalog_quota_and_request_candidates_for_tests(
                    candidates,
                    provider_catalog,
                    quotas,
                    request_candidates,
                ),
            );

    let selected = select_minimal_candidate(&state, "openai:chat", "gpt-4.1", false, None, 100)
        .await
        .expect("selection should succeed")
        .expect("candidate should exist");

    assert_eq!(selected.provider_id, "provider-b");
    assert_eq!(selected.key_id, "key-b");
}
