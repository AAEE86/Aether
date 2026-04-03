use super::{admin_monitoring_bad_request_response, admin_monitoring_usage_is_error};
use crate::gateway::handlers::{
    provider_key_health_summary, query_param_value, unix_secs_to_rfc3339,
};
use crate::gateway::{AppState, GatewayError, GatewayPublicRequestContext};
use axum::{
    body::Body,
    response::{IntoResponse, Response},
    Json,
};
use serde_json::json;
use std::collections::BTreeMap;

struct AdminMonitoringResilienceSnapshot {
    timestamp: chrono::DateTime<chrono::Utc>,
    health_score: i64,
    status: &'static str,
    error_statistics: serde_json::Value,
    recent_errors: Vec<serde_json::Value>,
    recommendations: Vec<String>,
    previous_stats: serde_json::Value,
}

fn build_admin_monitoring_resilience_recommendations(
    total_errors: usize,
    health_score: i64,
    open_breaker_labels: &[String],
) -> Vec<String> {
    let mut recommendations = Vec::new();
    if health_score < 50 {
        recommendations.push("系统健康状况严重，请立即检查错误日志".to_string());
    }
    if total_errors > 100 {
        recommendations.push("错误频率过高，建议检查系统配置和外部依赖".to_string());
    }
    if !open_breaker_labels.is_empty() {
        recommendations.push(format!(
            "以下服务熔断器已打开：{}",
            open_breaker_labels.join(", ")
        ));
    }
    if health_score > 90 {
        recommendations.push("系统运行良好".to_string());
    }
    recommendations
}

fn parse_admin_monitoring_circuit_history_limit(query: Option<&str>) -> Result<usize, String> {
    match query_param_value(query, "limit") {
        None => Ok(50),
        Some(value) => {
            let parsed = value
                .parse::<usize>()
                .map_err(|_| "limit must be an integer between 1 and 200".to_string())?;
            if (1..=200).contains(&parsed) {
                Ok(parsed)
            } else {
                Err("limit must be an integer between 1 and 200".to_string())
            }
        }
    }
}

fn build_admin_monitoring_circuit_history_items(
    keys: &[aether_data::repository::provider_catalog::StoredProviderCatalogKey],
    provider_name_by_id: &BTreeMap<String, String>,
    limit: usize,
) -> Vec<serde_json::Value> {
    let mut items = Vec::new();

    for key in keys {
        let health_by_format = key
            .health_by_format
            .as_ref()
            .and_then(serde_json::Value::as_object)
            .cloned()
            .unwrap_or_default();
        let circuit_by_format = key
            .circuit_breaker_by_format
            .as_ref()
            .and_then(serde_json::Value::as_object)
            .cloned()
            .unwrap_or_default();

        for (api_format, circuit_value) in circuit_by_format {
            let Some(circuit) = circuit_value.as_object() else {
                continue;
            };
            let is_open = circuit
                .get("open")
                .and_then(serde_json::Value::as_bool)
                .unwrap_or(false);
            let is_half_open = circuit
                .get("half_open_until")
                .and_then(serde_json::Value::as_str)
                .is_some();

            if !is_open && !is_half_open {
                continue;
            }

            let health = health_by_format
                .get(&api_format)
                .and_then(serde_json::Value::as_object);
            let timestamp = circuit
                .get("open_at")
                .and_then(serde_json::Value::as_str)
                .or_else(|| {
                    circuit
                        .get("half_open_until")
                        .and_then(serde_json::Value::as_str)
                })
                .or_else(|| {
                    health.and_then(|value| {
                        value
                            .get("last_failure_at")
                            .and_then(serde_json::Value::as_str)
                    })
                })
                .map(ToOwned::to_owned);
            let event = if is_half_open { "half_open" } else { "opened" };
            let reason = circuit
                .get("reason")
                .and_then(serde_json::Value::as_str)
                .map(ToOwned::to_owned)
                .or_else(|| {
                    health
                        .and_then(|value| {
                            value
                                .get("consecutive_failures")
                                .and_then(serde_json::Value::as_i64)
                        })
                        .filter(|value| *value > 0)
                        .map(|value| format!("连续失败 {value} 次"))
                })
                .or_else(|| {
                    Some(if is_half_open {
                        "熔断器处于半开状态".to_string()
                    } else {
                        "熔断器处于打开状态".to_string()
                    })
                });
            let recovery_seconds = circuit
                .get("recovery_seconds")
                .and_then(serde_json::Value::as_i64)
                .or_else(|| {
                    let open_at = circuit
                        .get("open_at")
                        .and_then(serde_json::Value::as_str)
                        .and_then(|value| chrono::DateTime::parse_from_rfc3339(value).ok());
                    let next_probe_at = circuit
                        .get("next_probe_at")
                        .and_then(serde_json::Value::as_str)
                        .and_then(|value| chrono::DateTime::parse_from_rfc3339(value).ok());
                    match (open_at, next_probe_at) {
                        (Some(open_at), Some(next_probe_at)) => {
                            Some((next_probe_at - open_at).num_seconds().max(0))
                        }
                        _ => None,
                    }
                });

            items.push(json!({
                "event": event,
                "key_id": key.id,
                "provider_id": key.provider_id,
                "provider_name": provider_name_by_id.get(&key.provider_id).cloned(),
                "key_name": key.name,
                "api_format": api_format,
                "reason": reason,
                "recovery_seconds": recovery_seconds,
                "timestamp": timestamp,
            }));
        }
    }

    items.sort_by(|left, right| {
        let left_ts = left
            .get("timestamp")
            .and_then(serde_json::Value::as_str)
            .unwrap_or_default();
        let right_ts = right
            .get("timestamp")
            .and_then(serde_json::Value::as_str)
            .unwrap_or_default();
        right_ts.cmp(left_ts)
    });
    items.truncate(limit);
    items
}

pub(super) async fn build_admin_monitoring_resilience_circuit_history_response(
    state: &AppState,
    request_context: &GatewayPublicRequestContext,
) -> Result<Response<Body>, GatewayError> {
    let limit = match parse_admin_monitoring_circuit_history_limit(
        request_context.request_query_string.as_deref(),
    ) {
        Ok(value) => value,
        Err(detail) => return Ok(admin_monitoring_bad_request_response(detail)),
    };

    let providers = state.list_provider_catalog_providers(false).await?;
    let provider_ids = providers
        .iter()
        .map(|item| item.id.clone())
        .collect::<Vec<_>>();
    let provider_name_by_id = providers
        .iter()
        .map(|item| (item.id.clone(), item.name.clone()))
        .collect::<BTreeMap<_, _>>();
    let keys = if provider_ids.is_empty() {
        Vec::new()
    } else {
        state
            .list_provider_catalog_keys_by_provider_ids(&provider_ids)
            .await?
    };

    let items = build_admin_monitoring_circuit_history_items(&keys, &provider_name_by_id, limit);
    let count = items.len();
    Ok(Json(json!({
        "items": items,
        "count": count,
    }))
    .into_response())
}

pub(super) async fn build_admin_monitoring_resilience_status_response(
    state: &AppState,
) -> Result<Response<Body>, GatewayError> {
    let snapshot = build_admin_monitoring_resilience_snapshot(state).await?;

    Ok(Json(json!({
        "timestamp": snapshot.timestamp.to_rfc3339(),
        "health_score": snapshot.health_score,
        "status": snapshot.status,
        "error_statistics": snapshot.error_statistics,
        "recent_errors": snapshot.recent_errors,
        "recommendations": snapshot.recommendations,
    }))
    .into_response())
}

async fn build_admin_monitoring_resilience_snapshot(
    state: &AppState,
) -> Result<AdminMonitoringResilienceSnapshot, GatewayError> {
    let now = chrono::Utc::now();
    let recent_error_from = std::cmp::max(
        now - chrono::Duration::hours(24),
        chrono::DateTime::<chrono::Utc>::from_timestamp(
            state
                .admin_monitoring_error_stats_reset_at()
                .unwrap_or_default() as i64,
            0,
        )
        .unwrap_or_else(|| {
            chrono::DateTime::<chrono::Utc>::from_timestamp(0, 0).expect("unix epoch should exist")
        }),
    );

    let providers = state.list_provider_catalog_providers(false).await?;
    let provider_ids = providers
        .iter()
        .map(|item| item.id.clone())
        .collect::<Vec<_>>();
    let provider_name_by_id = providers
        .iter()
        .map(|item| (item.id.clone(), item.name.clone()))
        .collect::<BTreeMap<_, _>>();
    let keys = if provider_ids.is_empty() {
        Vec::new()
    } else {
        state
            .list_provider_catalog_keys_by_provider_ids(&provider_ids)
            .await?
    };

    let active_keys = keys.iter().filter(|item| item.is_active).count();
    let mut degraded_keys = 0usize;
    let mut unhealthy_keys = 0usize;
    let mut open_circuit_breakers = 0usize;
    let mut open_breaker_labels = Vec::new();
    let mut circuit_breakers = serde_json::Map::new();
    let mut previous_circuit_breakers = serde_json::Map::new();

    for key in &keys {
        let (
            health_score,
            consecutive_failures,
            last_failure_at,
            circuit_breaker_open,
            circuit_by_format,
        ) = provider_key_health_summary(key);
        if health_score < 0.8 {
            degraded_keys += 1;
        }
        if health_score < 0.5 {
            unhealthy_keys += 1;
        }

        let open_formats = circuit_by_format
            .iter()
            .filter_map(|(api_format, value)| {
                value
                    .get("open")
                    .and_then(serde_json::Value::as_bool)
                    .filter(|open| *open)
                    .map(|_| api_format.clone())
            })
            .collect::<Vec<_>>();

        if circuit_breaker_open {
            open_circuit_breakers += 1;
            let provider_label = provider_name_by_id
                .get(&key.provider_id)
                .cloned()
                .unwrap_or_else(|| key.provider_id.clone());
            open_breaker_labels.push(format!("{provider_label}/{}", key.name));
        }

        if circuit_breaker_open || consecutive_failures > 0 || health_score < 1.0 {
            circuit_breakers.insert(
                key.id.clone(),
                json!({
                    "state": if circuit_breaker_open { "open" } else { "closed" },
                    "provider_id": key.provider_id,
                    "provider_name": provider_name_by_id.get(&key.provider_id).cloned(),
                    "key_name": key.name,
                    "health_score": health_score,
                    "consecutive_failures": consecutive_failures,
                    "last_failure_at": last_failure_at,
                    "open_formats": open_formats,
                }),
            );
            previous_circuit_breakers.insert(
                key.id.clone(),
                json!({
                    "state": if circuit_breaker_open { "open" } else { "closed" },
                    "failure_count": consecutive_failures,
                }),
            );
        }
    }

    let mut recent_usage_errors = state
        .list_usage_audits(&aether_data::repository::usage::UsageAuditListQuery {
            created_from_unix_secs: Some(recent_error_from.timestamp().max(0) as u64),
            ..Default::default()
        })
        .await?
        .into_iter()
        .filter(admin_monitoring_usage_is_error)
        .collect::<Vec<_>>();
    recent_usage_errors
        .sort_by(|left, right| right.created_at_unix_secs.cmp(&left.created_at_unix_secs));

    let total_errors = recent_usage_errors.len();
    let mut error_breakdown = std::collections::BTreeMap::<String, usize>::new();
    for item in &recent_usage_errors {
        let error_type = item
            .error_category
            .clone()
            .unwrap_or_else(|| item.status.clone());
        let operation = format!(
            "{}:{}",
            item.provider_name,
            item.api_format
                .clone()
                .unwrap_or_else(|| item.model.clone())
        );
        *error_breakdown
            .entry(format!("{error_type}:{operation}"))
            .or_default() += 1;
    }
    let recent_errors = recent_usage_errors
        .iter()
        .take(10)
        .map(|item| {
            let error_type = item
                .error_category
                .clone()
                .unwrap_or_else(|| item.status.clone());
            let operation = format!(
                "{}:{}",
                item.provider_name,
                item.api_format
                    .clone()
                    .unwrap_or_else(|| item.model.clone())
            );
            json!({
                "error_id": item.id,
                "error_type": error_type,
                "operation": operation,
                "timestamp": unix_secs_to_rfc3339(item.created_at_unix_secs),
                "context": {
                    "request_id": item.request_id,
                    "provider_id": item.provider_id,
                    "provider_name": item.provider_name,
                    "model": item.model,
                    "api_format": item.api_format,
                    "status_code": item.status_code,
                    "error_message": item.error_message,
                }
            })
        })
        .collect::<Vec<_>>();

    let health_score = (100_i64
        - i64::try_from(total_errors)
            .unwrap_or(i64::MAX)
            .saturating_mul(2)
        - i64::try_from(open_circuit_breakers)
            .unwrap_or(i64::MAX)
            .saturating_mul(20))
    .clamp(0, 100);
    let status = if health_score > 80 {
        "healthy"
    } else if health_score > 50 {
        "degraded"
    } else {
        "critical"
    };
    let recommendations = build_admin_monitoring_resilience_recommendations(
        total_errors,
        health_score,
        &open_breaker_labels,
    );

    Ok(AdminMonitoringResilienceSnapshot {
        timestamp: now,
        health_score,
        status,
        error_statistics: json!({
            "total_errors": total_errors,
            "active_keys": active_keys,
            "degraded_keys": degraded_keys,
            "unhealthy_keys": unhealthy_keys,
            "open_circuit_breakers": open_circuit_breakers,
            "circuit_breakers": circuit_breakers,
        }),
        recent_errors,
        recommendations,
        previous_stats: json!({
            "total_errors": total_errors,
            "error_breakdown": error_breakdown,
            "recent_errors": total_errors,
            "circuit_breakers": previous_circuit_breakers,
        }),
    })
}

pub(super) async fn build_admin_monitoring_reset_error_stats_response(
    state: &AppState,
    request_context: &GatewayPublicRequestContext,
) -> Result<Response<Body>, GatewayError> {
    let snapshot = build_admin_monitoring_resilience_snapshot(state).await?;
    let reset_at = chrono::Utc::now();
    state.mark_admin_monitoring_error_stats_reset(reset_at.timestamp().max(0) as u64);

    let reset_by = if let Some(user_id) = request_context
        .control_decision
        .as_ref()
        .and_then(|decision| decision.admin_principal.as_ref())
        .map(|principal| principal.user_id.clone())
    {
        state
            .find_user_auth_by_id(&user_id)
            .await?
            .and_then(|user| user.email.or(Some(user.username)))
            .or(Some(user_id))
    } else {
        None
    };

    Ok(Json(json!({
        "message": "错误统计已重置",
        "previous_stats": snapshot.previous_stats,
        "reset_by": reset_by,
        "reset_at": reset_at.to_rfc3339(),
    }))
    .into_response())
}
