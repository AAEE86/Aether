use super::INTERNAL_GATEWAY_PATH_PREFIXES;
use super::{
    admin_monitoring_bad_request_response, admin_monitoring_escape_like_pattern,
    admin_monitoring_usage_is_error, admin_monitoring_user_behavior_user_id_from_path,
    parse_admin_monitoring_days, parse_admin_monitoring_event_type_filter,
    parse_admin_monitoring_hours, parse_admin_monitoring_limit, parse_admin_monitoring_offset,
    parse_admin_monitoring_username_filter,
};
use crate::gateway::{AppState, GatewayError, GatewayPublicRequestContext};
use axum::{
    body::Body,
    response::{IntoResponse, Response},
    Json,
};
use serde_json::json;
use sqlx::Row;

fn build_admin_monitoring_audit_logs_payload(
    items: Vec<serde_json::Value>,
    total: usize,
    limit: usize,
    offset: usize,
    username: Option<String>,
    event_type: Option<String>,
    days: i64,
) -> Response<Body> {
    let count = items.len();
    Json(json!({
        "items": items,
        "meta": {
            "total": total,
            "limit": limit,
            "offset": offset,
            "count": count,
        },
        "filters": {
            "username": username,
            "event_type": event_type,
            "days": days,
        },
    }))
    .into_response()
}

fn build_admin_monitoring_suspicious_activities_payload(
    activities: Vec<serde_json::Value>,
    hours: i64,
) -> Response<Body> {
    let count = activities.len();
    Json(json!({
        "activities": activities,
        "count": count,
        "time_range_hours": hours,
    }))
    .into_response()
}

fn build_admin_monitoring_user_behavior_payload(
    user_id: String,
    days: i64,
    event_counts: std::collections::BTreeMap<String, u64>,
    failed_requests: u64,
    success_requests: u64,
    suspicious_activities: u64,
) -> Response<Body> {
    let total_requests = success_requests.saturating_add(failed_requests);
    let success_rate = if total_requests == 0 {
        0.0
    } else {
        success_requests as f64 / total_requests as f64
    };

    Json(json!({
        "user_id": user_id,
        "period_days": days,
        "event_counts": event_counts,
        "failed_requests": failed_requests,
        "success_requests": success_requests,
        "success_rate": success_rate,
        "suspicious_activities": suspicious_activities,
        "analysis_time": chrono::Utc::now().to_rfc3339(),
    }))
    .into_response()
}

pub(super) async fn build_admin_monitoring_audit_logs_response(
    state: &AppState,
    request_context: &GatewayPublicRequestContext,
) -> Result<Response<Body>, GatewayError> {
    let query = request_context.request_query_string.as_deref();
    let username = parse_admin_monitoring_username_filter(query);
    let event_type = parse_admin_monitoring_event_type_filter(query);
    let limit = match parse_admin_monitoring_limit(query) {
        Ok(value) => value,
        Err(detail) => return Ok(admin_monitoring_bad_request_response(detail)),
    };
    let offset = match parse_admin_monitoring_offset(query) {
        Ok(value) => value,
        Err(detail) => return Ok(admin_monitoring_bad_request_response(detail)),
    };
    let days = match parse_admin_monitoring_days(query) {
        Ok(value) => value,
        Err(detail) => return Ok(admin_monitoring_bad_request_response(detail)),
    };

    let Some(pool) = state.postgres_pool() else {
        return Ok(build_admin_monitoring_audit_logs_payload(
            Vec::new(),
            0,
            limit,
            offset,
            username,
            event_type,
            days,
        ));
    };

    let cutoff_time = chrono::Utc::now() - chrono::Duration::days(days);
    let username_pattern = username
        .as_deref()
        .map(admin_monitoring_escape_like_pattern)
        .map(|value| format!("%{value}%"));

    let total = sqlx::query_scalar::<_, i64>(
        r#"
SELECT COUNT(*)
FROM audit_logs AS a
LEFT JOIN users AS u ON a.user_id = u.id
WHERE a.created_at >= $1
  AND ($2::text IS NULL OR u.username ILIKE $2 ESCAPE '\')
  AND ($3::text IS NULL OR a.event_type = $3)
"#,
    )
    .bind(cutoff_time)
    .bind(username_pattern.as_deref())
    .bind(event_type.as_deref())
    .fetch_one(&pool)
    .await
    .map_err(|err| GatewayError::Internal(format!("admin audit logs count failed: {err}")))?;

    let rows = sqlx::query(
        r#"
SELECT
  a.id,
  a.event_type,
  a.user_id,
  u.email AS user_email,
  u.username AS user_username,
  a.description,
  a.ip_address,
  a.status_code,
  a.error_message,
  a.event_metadata AS metadata,
  a.created_at
FROM audit_logs AS a
LEFT JOIN users AS u ON a.user_id = u.id
WHERE a.created_at >= $1
  AND ($2::text IS NULL OR u.username ILIKE $2 ESCAPE '\')
  AND ($3::text IS NULL OR a.event_type = $3)
ORDER BY a.created_at DESC
LIMIT $4 OFFSET $5
"#,
    )
    .bind(cutoff_time)
    .bind(username_pattern.as_deref())
    .bind(event_type.as_deref())
    .bind(i64::try_from(limit).unwrap_or(i64::MAX))
    .bind(i64::try_from(offset).unwrap_or(i64::MAX))
    .fetch_all(&pool)
    .await
    .map_err(|err| GatewayError::Internal(format!("admin audit logs read failed: {err}")))?;

    let items = rows
        .into_iter()
        .map(|row| {
            let created_at = row
                .try_get::<chrono::DateTime<chrono::Utc>, _>("created_at")
                .ok()
                .map(|value| value.to_rfc3339());
            json!({
                "id": row.try_get::<String, _>("id").ok(),
                "event_type": row.try_get::<String, _>("event_type").ok(),
                "user_id": row.try_get::<Option<String>, _>("user_id").ok().flatten(),
                "user_email": row.try_get::<Option<String>, _>("user_email").ok().flatten(),
                "user_username": row.try_get::<Option<String>, _>("user_username").ok().flatten(),
                "description": row.try_get::<Option<String>, _>("description").ok().flatten(),
                "ip_address": row.try_get::<Option<String>, _>("ip_address").ok().flatten(),
                "status_code": row.try_get::<Option<i32>, _>("status_code").ok().flatten(),
                "error_message": row.try_get::<Option<String>, _>("error_message").ok().flatten(),
                "metadata": row.try_get::<Option<serde_json::Value>, _>("metadata").ok().flatten(),
                "created_at": created_at,
            })
        })
        .collect::<Vec<_>>();

    Ok(build_admin_monitoring_audit_logs_payload(
        items,
        usize::try_from(total.max(0)).unwrap_or(usize::MAX),
        limit,
        offset,
        username,
        event_type,
        days,
    ))
}

pub(super) async fn build_admin_monitoring_suspicious_activities_response(
    state: &AppState,
    request_context: &GatewayPublicRequestContext,
) -> Result<Response<Body>, GatewayError> {
    let query = request_context.request_query_string.as_deref();
    let hours = match parse_admin_monitoring_hours(query) {
        Ok(value) => value,
        Err(detail) => return Ok(admin_monitoring_bad_request_response(detail)),
    };

    let Some(pool) = state.postgres_pool() else {
        return Ok(build_admin_monitoring_suspicious_activities_payload(
            Vec::new(),
            hours,
        ));
    };

    let cutoff_time = chrono::Utc::now() - chrono::Duration::hours(hours);
    let rows = sqlx::query(
        r#"
SELECT
  id,
  event_type,
  user_id,
  description,
  ip_address,
  event_metadata AS metadata,
  created_at
FROM audit_logs
WHERE created_at >= $1
  AND event_type = ANY($2)
ORDER BY created_at DESC
LIMIT 100
"#,
    )
    .bind(cutoff_time)
    .bind(vec![
        "suspicious_activity",
        "unauthorized_access",
        "login_failed",
        "request_rate_limited",
    ])
    .fetch_all(&pool)
    .await
    .map_err(|err| {
        GatewayError::Internal(format!("admin suspicious activities read failed: {err}"))
    })?;

    let activities = rows
        .into_iter()
        .map(|row| {
            let created_at = row
                .try_get::<chrono::DateTime<chrono::Utc>, _>("created_at")
                .ok()
                .map(|value| value.to_rfc3339());
            json!({
                "id": row.try_get::<String, _>("id").ok(),
                "event_type": row.try_get::<String, _>("event_type").ok(),
                "user_id": row.try_get::<Option<String>, _>("user_id").ok().flatten(),
                "description": row.try_get::<Option<String>, _>("description").ok().flatten(),
                "ip_address": row.try_get::<Option<String>, _>("ip_address").ok().flatten(),
                "metadata": row.try_get::<Option<serde_json::Value>, _>("metadata").ok().flatten(),
                "created_at": created_at,
            })
        })
        .collect::<Vec<_>>();

    Ok(build_admin_monitoring_suspicious_activities_payload(
        activities, hours,
    ))
}

pub(super) async fn build_admin_monitoring_user_behavior_response(
    state: &AppState,
    request_context: &GatewayPublicRequestContext,
) -> Result<Response<Body>, GatewayError> {
    let Some(user_id) =
        admin_monitoring_user_behavior_user_id_from_path(&request_context.request_path)
    else {
        return Ok(admin_monitoring_bad_request_response("缺少 user_id"));
    };
    let days = match parse_admin_monitoring_days(request_context.request_query_string.as_deref()) {
        Ok(value) => value,
        Err(detail) => return Ok(admin_monitoring_bad_request_response(detail)),
    };

    let Some(pool) = state.postgres_pool() else {
        return Ok(build_admin_monitoring_user_behavior_payload(
            user_id,
            days,
            std::collections::BTreeMap::new(),
            0,
            0,
            0,
        ));
    };

    let cutoff_time = chrono::Utc::now() - chrono::Duration::days(days);

    let event_rows = sqlx::query(
        r#"
SELECT event_type, COUNT(*)::bigint AS count
FROM audit_logs
WHERE user_id = $1
  AND created_at >= $2
GROUP BY event_type
"#,
    )
    .bind(&user_id)
    .bind(cutoff_time)
    .fetch_all(&pool)
    .await
    .map_err(|err| GatewayError::Internal(format!("admin user behavior read failed: {err}")))?;

    let event_counts = event_rows
        .into_iter()
        .filter_map(|row| {
            let event_type = row.try_get::<String, _>("event_type").ok()?;
            let count = row
                .try_get::<i64, _>("count")
                .ok()
                .and_then(|value| u64::try_from(value.max(0)).ok())
                .unwrap_or(0);
            Some((event_type, count))
        })
        .collect::<std::collections::BTreeMap<_, _>>();

    let failed_requests = event_counts
        .get("request_failed")
        .copied()
        .unwrap_or_default();
    let success_requests = event_counts
        .get("request_success")
        .copied()
        .unwrap_or_default();
    let suspicious_activities = event_counts
        .get("suspicious_activity")
        .copied()
        .unwrap_or_default()
        .saturating_add(
            event_counts
                .get("unauthorized_access")
                .copied()
                .unwrap_or_default(),
        );

    Ok(build_admin_monitoring_user_behavior_payload(
        user_id,
        days,
        event_counts,
        failed_requests,
        success_requests,
        suspicious_activities,
    ))
}

pub(super) async fn build_admin_monitoring_system_status_response(
    state: &AppState,
) -> Result<Response<Body>, GatewayError> {
    let now = chrono::Utc::now();
    let today_start = now
        .date_naive()
        .and_hms_opt(0, 0, 0)
        .expect("midnight should be valid")
        .and_utc();
    let recent_error_from = now - chrono::Duration::hours(1);
    let now_unix_secs = now.timestamp().max(0) as u64;

    let user_summary = state.summarize_export_users().await?;
    let total_users = user_summary.total;
    let active_users = user_summary.active;

    let providers = state
        .data
        .list_provider_catalog_providers(false)
        .await
        .map_err(|err| GatewayError::Internal(err.to_string()))?;
    let total_providers = providers.len();
    let active_providers = providers.iter().filter(|item| item.is_active).count();

    let user_api_key_summary = state
        .summarize_auth_api_key_export_non_standalone_records(now_unix_secs)
        .await?;
    let standalone_api_key_summary = state
        .summarize_auth_api_key_export_standalone_records(now_unix_secs)
        .await?;
    let total_api_keys = user_api_key_summary
        .total
        .saturating_add(standalone_api_key_summary.total);
    let active_api_keys = user_api_key_summary
        .active
        .saturating_add(standalone_api_key_summary.active);

    let today_usage = state
        .list_usage_audits(&aether_data::repository::usage::UsageAuditListQuery {
            created_from_unix_secs: Some(today_start.timestamp().max(0) as u64),
            ..Default::default()
        })
        .await?;
    let today_requests = today_usage.len();
    let today_tokens = today_usage
        .iter()
        .map(|item| item.total_tokens)
        .sum::<u64>();
    let today_cost = today_usage
        .iter()
        .map(|item| item.total_cost_usd)
        .sum::<f64>();

    let recent_errors = state
        .list_usage_audits(&aether_data::repository::usage::UsageAuditListQuery {
            created_from_unix_secs: Some(recent_error_from.timestamp().max(0) as u64),
            ..Default::default()
        })
        .await?
        .into_iter()
        .filter(admin_monitoring_usage_is_error)
        .count();
    let tunnel = state.tunnel.stats();

    Ok(Json(json!({
        "timestamp": now.to_rfc3339(),
        "users": {
            "total": total_users,
            "active": active_users,
        },
        "providers": {
            "total": total_providers,
            "active": active_providers,
        },
        "api_keys": {
            "total": total_api_keys,
            "active": active_api_keys,
        },
        "today_stats": {
            "requests": today_requests,
            "tokens": today_tokens,
            "cost_usd": format!("${today_cost:.4}"),
        },
        "tunnel": {
            "proxy_connections": tunnel.proxy_connections,
            "nodes": tunnel.nodes,
            "active_streams": tunnel.active_streams,
        },
        "internal_gateway": {
            "status": "rust_native_control_plane",
            "path_prefixes": INTERNAL_GATEWAY_PATH_PREFIXES,
        },
        "recent_errors": recent_errors,
    }))
    .into_response())
}
