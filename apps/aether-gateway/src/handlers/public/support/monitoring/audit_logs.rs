use axum::{
    body::Body,
    http,
    response::{IntoResponse, Response},
    Json,
};
use chrono::Utc;
use serde_json::{json, Value};
use sqlx::Row;

use crate::gateway::handlers::shared::query_param_value;

use super::{
    build_auth_error_response, resolve_authenticated_local_user, AppState,
    GatewayPublicRequestContext,
};

fn parse_user_monitoring_limit(query: Option<&str>) -> Result<usize, String> {
    match query_param_value(query, "limit") {
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
        None => Ok(50),
    }
}

fn parse_user_monitoring_offset(query: Option<&str>) -> Result<usize, String> {
    match query_param_value(query, "offset") {
        Some(value) => value
            .parse::<usize>()
            .map_err(|_| "offset must be a non-negative integer".to_string()),
        None => Ok(0),
    }
}

fn parse_user_monitoring_days(query: Option<&str>) -> Result<i64, String> {
    match query_param_value(query, "days") {
        Some(value) => {
            let parsed = value
                .parse::<i64>()
                .map_err(|_| "days must be an integer between 1 and 365".to_string())?;
            if (1..=365).contains(&parsed) {
                Ok(parsed)
            } else {
                Err("days must be an integer between 1 and 365".to_string())
            }
        }
        None => Ok(30),
    }
}

fn build_user_monitoring_audit_logs_payload(
    items: Vec<Value>,
    total: usize,
    limit: usize,
    offset: usize,
    event_type: Option<String>,
    days: i64,
) -> Response<Body> {
    Json(json!({
        "items": items,
        "meta": {
            "total": total,
            "limit": limit,
            "offset": offset,
            "count": items.len(),
        },
        "filters": {
            "event_type": event_type,
            "days": days,
        }
    }))
    .into_response()
}

pub(super) async fn handle_user_audit_logs(
    state: &AppState,
    request_context: &GatewayPublicRequestContext,
    headers: &http::HeaderMap,
) -> Response<Body> {
    let auth = match resolve_authenticated_local_user(state, request_context, headers).await {
        Ok(value) => value,
        Err(response) => return response,
    };

    let query = request_context.request_query_string.as_deref();
    let event_type = query_param_value(query, "event_type").map(|value| value.trim().to_string());
    let event_type = event_type.filter(|value| !value.is_empty());
    let limit = match parse_user_monitoring_limit(query) {
        Ok(value) => value,
        Err(detail) => {
            return build_auth_error_response(http::StatusCode::BAD_REQUEST, detail, false)
        }
    };
    let offset = match parse_user_monitoring_offset(query) {
        Ok(value) => value,
        Err(detail) => {
            return build_auth_error_response(http::StatusCode::BAD_REQUEST, detail, false)
        }
    };
    let days = match parse_user_monitoring_days(query) {
        Ok(value) => value,
        Err(detail) => {
            return build_auth_error_response(http::StatusCode::BAD_REQUEST, detail, false)
        }
    };

    let Some(pool) = state.postgres_pool() else {
        return build_user_monitoring_audit_logs_payload(
            Vec::new(),
            0,
            limit,
            offset,
            event_type,
            days,
        );
    };

    let cutoff_time = Utc::now() - chrono::Duration::days(days);
    let total = if let Some(ref event_type) = event_type {
        match sqlx::query_scalar::<_, i64>(
            r#"
SELECT COUNT(*)
FROM audit_logs
WHERE user_id = $1
  AND created_at >= $2
  AND event_type = $3
"#,
        )
        .bind(&auth.user.id)
        .bind(cutoff_time)
        .bind(event_type)
        .fetch_one(&pool)
        .await
        {
            Ok(value) => usize::try_from(value.max(0)).unwrap_or(usize::MAX),
            Err(err) => {
                return build_auth_error_response(
                    http::StatusCode::INTERNAL_SERVER_ERROR,
                    format!("user audit logs count failed: {err}"),
                    false,
                )
            }
        }
    } else {
        match sqlx::query_scalar::<_, i64>(
            r#"
SELECT COUNT(*)
FROM audit_logs
WHERE user_id = $1
  AND created_at >= $2
"#,
        )
        .bind(&auth.user.id)
        .bind(cutoff_time)
        .fetch_one(&pool)
        .await
        {
            Ok(value) => usize::try_from(value.max(0)).unwrap_or(usize::MAX),
            Err(err) => {
                return build_auth_error_response(
                    http::StatusCode::INTERNAL_SERVER_ERROR,
                    format!("user audit logs count failed: {err}"),
                    false,
                )
            }
        }
    };

    let rows = if let Some(ref event_type) = event_type {
        match sqlx::query(
            r#"
SELECT id, event_type, description, ip_address, status_code, created_at
FROM audit_logs
WHERE user_id = $1
  AND created_at >= $2
  AND event_type = $3
ORDER BY created_at DESC
LIMIT $4 OFFSET $5
"#,
        )
        .bind(&auth.user.id)
        .bind(cutoff_time)
        .bind(event_type)
        .bind(i64::try_from(limit).unwrap_or(i64::MAX))
        .bind(i64::try_from(offset).unwrap_or(i64::MAX))
        .fetch_all(&pool)
        .await
        {
            Ok(value) => value,
            Err(err) => {
                return build_auth_error_response(
                    http::StatusCode::INTERNAL_SERVER_ERROR,
                    format!("user audit logs read failed: {err}"),
                    false,
                )
            }
        }
    } else {
        match sqlx::query(
            r#"
SELECT id, event_type, description, ip_address, status_code, created_at
FROM audit_logs
WHERE user_id = $1
  AND created_at >= $2
ORDER BY created_at DESC
LIMIT $3 OFFSET $4
"#,
        )
        .bind(&auth.user.id)
        .bind(cutoff_time)
        .bind(i64::try_from(limit).unwrap_or(i64::MAX))
        .bind(i64::try_from(offset).unwrap_or(i64::MAX))
        .fetch_all(&pool)
        .await
        {
            Ok(value) => value,
            Err(err) => {
                return build_auth_error_response(
                    http::StatusCode::INTERNAL_SERVER_ERROR,
                    format!("user audit logs read failed: {err}"),
                    false,
                )
            }
        }
    };

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
                "description": row.try_get::<String, _>("description").ok(),
                "ip_address": row.try_get::<Option<String>, _>("ip_address").ok().flatten(),
                "status_code": row.try_get::<Option<i32>, _>("status_code").ok().flatten(),
                "created_at": created_at,
            })
        })
        .collect::<Vec<_>>();

    build_user_monitoring_audit_logs_payload(items, total, limit, offset, event_type, days)
}
