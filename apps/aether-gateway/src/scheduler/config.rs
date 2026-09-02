use aether_data_contracts::repository::routing_profiles::RoutingGroupLookupKey;
use aether_routing_core::{
    ResolvedRoutingPolicy, RoutingDefaultPolicy, RoutingSchedulingMode, RoutingSetPriorityMode,
};
use aether_scheduler_core::SchedulerPriorityMode;
use tracing::warn;

use crate::{AppState, GatewayError};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub(crate) enum SchedulerSchedulingMode {
    FixedOrder,
    #[default]
    CacheAffinity,
    LoadBalance,
}

impl SchedulerSchedulingMode {
    pub(crate) fn as_str(self) -> &'static str {
        match self {
            Self::FixedOrder => "fixed_order",
            Self::CacheAffinity => "cache_affinity",
            Self::LoadBalance => "load_balance",
        }
    }
}

pub(crate) fn scheduler_priority_mode_as_str(mode: SchedulerPriorityMode) -> &'static str {
    match mode {
        SchedulerPriorityMode::Provider => "provider",
        SchedulerPriorityMode::GlobalKey => "global_key",
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct SchedulerOrderingConfig {
    pub(crate) priority_mode: SchedulerPriorityMode,
    pub(crate) scheduling_mode: SchedulerSchedulingMode,
    pub(crate) keep_priority_on_conversion: bool,
}

impl Default for SchedulerOrderingConfig {
    fn default() -> Self {
        Self {
            priority_mode: SchedulerPriorityMode::Provider,
            scheduling_mode: SchedulerSchedulingMode::CacheAffinity,
            keep_priority_on_conversion: false,
        }
    }
}

impl SchedulerOrderingConfig {
    /// Ordering config derived from a resolved routing policy. The policy is
    /// the single source of truth: no legacy system-config value is merged in.
    pub(crate) fn from_routing_policy(policy: &ResolvedRoutingPolicy) -> Self {
        Self {
            priority_mode: scheduler_priority_mode_from_routing(policy.priority_mode),
            scheduling_mode: scheduler_scheduling_mode_from_routing(policy.scheduling_mode),
            keep_priority_on_conversion: policy.keep_priority_on_conversion,
        }
    }

    pub(crate) fn from_routing_default_policy(policy: &RoutingDefaultPolicy) -> Self {
        Self {
            priority_mode: scheduler_priority_mode_from_routing(policy.priority_mode),
            scheduling_mode: scheduler_scheduling_mode_from_routing(policy.scheduling_mode),
            keep_priority_on_conversion: policy.keep_priority_on_conversion,
        }
    }

    pub(crate) fn to_routing_default_policy(self) -> RoutingDefaultPolicy {
        RoutingDefaultPolicy {
            priority_mode: match self.priority_mode {
                SchedulerPriorityMode::Provider => RoutingSetPriorityMode::Provider,
                SchedulerPriorityMode::GlobalKey => RoutingSetPriorityMode::GlobalKey,
            },
            scheduling_mode: match self.scheduling_mode {
                SchedulerSchedulingMode::FixedOrder => RoutingSchedulingMode::FixedOrder,
                SchedulerSchedulingMode::CacheAffinity => RoutingSchedulingMode::CacheAffinity,
                SchedulerSchedulingMode::LoadBalance => RoutingSchedulingMode::LoadBalance,
            },
            keep_priority_on_conversion: self.keep_priority_on_conversion,
        }
    }

    pub(crate) fn priority_mode_str(self) -> &'static str {
        scheduler_priority_mode_as_str(self.priority_mode)
    }

    pub(crate) fn scheduling_mode_str(self) -> &'static str {
        self.scheduling_mode.as_str()
    }
}

fn scheduler_priority_mode_from_routing(mode: RoutingSetPriorityMode) -> SchedulerPriorityMode {
    match mode {
        RoutingSetPriorityMode::Provider => SchedulerPriorityMode::Provider,
        RoutingSetPriorityMode::GlobalKey => SchedulerPriorityMode::GlobalKey,
    }
}

fn scheduler_scheduling_mode_from_routing(mode: RoutingSchedulingMode) -> SchedulerSchedulingMode {
    match mode {
        RoutingSchedulingMode::FixedOrder => SchedulerSchedulingMode::FixedOrder,
        RoutingSchedulingMode::CacheAffinity => SchedulerSchedulingMode::CacheAffinity,
        RoutingSchedulingMode::LoadBalance => SchedulerSchedulingMode::LoadBalance,
    }
}

pub(crate) fn parse_scheduler_priority_mode(
    value: Option<&serde_json::Value>,
) -> SchedulerPriorityMode {
    match value
        .and_then(serde_json::Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(|value| value.to_ascii_lowercase())
        .as_deref()
    {
        Some("global_key") => SchedulerPriorityMode::GlobalKey,
        _ => SchedulerPriorityMode::Provider,
    }
}

pub(crate) fn parse_keep_priority_on_conversion(value: Option<&serde_json::Value>) -> bool {
    value.and_then(serde_json::Value::as_bool).unwrap_or(false)
}

pub(crate) fn parse_scheduler_scheduling_mode(
    value: Option<&serde_json::Value>,
) -> SchedulerSchedulingMode {
    match value
        .and_then(serde_json::Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(|value| value.to_ascii_lowercase())
        .as_deref()
    {
        Some("fixed_order") => SchedulerSchedulingMode::FixedOrder,
        Some("load_balance") => SchedulerSchedulingMode::LoadBalance,
        _ => SchedulerSchedulingMode::CacheAffinity,
    }
}

/// Effective scheduler ordering config for requests that carry no resolved
/// routing policy.
///
/// Resolution order:
/// 1. the enabled system-default routing group's `default_policy`;
/// 2. the legacy system-config keys (`provider_priority_mode`,
///    `scheduling_mode`, `keep_priority_on_conversion`).
///
/// Step 2 only exists so deployments that never created a routing group keep
/// their behaviour; once the legacy keys are removed this function collapses
/// to step 1 plus `SchedulerOrderingConfig::default()`.
pub(crate) async fn read_scheduler_ordering_config(
    state: &AppState,
) -> Result<SchedulerOrderingConfig, GatewayError> {
    if let Some(config) = read_system_default_routing_ordering_config(state).await? {
        return Ok(config);
    }
    read_legacy_scheduler_ordering_config(state).await
}

/// Ordering config from the enabled system-default routing group, if any.
pub(crate) async fn read_system_default_routing_ordering_config(
    state: &AppState,
) -> Result<Option<SchedulerOrderingConfig>, GatewayError> {
    let Some(group) = state
        .find_routing_group(RoutingGroupLookupKey::SystemDefault)
        .await?
        .filter(|group| group.enabled)
    else {
        return Ok(None);
    };
    let default_policy = match group.config_json.get("default_policy") {
        None | Some(serde_json::Value::Null) => RoutingDefaultPolicy::default(),
        Some(value) => match serde_json::from_value::<RoutingDefaultPolicy>(value.clone()) {
            Ok(policy) => policy,
            Err(error) => {
                warn!(
                    event_name = "scheduler_system_default_routing_policy_invalid",
                    log_type = "event",
                    group_id = %group.id,
                    error = %error,
                    "system default routing group has an invalid default_policy; ignoring it"
                );
                return Ok(None);
            }
        },
    };
    Ok(Some(SchedulerOrderingConfig::from_routing_default_policy(
        &default_policy,
    )))
}

/// Legacy system-config based ordering config. Kept only as a migration
/// fallback; see `read_scheduler_ordering_config`.
pub(crate) async fn read_legacy_scheduler_ordering_config(
    state: &AppState,
) -> Result<SchedulerOrderingConfig, GatewayError> {
    let priority_mode = parse_scheduler_priority_mode(
        state
            .read_system_config_json_value("provider_priority_mode")
            .await?
            .as_ref(),
    );
    let scheduling_mode = parse_scheduler_scheduling_mode(
        state
            .read_system_config_json_value("scheduling_mode")
            .await?
            .as_ref(),
    );
    let keep_priority_on_conversion = parse_keep_priority_on_conversion(
        state
            .read_system_config_json_value("keep_priority_on_conversion")
            .await?
            .as_ref(),
    );
    Ok(SchedulerOrderingConfig {
        priority_mode,
        scheduling_mode,
        keep_priority_on_conversion,
    })
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use aether_data::repository::routing_profiles::InMemoryRoutingGroupRepository;
    use aether_data_contracts::repository::routing_profiles::{
        CreateRoutingGroupRecord, RoutingGroupLookupKey, RoutingGroupReadRepository,
        RoutingGroupWriteRepository,
    };
    use serde_json::json;

    use super::*;
    use crate::data::GatewayDataState;

    fn legacy_values() -> [(String, serde_json::Value); 3] {
        [
            ("provider_priority_mode".to_string(), json!("global_key")),
            ("scheduling_mode".to_string(), json!("load_balance")),
            ("keep_priority_on_conversion".to_string(), json!(true)),
        ]
    }

    async fn create_system_default(
        repository: &InMemoryRoutingGroupRepository,
        enabled: bool,
        config_json: serde_json::Value,
    ) {
        repository
            .create_routing_group(CreateRoutingGroupRecord {
                id: "system-default".to_string(),
                name: "system-default".to_string(),
                description: None,
                enabled,
                is_system_default: true,
                config_json,
                version: 1,
                created_at: 1,
                updated_at: 1,
                published_at: None,
            })
            .await
            .unwrap();
    }

    #[tokio::test]
    async fn system_default_routing_group_overrides_legacy_keys() {
        let repository = Arc::new(InMemoryRoutingGroupRepository::default());
        create_system_default(
            &repository,
            true,
            json!({
                "default_policy": {
                    "priority_mode": "provider",
                    "scheduling_mode": "fixed_order",
                    "keep_priority_on_conversion": false
                }
            }),
        )
        .await;
        let state = AppState::new().unwrap().with_data_state_for_tests(
            GatewayDataState::disabled()
                .with_system_config_values_for_tests(legacy_values())
                .with_routing_group_repository_for_tests(repository),
        );

        let config = read_scheduler_ordering_config(&state).await.unwrap();

        assert_eq!(config.priority_mode, SchedulerPriorityMode::Provider);
        assert_eq!(config.scheduling_mode, SchedulerSchedulingMode::FixedOrder);
        assert!(!config.keep_priority_on_conversion);
    }

    #[tokio::test]
    async fn missing_default_policy_in_system_default_group_uses_routing_defaults() {
        let repository = Arc::new(InMemoryRoutingGroupRepository::default());
        create_system_default(&repository, true, json!({})).await;
        let state = AppState::new().unwrap().with_data_state_for_tests(
            GatewayDataState::disabled()
                .with_system_config_values_for_tests(legacy_values())
                .with_routing_group_repository_for_tests(repository),
        );

        let config = read_scheduler_ordering_config(&state).await.unwrap();

        assert_eq!(config, SchedulerOrderingConfig::default());
    }

    #[tokio::test]
    async fn disabled_or_missing_system_default_group_falls_back_to_legacy_keys() {
        let repository = Arc::new(InMemoryRoutingGroupRepository::default());
        create_system_default(
            &repository,
            false,
            json!({"default_policy": {"scheduling_mode": "fixed_order"}}),
        )
        .await;
        let with_disabled_group = AppState::new().unwrap().with_data_state_for_tests(
            GatewayDataState::disabled()
                .with_system_config_values_for_tests(legacy_values())
                .with_routing_group_repository_for_tests(repository),
        );
        let without_repository = AppState::new().unwrap().with_data_state_for_tests(
            GatewayDataState::disabled().with_system_config_values_for_tests(legacy_values()),
        );

        for state in [with_disabled_group, without_repository] {
            let config = read_scheduler_ordering_config(&state).await.unwrap();
            assert_eq!(config.priority_mode, SchedulerPriorityMode::GlobalKey);
            assert_eq!(config.scheduling_mode, SchedulerSchedulingMode::LoadBalance);
            assert!(config.keep_priority_on_conversion);
        }
    }

    #[tokio::test]
    async fn bootstrap_creates_system_default_group_from_legacy_keys_once() {
        let repository = Arc::new(InMemoryRoutingGroupRepository::default());
        let state = AppState::new().unwrap().with_data_state_for_tests(
            GatewayDataState::disabled()
                .with_system_config_values_for_tests(legacy_values())
                .with_routing_group_repository_for_tests(repository.clone()),
        );

        let created = state
            .ensure_system_default_routing_group_inner()
            .await
            .unwrap()
            .expect("first bootstrap should create the system default group");
        assert!(created.enabled);
        assert!(created.is_system_default);
        assert_eq!(
            created.config_json["default_policy"],
            json!({
                "priority_mode": "global_key",
                "scheduling_mode": "load_balance",
                "keep_priority_on_conversion": true
            })
        );

        let second = state
            .ensure_system_default_routing_group_inner()
            .await
            .unwrap();
        assert!(second.is_none(), "bootstrap must be idempotent");
        assert_eq!(
            repository
                .find_routing_group(RoutingGroupLookupKey::SystemDefault)
                .await
                .unwrap()
                .map(|group| group.id),
            Some(created.id)
        );

        let config = read_scheduler_ordering_config(&state).await.unwrap();
        assert_eq!(config.priority_mode, SchedulerPriorityMode::GlobalKey);
        assert_eq!(config.scheduling_mode, SchedulerSchedulingMode::LoadBalance);
        assert!(config.keep_priority_on_conversion);
    }
}
