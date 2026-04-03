use super::{
    build_admin_monitoring_audit_logs_response, build_admin_monitoring_cache_affinities_response,
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
    build_admin_monitoring_reset_error_stats_response,
    build_admin_monitoring_resilience_circuit_history_response,
    build_admin_monitoring_resilience_status_response,
    build_admin_monitoring_suspicious_activities_response,
    build_admin_monitoring_system_status_response,
    build_admin_monitoring_trace_provider_stats_response,
    build_admin_monitoring_trace_request_response, build_admin_monitoring_user_behavior_response,
};
use crate::gateway::{AppState, GatewayError, GatewayPublicRequestContext};
use axum::{body::Body, http, response::Response};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum AdminMonitoringRoute {
    AuditLogs,
    SystemStatus,
    SuspiciousActivities,
    UserBehavior,
    ResilienceStatus,
    ResilienceErrorStats,
    ResilienceCircuitHistory,
    TraceRequest,
    TraceProviderStats,
    CacheStats,
    CacheAffinity,
    CacheAffinities,
    CacheUsersDelete,
    CacheAffinityDelete,
    CacheFlush,
    CacheProviderDelete,
    CacheConfig,
    CacheMetrics,
    CacheModelMappingStats,
    CacheModelMappingDelete,
    CacheModelMappingDeleteModel,
    CacheModelMappingDeleteProvider,
    CacheRedisKeys,
    CacheRedisKeysDelete,
}

pub(super) async fn maybe_build_local_admin_monitoring_response(
    state: &AppState,
    request_context: &GatewayPublicRequestContext,
) -> Result<Option<Response<Body>>, GatewayError> {
    let Some(route) = match_admin_monitoring_route(
        &request_context.request_method,
        request_context.request_path.as_str(),
    ) else {
        return Ok(None);
    };

    match route {
        AdminMonitoringRoute::AuditLogs => Ok(Some(
            build_admin_monitoring_audit_logs_response(state, request_context).await?,
        )),
        AdminMonitoringRoute::ResilienceStatus => Ok(Some(
            build_admin_monitoring_resilience_status_response(state).await?,
        )),
        AdminMonitoringRoute::ResilienceErrorStats => Ok(Some(
            build_admin_monitoring_reset_error_stats_response(state, request_context).await?,
        )),
        AdminMonitoringRoute::ResilienceCircuitHistory => Ok(Some(
            build_admin_monitoring_resilience_circuit_history_response(state, request_context)
                .await?,
        )),
        AdminMonitoringRoute::CacheStats => Ok(Some(
            build_admin_monitoring_cache_stats_response(state).await?,
        )),
        AdminMonitoringRoute::CacheAffinities => Ok(Some(
            build_admin_monitoring_cache_affinities_response(state, request_context).await?,
        )),
        AdminMonitoringRoute::CacheAffinity => Ok(Some(
            build_admin_monitoring_cache_affinity_response(state, request_context).await?,
        )),
        AdminMonitoringRoute::CacheUsersDelete => Ok(Some(
            build_admin_monitoring_cache_users_delete_response(state, request_context).await?,
        )),
        AdminMonitoringRoute::CacheAffinityDelete => Ok(Some(
            build_admin_monitoring_cache_affinity_delete_response(state, request_context).await?,
        )),
        AdminMonitoringRoute::CacheFlush => Ok(Some(
            build_admin_monitoring_cache_flush_response(state).await?,
        )),
        AdminMonitoringRoute::CacheProviderDelete => Ok(Some(
            build_admin_monitoring_cache_provider_delete_response(state, request_context).await?,
        )),
        AdminMonitoringRoute::CacheModelMappingDelete => Ok(Some(
            build_admin_monitoring_model_mapping_delete_response(state).await?,
        )),
        AdminMonitoringRoute::CacheModelMappingDeleteModel => Ok(Some(
            build_admin_monitoring_model_mapping_delete_model_response(state, request_context)
                .await?,
        )),
        AdminMonitoringRoute::CacheModelMappingDeleteProvider => Ok(Some(
            build_admin_monitoring_model_mapping_delete_provider_response(state, request_context)
                .await?,
        )),
        AdminMonitoringRoute::CacheRedisKeysDelete => Ok(Some(
            build_admin_monitoring_redis_keys_delete_response(state, request_context).await?,
        )),
        AdminMonitoringRoute::CacheMetrics => Ok(Some(
            build_admin_monitoring_cache_metrics_response(state).await?,
        )),
        AdminMonitoringRoute::CacheConfig => {
            Ok(Some(build_admin_monitoring_cache_config_response().await?))
        }
        AdminMonitoringRoute::CacheModelMappingStats => Ok(Some(
            build_admin_monitoring_model_mapping_stats_response(state).await?,
        )),
        AdminMonitoringRoute::CacheRedisKeys => Ok(Some(
            build_admin_monitoring_redis_cache_categories_response(state).await?,
        )),
        AdminMonitoringRoute::SystemStatus => Ok(Some(
            build_admin_monitoring_system_status_response(state).await?,
        )),
        AdminMonitoringRoute::SuspiciousActivities => Ok(Some(
            build_admin_monitoring_suspicious_activities_response(state, request_context).await?,
        )),
        AdminMonitoringRoute::UserBehavior => Ok(Some(
            build_admin_monitoring_user_behavior_response(state, request_context).await?,
        )),
        AdminMonitoringRoute::TraceRequest => Ok(Some(
            build_admin_monitoring_trace_request_response(state, request_context).await?,
        )),
        AdminMonitoringRoute::TraceProviderStats => Ok(Some(
            build_admin_monitoring_trace_provider_stats_response(state, request_context).await?,
        )),
    }
}

pub(super) fn match_admin_monitoring_route(
    method: &http::Method,
    path: &str,
) -> Option<AdminMonitoringRoute> {
    let path = normalize_admin_monitoring_path(path);

    match *method {
        http::Method::GET => match path {
            "/api/admin/monitoring/audit-logs" => Some(AdminMonitoringRoute::AuditLogs),
            "/api/admin/monitoring/system-status" => Some(AdminMonitoringRoute::SystemStatus),
            "/api/admin/monitoring/suspicious-activities" => {
                Some(AdminMonitoringRoute::SuspiciousActivities)
            }
            "/api/admin/monitoring/resilience-status" => {
                Some(AdminMonitoringRoute::ResilienceStatus)
            }
            "/api/admin/monitoring/resilience/circuit-history" => {
                Some(AdminMonitoringRoute::ResilienceCircuitHistory)
            }
            "/api/admin/monitoring/cache/stats" => Some(AdminMonitoringRoute::CacheStats),
            "/api/admin/monitoring/cache/affinities" => Some(AdminMonitoringRoute::CacheAffinities),
            "/api/admin/monitoring/cache/config" => Some(AdminMonitoringRoute::CacheConfig),
            "/api/admin/monitoring/cache/metrics" => Some(AdminMonitoringRoute::CacheMetrics),
            "/api/admin/monitoring/cache/model-mapping/stats" => {
                Some(AdminMonitoringRoute::CacheModelMappingStats)
            }
            "/api/admin/monitoring/cache/redis-keys" => Some(AdminMonitoringRoute::CacheRedisKeys),
            _ if matches_dynamic_segments(path, "/api/admin/monitoring/user-behavior/", 1) => {
                Some(AdminMonitoringRoute::UserBehavior)
            }
            _ if matches_dynamic_segments(
                path,
                "/api/admin/monitoring/trace/stats/provider/",
                1,
            ) =>
            {
                Some(AdminMonitoringRoute::TraceProviderStats)
            }
            _ if matches_dynamic_segments(path, "/api/admin/monitoring/trace/", 1) => {
                Some(AdminMonitoringRoute::TraceRequest)
            }
            _ if matches_dynamic_segments(path, "/api/admin/monitoring/cache/affinity/", 1) => {
                Some(AdminMonitoringRoute::CacheAffinity)
            }
            _ => None,
        },
        http::Method::DELETE => match path {
            "/api/admin/monitoring/resilience/error-stats" => {
                Some(AdminMonitoringRoute::ResilienceErrorStats)
            }
            "/api/admin/monitoring/cache" => Some(AdminMonitoringRoute::CacheFlush),
            "/api/admin/monitoring/cache/model-mapping" => {
                Some(AdminMonitoringRoute::CacheModelMappingDelete)
            }
            _ if matches_dynamic_segments(path, "/api/admin/monitoring/cache/users/", 1) => {
                Some(AdminMonitoringRoute::CacheUsersDelete)
            }
            _ if matches_dynamic_segments(path, "/api/admin/monitoring/cache/providers/", 1) => {
                Some(AdminMonitoringRoute::CacheProviderDelete)
            }
            _ if matches_dynamic_segments(path, "/api/admin/monitoring/cache/redis-keys/", 1) => {
                Some(AdminMonitoringRoute::CacheRedisKeysDelete)
            }
            _ if matches_dynamic_segments(
                path,
                "/api/admin/monitoring/cache/model-mapping/provider/",
                2,
            ) =>
            {
                Some(AdminMonitoringRoute::CacheModelMappingDeleteProvider)
            }
            _ if matches_dynamic_segments(
                path,
                "/api/admin/monitoring/cache/model-mapping/",
                1,
            ) =>
            {
                Some(AdminMonitoringRoute::CacheModelMappingDeleteModel)
            }
            _ if matches_dynamic_segments(path, "/api/admin/monitoring/cache/affinity/", 4) => {
                Some(AdminMonitoringRoute::CacheAffinityDelete)
            }
            _ => None,
        },
        _ => None,
    }
}

fn normalize_admin_monitoring_path(path: &str) -> &str {
    let normalized = path.trim_end_matches('/');
    if normalized.is_empty() {
        "/"
    } else {
        normalized
    }
}

fn matches_dynamic_segments(path: &str, prefix: &str, dynamic_segments: usize) -> bool {
    let Some(suffix) = path.strip_prefix(prefix) else {
        return false;
    };

    let segments = suffix
        .split('/')
        .filter(|segment| !segment.is_empty())
        .collect::<Vec<_>>();
    segments.len() == dynamic_segments
}
