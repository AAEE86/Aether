use axum::{
    body::Body,
    http,
    response::{IntoResponse, Response},
    Json,
};
use chrono::Utc;
use serde_json::json;

use super::{
    build_auth_error_response, resolve_authenticated_local_user, AppState,
    GatewayPublicRequestContext,
};

fn normalize_rate_limit_value(value: Option<i32>) -> u32 {
    value
        .map(|raw| raw.max(0))
        .and_then(|raw| u32::try_from(raw).ok())
        .unwrap_or(0)
}

pub(super) async fn handle_user_rate_limit_status(
    state: &AppState,
    request_context: &GatewayPublicRequestContext,
    headers: &http::HeaderMap,
) -> Response<Body> {
    let auth = match resolve_authenticated_local_user(state, request_context, headers).await {
        Ok(value) => value,
        Err(response) => return response,
    };

    let limiter = state.frontdoor_user_rpm();
    let now = Utc::now();
    let now_unix_secs = u64::try_from(now.timestamp()).unwrap_or(0);
    let system_default_limit = match limiter.current_system_default_limit(state).await {
        Ok(value) => value,
        Err(err) => {
            tracing::warn!(error = ?err, "user rate limit status system default read failed");
            0
        }
    };
    let bucket = limiter.current_bucket(now_unix_secs);
    let reset_time = (now
        + chrono::Duration::seconds(
            i64::try_from(limiter.retry_after(now_unix_secs)).unwrap_or(0),
        ))
    .to_rfc3339();
    let window = format!("{}s", limiter.config().bucket_seconds());

    let export_records = match state
        .list_auth_api_key_export_records_by_user_ids(std::slice::from_ref(&auth.user.id))
        .await
    {
        Ok(value) => value,
        Err(err) => {
            return build_auth_error_response(
                http::StatusCode::INTERNAL_SERVER_ERROR,
                format!("user rate limit status read failed: {err:?}"),
                false,
            )
        }
    };

    let mut api_keys = Vec::new();
    for record in export_records {
        if !record.is_active {
            continue;
        }

        let snapshot = match state
            .read_auth_api_key_snapshot(&auth.user.id, &record.api_key_id, now_unix_secs)
            .await
        {
            Ok(value) => value,
            Err(err) => {
                return build_auth_error_response(
                    http::StatusCode::INTERNAL_SERVER_ERROR,
                    format!("user api key snapshot read failed: {err:?}"),
                    false,
                )
            }
        };
        let is_standalone = snapshot
            .as_ref()
            .map(|value| value.api_key_is_standalone)
            .unwrap_or(record.is_standalone);
        let user_limit = if is_standalone {
            match record.rate_limit {
                Some(value) => normalize_rate_limit_value(Some(value)),
                None => system_default_limit,
            }
        } else {
            normalize_rate_limit_value(
                snapshot
                    .as_ref()
                    .and_then(|value| value.user_rate_limit)
                    .or(Some(
                        i32::try_from(system_default_limit).unwrap_or(i32::MAX),
                    )),
            )
        };
        let key_limit = if is_standalone {
            0
        } else {
            normalize_rate_limit_value(
                snapshot
                    .as_ref()
                    .and_then(|value| value.api_key_rate_limit)
                    .or(record.rate_limit),
            )
        };

        let user_scope_key = if is_standalone {
            limiter.standalone_scope_key(&record.api_key_id, bucket)
        } else {
            limiter.user_scope_key(&auth.user.id, bucket)
        };
        let key_scope_key = limiter.key_scope_key(&record.api_key_id, bucket);

        let user_count = if user_limit > 0 {
            match limiter
                .get_scope_count(state, &user_scope_key, bucket)
                .await
            {
                Ok(value) => value,
                Err(err) => {
                    tracing::warn!(error = ?err, scope_key = %user_scope_key, "user rpm scope read failed");
                    0
                }
            }
        } else {
            0
        };
        let key_count = if key_limit > 0 {
            match limiter.get_scope_count(state, &key_scope_key, bucket).await {
                Ok(value) => value,
                Err(err) => {
                    tracing::warn!(error = ?err, scope_key = %key_scope_key, "api key rpm scope read failed");
                    0
                }
            }
        } else {
            0
        };

        let user_remaining = if user_limit > 0 {
            Some(user_limit.saturating_sub(user_count))
        } else {
            None
        };
        let key_remaining = if key_limit > 0 {
            Some(key_limit.saturating_sub(key_count))
        } else {
            None
        };

        let primary_scope = match (user_remaining, key_remaining) {
            (Some(user_remaining), Some(key_remaining)) => {
                if user_remaining <= key_remaining {
                    Some(("user", user_limit, user_remaining))
                } else {
                    Some(("key", key_limit, key_remaining))
                }
            }
            (Some(user_remaining), None) => Some(("user", user_limit, user_remaining)),
            (None, Some(key_remaining)) => Some(("key", key_limit, key_remaining)),
            (None, None) => None,
        };

        api_keys.push(json!({
            "api_key_name": record
                .name
                .clone()
                .unwrap_or_else(|| format!("Key-{}", record.api_key_id)),
            "limit": primary_scope.map(|(_, limit, _)| limit),
            "remaining": primary_scope.map(|(_, _, remaining)| remaining),
            "scope": primary_scope.map(|(scope, _, _)| scope),
            "reset_time": primary_scope.map(|_| reset_time.clone()),
            "window": primary_scope.map(|_| window.clone()),
            "user_limit": if user_limit > 0 { Some(user_limit) } else { None::<u32> },
            "user_remaining": user_remaining,
            "key_limit": if key_limit > 0 { Some(key_limit) } else { None::<u32> },
            "key_remaining": key_remaining,
        }));
    }

    Json(json!({
        "user_id": auth.user.id,
        "api_keys": api_keys,
    }))
    .into_response()
}
