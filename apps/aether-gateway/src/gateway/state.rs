use std::collections::HashMap;
use std::sync::Arc;
use std::sync::Mutex as StdMutex;
use std::time::Duration;

use aether_runtime::{ConcurrencyGate, DistributedConcurrencyGate};

use super::async_task::{VideoTaskPollerConfig, VideoTaskService};
use super::error::GatewayError;
use super::fallback_metrics;
use super::gateway_cache::{
    AuthApiKeyLastUsedCache, AuthContextCache, DirectPlanBypassCache, SchedulerAffinityCache,
};
use super::gateway_data::GatewayDataState;
use super::rate_limit::FrontdoorUserRpmLimiter;
use super::{provider_transport, usage};
#[path = "state/catalog.rs"]
mod catalog;
#[path = "state/core.rs"]
mod core;
#[path = "state/oauth.rs"]
mod oauth;
#[path = "state/runtime.rs"]
mod runtime;
#[cfg(test)]
#[path = "state/testing.rs"]
mod testing;
#[path = "state/video.rs"]
mod video;

const AUTH_API_KEY_LAST_USED_TTL: Duration = Duration::from_secs(60);
const AUTH_API_KEY_LAST_USED_MAX_ENTRIES: usize = 10_000;
const PROVIDER_TRANSPORT_SNAPSHOT_CACHE_TTL: Duration = Duration::from_secs(1);
const PROVIDER_TRANSPORT_SNAPSHOT_CACHE_MAX_ENTRIES: usize = 1_024;

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
struct ProviderTransportSnapshotCacheKey {
    provider_id: String,
    endpoint_id: String,
    key_id: String,
}

impl ProviderTransportSnapshotCacheKey {
    fn new(provider_id: &str, endpoint_id: &str, key_id: &str) -> Option<Self> {
        let provider_id = provider_id.trim();
        let endpoint_id = endpoint_id.trim();
        let key_id = key_id.trim();
        if provider_id.is_empty() || endpoint_id.is_empty() || key_id.is_empty() {
            return None;
        }
        Some(Self {
            provider_id: provider_id.to_string(),
            endpoint_id: endpoint_id.to_string(),
            key_id: key_id.to_string(),
        })
    }
}

#[derive(Debug, Clone)]
struct CachedProviderTransportSnapshot {
    loaded_at: std::time::Instant,
    snapshot: provider_transport::GatewayProviderTransportSnapshot,
}

#[derive(Debug, Clone, Default)]
pub(crate) struct LocalProviderDeleteTaskState {
    pub task_id: String,
    pub provider_id: String,
    pub status: String,
    pub stage: String,
    pub total_keys: usize,
    pub deleted_keys: usize,
    pub total_endpoints: usize,
    pub deleted_endpoints: usize,
    pub message: String,
}

#[derive(Debug, Clone, PartialEq)]
pub(crate) enum LocalMutationOutcome<T> {
    Applied(T),
    NotFound,
    Invalid(String),
    Unavailable,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(crate) struct LocalExecutionRuntimeMissDiagnostic {
    pub(crate) reason: String,
    pub(crate) route_family: Option<String>,
    pub(crate) route_kind: Option<String>,
    pub(crate) public_path: Option<String>,
    pub(crate) plan_kind: Option<String>,
    pub(crate) requested_model: Option<String>,
    pub(crate) candidate_count: Option<usize>,
    pub(crate) skipped_candidate_count: Option<usize>,
    pub(crate) skip_reasons: std::collections::BTreeMap<String, usize>,
}

impl LocalExecutionRuntimeMissDiagnostic {
    pub(crate) fn skip_reasons_summary(&self) -> Option<String> {
        if self.skip_reasons.is_empty() {
            return None;
        }
        Some(
            self.skip_reasons
                .iter()
                .map(|(reason, count)| format!("{reason}={count}"))
                .collect::<Vec<_>>()
                .join(","),
        )
    }
}

#[derive(Debug, Clone, serde::Serialize)]
pub(crate) struct AdminSecurityBlacklistEntry {
    pub(crate) ip_address: String,
    pub(crate) reason: String,
    pub(crate) ttl_seconds: Option<i64>,
}

#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub(crate) struct AdminBillingRuleRecord {
    pub(crate) id: String,
    pub(crate) name: String,
    pub(crate) task_type: String,
    pub(crate) global_model_id: Option<String>,
    pub(crate) model_id: Option<String>,
    pub(crate) expression: String,
    pub(crate) variables: serde_json::Value,
    pub(crate) dimension_mappings: serde_json::Value,
    pub(crate) is_enabled: bool,
    pub(crate) created_at_unix_secs: u64,
    pub(crate) updated_at_unix_secs: u64,
}

#[derive(Debug, Clone, PartialEq)]
pub(crate) struct AdminBillingRuleWriteInput {
    pub(crate) name: String,
    pub(crate) task_type: String,
    pub(crate) global_model_id: Option<String>,
    pub(crate) model_id: Option<String>,
    pub(crate) expression: String,
    pub(crate) variables: serde_json::Value,
    pub(crate) dimension_mappings: serde_json::Value,
    pub(crate) is_enabled: bool,
}

#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub(crate) struct AdminBillingCollectorRecord {
    pub(crate) id: String,
    pub(crate) api_format: String,
    pub(crate) task_type: String,
    pub(crate) dimension_name: String,
    pub(crate) source_type: String,
    pub(crate) source_path: Option<String>,
    pub(crate) value_type: String,
    pub(crate) transform_expression: Option<String>,
    pub(crate) default_value: Option<String>,
    pub(crate) priority: i32,
    pub(crate) is_enabled: bool,
    pub(crate) created_at_unix_secs: u64,
    pub(crate) updated_at_unix_secs: u64,
}

#[derive(Debug, Clone, PartialEq)]
pub(crate) struct AdminBillingCollectorWriteInput {
    pub(crate) api_format: String,
    pub(crate) task_type: String,
    pub(crate) dimension_name: String,
    pub(crate) source_type: String,
    pub(crate) source_path: Option<String>,
    pub(crate) value_type: String,
    pub(crate) transform_expression: Option<String>,
    pub(crate) default_value: Option<String>,
    pub(crate) priority: i32,
    pub(crate) is_enabled: bool,
}

#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub(crate) struct AdminBillingPresetApplyResult {
    pub(crate) preset: String,
    pub(crate) mode: String,
    pub(crate) created: u64,
    pub(crate) updated: u64,
    pub(crate) skipped: u64,
    pub(crate) errors: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub(crate) struct AdminWalletTransactionRecord {
    pub(crate) id: String,
    pub(crate) wallet_id: String,
    pub(crate) category: String,
    pub(crate) reason_code: String,
    pub(crate) amount: f64,
    pub(crate) balance_before: f64,
    pub(crate) balance_after: f64,
    pub(crate) recharge_balance_before: f64,
    pub(crate) recharge_balance_after: f64,
    pub(crate) gift_balance_before: f64,
    pub(crate) gift_balance_after: f64,
    pub(crate) link_type: Option<String>,
    pub(crate) link_id: Option<String>,
    pub(crate) operator_id: Option<String>,
    pub(crate) description: Option<String>,
    pub(crate) created_at_unix_secs: u64,
}

#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub(crate) struct AdminWalletPaymentOrderRecord {
    pub(crate) id: String,
    pub(crate) order_no: String,
    pub(crate) wallet_id: String,
    pub(crate) user_id: Option<String>,
    pub(crate) amount_usd: f64,
    pub(crate) pay_amount: Option<f64>,
    pub(crate) pay_currency: Option<String>,
    pub(crate) exchange_rate: Option<f64>,
    pub(crate) refunded_amount_usd: f64,
    pub(crate) refundable_amount_usd: f64,
    pub(crate) payment_method: String,
    pub(crate) gateway_order_id: Option<String>,
    pub(crate) status: String,
    pub(crate) gateway_response: Option<serde_json::Value>,
    pub(crate) created_at_unix_secs: u64,
    pub(crate) paid_at_unix_secs: Option<u64>,
    pub(crate) credited_at_unix_secs: Option<u64>,
    pub(crate) expires_at_unix_secs: Option<u64>,
}

#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub(crate) struct AdminPaymentCallbackRecord {
    pub(crate) id: String,
    pub(crate) payment_order_id: Option<String>,
    pub(crate) payment_method: String,
    pub(crate) callback_key: String,
    pub(crate) order_no: Option<String>,
    pub(crate) gateway_order_id: Option<String>,
    pub(crate) payload_hash: Option<String>,
    pub(crate) signature_valid: bool,
    pub(crate) status: String,
    pub(crate) payload: Option<serde_json::Value>,
    pub(crate) error_message: Option<String>,
    pub(crate) created_at_unix_secs: u64,
    pub(crate) processed_at_unix_secs: Option<u64>,
}

#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub(crate) struct AdminWalletRefundRecord {
    pub(crate) id: String,
    pub(crate) refund_no: String,
    pub(crate) wallet_id: String,
    pub(crate) user_id: Option<String>,
    pub(crate) payment_order_id: Option<String>,
    pub(crate) source_type: String,
    pub(crate) source_id: Option<String>,
    pub(crate) refund_mode: String,
    pub(crate) amount_usd: f64,
    pub(crate) status: String,
    pub(crate) reason: Option<String>,
    pub(crate) failure_reason: Option<String>,
    pub(crate) gateway_refund_id: Option<String>,
    pub(crate) payout_method: Option<String>,
    pub(crate) payout_reference: Option<String>,
    pub(crate) payout_proof: Option<serde_json::Value>,
    pub(crate) requested_by: Option<String>,
    pub(crate) approved_by: Option<String>,
    pub(crate) processed_by: Option<String>,
    pub(crate) created_at_unix_secs: u64,
    pub(crate) updated_at_unix_secs: u64,
    pub(crate) processed_at_unix_secs: Option<u64>,
    pub(crate) completed_at_unix_secs: Option<u64>,
}

#[derive(Debug, Clone, PartialEq)]
pub(crate) enum AdminWalletMutationOutcome<T> {
    Applied(T),
    NotFound,
    Invalid(String),
    Unavailable,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FrontdoorCorsConfig {
    allowed_origins: Vec<String>,
    allow_credentials: bool,
}

impl FrontdoorCorsConfig {
    pub fn new(allowed_origins: Vec<String>, allow_credentials: bool) -> Option<Self> {
        let allowed_origins = allowed_origins
            .into_iter()
            .map(|value| value.trim().to_string())
            .filter(|value| !value.is_empty())
            .collect::<Vec<_>>();
        if allowed_origins.is_empty() {
            return None;
        }
        let allow_any_origin = allowed_origins.iter().any(|value| value == "*");
        Some(Self {
            allowed_origins,
            allow_credentials: allow_credentials && !allow_any_origin,
        })
    }

    pub fn from_environment(
        environment: &str,
        cors_origins: Option<&str>,
        allow_credentials: bool,
    ) -> Option<Self> {
        let configured = cors_origins
            .unwrap_or_default()
            .split(',')
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(ToOwned::to_owned)
            .collect::<Vec<_>>();
        if !configured.is_empty() {
            return Self::new(configured, allow_credentials);
        }
        if environment.eq_ignore_ascii_case("development") {
            return Self::new(
                vec![
                    "http://localhost:3000".to_string(),
                    "http://localhost:5173".to_string(),
                    "http://127.0.0.1:3000".to_string(),
                    "http://127.0.0.1:5173".to_string(),
                ],
                allow_credentials,
            );
        }
        None
    }

    pub(crate) fn allows_origin(&self, origin: &str) -> bool {
        self.allowed_origins
            .iter()
            .any(|value| value == "*" || value == origin)
    }

    pub(crate) fn allow_any_origin(&self) -> bool {
        self.allowed_origins.iter().any(|value| value == "*")
    }

    pub(crate) fn allow_credentials(&self) -> bool {
        self.allow_credentials
    }

    pub(crate) fn allowed_origins(&self) -> &[String] {
        &self.allowed_origins
    }
}

#[derive(Debug, Clone)]
pub struct AppState {
    #[cfg(test)]
    pub(in crate::gateway) execution_runtime_override_base_url: Option<String>,
    pub(in crate::gateway) data: Arc<GatewayDataState>,
    pub(in crate::gateway) usage_runtime: Arc<usage::UsageRuntime>,
    pub(in crate::gateway) video_tasks: Arc<VideoTaskService>,
    pub(in crate::gateway) video_task_poller: Option<VideoTaskPollerConfig>,
    pub(in crate::gateway) request_gate: Option<Arc<ConcurrencyGate>>,
    pub(in crate::gateway) distributed_request_gate: Option<Arc<DistributedConcurrencyGate>>,
    pub(in crate::gateway) client: reqwest::Client,
    pub(in crate::gateway) auth_context_cache: Arc<AuthContextCache>,
    pub(in crate::gateway) auth_api_key_last_used_cache: Arc<AuthApiKeyLastUsedCache>,
    pub(in crate::gateway) oauth_refresh: Arc<provider_transport::LocalOAuthRefreshCoordinator>,
    pub(in crate::gateway) direct_plan_bypass_cache: Arc<DirectPlanBypassCache>,
    pub(in crate::gateway) scheduler_affinity_cache: Arc<SchedulerAffinityCache>,
    pub(in crate::gateway) fallback_metrics: Arc<fallback_metrics::GatewayFallbackMetrics>,
    pub(in crate::gateway) frontdoor_cors: Option<Arc<FrontdoorCorsConfig>>,
    pub(in crate::gateway) frontdoor_user_rpm: Arc<FrontdoorUserRpmLimiter>,
    pub(in crate::gateway) tunnel: crate::gateway::tunnel::EmbeddedTunnelState,
    provider_transport_snapshot_cache:
        Arc<StdMutex<HashMap<ProviderTransportSnapshotCacheKey, CachedProviderTransportSnapshot>>>,
    pub(in crate::gateway) provider_key_rpm_resets: Arc<StdMutex<HashMap<String, u64>>>,
    pub(in crate::gateway) local_execution_runtime_miss_diagnostics:
        Arc<StdMutex<HashMap<String, LocalExecutionRuntimeMissDiagnostic>>>,
    pub(in crate::gateway) admin_monitoring_error_stats_reset_at: Arc<StdMutex<Option<u64>>>,
    pub(in crate::gateway) provider_delete_tasks:
        Arc<StdMutex<HashMap<String, LocalProviderDeleteTaskState>>>,
    #[cfg(test)]
    pub(in crate::gateway) provider_oauth_state_store:
        Option<Arc<StdMutex<HashMap<String, String>>>>,
    #[cfg(test)]
    pub(in crate::gateway) provider_oauth_device_session_store:
        Option<Arc<StdMutex<HashMap<String, String>>>>,
    #[cfg(test)]
    pub(in crate::gateway) provider_oauth_batch_task_store:
        Option<Arc<StdMutex<HashMap<String, String>>>>,
    #[cfg(test)]
    pub(in crate::gateway) auth_session_store: Option<
        Arc<StdMutex<HashMap<String, crate::gateway::gateway_data::StoredUserSessionRecord>>>,
    >,
    #[cfg(test)]
    pub(in crate::gateway) auth_email_verification_store:
        Option<Arc<StdMutex<HashMap<String, String>>>>,
    #[cfg(test)]
    pub(in crate::gateway) auth_email_delivery_store: Option<Arc<StdMutex<Vec<serde_json::Value>>>>,
    #[cfg(test)]
    pub(in crate::gateway) auth_user_store: Option<
        Arc<StdMutex<HashMap<String, aether_data::repository::users::StoredUserAuthRecord>>>,
    >,
    #[cfg(test)]
    pub(in crate::gateway) auth_user_model_capability_store:
        Option<Arc<StdMutex<HashMap<String, serde_json::Value>>>>,
    #[cfg(test)]
    pub(in crate::gateway) auth_wallet_store: Option<
        Arc<StdMutex<HashMap<String, aether_data::repository::wallet::StoredWalletSnapshot>>>,
    >,
    #[cfg(test)]
    pub(in crate::gateway) admin_wallet_payment_order_store:
        Option<Arc<StdMutex<HashMap<String, AdminWalletPaymentOrderRecord>>>>,
    #[cfg(test)]
    pub(in crate::gateway) admin_payment_callback_store:
        Option<Arc<StdMutex<HashMap<String, AdminPaymentCallbackRecord>>>>,
    #[cfg(test)]
    pub(in crate::gateway) admin_wallet_transaction_store:
        Option<Arc<StdMutex<HashMap<String, AdminWalletTransactionRecord>>>>,
    #[cfg(test)]
    pub(in crate::gateway) admin_wallet_refund_store:
        Option<Arc<StdMutex<HashMap<String, AdminWalletRefundRecord>>>>,
    #[cfg(test)]
    pub(in crate::gateway) admin_billing_rule_store:
        Option<Arc<StdMutex<HashMap<String, AdminBillingRuleRecord>>>>,
    #[cfg(test)]
    pub(in crate::gateway) admin_billing_collector_store:
        Option<Arc<StdMutex<HashMap<String, AdminBillingCollectorRecord>>>>,
    #[cfg(test)]
    pub(in crate::gateway) admin_security_blacklist_store:
        Option<Arc<StdMutex<HashMap<String, String>>>>,
    #[cfg(test)]
    pub(in crate::gateway) admin_security_whitelist_store:
        Option<Arc<StdMutex<std::collections::BTreeSet<String>>>>,
    #[cfg(test)]
    pub(in crate::gateway) admin_monitoring_cache_affinity_store:
        Option<Arc<StdMutex<HashMap<String, String>>>>,
    #[cfg(test)]
    pub(in crate::gateway) admin_monitoring_redis_key_store:
        Option<Arc<StdMutex<HashMap<String, String>>>>,
    #[cfg(test)]
    pub(in crate::gateway) provider_oauth_token_url_overrides:
        Arc<StdMutex<HashMap<String, String>>>,
}

pub(super) fn provider_transport_snapshot_looks_refreshed(
    current: &provider_transport::GatewayProviderTransportSnapshot,
    refreshed: &provider_transport::GatewayProviderTransportSnapshot,
) -> bool {
    current.key.decrypted_api_key != refreshed.key.decrypted_api_key
        || current.key.decrypted_auth_config != refreshed.key.decrypted_auth_config
        || current.key.expires_at_unix_secs != refreshed.key.expires_at_unix_secs
}
