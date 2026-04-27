pub mod capability;
pub mod enumeration;
pub mod identity;
pub mod selectability;
pub mod types;

pub use capability::{
    candidate_supports_required_capability, requested_capability_priority_for_candidate,
};
pub use enumeration::{
    build_minimal_candidate_selection, collect_global_model_names_for_required_capability,
    enumerate_minimal_candidate_selection,
};
pub use identity::compare_candidates_by_priority_mode;
pub use selectability::{
    auth_api_key_concurrency_limit_reached, candidate_is_selectable_with_runtime_state,
    candidate_runtime_skip_reason_with_state, collect_selectable_candidates_from_keys,
    reorder_candidates_by_scheduler_health, CandidateRuntimeSelectabilityInput,
};
pub use types::{
    BuildMinimalCandidateSelectionInput, SchedulerMinimalCandidateSelectionCandidate,
    SchedulerPriorityMode,
};

#[cfg(test)]
mod tests {
    use std::collections::{BTreeMap, BTreeSet};

    use aether_data_contracts::repository::candidate_selection::{
        StoredMinimalCandidateSelectionRow, StoredProviderModelMapping,
    };
    use aether_data_contracts::repository::candidates::{
        RequestCandidateStatus, StoredRequestCandidate,
    };
    use aether_data_contracts::repository::provider_catalog::StoredProviderCatalogKey;

    use super::{
        auth_api_key_concurrency_limit_reached, build_minimal_candidate_selection,
        candidate_is_selectable_with_runtime_state, candidate_supports_required_capability,
        collect_global_model_names_for_required_capability,
        collect_selectable_candidates_from_keys, reorder_candidates_by_scheduler_health,
        BuildMinimalCandidateSelectionInput, CandidateRuntimeSelectabilityInput,
        SchedulerMinimalCandidateSelectionCandidate, SchedulerPriorityMode,
    };
    use crate::SchedulerAuthConstraints;

    fn sample_row(id: &str) -> StoredMinimalCandidateSelectionRow {
        StoredMinimalCandidateSelectionRow {
            provider_id: format!("provider-{id}"),
            provider_name: format!("Provider {id}"),
            provider_type: "custom".to_string(),
            provider_priority: 10,
            provider_is_active: true,
            endpoint_id: format!("endpoint-{id}"),
            endpoint_api_format: "openai:chat".to_string(),
            endpoint_api_family: Some("openai".to_string()),
            endpoint_kind: Some("chat".to_string()),
            endpoint_is_active: true,
            key_id: format!("key-{id}"),
            key_name: format!("prod-{id}"),
            key_auth_type: "api_key".to_string(),
            key_is_active: true,
            key_api_formats: Some(vec!["openai:chat".to_string()]),
            key_allowed_models: None,
            key_capabilities: Some(serde_json::json!({"cache_1h": true})),
            key_internal_priority: 50,
            key_global_priority_by_format: Some(serde_json::json!({"openai:chat": 2})),
            model_id: format!("model-{id}"),
            global_model_id: format!("global-model-{id}"),
            global_model_name: "gpt-5".to_string(),
            global_model_mappings: Some(vec!["gpt-5(?:\\.\\d+)?".to_string()]),
            global_model_supports_streaming: Some(true),
            model_provider_model_name: format!("gpt-5-upstream-{id}"),
            model_provider_model_mappings: Some(vec![StoredProviderModelMapping {
                name: format!("gpt-5-canary-{id}"),
                priority: 1,
                api_formats: Some(vec!["openai:chat".to_string()]),
            }]),
            model_supports_streaming: None,
            model_is_active: true,
            model_is_available: true,
        }
    }
    fn sample_candidate(
        id: &str,
        capabilities: Option<serde_json::Value>,
    ) -> SchedulerMinimalCandidateSelectionCandidate {
        SchedulerMinimalCandidateSelectionCandidate {
            provider_id: format!("provider-{id}"),
            provider_name: format!("Provider {id}"),
            provider_type: "openai".to_string(),
            provider_priority: 0,
            endpoint_id: format!("endpoint-{id}"),
            endpoint_api_format: "openai:chat".to_string(),
            key_id: format!("key-{id}"),
            key_name: format!("key-{id}"),
            key_auth_type: "bearer".to_string(),
            key_internal_priority: 0,
            key_global_priority_for_format: None,
            key_capabilities: capabilities,
            model_id: format!("model-{id}"),
            global_model_id: format!("global-model-{id}"),
            global_model_name: "gpt-5".to_string(),
            selected_provider_model_name: "gpt-5".to_string(),
            mapping_matched_model: None,
        }
    }

    fn sample_key(id: &str, health_score: f64) -> StoredProviderCatalogKey {
        let mut key = StoredProviderCatalogKey::new(
            format!("key-{id}"),
            format!("provider-{id}"),
            format!("key-{id}"),
            "api_key".to_string(),
            None,
            true,
        )
        .expect("provider key should build");
        key.health_by_format = Some(serde_json::json!({
            "openai:chat": {
                "health_score": health_score
            }
        }));
        key
    }

    fn stored_candidate(
        id: &str,
        status: RequestCandidateStatus,
        created_at_unix_ms: i64,
    ) -> StoredRequestCandidate {
        let finished_at_unix_ms = match status {
            RequestCandidateStatus::Pending | RequestCandidateStatus::Streaming => None,
            _ => Some(created_at_unix_ms),
        };
        StoredRequestCandidate::new(
            id.to_string(),
            format!("req-{id}"),
            None,
            None,
            None,
            None,
            0,
            0,
            Some("provider-1".to_string()),
            Some("endpoint-1".to_string()),
            Some("key-1".to_string()),
            status,
            None,
            false,
            None,
            None,
            None,
            None,
            None,
            None,
            None,
            created_at_unix_ms,
            Some(created_at_unix_ms),
            finished_at_unix_ms,
        )
        .expect("candidate should build")
    }

    #[test]
    fn reads_required_capability_from_object_and_array_forms() {
        assert!(candidate_supports_required_capability(
            &sample_candidate("1", Some(serde_json::json!({"vision": true}))),
            "vision"
        ));
        assert!(candidate_supports_required_capability(
            &sample_candidate("1", Some(serde_json::json!(["vision", "tools"]))),
            "tools"
        ));
        assert!(!candidate_supports_required_capability(
            &sample_candidate("1", Some(serde_json::json!({"vision": false}))),
            "vision"
        ));
    }

    #[test]
    fn builds_minimal_candidate_selection_with_auth_constraints() {
        let mut disallowed = sample_row("2");
        disallowed.provider_id = "provider-blocked".to_string();
        disallowed.provider_name = "Blocked".to_string();

        let constraints = SchedulerAuthConstraints {
            allowed_providers: Some(vec!["provider-1".to_string()]),
            allowed_api_formats: Some(vec!["OPENAI:CHAT".to_string()]),
            allowed_models: Some(vec!["gpt-5".to_string()]),
        };
        let candidates = build_minimal_candidate_selection(BuildMinimalCandidateSelectionInput {
            rows: vec![sample_row("1"), disallowed],
            normalized_api_format: "openai:chat",
            requested_model_name: "gpt-5",
            resolved_global_model_name: "gpt-5",
            require_streaming: false,
            required_capabilities: None,
            auth_constraints: Some(&constraints),
            affinity_key: None,
            priority_mode: SchedulerPriorityMode::Provider,
        })
        .expect("candidate selection should build");

        assert_eq!(candidates.len(), 1);
        assert_eq!(candidates[0].provider_id, "provider-1");
        assert_eq!(candidates[0].selected_provider_model_name, "gpt-5-canary-1");
    }

    #[test]
    fn enumeration_preserves_theoretical_candidate_order_without_final_sorting() {
        let mut later_priority = sample_row("1");
        later_priority.provider_priority = 10;
        let mut earlier_priority = sample_row("2");
        earlier_priority.provider_priority = 0;

        let candidates =
            super::enumerate_minimal_candidate_selection(BuildMinimalCandidateSelectionInput {
                rows: vec![later_priority, earlier_priority],
                normalized_api_format: "openai:chat",
                requested_model_name: "gpt-5",
                resolved_global_model_name: "gpt-5",
                require_streaming: false,
                required_capabilities: None,
                auth_constraints: None,
                affinity_key: None,
                priority_mode: SchedulerPriorityMode::Provider,
            })
            .expect("candidate enumeration should build");

        assert_eq!(candidates.len(), 2);
        assert_eq!(candidates[0].provider_id, "provider-1");
        assert_eq!(candidates[1].provider_id, "provider-2");
    }

    #[test]
    fn collects_global_model_names_for_required_capability_with_auth_constraints() {
        let mut disallowed = sample_row("2");
        disallowed.global_model_name = "gpt-4.1".to_string();
        disallowed.provider_id = "provider-blocked".to_string();
        disallowed.provider_name = "Blocked".to_string();

        let constraints = SchedulerAuthConstraints {
            allowed_providers: Some(vec!["provider-1".to_string()]),
            allowed_api_formats: Some(vec!["openai:chat".to_string()]),
            allowed_models: Some(vec!["gpt-5".to_string()]),
        };
        let model_names = collect_global_model_names_for_required_capability(
            vec![sample_row("1"), disallowed],
            "openai:chat",
            "cache_1h",
            false,
            Some(&constraints),
        );

        assert_eq!(model_names, vec!["gpt-5".to_string()]);
    }

    #[test]
    fn minimal_candidate_selection_prefers_matching_requested_capabilities_before_priority() {
        let mut missing_capability = sample_row("1");
        missing_capability.key_capabilities = Some(serde_json::json!({"cache_1h": false}));
        missing_capability.provider_priority = 0;

        let mut matching_capability = sample_row("2");
        matching_capability.key_capabilities = Some(serde_json::json!({"cache_1h": true}));
        matching_capability.provider_priority = 10;

        let required_capabilities = serde_json::json!({"cache_1h": true});
        let candidates = build_minimal_candidate_selection(BuildMinimalCandidateSelectionInput {
            rows: vec![missing_capability, matching_capability],
            normalized_api_format: "openai:chat",
            requested_model_name: "gpt-5",
            resolved_global_model_name: "gpt-5",
            require_streaming: false,
            required_capabilities: Some(&required_capabilities),
            auth_constraints: None,
            affinity_key: None,
            priority_mode: SchedulerPriorityMode::Provider,
        })
        .expect("candidate selection should build");

        assert_eq!(candidates.len(), 2);
        assert_eq!(candidates[0].key_id, "key-2");
        assert_eq!(candidates[1].key_id, "key-1");
    }

    #[test]
    fn reorders_candidates_by_health_before_affinity_tiebreak() {
        let mut candidates = vec![
            sample_candidate("1", None),
            sample_candidate("2", None),
            sample_candidate("3", None),
        ];
        let provider_key_rpm_states = BTreeMap::from([
            ("key-1".to_string(), sample_key("1", 0.95)),
            ("key-2".to_string(), sample_key("2", 0.40)),
            ("key-3".to_string(), sample_key("3", 0.95)),
        ]);

        reorder_candidates_by_scheduler_health(
            &mut candidates,
            &provider_key_rpm_states,
            None,
            Some("api-key-1"),
            SchedulerPriorityMode::GlobalKey,
        );

        assert_ne!(candidates[0].key_id, "key-2");
        assert_ne!(candidates[1].key_id, "key-2");
        assert_eq!(candidates[2].key_id, "key-2");
    }

    #[test]
    fn collects_selectable_candidates_with_affinity_priority_and_dedup() {
        let candidates = vec![
            sample_candidate("1", None),
            sample_candidate("2", None),
            sample_candidate("1", None),
        ];
        let selectable_keys = BTreeSet::from([
            (
                "provider-1".to_string(),
                "endpoint-1".to_string(),
                "key-1".to_string(),
            ),
            (
                "provider-2".to_string(),
                "endpoint-2".to_string(),
                "key-2".to_string(),
            ),
        ]);
        let selected = collect_selectable_candidates_from_keys(
            candidates,
            &selectable_keys,
            Some(&crate::SchedulerAffinityTarget {
                provider_id: "provider-2".to_string(),
                endpoint_id: "endpoint-2".to_string(),
                key_id: "key-2".to_string(),
            }),
        );

        assert_eq!(selected.len(), 2);
        assert_eq!(selected[0].key_id, "key-2");
        assert_eq!(selected[1].key_id, "key-1");
    }

    #[test]
    fn candidate_selectability_respects_provider_concurrency_limit() {
        let recent_candidates = vec![stored_candidate("one", RequestCandidateStatus::Pending, 95)];
        let provider_concurrent_limits = BTreeMap::from([("provider-1".to_string(), 1)]);

        assert!(!candidate_is_selectable_with_runtime_state(
            CandidateRuntimeSelectabilityInput {
                candidate: &sample_candidate("1", None),
                recent_candidates: &recent_candidates,
                provider_concurrent_limits: &provider_concurrent_limits,
                provider_key_rpm_states: &BTreeMap::new(),
                now_unix_secs: 100,
                cached_affinity_target: None,
                provider_quota_blocks_requests: false,
                account_quota_exhausted: false,
                oauth_invalid: false,
                rpm_reset_at: None,
            },
        ));
    }

    #[test]
    fn candidate_selectability_rejects_quota_or_zero_health() {
        let provider_key_rpm_states = BTreeMap::from([("key-1".to_string(), sample_key("1", 0.0))]);

        assert!(!candidate_is_selectable_with_runtime_state(
            CandidateRuntimeSelectabilityInput {
                candidate: &sample_candidate("1", None),
                recent_candidates: &[],
                provider_concurrent_limits: &BTreeMap::new(),
                provider_key_rpm_states: &provider_key_rpm_states,
                now_unix_secs: 100,
                cached_affinity_target: None,
                provider_quota_blocks_requests: false,
                account_quota_exhausted: false,
                oauth_invalid: false,
                rpm_reset_at: None,
            },
        ));
        assert!(!candidate_is_selectable_with_runtime_state(
            CandidateRuntimeSelectabilityInput {
                candidate: &sample_candidate("1", None),
                recent_candidates: &[],
                provider_concurrent_limits: &BTreeMap::new(),
                provider_key_rpm_states: &BTreeMap::new(),
                now_unix_secs: 100,
                cached_affinity_target: None,
                provider_quota_blocks_requests: true,
                account_quota_exhausted: false,
                oauth_invalid: false,
                rpm_reset_at: None,
            },
        ));
    }

    #[test]
    fn candidate_selectability_rejects_exhausted_account_quota() {
        assert!(!candidate_is_selectable_with_runtime_state(
            CandidateRuntimeSelectabilityInput {
                candidate: &sample_candidate("1", None),
                recent_candidates: &[],
                provider_concurrent_limits: &BTreeMap::new(),
                provider_key_rpm_states: &BTreeMap::new(),
                now_unix_secs: 100,
                cached_affinity_target: None,
                provider_quota_blocks_requests: false,
                account_quota_exhausted: true,
                oauth_invalid: false,
                rpm_reset_at: None,
            },
        ));
    }

    #[test]
    fn candidate_selectability_rejects_oauth_invalid_keys() {
        assert!(!candidate_is_selectable_with_runtime_state(
            CandidateRuntimeSelectabilityInput {
                candidate: &sample_candidate("1", None),
                recent_candidates: &[],
                provider_concurrent_limits: &BTreeMap::new(),
                provider_key_rpm_states: &BTreeMap::new(),
                now_unix_secs: 100,
                cached_affinity_target: None,
                provider_quota_blocks_requests: false,
                account_quota_exhausted: false,
                oauth_invalid: true,
                rpm_reset_at: None,
            },
        ));
    }

    #[test]
    fn detects_auth_api_key_concurrency_limit_from_recent_active_requests() {
        let recent_candidates = vec![StoredRequestCandidate::new(
            "one".to_string(),
            "req-one".to_string(),
            None,
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
        .expect("candidate should build")];

        assert!(auth_api_key_concurrency_limit_reached(
            &recent_candidates,
            100,
            "api-key-1",
            1,
        ));
        assert!(!auth_api_key_concurrency_limit_reached(
            &recent_candidates,
            100,
            "api-key-1",
            2,
        ));
    }
}
