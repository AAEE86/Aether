use super::INTERNAL_GATEWAY_PATH_PREFIXES;
use crate::gateway::handlers::{query_param_value, unix_secs_to_rfc3339};
use crate::gateway::{AppState, GatewayError, GatewayPublicRequestContext};
use aether_crypto::decrypt_python_fernet_ciphertext;
#[cfg(test)]
use aether_crypto::DEVELOPMENT_ENCRYPTION_KEY;
use axum::{body::Body, response::Response, Json};
use chrono::Utc;
use serde_json::json;

#[path = "monitoring/activity.rs"]
mod activity;
#[path = "monitoring/cache.rs"]
mod cache;
#[path = "monitoring/cache_affinity.rs"]
mod cache_affinity;
#[path = "monitoring/cache_identity.rs"]
mod cache_identity;
#[path = "monitoring/cache_payloads.rs"]
mod cache_payloads;
#[path = "monitoring/cache_route_helpers.rs"]
mod cache_route_helpers;
#[path = "monitoring/cache_store.rs"]
mod cache_store;
#[path = "monitoring/common.rs"]
mod common;
#[path = "monitoring/resilience.rs"]
mod resilience;
#[path = "monitoring/route_filters.rs"]
mod route_filters;
#[path = "monitoring/routes.rs"]
mod routes;
#[cfg(test)]
#[path = "monitoring/test_support.rs"]
mod test_support;
#[path = "monitoring/trace.rs"]
mod trace;
use self::activity::{
    build_admin_monitoring_audit_logs_response,
    build_admin_monitoring_suspicious_activities_response,
    build_admin_monitoring_system_status_response, build_admin_monitoring_user_behavior_response,
};
use self::cache::{
    build_admin_monitoring_cache_affinities_response,
    build_admin_monitoring_cache_affinity_delete_response,
    build_admin_monitoring_cache_affinity_response, build_admin_monitoring_cache_config_response,
    build_admin_monitoring_cache_flush_response, build_admin_monitoring_cache_metrics_response,
    build_admin_monitoring_cache_provider_delete_response,
    build_admin_monitoring_cache_stats_response,
    build_admin_monitoring_cache_users_delete_response,
    build_admin_monitoring_model_mapping_delete_model_response,
    build_admin_monitoring_model_mapping_delete_provider_response,
    build_admin_monitoring_model_mapping_delete_response,
    build_admin_monitoring_model_mapping_stats_response,
    build_admin_monitoring_redis_cache_categories_response,
    build_admin_monitoring_redis_keys_delete_response,
};
use self::cache_affinity::{
    admin_monitoring_cache_affinity_record, admin_monitoring_scheduler_affinity_cache_key,
    clear_admin_monitoring_scheduler_affinity_entries,
    delete_admin_monitoring_cache_affinity_entries_for_tests,
    delete_admin_monitoring_cache_affinity_raw_keys,
};
use self::cache_identity::{
    admin_monitoring_find_user_summary_by_id, admin_monitoring_list_export_api_key_records_by_ids,
    admin_monitoring_load_affinity_identity_maps,
};
use self::cache_payloads::{
    admin_monitoring_cache_affinity_sort_value, admin_monitoring_masked_provider_key_prefix,
    admin_monitoring_masked_user_api_key_prefix,
};
use self::cache_route_helpers::{
    admin_monitoring_cache_affinity_delete_params_from_path,
    admin_monitoring_cache_affinity_not_found_response,
    admin_monitoring_cache_affinity_unavailable_response,
    admin_monitoring_cache_affinity_user_identifier_from_path,
    admin_monitoring_cache_model_mapping_provider_params_from_path,
    admin_monitoring_cache_model_name_from_path, admin_monitoring_cache_provider_id_from_path,
    admin_monitoring_cache_redis_category_from_path,
    admin_monitoring_cache_users_not_found_response,
    admin_monitoring_cache_users_user_identifier_from_path,
    admin_monitoring_redis_unavailable_response, parse_admin_monitoring_keyword_filter,
};
use self::cache_store::{
    admin_monitoring_has_test_redis_keys, build_admin_monitoring_cache_snapshot,
    delete_admin_monitoring_namespaced_keys, list_admin_monitoring_cache_affinity_records,
    list_admin_monitoring_cache_affinity_records_by_affinity_keys,
    list_admin_monitoring_namespaced_keys, load_admin_monitoring_cache_affinity_entries_for_tests,
};
use self::common::{
    admin_monitoring_bad_request_response, admin_monitoring_data_unavailable_response,
    admin_monitoring_not_found_response, admin_monitoring_usage_is_error,
    admin_monitoring_user_behavior_user_id_from_path, AdminMonitoringCacheAffinityRecord,
    AdminMonitoringCacheSnapshot,
};
use self::resilience::{
    build_admin_monitoring_reset_error_stats_response,
    build_admin_monitoring_resilience_circuit_history_response,
    build_admin_monitoring_resilience_status_response,
};
use self::route_filters::{
    admin_monitoring_escape_like_pattern, parse_admin_monitoring_days,
    parse_admin_monitoring_event_type_filter, parse_admin_monitoring_hours,
    parse_admin_monitoring_limit, parse_admin_monitoring_offset,
    parse_admin_monitoring_username_filter,
};
use self::routes::{
    match_admin_monitoring_route, maybe_build_local_admin_monitoring_response, AdminMonitoringRoute,
};
use self::trace::{
    build_admin_monitoring_trace_provider_stats_response,
    build_admin_monitoring_trace_request_response,
};

const ADMIN_MONITORING_DATA_UNAVAILABLE_DETAIL: &str = "Admin monitoring data unavailable";
const ADMIN_MONITORING_CACHE_AFFINITY_REDIS_REQUIRED_DETAIL: &str =
    "Redis未初始化，无法获取缓存亲和性";
const ADMIN_MONITORING_REDIS_REQUIRED_DETAIL: &str = "Redis 未启用";
const ADMIN_MONITORING_CACHE_AFFINITY_DEFAULT_TTL_SECS: u64 = 300;
const ADMIN_MONITORING_CACHE_RESERVATION_RATIO: f64 = 0.1;
const ADMIN_MONITORING_DYNAMIC_RESERVATION_PROBE_PHASE_REQUESTS: u64 = 100;

pub(crate) async fn maybe_build_local_admin_monitoring_root_response(
    state: &AppState,
    request_context: &GatewayPublicRequestContext,
) -> Result<Option<Response<Body>>, GatewayError> {
    routes::maybe_build_local_admin_monitoring_response(state, request_context).await
}
const ADMIN_MONITORING_DYNAMIC_RESERVATION_PROBE_RESERVATION: f64 = 0.1;
const ADMIN_MONITORING_DYNAMIC_RESERVATION_STABLE_MIN_RESERVATION: f64 = 0.1;
const ADMIN_MONITORING_DYNAMIC_RESERVATION_STABLE_MAX_RESERVATION: f64 = 0.35;
const ADMIN_MONITORING_DYNAMIC_RESERVATION_LOW_LOAD_THRESHOLD: f64 = 0.5;
const ADMIN_MONITORING_DYNAMIC_RESERVATION_HIGH_LOAD_THRESHOLD: f64 = 0.8;
const ADMIN_MONITORING_REDIS_CACHE_CATEGORIES: &[(&str, &str, &str, &str)] = &[
    (
        "upstream_models",
        "上游模型",
        "upstream_models:*",
        "Provider 上游获取的模型列表缓存",
    ),
    ("model_id", "模型 ID", "model:id:*", "Model 按 ID 缓存"),
    (
        "model_provider_global",
        "模型映射",
        "model:provider_global:*",
        "Provider-GlobalModel 模型映射缓存",
    ),
    (
        "provider_mapping_preview",
        "映射预览",
        "admin:providers:mapping-preview:*",
        "Provider 详情页 mapping-preview 缓存",
    ),
    (
        "global_model",
        "全局模型",
        "global_model:*",
        "GlobalModel 缓存（ID/名称/解析）",
    ),
    (
        "models_list",
        "模型列表",
        "models:list:*",
        "/v1/models 端点模型列表缓存",
    ),
    ("user", "用户", "user:*", "用户信息缓存（ID/Email）"),
    (
        "apikey",
        "API Key",
        "apikey:*",
        "API Key 认证缓存（Hash/Auth）",
    ),
    (
        "api_key_id",
        "API Key ID",
        "api_key:id:*",
        "API Key 按 ID 缓存",
    ),
    (
        "cache_affinity",
        "缓存亲和性",
        "cache_affinity:*",
        "请求路由亲和性缓存",
    ),
    (
        "provider_billing",
        "Provider 计费",
        "provider:billing_type:*",
        "Provider 计费类型缓存",
    ),
    (
        "provider_rate",
        "Provider 费率",
        "provider_api_key:rate_multiplier:*",
        "ProviderAPIKey 费率倍数缓存",
    ),
    (
        "provider_balance",
        "Provider 余额",
        "provider_ops:balance:*",
        "Provider 余额查询缓存",
    ),
    ("health", "健康检查", "health:*", "端点健康状态缓存"),
    (
        "endpoint_status",
        "端点状态",
        "endpoint_status:*",
        "用户端点状态缓存",
    ),
    ("dashboard", "仪表盘", "dashboard:*", "仪表盘统计缓存"),
    (
        "activity_heatmap",
        "活动热力图",
        "activity_heatmap:*",
        "用户活动热力图缓存",
    ),
    (
        "gemini_files",
        "Gemini 文件映射",
        "gemini_files:*",
        "Gemini Files API 文件-Key 映射缓存",
    ),
    (
        "provider_oauth",
        "OAuth 状态",
        "provider_oauth_state:*",
        "Provider OAuth 授权流程临时状态",
    ),
    (
        "oauth_refresh_lock",
        "OAuth 刷新锁",
        "provider_oauth_refresh_lock:*",
        "OAuth Token 刷新分布式锁",
    ),
    (
        "concurrency_lock",
        "并发锁",
        "concurrency:*",
        "请求并发控制锁",
    ),
];

#[cfg(test)]
#[path = "monitoring/tests.rs"]
mod tests;
