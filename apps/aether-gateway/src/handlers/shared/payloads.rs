use serde::Deserialize;
use std::collections::BTreeMap;

#[derive(Debug, Deserialize)]
pub(crate) struct AdminProviderKeyCreateRequest {
    #[serde(default)]
    pub(crate) api_formats: Option<Vec<String>>,
    #[serde(default)]
    pub(crate) api_key: Option<String>,
    #[serde(default)]
    pub(crate) auth_type: Option<String>,
    #[serde(default)]
    pub(crate) auth_config: Option<serde_json::Value>,
    pub(crate) name: String,
    #[serde(default)]
    pub(crate) rate_multipliers: Option<serde_json::Value>,
    #[serde(default)]
    pub(crate) internal_priority: Option<i32>,
    #[serde(default)]
    pub(crate) rpm_limit: Option<u32>,
    #[serde(default)]
    pub(crate) allowed_models: Option<Vec<String>>,
    #[serde(default)]
    pub(crate) capabilities: Option<serde_json::Value>,
    #[serde(default)]
    pub(crate) cache_ttl_minutes: Option<i32>,
    #[serde(default)]
    pub(crate) max_probe_interval_minutes: Option<i32>,
    #[serde(default)]
    pub(crate) note: Option<String>,
    #[serde(default)]
    pub(crate) auto_fetch_models: Option<bool>,
    #[serde(default)]
    pub(crate) locked_models: Option<Vec<String>>,
    #[serde(default)]
    pub(crate) model_include_patterns: Option<Vec<String>>,
    #[serde(default)]
    pub(crate) model_exclude_patterns: Option<Vec<String>>,
}

#[derive(Debug, Deserialize)]
pub(crate) struct AdminProviderKeyUpdateRequest {
    #[serde(default)]
    pub(crate) api_formats: Option<Vec<String>>,
    #[serde(default)]
    pub(crate) api_key: Option<String>,
    #[serde(default)]
    pub(crate) auth_type: Option<String>,
    #[serde(default)]
    pub(crate) auth_config: Option<serde_json::Value>,
    #[serde(default)]
    pub(crate) name: Option<String>,
    #[serde(default)]
    pub(crate) rate_multipliers: Option<serde_json::Value>,
    #[serde(default)]
    pub(crate) internal_priority: Option<i32>,
    #[serde(default)]
    pub(crate) global_priority_by_format: Option<serde_json::Value>,
    #[serde(default)]
    pub(crate) rpm_limit: Option<u32>,
    #[serde(default)]
    pub(crate) allowed_models: Option<Vec<String>>,
    #[serde(default)]
    pub(crate) capabilities: Option<serde_json::Value>,
    #[serde(default)]
    pub(crate) cache_ttl_minutes: Option<i32>,
    #[serde(default)]
    pub(crate) max_probe_interval_minutes: Option<i32>,
    #[serde(default)]
    pub(crate) is_active: Option<bool>,
    #[serde(default)]
    pub(crate) note: Option<String>,
    #[serde(default)]
    pub(crate) auto_fetch_models: Option<bool>,
    #[serde(default)]
    pub(crate) locked_models: Option<Vec<String>>,
    #[serde(default)]
    pub(crate) model_include_patterns: Option<Vec<String>>,
    #[serde(default)]
    pub(crate) model_exclude_patterns: Option<Vec<String>>,
    #[serde(default)]
    pub(crate) proxy: Option<serde_json::Value>,
    #[serde(default)]
    pub(crate) fingerprint: Option<serde_json::Value>,
}

#[derive(Debug, Deserialize)]
pub(crate) struct AdminProviderKeyBatchDeleteRequest {
    pub(crate) ids: Vec<String>,
}

#[derive(Debug, Deserialize)]
pub(crate) struct AdminProviderQuotaRefreshRequest {
    #[serde(default)]
    pub(crate) key_ids: Option<Vec<String>>,
}

#[derive(Debug, Deserialize)]
pub(crate) struct AdminOAuthProviderUpsertRequest {
    pub(crate) display_name: String,
    pub(crate) client_id: String,
    #[serde(default)]
    pub(crate) client_secret: Option<String>,
    #[serde(default)]
    pub(crate) authorization_url_override: Option<String>,
    #[serde(default)]
    pub(crate) token_url_override: Option<String>,
    #[serde(default)]
    pub(crate) userinfo_url_override: Option<String>,
    #[serde(default)]
    pub(crate) scopes: Option<Vec<String>>,
    pub(crate) redirect_uri: String,
    pub(crate) frontend_callback_url: String,
    #[serde(default)]
    pub(crate) attribute_mapping: Option<serde_json::Value>,
    #[serde(default)]
    pub(crate) extra_config: Option<serde_json::Value>,
    #[serde(default)]
    pub(crate) is_enabled: bool,
    #[serde(default)]
    pub(crate) force: bool,
}

#[derive(Debug, Deserialize)]
pub(crate) struct InternalTunnelHeartbeatRequest {
    pub(crate) node_id: String,
    #[serde(default)]
    pub(crate) heartbeat_interval: Option<i32>,
    #[serde(default)]
    pub(crate) active_connections: Option<i32>,
    #[serde(default)]
    pub(crate) total_requests: Option<i64>,
    #[serde(default)]
    pub(crate) avg_latency_ms: Option<f64>,
    #[serde(default)]
    pub(crate) failed_requests: Option<i64>,
    #[serde(default)]
    pub(crate) dns_failures: Option<i64>,
    #[serde(default)]
    pub(crate) stream_errors: Option<i64>,
    #[serde(default)]
    pub(crate) proxy_metadata: Option<serde_json::Value>,
    #[serde(default)]
    pub(crate) proxy_version: Option<String>,
}

#[derive(Debug, Deserialize)]
pub(crate) struct InternalTunnelNodeStatusRequest {
    pub(crate) node_id: String,
    pub(crate) connected: bool,
    #[serde(default)]
    pub(crate) conn_count: i32,
}

#[derive(Debug, Deserialize)]
pub(crate) struct InternalGatewayResolveRequest {
    #[serde(default)]
    pub(crate) trace_id: Option<String>,
    pub(crate) method: String,
    pub(crate) path: String,
    #[serde(default)]
    pub(crate) query_string: Option<String>,
    #[serde(default)]
    pub(crate) headers: BTreeMap<String, String>,
}

#[derive(Debug, Deserialize)]
pub(crate) struct InternalGatewayAuthContextRequest {
    #[serde(default)]
    pub(crate) trace_id: Option<String>,
    #[serde(default)]
    pub(crate) query_string: Option<String>,
    #[serde(default)]
    pub(crate) headers: BTreeMap<String, String>,
    pub(crate) auth_endpoint_signature: String,
}

#[derive(Debug, Deserialize)]
pub(crate) struct InternalGatewayExecuteRequest {
    #[serde(default)]
    pub(crate) trace_id: Option<String>,
    pub(crate) method: String,
    pub(crate) path: String,
    #[serde(default)]
    pub(crate) query_string: Option<String>,
    #[serde(default)]
    pub(crate) headers: BTreeMap<String, String>,
    #[serde(default)]
    pub(crate) body_json: serde_json::Value,
    #[serde(default)]
    pub(crate) body_base64: Option<String>,
    #[serde(default)]
    pub(crate) auth_context: Option<crate::gateway::GatewayControlAuthContext>,
}

#[derive(Debug, Deserialize)]
pub(crate) struct AdminProviderCreateRequest {
    pub(crate) name: String,
    #[serde(default)]
    pub(crate) provider_type: Option<String>,
    #[serde(default)]
    pub(crate) description: Option<String>,
    #[serde(default)]
    pub(crate) website: Option<String>,
    #[serde(default)]
    pub(crate) billing_type: Option<String>,
    #[serde(default)]
    pub(crate) monthly_quota_usd: Option<f64>,
    #[serde(default)]
    pub(crate) quota_reset_day: Option<u64>,
    #[serde(default)]
    pub(crate) quota_last_reset_at: Option<String>,
    #[serde(default)]
    pub(crate) quota_expires_at: Option<String>,
    #[serde(default)]
    pub(crate) provider_priority: Option<i32>,
    #[serde(default)]
    pub(crate) keep_priority_on_conversion: Option<bool>,
    #[serde(default)]
    pub(crate) is_active: Option<bool>,
    #[serde(default)]
    pub(crate) concurrent_limit: Option<i32>,
    #[serde(default)]
    pub(crate) max_retries: Option<i32>,
    #[serde(default)]
    pub(crate) proxy: Option<serde_json::Value>,
    #[serde(default)]
    pub(crate) stream_first_byte_timeout: Option<f64>,
    #[serde(default)]
    pub(crate) request_timeout: Option<f64>,
    #[serde(default)]
    pub(crate) pool_advanced: Option<serde_json::Value>,
    #[serde(default)]
    pub(crate) claude_code_advanced: Option<serde_json::Value>,
    #[serde(default)]
    pub(crate) failover_rules: Option<serde_json::Value>,
    #[serde(default)]
    pub(crate) config: Option<serde_json::Value>,
}

#[derive(Debug, Deserialize)]
pub(crate) struct AdminProviderUpdateRequest {
    #[serde(default)]
    pub(crate) name: Option<String>,
    #[serde(default)]
    pub(crate) provider_type: Option<String>,
    #[serde(default)]
    pub(crate) description: Option<String>,
    #[serde(default)]
    pub(crate) website: Option<String>,
    #[serde(default)]
    pub(crate) billing_type: Option<String>,
    #[serde(default)]
    pub(crate) monthly_quota_usd: Option<f64>,
    #[serde(default)]
    pub(crate) quota_reset_day: Option<u64>,
    #[serde(default)]
    pub(crate) quota_last_reset_at: Option<String>,
    #[serde(default)]
    pub(crate) quota_expires_at: Option<String>,
    #[serde(default)]
    pub(crate) provider_priority: Option<i32>,
    #[serde(default)]
    pub(crate) keep_priority_on_conversion: Option<bool>,
    #[serde(default)]
    pub(crate) is_active: Option<bool>,
    #[serde(default)]
    pub(crate) concurrent_limit: Option<i32>,
    #[serde(default)]
    pub(crate) max_retries: Option<i32>,
    #[serde(default)]
    pub(crate) proxy: Option<serde_json::Value>,
    #[serde(default)]
    pub(crate) stream_first_byte_timeout: Option<f64>,
    #[serde(default)]
    pub(crate) request_timeout: Option<f64>,
    #[serde(default)]
    pub(crate) pool_advanced: Option<serde_json::Value>,
    #[serde(default)]
    pub(crate) claude_code_advanced: Option<serde_json::Value>,
    #[serde(default)]
    pub(crate) failover_rules: Option<serde_json::Value>,
    #[serde(default)]
    pub(crate) enable_format_conversion: Option<bool>,
    #[serde(default)]
    pub(crate) config: Option<serde_json::Value>,
}

pub(crate) const CODEX_WHAM_USAGE_URL: &str = "https://chatgpt.com/backend-api/wham/usage";
pub(crate) const KIRO_USAGE_LIMITS_PATH: &str = "/getUsageLimits";
pub(crate) const KIRO_USAGE_SDK_VERSION: &str = "1.0.0";
pub(crate) const ANTIGRAVITY_FETCH_AVAILABLE_MODELS_PATH: &str = "/v1internal:fetchAvailableModels";
pub(crate) const OAUTH_ACCOUNT_BLOCK_PREFIX: &str = "[ACCOUNT_BLOCK] ";
pub(crate) const OAUTH_REFRESH_FAILED_PREFIX: &str = "[REFRESH_FAILED] ";
pub(crate) const OAUTH_EXPIRED_PREFIX: &str = "[OAUTH_EXPIRED] ";
pub(crate) const OAUTH_REQUEST_FAILED_PREFIX: &str = "[REQUEST_FAILED] ";

pub(crate) fn default_admin_endpoint_max_retries() -> i32 {
    2
}

#[derive(Debug, Deserialize)]
pub(crate) struct AdminProviderEndpointCreateRequest {
    pub(crate) provider_id: String,
    pub(crate) api_format: String,
    pub(crate) base_url: String,
    #[serde(default)]
    pub(crate) custom_path: Option<String>,
    #[serde(default)]
    pub(crate) header_rules: Option<serde_json::Value>,
    #[serde(default)]
    pub(crate) body_rules: Option<serde_json::Value>,
    #[serde(default = "default_admin_endpoint_max_retries")]
    pub(crate) max_retries: i32,
    #[serde(default)]
    pub(crate) config: Option<serde_json::Value>,
    #[serde(default)]
    pub(crate) proxy: Option<serde_json::Value>,
    #[serde(default)]
    pub(crate) format_acceptance_config: Option<serde_json::Value>,
}

#[derive(Debug, Deserialize)]
pub(crate) struct AdminProviderEndpointUpdateRequest {
    #[serde(default)]
    pub(crate) base_url: Option<String>,
    #[serde(default)]
    pub(crate) custom_path: Option<String>,
    #[serde(default)]
    pub(crate) header_rules: Option<serde_json::Value>,
    #[serde(default)]
    pub(crate) body_rules: Option<serde_json::Value>,
    #[serde(default)]
    pub(crate) max_retries: Option<i32>,
    #[serde(default)]
    pub(crate) is_active: Option<bool>,
    #[serde(default)]
    pub(crate) config: Option<serde_json::Value>,
    #[serde(default)]
    pub(crate) proxy: Option<serde_json::Value>,
    #[serde(default)]
    pub(crate) format_acceptance_config: Option<serde_json::Value>,
}

#[derive(Debug, Deserialize)]
pub(crate) struct AdminProviderModelCreateRequest {
    pub(crate) provider_model_name: String,
    #[serde(default)]
    pub(crate) provider_model_mappings: Option<serde_json::Value>,
    pub(crate) global_model_id: String,
    #[serde(default)]
    pub(crate) price_per_request: Option<f64>,
    #[serde(default)]
    pub(crate) tiered_pricing: Option<serde_json::Value>,
    #[serde(default)]
    pub(crate) supports_vision: Option<bool>,
    #[serde(default)]
    pub(crate) supports_function_calling: Option<bool>,
    #[serde(default)]
    pub(crate) supports_streaming: Option<bool>,
    #[serde(default)]
    pub(crate) supports_extended_thinking: Option<bool>,
    #[serde(default)]
    pub(crate) is_active: Option<bool>,
    #[serde(default)]
    pub(crate) config: Option<serde_json::Value>,
}

#[derive(Debug, Deserialize)]
pub(crate) struct AdminProviderModelUpdateRequest {
    #[serde(default)]
    pub(crate) provider_model_name: Option<String>,
    #[serde(default)]
    pub(crate) provider_model_mappings: Option<serde_json::Value>,
    #[serde(default)]
    pub(crate) global_model_id: Option<String>,
    #[serde(default)]
    pub(crate) price_per_request: Option<f64>,
    #[serde(default)]
    pub(crate) tiered_pricing: Option<serde_json::Value>,
    #[serde(default)]
    pub(crate) supports_vision: Option<bool>,
    #[serde(default)]
    pub(crate) supports_function_calling: Option<bool>,
    #[serde(default)]
    pub(crate) supports_streaming: Option<bool>,
    #[serde(default)]
    pub(crate) supports_extended_thinking: Option<bool>,
    #[serde(default)]
    pub(crate) is_active: Option<bool>,
    #[serde(default)]
    pub(crate) is_available: Option<bool>,
    #[serde(default)]
    pub(crate) config: Option<serde_json::Value>,
}

#[derive(Debug, Deserialize)]
pub(crate) struct AdminGlobalModelCreateRequest {
    pub(crate) name: String,
    pub(crate) display_name: String,
    #[serde(default)]
    pub(crate) default_price_per_request: Option<f64>,
    #[serde(default)]
    pub(crate) default_tiered_pricing: Option<serde_json::Value>,
    #[serde(default)]
    pub(crate) supported_capabilities: Option<Vec<String>>,
    #[serde(default)]
    pub(crate) config: Option<serde_json::Value>,
    #[serde(default)]
    pub(crate) is_active: Option<bool>,
}

#[derive(Debug, Deserialize)]
pub(crate) struct AdminGlobalModelUpdateRequest {
    #[serde(default)]
    pub(crate) display_name: Option<String>,
    #[serde(default)]
    pub(crate) is_active: Option<bool>,
    #[serde(default)]
    pub(crate) default_price_per_request: Option<f64>,
    #[serde(default)]
    pub(crate) default_tiered_pricing: Option<serde_json::Value>,
    #[serde(default)]
    pub(crate) supported_capabilities: Option<Vec<String>>,
    #[serde(default)]
    pub(crate) config: Option<serde_json::Value>,
}

#[derive(Debug, Deserialize)]
pub(crate) struct AdminBatchDeleteIdsRequest {
    pub(crate) ids: Vec<String>,
}

#[derive(Debug, Deserialize)]
pub(crate) struct AdminBatchAssignToProvidersRequest {
    pub(crate) provider_ids: Vec<String>,
    #[serde(default)]
    pub(crate) create_models: Option<bool>,
}

#[derive(Debug, Deserialize)]
pub(crate) struct AdminBatchAssignGlobalModelsRequest {
    pub(crate) global_model_ids: Vec<String>,
}

#[derive(Debug, Deserialize)]
pub(crate) struct AdminImportProviderModelsRequest {
    pub(crate) model_ids: Vec<String>,
    #[serde(default)]
    pub(crate) tiered_pricing: Option<serde_json::Value>,
    #[serde(default)]
    pub(crate) price_per_request: Option<f64>,
}
