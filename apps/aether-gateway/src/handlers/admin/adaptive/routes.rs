use super::super::build_proxy_error_response;
use super::adaptive_shared::{
    admin_adaptive_adjustment_items, admin_adaptive_dispatcher_not_found_response,
    admin_adaptive_effective_limit, admin_adaptive_find_key, admin_adaptive_key_id_from_path,
    admin_adaptive_key_not_found_response, admin_adaptive_key_payload,
    admin_adaptive_load_candidate_keys,
};
use crate::gateway::handlers::{query_param_value, unix_secs_to_rfc3339};
use crate::gateway::{AppState, GatewayError, GatewayPublicRequestContext};
use axum::{
    body::{Body, Bytes},
    http,
    response::{IntoResponse, Response},
    Json,
};
use serde::Deserialize;
use serde_json::json;

const ADMIN_ADAPTIVE_DATA_UNAVAILABLE_DETAIL: &str = "Admin adaptive data unavailable";

#[derive(Debug, Deserialize)]
struct AdminAdaptiveToggleModeRequest {
    enabled: bool,
    #[serde(default)]
    fixed_limit: Option<u32>,
}

pub(super) async fn maybe_build_local_admin_adaptive_response(
    state: &AppState,
    request_context: &GatewayPublicRequestContext,
    request_body: Option<&Bytes>,
) -> Result<Option<Response<Body>>, GatewayError> {
    let Some(decision) = request_context.control_decision.as_ref() else {
        return Ok(None);
    };

    if decision.route_family.as_deref() != Some("adaptive_manage") {
        return Ok(None);
    }

    if !state.has_provider_catalog_data_reader() {
        return Ok(Some(build_proxy_error_response(
            http::StatusCode::SERVICE_UNAVAILABLE,
            "data_unavailable",
            ADMIN_ADAPTIVE_DATA_UNAVAILABLE_DETAIL,
            Some(json!({
                "error": ADMIN_ADAPTIVE_DATA_UNAVAILABLE_DETAIL,
            })),
        )));
    }

    if decision.route_kind.as_deref() == Some("list_keys")
        && request_context.request_method == http::Method::GET
        && matches!(
            request_context.request_path.as_str(),
            "/api/admin/adaptive/keys" | "/api/admin/adaptive/keys/"
        )
    {
        let provider_id = query_param_value(
            request_context.request_query_string.as_deref(),
            "provider_id",
        );
        let payload = admin_adaptive_load_candidate_keys(state, provider_id.as_deref())
            .await?
            .into_iter()
            .filter(|key| key.rpm_limit.is_none())
            .map(|key| admin_adaptive_key_payload(&key))
            .collect::<Vec<_>>();
        return Ok(Some(Json(payload).into_response()));
    }

    if decision.route_kind.as_deref() == Some("summary")
        && request_context.request_method == http::Method::GET
        && matches!(
            request_context.request_path.as_str(),
            "/api/admin/adaptive/summary" | "/api/admin/adaptive/summary/"
        )
    {
        let keys = admin_adaptive_load_candidate_keys(state, None).await?;
        let adaptive_keys = keys
            .into_iter()
            .filter(|key| key.rpm_limit.is_none())
            .collect::<Vec<_>>();

        let total_keys = adaptive_keys.len() as u64;
        let total_concurrent_429 = adaptive_keys
            .iter()
            .map(|key| u64::from(key.concurrent_429_count.unwrap_or(0)))
            .sum::<u64>();
        let total_rpm_429 = adaptive_keys
            .iter()
            .map(|key| u64::from(key.rpm_429_count.unwrap_or(0)))
            .sum::<u64>();
        let mut recent_adjustments = vec![];
        let mut total_adjustments = 0usize;
        for key in adaptive_keys {
            let history = admin_adaptive_adjustment_items(key.adjustment_history.as_ref());
            total_adjustments += history.len();
            for adjustment in history.into_iter().rev().take(3) {
                let mut payload = adjustment;
                payload.insert("key_id".to_string(), json!(key.id));
                payload.insert("key_name".to_string(), json!(key.name));
                recent_adjustments.push(serde_json::Value::Object(payload));
            }
        }
        recent_adjustments.sort_by(|left, right| {
            let lhs = left
                .get("timestamp")
                .and_then(serde_json::Value::as_str)
                .unwrap_or_default();
            let rhs = right
                .get("timestamp")
                .and_then(serde_json::Value::as_str)
                .unwrap_or_default();
            rhs.cmp(lhs)
        });

        return Ok(Some(
            Json(json!({
                "total_adaptive_keys": total_keys,
                "total_concurrent_429_errors": total_concurrent_429,
                "total_rpm_429_errors": total_rpm_429,
                "total_adjustments": total_adjustments,
                "recent_adjustments": recent_adjustments.into_iter().take(10).collect::<Vec<_>>(),
            }))
            .into_response(),
        ));
    }

    let key_id = admin_adaptive_key_id_from_path(&request_context.request_path);
    if key_id.is_none() {
        return Ok(Some(admin_adaptive_dispatcher_not_found_response()));
    }
    let key_id = key_id.expect("checked is_some above");

    if decision.route_kind.as_deref() == Some("get_stats")
        && request_context.request_method == http::Method::GET
        && request_context.request_path.ends_with("/stats")
    {
        let Some(key) = admin_adaptive_find_key(state, &key_id).await? else {
            return Ok(Some(admin_adaptive_key_not_found_response(&key_id)));
        };
        let status_snapshot = key
            .status_snapshot
            .as_ref()
            .and_then(serde_json::Value::as_object);
        let adjustments = admin_adaptive_adjustment_items(key.adjustment_history.as_ref());
        let adjustment_count = adjustments.len();
        let recent_adjustments = adjustments
            .into_iter()
            .rev()
            .take(10)
            .map(serde_json::Value::Object)
            .collect::<Vec<_>>();

        return Ok(Some(
            Json(json!({
                "adaptive_mode": key.rpm_limit.is_none(),
                "rpm_limit": key.rpm_limit,
                "effective_limit": admin_adaptive_effective_limit(&key),
                "learned_limit": key.learned_rpm_limit,
                "concurrent_429_count": key.concurrent_429_count.unwrap_or(0),
                "rpm_429_count": key.rpm_429_count.unwrap_or(0),
                "last_429_at": key.last_429_at_unix_secs.and_then(unix_secs_to_rfc3339),
                "last_429_type": key.last_429_type,
                "adjustment_count": adjustment_count,
                "recent_adjustments": recent_adjustments,
                "learning_confidence": status_snapshot.and_then(|value| value.get("learning_confidence")).cloned(),
                "enforcement_active": status_snapshot.and_then(|value| value.get("enforcement_active")).cloned(),
                "observation_count": status_snapshot
                    .and_then(|value| value.get("observation_count"))
                    .and_then(serde_json::Value::as_u64)
                    .unwrap_or(0),
                "header_observation_count": status_snapshot
                    .and_then(|value| value.get("header_observation_count"))
                    .and_then(serde_json::Value::as_u64)
                    .unwrap_or(0),
                "latest_upstream_limit": status_snapshot
                    .and_then(|value| value.get("latest_upstream_limit"))
                    .and_then(serde_json::Value::as_u64),
            }))
            .into_response(),
        ));
    }

    if !state.has_provider_catalog_data_writer() {
        return Ok(Some(build_proxy_error_response(
            http::StatusCode::SERVICE_UNAVAILABLE,
            "data_unavailable",
            ADMIN_ADAPTIVE_DATA_UNAVAILABLE_DETAIL,
            Some(json!({
                "error": ADMIN_ADAPTIVE_DATA_UNAVAILABLE_DETAIL,
            })),
        )));
    }

    if decision.route_kind.as_deref() == Some("toggle_mode")
        && request_context.request_method == http::Method::PATCH
        && request_context.request_path.ends_with("/mode")
    {
        let Some(request_body) = request_body else {
            return Ok(Some(build_proxy_error_response(
                http::StatusCode::BAD_REQUEST,
                "invalid_request",
                "请求数据验证失败",
                None,
            )));
        };
        let body = match serde_json::from_slice::<AdminAdaptiveToggleModeRequest>(request_body) {
            Ok(payload) => payload,
            Err(_) => {
                return Ok(Some(build_proxy_error_response(
                    http::StatusCode::BAD_REQUEST,
                    "invalid_request",
                    "请求数据验证失败",
                    None,
                )));
            }
        };
        let Some(mut key) = admin_adaptive_find_key(state, &key_id).await? else {
            return Ok(Some(admin_adaptive_key_not_found_response(&key_id)));
        };
        let message = if body.enabled {
            key.rpm_limit = None;
            "已切换为自适应模式，系统将自动学习并调整 RPM 限制".to_string()
        } else {
            let Some(fixed_limit) = body.fixed_limit else {
                return Ok(Some(
                    (
                        http::StatusCode::BAD_REQUEST,
                        Json(json!({
                            "detail": "禁用自适应模式时必须提供 fixed_limit 参数",
                        })),
                    )
                        .into_response(),
                ));
            };
            if !(1..=100).contains(&fixed_limit) {
                return Ok(Some(
                    (
                        http::StatusCode::BAD_REQUEST,
                        Json(json!({
                            "detail": "fixed_limit 超出范围（1-100）",
                        })),
                    )
                        .into_response(),
                ));
            }
            key.rpm_limit = Some(fixed_limit);
            format!("已切换为固定限制模式，RPM 限制设为 {fixed_limit}")
        };
        let Some(updated) = state.update_provider_catalog_key(&key).await? else {
            return Ok(Some(admin_adaptive_key_not_found_response(&key_id)));
        };
        return Ok(Some(
            Json(json!({
                "message": message,
                "key_id": updated.id,
                "is_adaptive": updated.rpm_limit.is_none(),
                "rpm_limit": updated.rpm_limit,
                "effective_limit": admin_adaptive_effective_limit(&updated),
            }))
            .into_response(),
        ));
    }

    if decision.route_kind.as_deref() == Some("set_limit")
        && request_context.request_method == http::Method::PATCH
        && request_context.request_path.ends_with("/limit")
    {
        let Some(limit_value) =
            query_param_value(request_context.request_query_string.as_deref(), "limit")
        else {
            return Ok(Some(
                (
                    http::StatusCode::BAD_REQUEST,
                    Json(json!({ "detail": "缺少 limit 参数" })),
                )
                    .into_response(),
            ));
        };
        let Ok(limit) = limit_value.parse::<u32>() else {
            return Ok(Some(
                (
                    http::StatusCode::BAD_REQUEST,
                    Json(json!({ "detail": "limit 必须是数字" })),
                )
                    .into_response(),
            ));
        };
        if !(1..=100).contains(&limit) {
            return Ok(Some(
                (
                    http::StatusCode::BAD_REQUEST,
                    Json(json!({ "detail": "limit 超出范围（1-100）" })),
                )
                    .into_response(),
            ));
        }

        let Some(mut key) = admin_adaptive_find_key(state, &key_id).await? else {
            return Ok(Some(admin_adaptive_key_not_found_response(&key_id)));
        };
        let was_adaptive = key.rpm_limit.is_none();
        key.rpm_limit = Some(limit);
        let Some(updated) = state.update_provider_catalog_key(&key).await? else {
            return Ok(Some(admin_adaptive_key_not_found_response(&key_id)));
        };
        return Ok(Some(
            Json(json!({
                "message": format!("已设置为固定限制模式，RPM 限制为 {limit}"),
                "key_id": updated.id,
                "is_adaptive": false,
                "rpm_limit": updated.rpm_limit,
                "previous_mode": if was_adaptive { "adaptive" } else { "fixed" },
            }))
            .into_response(),
        ));
    }

    if decision.route_kind.as_deref() == Some("reset_learning")
        && request_context.request_method == http::Method::DELETE
        && request_context.request_path.ends_with("/learning")
    {
        let Some(mut key) = admin_adaptive_find_key(state, &key_id).await? else {
            return Ok(Some(admin_adaptive_key_not_found_response(&key_id)));
        };
        key.learned_rpm_limit = None;
        key.concurrent_429_count = None;
        key.rpm_429_count = None;
        key.last_429_at_unix_secs = None;
        key.last_429_type = None;
        key.adjustment_history = None;
        let Some(updated) = state.update_provider_catalog_key(&key).await? else {
            return Ok(Some(admin_adaptive_key_not_found_response(&key_id)));
        };
        return Ok(Some(
            Json(json!({
                "message": "学习状态已重置",
                "key_id": updated.id,
            }))
            .into_response(),
        ));
    }

    Ok(Some(admin_adaptive_dispatcher_not_found_response()))
}
