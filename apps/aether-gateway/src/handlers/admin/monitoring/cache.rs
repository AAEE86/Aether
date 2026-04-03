use super::{
    admin_monitoring_bad_request_response, admin_monitoring_cache_affinity_delete_params_from_path,
    admin_monitoring_cache_affinity_not_found_response, admin_monitoring_cache_affinity_sort_value,
    admin_monitoring_cache_affinity_unavailable_response,
    admin_monitoring_cache_affinity_user_identifier_from_path,
    admin_monitoring_cache_model_mapping_provider_params_from_path,
    admin_monitoring_cache_model_name_from_path, admin_monitoring_cache_provider_id_from_path,
    admin_monitoring_cache_redis_category_from_path,
    admin_monitoring_cache_users_not_found_response,
    admin_monitoring_cache_users_user_identifier_from_path,
    admin_monitoring_find_user_summary_by_id, admin_monitoring_has_test_redis_keys,
    admin_monitoring_list_export_api_key_records_by_ids,
    admin_monitoring_load_affinity_identity_maps, admin_monitoring_masked_provider_key_prefix,
    admin_monitoring_masked_user_api_key_prefix, admin_monitoring_not_found_response,
    admin_monitoring_redis_unavailable_response, build_admin_monitoring_cache_snapshot,
    clear_admin_monitoring_scheduler_affinity_entries,
    delete_admin_monitoring_cache_affinity_raw_keys, delete_admin_monitoring_namespaced_keys,
    list_admin_monitoring_cache_affinity_records,
    list_admin_monitoring_cache_affinity_records_by_affinity_keys,
    list_admin_monitoring_namespaced_keys, load_admin_monitoring_cache_affinity_entries_for_tests,
    parse_admin_monitoring_keyword_filter, parse_admin_monitoring_limit,
    parse_admin_monitoring_offset, ADMIN_MONITORING_CACHE_AFFINITY_DEFAULT_TTL_SECS,
    ADMIN_MONITORING_CACHE_RESERVATION_RATIO,
    ADMIN_MONITORING_DYNAMIC_RESERVATION_HIGH_LOAD_THRESHOLD,
    ADMIN_MONITORING_DYNAMIC_RESERVATION_LOW_LOAD_THRESHOLD,
    ADMIN_MONITORING_DYNAMIC_RESERVATION_PROBE_PHASE_REQUESTS,
    ADMIN_MONITORING_DYNAMIC_RESERVATION_PROBE_RESERVATION,
    ADMIN_MONITORING_DYNAMIC_RESERVATION_STABLE_MAX_RESERVATION,
    ADMIN_MONITORING_DYNAMIC_RESERVATION_STABLE_MIN_RESERVATION,
    ADMIN_MONITORING_REDIS_CACHE_CATEGORIES,
};
use crate::gateway::{AppState, GatewayError, GatewayPublicRequestContext};
use axum::{
    body::Body,
    http,
    response::{IntoResponse, Response},
    Json,
};
use serde_json::json;

pub(super) async fn build_admin_monitoring_cache_stats_response(
    state: &AppState,
) -> Result<Response<Body>, GatewayError> {
    let snapshot = build_admin_monitoring_cache_snapshot(state).await?;

    Ok(Json(json!({
        "status": "ok",
        "data": {
            "scheduler": snapshot.scheduler_name,
            "total_affinities": snapshot.total_affinities,
            "cache_hit_rate": snapshot.cache_hit_rate,
            "provider_switches": snapshot.provider_switches,
            "key_switches": snapshot.key_switches,
            "cache_hits": snapshot.cache_hits,
            "cache_misses": snapshot.cache_misses,
            "scheduler_metrics": {
                "cache_hits": snapshot.cache_hits,
                "cache_misses": snapshot.cache_misses,
                "cache_hit_rate": snapshot.cache_hit_rate,
                "total_batches": 0,
                "last_batch_size": 0,
                "total_candidates": 0,
                "last_candidate_count": 0,
                "concurrency_denied": 0,
                "avg_candidates_per_batch": 0.0,
                "scheduling_mode": snapshot.scheduling_mode,
                "provider_priority_mode": snapshot.provider_priority_mode,
            },
            "affinity_stats": {
                "storage_type": snapshot.storage_type,
                "total_affinities": snapshot.total_affinities,
                "cache_hits": snapshot.cache_hits,
                "cache_misses": snapshot.cache_misses,
                "cache_hit_rate": snapshot.cache_hit_rate,
                "cache_invalidations": snapshot.cache_invalidations,
                "provider_switches": snapshot.provider_switches,
                "key_switches": snapshot.key_switches,
                "config": {
                    "default_ttl": ADMIN_MONITORING_CACHE_AFFINITY_DEFAULT_TTL_SECS,
                }
            }
        }
    }))
    .into_response())
}

pub(super) async fn build_admin_monitoring_cache_metrics_response(
    state: &AppState,
) -> Result<Response<Body>, GatewayError> {
    let snapshot = build_admin_monitoring_cache_snapshot(state).await?;
    let metrics = [
        (
            "cache_scheduler_total_batches",
            "Number of scheduling batches processed",
            0.0,
        ),
        (
            "cache_scheduler_last_batch_size",
            "Size of the most recent scheduling batch",
            0.0,
        ),
        (
            "cache_scheduler_total_candidates",
            "Total candidates seen during scheduling",
            0.0,
        ),
        (
            "cache_scheduler_last_candidate_count",
            "Number of candidates in the most recent batch",
            0.0,
        ),
        (
            "cache_scheduler_cache_hits",
            "Cache hits counted during scheduling",
            snapshot.cache_hits as f64,
        ),
        (
            "cache_scheduler_cache_misses",
            "Cache misses counted during scheduling",
            snapshot.cache_misses as f64,
        ),
        (
            "cache_scheduler_cache_hit_rate",
            "Cache hit rate during scheduling",
            snapshot.cache_hit_rate,
        ),
        (
            "cache_scheduler_concurrency_denied",
            "Times candidate rejected due to concurrency limits",
            0.0,
        ),
        (
            "cache_scheduler_avg_candidates_per_batch",
            "Average candidates per batch",
            0.0,
        ),
        (
            "cache_affinity_total",
            "Total cache affinities stored",
            snapshot.total_affinities as f64,
        ),
        (
            "cache_affinity_hits",
            "Affinity cache hits",
            snapshot.cache_hits as f64,
        ),
        (
            "cache_affinity_misses",
            "Affinity cache misses",
            snapshot.cache_misses as f64,
        ),
        (
            "cache_affinity_hit_rate",
            "Affinity cache hit rate",
            snapshot.cache_hit_rate,
        ),
        (
            "cache_affinity_invalidations",
            "Affinity invalidations",
            snapshot.cache_invalidations as f64,
        ),
        (
            "cache_affinity_provider_switches",
            "Affinity provider switches",
            snapshot.provider_switches as f64,
        ),
        (
            "cache_affinity_key_switches",
            "Affinity key switches",
            snapshot.key_switches as f64,
        ),
    ];

    let mut lines = Vec::with_capacity(metrics.len() * 3 + 1);
    for (name, help_text, value) in metrics {
        lines.push(format!("# HELP {name} {help_text}"));
        lines.push(format!("# TYPE {name} gauge"));
        lines.push(format!("{name} {value}"));
    }
    lines.push(format!(
        "cache_scheduler_info{{scheduler=\"{}\"}} 1",
        snapshot.scheduler_name
    ));

    Ok((
        [(
            http::header::CONTENT_TYPE,
            "text/plain; version=0.0.4; charset=utf-8",
        )],
        lines.join("\n") + "\n",
    )
        .into_response())
}

pub(super) async fn build_admin_monitoring_cache_config_response(
) -> Result<Response<Body>, GatewayError> {
    Ok(Json(json!({
        "status": "ok",
        "data": {
            "cache_ttl_seconds": ADMIN_MONITORING_CACHE_AFFINITY_DEFAULT_TTL_SECS,
            "cache_reservation_ratio": ADMIN_MONITORING_CACHE_RESERVATION_RATIO,
            "dynamic_reservation": {
                "enabled": true,
                "config": {
                    "probe_phase_requests": ADMIN_MONITORING_DYNAMIC_RESERVATION_PROBE_PHASE_REQUESTS,
                    "probe_reservation": ADMIN_MONITORING_DYNAMIC_RESERVATION_PROBE_RESERVATION,
                    "stable_min_reservation": ADMIN_MONITORING_DYNAMIC_RESERVATION_STABLE_MIN_RESERVATION,
                    "stable_max_reservation": ADMIN_MONITORING_DYNAMIC_RESERVATION_STABLE_MAX_RESERVATION,
                    "low_load_threshold": ADMIN_MONITORING_DYNAMIC_RESERVATION_LOW_LOAD_THRESHOLD,
                    "high_load_threshold": ADMIN_MONITORING_DYNAMIC_RESERVATION_HIGH_LOAD_THRESHOLD,
                },
                "description": {
                    "probe_phase_requests": "探测阶段请求数阈值",
                    "probe_reservation": "探测阶段预留比例",
                    "stable_min_reservation": "稳定阶段最小预留比例",
                    "stable_max_reservation": "稳定阶段最大预留比例",
                    "low_load_threshold": "低负载阈值（低于此值使用最小预留）",
                    "high_load_threshold": "高负载阈值（高于此值根据置信度使用较高预留）",
                },
            },
            "description": {
                "cache_ttl": "缓存亲和性有效期（秒）",
                "cache_reservation_ratio": "静态预留比例（已被动态预留替代）",
                "dynamic_reservation": "动态预留机制配置",
            },
        }
    }))
    .into_response())
}

pub(super) async fn build_admin_monitoring_model_mapping_stats_response(
    state: &AppState,
) -> Result<Response<Body>, GatewayError> {
    if state.redis_kv_runner().is_none() && !admin_monitoring_has_test_redis_keys(state) {
        return Ok(Json(json!({
            "status": "ok",
            "data": {
                "available": false,
                "message": "Redis 未启用，模型映射缓存不可用",
            }
        }))
        .into_response());
    };

    let model_id_keys = list_admin_monitoring_namespaced_keys(state, "model:id:*").await?;
    let global_model_id_keys =
        list_admin_monitoring_namespaced_keys(state, "global_model:id:*").await?;
    let global_model_name_keys =
        list_admin_monitoring_namespaced_keys(state, "global_model:name:*").await?;
    let global_model_resolve_keys =
        list_admin_monitoring_namespaced_keys(state, "global_model:resolve:*").await?;
    let provider_global_keys =
        list_admin_monitoring_namespaced_keys(state, "model:provider_global:*")
            .await?
            .into_iter()
            .filter(|key| !key.starts_with("model:provider_global:hits:"))
            .collect::<Vec<_>>();

    let total_keys = model_id_keys.len()
        + global_model_id_keys.len()
        + global_model_name_keys.len()
        + global_model_resolve_keys.len()
        + provider_global_keys.len();

    Ok(Json(json!({
        "status": "ok",
        "data": {
            "available": true,
            "ttl_seconds": 300,
            "total_keys": total_keys,
            "breakdown": {
                "model_by_id": model_id_keys.len(),
                "model_by_provider_global": provider_global_keys.len(),
                "global_model_by_id": global_model_id_keys.len(),
                "global_model_by_name": global_model_name_keys.len(),
                "global_model_resolve": global_model_resolve_keys.len(),
            },
            "mappings": [],
            "provider_model_mappings": serde_json::Value::Null,
            "unmapped": serde_json::Value::Null,
        }
    }))
    .into_response())
}

pub(super) async fn build_admin_monitoring_redis_cache_categories_response(
    state: &AppState,
) -> Result<Response<Body>, GatewayError> {
    if state.redis_kv_runner().is_none() && !admin_monitoring_has_test_redis_keys(state) {
        return Ok(Json(json!({
            "status": "ok",
            "data": {
                "available": false,
                "message": "Redis 未启用",
            }
        }))
        .into_response());
    };

    let mut categories = Vec::with_capacity(ADMIN_MONITORING_REDIS_CACHE_CATEGORIES.len());
    let mut total_keys = 0usize;

    for (key, name, pattern, description) in ADMIN_MONITORING_REDIS_CACHE_CATEGORIES {
        let count = list_admin_monitoring_namespaced_keys(state, pattern)
            .await?
            .len();
        total_keys += count;
        categories.push(json!({
            "key": key,
            "name": name,
            "pattern": pattern,
            "description": description,
            "count": count,
        }));
    }

    Ok(Json(json!({
        "status": "ok",
        "data": {
            "available": true,
            "categories": categories,
            "total_keys": total_keys,
        }
    }))
    .into_response())
}

pub(super) async fn build_admin_monitoring_cache_affinities_response(
    state: &AppState,
    request_context: &GatewayPublicRequestContext,
) -> Result<Response<Body>, GatewayError> {
    let limit = match parse_admin_monitoring_limit(request_context.request_query_string.as_deref())
    {
        Ok(value) => value,
        Err(detail) => return Ok(admin_monitoring_bad_request_response(detail)),
    };
    let offset =
        match parse_admin_monitoring_offset(request_context.request_query_string.as_deref()) {
            Ok(value) => value,
            Err(detail) => return Ok(admin_monitoring_bad_request_response(detail)),
        };
    let keyword =
        parse_admin_monitoring_keyword_filter(request_context.request_query_string.as_deref());

    let mut matched_user_id = None::<String>;
    let mut matched_api_key_id = None::<String>;
    let filtered_affinities = if let Some(keyword_value) = keyword.as_deref() {
        let direct_affinity_keys =
            std::iter::once(keyword_value.to_string()).collect::<std::collections::BTreeSet<_>>();
        let direct_affinities = list_admin_monitoring_cache_affinity_records_by_affinity_keys(
            state,
            &direct_affinity_keys,
        )
        .await?;
        if !direct_affinities.is_empty() {
            matched_api_key_id = Some(keyword_value.to_string());
            matched_user_id = admin_monitoring_list_export_api_key_records_by_ids(
                state,
                &[keyword_value.to_string()],
            )
            .await?
            .get(keyword_value)
            .map(|item| item.user_id.clone());
            direct_affinities
        } else if let Some(user) = state.find_user_auth_by_identifier(keyword_value).await? {
            matched_user_id = Some(user.id.clone());
            let user_api_key_ids = state
                .list_auth_api_key_export_records_by_user_ids(std::slice::from_ref(&user.id))
                .await?
                .into_iter()
                .map(|item| item.api_key_id)
                .collect::<std::collections::BTreeSet<_>>();
            list_admin_monitoring_cache_affinity_records_by_affinity_keys(state, &user_api_key_ids)
                .await?
        } else {
            list_admin_monitoring_cache_affinity_records(state).await?
        }
    } else {
        list_admin_monitoring_cache_affinity_records(state).await?
    };
    let (api_key_by_id, user_by_id) =
        admin_monitoring_load_affinity_identity_maps(state, &filtered_affinities).await?;

    let provider_ids = filtered_affinities
        .iter()
        .filter_map(|item| item.provider_id.clone())
        .collect::<std::collections::BTreeSet<_>>()
        .into_iter()
        .collect::<Vec<_>>();
    let endpoint_ids = filtered_affinities
        .iter()
        .filter_map(|item| item.endpoint_id.clone())
        .collect::<std::collections::BTreeSet<_>>()
        .into_iter()
        .collect::<Vec<_>>();
    let key_ids = filtered_affinities
        .iter()
        .filter_map(|item| item.key_id.clone())
        .collect::<std::collections::BTreeSet<_>>()
        .into_iter()
        .collect::<Vec<_>>();

    let provider_by_id = state
        .data
        .list_provider_catalog_providers_by_ids(&provider_ids)
        .await
        .map_err(|err| GatewayError::Internal(err.to_string()))?
        .into_iter()
        .map(|item| (item.id.clone(), item))
        .collect::<std::collections::BTreeMap<_, _>>();
    let endpoint_by_id = state
        .data
        .list_provider_catalog_endpoints_by_ids(&endpoint_ids)
        .await
        .map_err(|err| GatewayError::Internal(err.to_string()))?
        .into_iter()
        .map(|item| (item.id.clone(), item))
        .collect::<std::collections::BTreeMap<_, _>>();
    let key_by_id = state
        .data
        .list_provider_catalog_keys_by_ids(&key_ids)
        .await
        .map_err(|err| GatewayError::Internal(err.to_string()))?
        .into_iter()
        .map(|item| (item.id.clone(), item))
        .collect::<std::collections::BTreeMap<_, _>>();

    let keyword_lower = keyword.as_ref().map(|value| value.to_ascii_lowercase());
    let mut items = Vec::new();
    for affinity in filtered_affinities {
        let user_api_key = api_key_by_id.get(&affinity.affinity_key);
        let user_id = user_api_key.map(|item| item.user_id.clone());
        let user = user_id.as_ref().and_then(|id| user_by_id.get(id));
        let provider = affinity
            .provider_id
            .as_ref()
            .and_then(|id| provider_by_id.get(id));
        let endpoint = affinity
            .endpoint_id
            .as_ref()
            .and_then(|id| endpoint_by_id.get(id));
        let key = affinity.key_id.as_ref().and_then(|id| key_by_id.get(id));

        let user_api_key_name = user_api_key.and_then(|item| item.name.clone());
        let user_api_key_prefix = user_api_key.and_then(|item| {
            admin_monitoring_masked_user_api_key_prefix(state, item.key_encrypted.as_deref())
        });
        let provider_name = provider.map(|item| item.name.clone());
        let endpoint_url = endpoint
            .map(|item| item.base_url.clone())
            .filter(|value| !value.trim().is_empty());
        let key_name = key.map(|item| item.name.clone());
        let key_prefix =
            key.and_then(|item| admin_monitoring_masked_provider_key_prefix(state, item));
        let user_id_text = user_id.clone();
        let username = user.map(|item| item.username.clone());
        let email = user.and_then(|item| item.email.clone());
        let provider_id = affinity.provider_id.clone();
        let key_id = affinity.key_id.clone();

        if let Some(keyword_value) = keyword_lower.as_deref() {
            if matched_user_id.is_none() && matched_api_key_id.is_none() {
                let searchable = [
                    Some(affinity.affinity_key.as_str()),
                    user_api_key_name.as_deref(),
                    user_id_text.as_deref(),
                    username.as_deref(),
                    email.as_deref(),
                    provider_id.as_deref(),
                    key_id.as_deref(),
                ];
                if !searchable
                    .into_iter()
                    .flatten()
                    .any(|value| value.to_ascii_lowercase().contains(keyword_value))
                {
                    continue;
                }
            }
        }

        items.push(json!({
            "affinity_key": affinity.affinity_key,
            "user_api_key_name": user_api_key_name,
            "user_api_key_prefix": user_api_key_prefix,
            "is_standalone": user_api_key.map(|item| item.is_standalone).unwrap_or(false),
            "user_id": user_id_text,
            "username": username,
            "email": email,
            "provider_id": provider_id,
            "provider_name": provider_name,
            "endpoint_id": affinity.endpoint_id,
            "endpoint_url": endpoint_url,
            "key_id": key_id,
            "key_name": key_name,
            "key_prefix": key_prefix,
            "rate_multipliers": key.and_then(|item| item.rate_multipliers.clone()),
            "global_model_id": affinity.model_name,
            "model_name": affinity.model_name,
            "model_display_name": serde_json::Value::Null,
            "api_format": affinity.api_format,
            "created_at": affinity.created_at,
            "expire_at": affinity.expire_at,
            "request_count": affinity.request_count,
        }));
    }

    items.sort_by(|left, right| {
        admin_monitoring_cache_affinity_sort_value(right.get("expire_at"))
            .partial_cmp(&admin_monitoring_cache_affinity_sort_value(
                left.get("expire_at"),
            ))
            .unwrap_or(std::cmp::Ordering::Equal)
    });
    let total = items.len();
    let paged_items = items
        .into_iter()
        .skip(offset)
        .take(limit)
        .collect::<Vec<_>>();
    let paged_count = paged_items.len();

    Ok(Json(json!({
        "status": "ok",
        "data": {
            "items": paged_items,
            "meta": {
                "total": total,
                "limit": limit,
                "offset": offset,
                "count": paged_count,
            },
            "matched_user_id": matched_user_id,
        }
    }))
    .into_response())
}

pub(super) async fn build_admin_monitoring_cache_affinity_response(
    state: &AppState,
    request_context: &GatewayPublicRequestContext,
) -> Result<Response<Body>, GatewayError> {
    let Some(user_identifier) =
        admin_monitoring_cache_affinity_user_identifier_from_path(&request_context.request_path)
    else {
        return Ok(admin_monitoring_bad_request_response(
            "缺少 user_identifier",
        ));
    };
    let direct_api_key_by_id =
        admin_monitoring_list_export_api_key_records_by_ids(state, &[user_identifier.clone()])
            .await?;
    let direct_affinity_keys =
        std::iter::once(user_identifier.clone()).collect::<std::collections::BTreeSet<_>>();
    let direct_affinities =
        list_admin_monitoring_cache_affinity_records_by_affinity_keys(state, &direct_affinity_keys)
            .await?;

    let (resolved_user_id, username, email, filtered_affinities) = if !direct_affinities.is_empty()
        || direct_api_key_by_id.contains_key(&user_identifier)
    {
        let user_id = direct_api_key_by_id
            .get(&user_identifier)
            .map(|item| item.user_id.clone());
        let user = match user_id.as_deref() {
            Some(user_id) => admin_monitoring_find_user_summary_by_id(state, user_id).await?,
            None => None,
        };
        (
            user_id,
            user.as_ref().map(|item| item.username.clone()),
            user.and_then(|item| item.email),
            direct_affinities,
        )
    } else if let Some(user) = state.find_user_auth_by_identifier(&user_identifier).await? {
        let user_api_key_ids = state
            .list_auth_api_key_export_records_by_user_ids(std::slice::from_ref(&user.id))
            .await?
            .into_iter()
            .map(|item| item.api_key_id)
            .collect::<std::collections::BTreeSet<_>>();
        let affinities =
            list_admin_monitoring_cache_affinity_records_by_affinity_keys(state, &user_api_key_ids)
                .await?;
        (Some(user.id), Some(user.username), user.email, affinities)
    } else {
        return Ok(admin_monitoring_cache_affinity_not_found_response(
            &user_identifier,
        ));
    };

    if filtered_affinities.is_empty() {
        let display_name = username.clone().unwrap_or_else(|| user_identifier.clone());
        return Ok(Json(json!({
            "status": "not_found",
            "message": format!(
                "用户 {} ({}) 没有缓存亲和性",
                display_name,
                email.clone().unwrap_or_else(|| "null".to_string()),
            ),
            "user_info": {
                "user_id": resolved_user_id,
                "username": username,
                "email": email,
            },
            "affinities": [],
        }))
        .into_response());
    }

    let mut affinities = filtered_affinities
        .into_iter()
        .map(|item| {
            json!({
                "provider_id": item.provider_id,
                "endpoint_id": item.endpoint_id,
                "key_id": item.key_id,
                "api_format": item.api_format,
                "model_name": item.model_name,
                "created_at": item.created_at,
                "expire_at": item.expire_at,
                "request_count": item.request_count,
            })
        })
        .collect::<Vec<_>>();
    affinities.sort_by(|left, right| {
        admin_monitoring_cache_affinity_sort_value(right.get("expire_at"))
            .partial_cmp(&admin_monitoring_cache_affinity_sort_value(
                left.get("expire_at"),
            ))
            .unwrap_or(std::cmp::Ordering::Equal)
    });
    let total_endpoints = affinities.len();

    Ok(Json(json!({
        "status": "ok",
        "user_info": {
            "user_id": resolved_user_id,
            "username": username,
            "email": email,
        },
        "affinities": affinities,
        "total_endpoints": total_endpoints,
    }))
    .into_response())
}

pub(super) async fn build_admin_monitoring_cache_users_delete_response(
    state: &AppState,
    request_context: &GatewayPublicRequestContext,
) -> Result<Response<Body>, GatewayError> {
    let Some(user_identifier) =
        admin_monitoring_cache_users_user_identifier_from_path(&request_context.request_path)
    else {
        return Ok(admin_monitoring_bad_request_response(
            "缺少 user_identifier",
        ));
    };

    if state.redis_kv_runner().is_none()
        && load_admin_monitoring_cache_affinity_entries_for_tests(state).is_empty()
    {
        return Ok(admin_monitoring_cache_affinity_unavailable_response());
    }

    let direct_api_key_by_id =
        admin_monitoring_list_export_api_key_records_by_ids(state, &[user_identifier.clone()])
            .await?;

    if let Some(api_key) = direct_api_key_by_id.get(&user_identifier) {
        let target_affinity_keys =
            std::iter::once(user_identifier.clone()).collect::<std::collections::BTreeSet<_>>();
        let target_affinities = list_admin_monitoring_cache_affinity_records_by_affinity_keys(
            state,
            &target_affinity_keys,
        )
        .await?;
        let raw_keys = target_affinities
            .iter()
            .map(|item| item.raw_key.clone())
            .collect::<Vec<_>>();
        let _ = delete_admin_monitoring_cache_affinity_raw_keys(state, &raw_keys).await?;
        clear_admin_monitoring_scheduler_affinity_entries(state, &target_affinities);

        let user = admin_monitoring_find_user_summary_by_id(state, &api_key.user_id).await?;
        let api_key_name = api_key
            .name
            .clone()
            .unwrap_or_else(|| user_identifier.clone());
        return Ok(Json(json!({
            "status": "ok",
            "message": format!("已清除 API Key {api_key_name} 的缓存亲和性"),
            "user_info": {
                "user_id": Some(api_key.user_id.clone()),
                "username": user.as_ref().map(|item| item.username.clone()),
                "email": user.and_then(|item| item.email),
                "api_key_id": user_identifier,
                "api_key_name": api_key.name.clone(),
            },
        }))
        .into_response());
    }

    let Some(user) = state.find_user_auth_by_identifier(&user_identifier).await? else {
        return Ok(admin_monitoring_cache_users_not_found_response(
            &user_identifier,
        ));
    };

    let user_api_key_ids = state
        .list_auth_api_key_export_records_by_user_ids(std::slice::from_ref(&user.id))
        .await?
        .into_iter()
        .map(|item| item.api_key_id.clone())
        .collect::<std::collections::BTreeSet<_>>();
    let target_affinities =
        list_admin_monitoring_cache_affinity_records_by_affinity_keys(state, &user_api_key_ids)
            .await?;
    let raw_keys = target_affinities
        .iter()
        .map(|item| item.raw_key.clone())
        .collect::<Vec<_>>();
    let _ = delete_admin_monitoring_cache_affinity_raw_keys(state, &raw_keys).await?;
    clear_admin_monitoring_scheduler_affinity_entries(state, &target_affinities);

    Ok(Json(json!({
        "status": "ok",
        "message": format!("已清除用户 {} 的所有缓存亲和性", user.username),
        "user_info": {
            "user_id": user.id,
            "username": user.username,
            "email": user.email,
        },
    }))
    .into_response())
}

pub(super) async fn build_admin_monitoring_cache_affinity_delete_response(
    state: &AppState,
    request_context: &GatewayPublicRequestContext,
) -> Result<Response<Body>, GatewayError> {
    let Some((affinity_key, endpoint_id, model_id, api_format)) =
        admin_monitoring_cache_affinity_delete_params_from_path(&request_context.request_path)
    else {
        return Ok(admin_monitoring_bad_request_response(
            "缺少 affinity_key、endpoint_id、model_id 或 api_format",
        ));
    };

    if state.redis_kv_runner().is_none()
        && load_admin_monitoring_cache_affinity_entries_for_tests(state).is_empty()
    {
        return Ok(admin_monitoring_cache_affinity_unavailable_response());
    }

    let target_affinity_keys =
        std::iter::once(affinity_key.clone()).collect::<std::collections::BTreeSet<_>>();
    let target_affinity =
        list_admin_monitoring_cache_affinity_records_by_affinity_keys(state, &target_affinity_keys)
            .await?
            .into_iter()
            .find(|item| {
                item.affinity_key == affinity_key
                    && item.endpoint_id.as_deref() == Some(endpoint_id.as_str())
                    && item.model_name == model_id
                    && item.api_format.eq_ignore_ascii_case(&api_format)
            });
    let Some(target_affinity) = target_affinity else {
        return Ok(admin_monitoring_not_found_response(
            "未找到指定的缓存亲和性记录",
        ));
    };

    let _ = delete_admin_monitoring_cache_affinity_raw_keys(
        state,
        std::slice::from_ref(&target_affinity.raw_key),
    )
    .await?;
    clear_admin_monitoring_scheduler_affinity_entries(
        state,
        std::slice::from_ref(&target_affinity),
    );

    let mut api_key_by_id = admin_monitoring_list_export_api_key_records_by_ids(
        state,
        std::slice::from_ref(&affinity_key),
    )
    .await?;
    let api_key_name = api_key_by_id
        .remove(&affinity_key)
        .and_then(|item| item.name)
        .unwrap_or_else(|| affinity_key.chars().take(8).collect::<String>());

    Ok(Json(json!({
        "status": "ok",
        "message": format!("已清除缓存亲和性: {api_key_name}"),
        "affinity_key": affinity_key,
        "endpoint_id": endpoint_id,
        "model_id": model_id,
    }))
    .into_response())
}

pub(super) async fn build_admin_monitoring_cache_flush_response(
    state: &AppState,
) -> Result<Response<Body>, GatewayError> {
    let raw_affinities = list_admin_monitoring_cache_affinity_records(state).await?;
    if state.redis_kv_runner().is_none() && raw_affinities.is_empty() {
        return Ok(admin_monitoring_cache_affinity_unavailable_response());
    }

    let raw_keys = raw_affinities
        .iter()
        .map(|item| item.raw_key.clone())
        .collect::<Vec<_>>();
    let deleted = delete_admin_monitoring_cache_affinity_raw_keys(state, &raw_keys).await?;
    clear_admin_monitoring_scheduler_affinity_entries(state, &raw_affinities);

    Ok(Json(json!({
        "status": "ok",
        "message": "已清除全部缓存亲和性",
        "deleted_affinities": deleted,
    }))
    .into_response())
}

pub(super) async fn build_admin_monitoring_cache_provider_delete_response(
    state: &AppState,
    request_context: &GatewayPublicRequestContext,
) -> Result<Response<Body>, GatewayError> {
    let Some(provider_id) =
        admin_monitoring_cache_provider_id_from_path(&request_context.request_path)
    else {
        return Ok(admin_monitoring_bad_request_response("缺少 provider_id"));
    };

    let raw_affinities = list_admin_monitoring_cache_affinity_records(state).await?;
    if state.redis_kv_runner().is_none() && raw_affinities.is_empty() {
        return Ok(admin_monitoring_cache_affinity_unavailable_response());
    }

    let target_affinities = raw_affinities
        .into_iter()
        .filter(|item| item.provider_id.as_deref() == Some(provider_id.as_str()))
        .collect::<Vec<_>>();
    if target_affinities.is_empty() {
        return Ok((
            http::StatusCode::NOT_FOUND,
            Json(json!({
                "detail": format!("未找到 provider {provider_id} 的缓存亲和性记录")
            })),
        )
            .into_response());
    }

    let raw_keys = target_affinities
        .iter()
        .map(|item| item.raw_key.clone())
        .collect::<Vec<_>>();
    let deleted = delete_admin_monitoring_cache_affinity_raw_keys(state, &raw_keys).await?;
    clear_admin_monitoring_scheduler_affinity_entries(state, &target_affinities);

    Ok(Json(json!({
        "status": "ok",
        "message": format!("已清除 provider {provider_id} 的缓存亲和性"),
        "provider_id": provider_id,
        "deleted_affinities": deleted,
    }))
    .into_response())
}

pub(super) async fn build_admin_monitoring_model_mapping_delete_response(
    state: &AppState,
) -> Result<Response<Body>, GatewayError> {
    if state.redis_kv_runner().is_none() && !admin_monitoring_has_test_redis_keys(state) {
        return Ok(admin_monitoring_redis_unavailable_response());
    }

    let mut raw_keys = list_admin_monitoring_namespaced_keys(state, "model:*").await?;
    raw_keys.extend(list_admin_monitoring_namespaced_keys(state, "global_model:*").await?);
    raw_keys.sort();
    raw_keys.dedup();
    let deleted_count = delete_admin_monitoring_namespaced_keys(state, &raw_keys).await?;

    Ok(Json(json!({
        "status": "ok",
        "message": "已清除所有模型映射缓存",
        "deleted_count": deleted_count,
    }))
    .into_response())
}

pub(super) async fn build_admin_monitoring_model_mapping_delete_model_response(
    state: &AppState,
    request_context: &GatewayPublicRequestContext,
) -> Result<Response<Body>, GatewayError> {
    let Some(model_name) =
        admin_monitoring_cache_model_name_from_path(&request_context.request_path)
    else {
        return Ok(admin_monitoring_bad_request_response("缺少 model_name"));
    };
    if state.redis_kv_runner().is_none() && !admin_monitoring_has_test_redis_keys(state) {
        return Ok(admin_monitoring_redis_unavailable_response());
    }

    let candidate_keys = [
        format!("global_model:resolve:{model_name}"),
        format!("global_model:name:{model_name}"),
    ];
    let mut existing_keys = Vec::new();
    for key in candidate_keys {
        let matches = list_admin_monitoring_namespaced_keys(state, key.as_str()).await?;
        existing_keys.extend(matches);
    }
    existing_keys.sort();
    existing_keys.dedup();

    let deleted_count = delete_admin_monitoring_namespaced_keys(state, &existing_keys).await?;
    let deleted_keys = if deleted_count == 0 {
        Vec::new()
    } else {
        existing_keys
    };

    Ok(Json(json!({
        "status": "ok",
        "message": format!("已清除模型 {model_name} 的映射缓存"),
        "model_name": model_name,
        "deleted_keys": deleted_keys,
    }))
    .into_response())
}

pub(super) async fn build_admin_monitoring_model_mapping_delete_provider_response(
    state: &AppState,
    request_context: &GatewayPublicRequestContext,
) -> Result<Response<Body>, GatewayError> {
    let Some((provider_id, global_model_id)) =
        admin_monitoring_cache_model_mapping_provider_params_from_path(
            &request_context.request_path,
        )
    else {
        return Ok(admin_monitoring_bad_request_response(
            "缺少 provider_id 或 global_model_id",
        ));
    };
    if state.redis_kv_runner().is_none() && !admin_monitoring_has_test_redis_keys(state) {
        return Ok(admin_monitoring_redis_unavailable_response());
    }

    let candidate_keys = [
        format!("model:provider_global:{provider_id}:{global_model_id}"),
        format!("model:provider_global:hits:{provider_id}:{global_model_id}"),
    ];
    let mut existing_keys = Vec::new();
    for key in candidate_keys {
        let matches = list_admin_monitoring_namespaced_keys(state, key.as_str()).await?;
        existing_keys.extend(matches);
    }
    existing_keys.sort();
    existing_keys.dedup();

    let _ = delete_admin_monitoring_namespaced_keys(state, &existing_keys).await?;

    Ok(Json(json!({
        "status": "ok",
        "message": "已清除 Provider 模型映射缓存",
        "provider_id": provider_id,
        "global_model_id": global_model_id,
        "deleted_keys": existing_keys,
    }))
    .into_response())
}

pub(super) async fn build_admin_monitoring_redis_keys_delete_response(
    state: &AppState,
    request_context: &GatewayPublicRequestContext,
) -> Result<Response<Body>, GatewayError> {
    let Some(category) =
        admin_monitoring_cache_redis_category_from_path(&request_context.request_path)
    else {
        return Ok(admin_monitoring_bad_request_response("缺少 category"));
    };

    let Some((cat_key, name, pattern, _description)) = ADMIN_MONITORING_REDIS_CACHE_CATEGORIES
        .iter()
        .find(|(cat_key, _, _, _)| *cat_key == category)
    else {
        return Ok((
            http::StatusCode::NOT_FOUND,
            Json(json!({ "detail": format!("未知的缓存分类: {category}") })),
        )
            .into_response());
    };

    if state.redis_kv_runner().is_none() && !admin_monitoring_has_test_redis_keys(state) {
        return Ok(admin_monitoring_redis_unavailable_response());
    }

    let raw_keys = list_admin_monitoring_namespaced_keys(state, pattern).await?;
    let deleted_count = delete_admin_monitoring_namespaced_keys(state, &raw_keys).await?;

    Ok(Json(json!({
        "status": "ok",
        "message": format!("已清除 {name} 缓存"),
        "category": cat_key,
        "deleted_count": deleted_count,
    }))
    .into_response())
}
