use super::super::{
    list_usage_for_optional_range, round_to, AdminStatsTimeRange, AdminStatsUsageFilter,
};
use super::{
    admin_usage_aggregation_by_api_format_json, admin_usage_aggregation_by_model_json,
    admin_usage_aggregation_by_provider_json, admin_usage_aggregation_by_user_json,
    admin_usage_bad_request_response, admin_usage_calculate_recommended_ttl,
    admin_usage_collect_request_intervals_minutes, admin_usage_data_unavailable_response,
    admin_usage_group_completed_by_api_key, admin_usage_group_completed_by_user,
    admin_usage_heatmap_json, admin_usage_matches_optional_id, admin_usage_parse_aggregation_limit,
    admin_usage_parse_recent_hours, admin_usage_parse_timeline_limit, admin_usage_percentile_cont,
    admin_usage_point_sort_key, admin_usage_proportional_limits,
    admin_usage_ttl_recommendation_reason, list_recent_completed_usage_for_cache_affinity,
    ADMIN_USAGE_DATA_UNAVAILABLE_DETAIL,
};
use crate::gateway::handlers::{query_param_bool, query_param_value, unix_secs_to_rfc3339};
use crate::gateway::{AppState, GatewayError, GatewayPublicRequestContext};
use axum::{
    body::Body,
    http,
    response::{IntoResponse, Response},
    Json,
};
use serde_json::{json, Value};
use std::collections::{BTreeMap, BTreeSet};

pub(super) async fn maybe_build_local_admin_usage_analytics_response(
    state: &AppState,
    request_context: &GatewayPublicRequestContext,
) -> Result<Option<Response<Body>>, GatewayError> {
    let route_kind = request_context
        .control_decision
        .as_ref()
        .and_then(|decision| decision.route_kind.as_deref());

    match route_kind {
        Some("aggregation_stats")
            if request_context.request_method == http::Method::GET
                && matches!(
                    request_context.request_path.as_str(),
                    "/api/admin/usage/aggregation/stats" | "/api/admin/usage/aggregation/stats/"
                ) =>
        {
            if !state.has_usage_data_reader() {
                return Ok(Some(admin_usage_data_unavailable_response(
                    ADMIN_USAGE_DATA_UNAVAILABLE_DETAIL,
                )));
            }

            let query = request_context.request_query_string.as_deref();
            let group_by = query_param_value(query, "group_by")
                .unwrap_or_default()
                .trim()
                .to_ascii_lowercase();
            if !matches!(
                group_by.as_str(),
                "model" | "user" | "provider" | "api_format"
            ) {
                return Ok(Some(admin_usage_bad_request_response(
                    "Invalid group_by value: must be one of model, user, provider, api_format",
                )));
            }
            let limit = match admin_usage_parse_aggregation_limit(query) {
                Ok(value) => value,
                Err(detail) => return Ok(Some(admin_usage_bad_request_response(detail))),
            };
            let time_range = match AdminStatsTimeRange::resolve_optional(query) {
                Ok(value) => value,
                Err(detail) => return Ok(Some(admin_usage_bad_request_response(detail))),
            };

            let mut usage = list_usage_for_optional_range(
                state,
                time_range.as_ref(),
                &AdminStatsUsageFilter::default(),
            )
            .await?;
            usage.retain(|item| item.status != "pending" && item.status != "streaming");

            let response = match group_by.as_str() {
                "model" => admin_usage_aggregation_by_model_json(&usage, limit),
                "user" => admin_usage_aggregation_by_user_json(state, &usage, limit).await?,
                "provider" => admin_usage_aggregation_by_provider_json(&usage, limit),
                "api_format" => admin_usage_aggregation_by_api_format_json(&usage, limit),
                _ => unreachable!(),
            };
            return Ok(Some(Json(response).into_response()));
        }
        Some("heatmap")
            if request_context.request_method == http::Method::GET
                && matches!(
                    request_context.request_path.as_str(),
                    "/api/admin/usage/heatmap" | "/api/admin/usage/heatmap/"
                ) =>
        {
            if !state.has_usage_data_reader() {
                return Ok(Some(admin_usage_data_unavailable_response(
                    ADMIN_USAGE_DATA_UNAVAILABLE_DETAIL,
                )));
            }
            let now_unix_secs = u64::try_from(chrono::Utc::now().timestamp()).unwrap_or_default();
            let created_from_unix_secs = now_unix_secs.saturating_sub(365 * 24 * 3600);
            let mut usage = state
                .list_usage_audits(&aether_data::repository::usage::UsageAuditListQuery {
                    created_from_unix_secs: Some(created_from_unix_secs),
                    ..Default::default()
                })
                .await?;
            usage.retain(|item| item.status != "pending" && item.status != "streaming");
            return Ok(Some(Json(admin_usage_heatmap_json(&usage)).into_response()));
        }
        Some("cache_affinity_hit_analysis")
            if request_context.request_method == http::Method::GET
                && matches!(
                    request_context.request_path.as_str(),
                    "/api/admin/usage/cache-affinity/hit-analysis"
                        | "/api/admin/usage/cache-affinity/hit-analysis/"
                ) =>
        {
            if !state.has_usage_data_reader() {
                return Ok(Some(admin_usage_data_unavailable_response(
                    ADMIN_USAGE_DATA_UNAVAILABLE_DETAIL,
                )));
            }

            let query = request_context.request_query_string.as_deref();
            let hours = match admin_usage_parse_recent_hours(query, 168) {
                Ok(value) => value,
                Err(detail) => return Ok(Some(admin_usage_bad_request_response(detail))),
            };
            let user_id = query_param_value(query, "user_id");
            let api_key_id = query_param_value(query, "api_key_id");
            let usage =
                list_recent_completed_usage_for_cache_affinity(state, hours, user_id.as_deref())
                    .await?;
            let filtered: Vec<_> = usage
                .into_iter()
                .filter(|item| {
                    admin_usage_matches_optional_id(item.user_id.as_deref(), user_id.as_deref())
                        && admin_usage_matches_optional_id(
                            item.api_key_id.as_deref(),
                            api_key_id.as_deref(),
                        )
                })
                .collect();
            let total_requests = filtered.len();
            let total_input_tokens: u64 = filtered.iter().map(|item| item.input_tokens).sum();
            let total_cache_read_tokens: u64 = filtered
                .iter()
                .map(|item| item.cache_read_input_tokens)
                .sum();
            let total_cache_creation_tokens: u64 = filtered
                .iter()
                .map(|item| item.cache_creation_input_tokens)
                .sum();
            let total_cache_read_cost: f64 =
                filtered.iter().map(|item| item.cache_read_cost_usd).sum();
            let total_cache_creation_cost: f64 = filtered
                .iter()
                .map(|item| item.cache_creation_cost_usd)
                .sum();
            let requests_with_cache_hit = filtered
                .iter()
                .filter(|item| item.cache_read_input_tokens > 0)
                .count();
            let total_context_tokens = total_input_tokens.saturating_add(total_cache_read_tokens);
            let token_cache_hit_rate = if total_context_tokens == 0 {
                0.0
            } else {
                round_to(
                    total_cache_read_tokens as f64 / total_context_tokens as f64 * 100.0,
                    2,
                )
            };
            let request_cache_hit_rate = if total_requests == 0 {
                0.0
            } else {
                round_to(
                    requests_with_cache_hit as f64 / total_requests as f64 * 100.0,
                    2,
                )
            };

            return Ok(Some(
                Json(json!({
                    "analysis_period_hours": hours,
                    "total_requests": total_requests,
                    "requests_with_cache_hit": requests_with_cache_hit,
                    "request_cache_hit_rate": request_cache_hit_rate,
                    "total_input_tokens": total_input_tokens,
                    "total_cache_read_tokens": total_cache_read_tokens,
                    "total_cache_creation_tokens": total_cache_creation_tokens,
                    "token_cache_hit_rate": token_cache_hit_rate,
                    "total_cache_read_cost_usd": round_to(total_cache_read_cost, 4),
                    "total_cache_creation_cost_usd": round_to(total_cache_creation_cost, 4),
                    "estimated_savings_usd": round_to(total_cache_read_cost * 9.0, 4),
                }))
                .into_response(),
            ));
        }
        Some("cache_affinity_interval_timeline")
            if request_context.request_method == http::Method::GET
                && matches!(
                    request_context.request_path.as_str(),
                    "/api/admin/usage/cache-affinity/interval-timeline"
                        | "/api/admin/usage/cache-affinity/interval-timeline/"
                ) =>
        {
            if !state.has_usage_data_reader() {
                return Ok(Some(admin_usage_data_unavailable_response(
                    ADMIN_USAGE_DATA_UNAVAILABLE_DETAIL,
                )));
            }

            let query = request_context.request_query_string.as_deref();
            let hours = match admin_usage_parse_recent_hours(query, 24) {
                Ok(value) => value,
                Err(detail) => return Ok(Some(admin_usage_bad_request_response(detail))),
            };
            let limit = match admin_usage_parse_timeline_limit(query) {
                Ok(value) => value,
                Err(detail) => return Ok(Some(admin_usage_bad_request_response(detail))),
            };
            let user_id = query_param_value(query, "user_id");
            let include_user_info = query_param_bool(query, "include_user_info", false);
            let usage =
                list_recent_completed_usage_for_cache_affinity(state, hours, user_id.as_deref())
                    .await?;
            let mut grouped: BTreeMap<String, Vec<serde_json::Value>> = BTreeMap::new();
            let mut models = BTreeSet::new();
            let mut usernames_by_user_id = BTreeMap::new();

            for (group_user_id, items) in admin_usage_group_completed_by_user(&usage) {
                if let Some(ref requested_user_id) = user_id {
                    if &group_user_id != requested_user_id {
                        continue;
                    }
                }

                let mut previous_created_at_unix_secs = None;
                for item in items {
                    if let Some(previous) = previous_created_at_unix_secs {
                        let interval_minutes =
                            item.created_at_unix_secs.saturating_sub(previous) as f64 / 60.0;
                        if interval_minutes <= 120.0 {
                            let mut point = json!({
                                "x": unix_secs_to_rfc3339(item.created_at_unix_secs),
                                "y": round_to(interval_minutes, 2),
                            });
                            if !item.model.trim().is_empty() {
                                point["model"] = json!(item.model.clone());
                                models.insert(item.model.clone());
                            }
                            if include_user_info && user_id.is_none() {
                                point["user_id"] = json!(group_user_id.clone());
                                if let Some(username) = item.username.clone() {
                                    usernames_by_user_id
                                        .entry(group_user_id.clone())
                                        .or_insert(username);
                                }
                            }
                            grouped
                                .entry(group_user_id.clone())
                                .or_default()
                                .push(point);
                        }
                    }
                    previous_created_at_unix_secs = Some(item.created_at_unix_secs);
                }
            }

            if include_user_info && user_id.is_none() && state.has_user_data_reader() {
                let user_ids: Vec<_> = grouped.keys().cloned().collect();
                let user_map: BTreeMap<_, _> = state
                    .list_users_by_ids(&user_ids)
                    .await?
                    .into_iter()
                    .map(|user| (user.id, user.username))
                    .collect();
                for (user_id, username) in user_map {
                    usernames_by_user_id.insert(user_id, username);
                }
            }

            let total_points_before_limit: usize = grouped.values().map(Vec::len).sum();
            let points: Vec<serde_json::Value> = if include_user_info && user_id.is_none() {
                let user_limits =
                    admin_usage_proportional_limits(&grouped, limit, total_points_before_limit);
                let mut selected = Vec::new();
                for (group_user_id, mut items) in grouped {
                    let take = user_limits
                        .get(&group_user_id)
                        .copied()
                        .unwrap_or(items.len());
                    selected.extend(items.drain(..std::cmp::min(take, items.len())));
                }
                selected.sort_by(admin_usage_point_sort_key);
                selected
            } else {
                let mut selected = grouped
                    .into_values()
                    .flatten()
                    .collect::<Vec<serde_json::Value>>();
                selected.sort_by(admin_usage_point_sort_key);
                if selected.len() > limit {
                    selected.truncate(limit);
                }
                selected
            };

            let mut response = json!({
                "analysis_period_hours": hours,
                "total_points": points.len(),
                "points": points,
            });
            if include_user_info && user_id.is_none() {
                response["users"] = json!(usernames_by_user_id);
            }
            if !models.is_empty() {
                response["models"] = json!(models.into_iter().collect::<Vec<_>>());
            }

            return Ok(Some(Json(response).into_response()));
        }
        Some("cache_affinity_ttl_analysis")
            if request_context.request_method == http::Method::GET
                && matches!(
                    request_context.request_path.as_str(),
                    "/api/admin/usage/cache-affinity/ttl-analysis"
                        | "/api/admin/usage/cache-affinity/ttl-analysis/"
                ) =>
        {
            if !state.has_usage_data_reader() {
                return Ok(Some(admin_usage_data_unavailable_response(
                    ADMIN_USAGE_DATA_UNAVAILABLE_DETAIL,
                )));
            }

            let query = request_context.request_query_string.as_deref();
            let hours = match admin_usage_parse_recent_hours(query, 168) {
                Ok(value) => value,
                Err(detail) => return Ok(Some(admin_usage_bad_request_response(detail))),
            };
            let user_id = query_param_value(query, "user_id");
            let api_key_id = query_param_value(query, "api_key_id");
            let group_by_api_key = api_key_id.is_some();
            let usage =
                list_recent_completed_usage_for_cache_affinity(state, hours, user_id.as_deref())
                    .await?;

            let grouped = if group_by_api_key {
                admin_usage_group_completed_by_api_key(&usage, api_key_id.as_deref())
            } else {
                admin_usage_group_completed_by_user(&usage)
                    .into_iter()
                    .filter(|(group_user_id, _)| {
                        admin_usage_matches_optional_id(
                            Some(group_user_id.as_str()),
                            user_id.as_deref(),
                        )
                    })
                    .collect()
            };

            let user_map: BTreeMap<String, aether_data::repository::users::StoredUserSummary> =
                if !group_by_api_key && state.has_user_data_reader() {
                    let user_ids = grouped.keys().cloned().collect::<Vec<_>>();
                    state
                        .list_users_by_ids(&user_ids)
                        .await?
                        .into_iter()
                        .map(|user| (user.id.clone(), user))
                        .collect()
                } else {
                    BTreeMap::new()
                };

            let mut ttl_distribution = json!({
                "5min": 0_u64,
                "15min": 0_u64,
                "30min": 0_u64,
                "60min": 0_u64,
            });
            let mut users = Vec::new();

            for (group_id, items) in grouped {
                let intervals = admin_usage_collect_request_intervals_minutes(&items);
                if intervals.len() < 2 {
                    continue;
                }

                let within_5min = intervals.iter().filter(|value| **value <= 5.0).count() as u64;
                let within_15min = intervals
                    .iter()
                    .filter(|value| **value > 5.0 && **value <= 15.0)
                    .count() as u64;
                let within_30min = intervals
                    .iter()
                    .filter(|value| **value > 15.0 && **value <= 30.0)
                    .count() as u64;
                let within_60min = intervals
                    .iter()
                    .filter(|value| **value > 30.0 && **value <= 60.0)
                    .count() as u64;
                let over_60min = intervals.iter().filter(|value| **value > 60.0).count() as u64;
                let request_count = intervals.len() as u64;
                let p50 = admin_usage_percentile_cont(&intervals, 0.5);
                let p75 = admin_usage_percentile_cont(&intervals, 0.75);
                let p90 = admin_usage_percentile_cont(&intervals, 0.90);
                let avg_interval = intervals.iter().copied().sum::<f64>() / intervals.len() as f64;
                let min_interval = intervals.iter().copied().reduce(f64::min);
                let max_interval = intervals.iter().copied().reduce(f64::max);
                let recommended_ttl = admin_usage_calculate_recommended_ttl(p75, p90);
                match recommended_ttl {
                    0..=5 => {
                        ttl_distribution["5min"] = json!(ttl_distribution["5min"]
                            .as_u64()
                            .unwrap_or(0)
                            .saturating_add(1))
                    }
                    6..=15 => {
                        ttl_distribution["15min"] = json!(ttl_distribution["15min"]
                            .as_u64()
                            .unwrap_or(0)
                            .saturating_add(1))
                    }
                    16..=30 => {
                        ttl_distribution["30min"] = json!(ttl_distribution["30min"]
                            .as_u64()
                            .unwrap_or(0)
                            .saturating_add(1))
                    }
                    _ => {
                        ttl_distribution["60min"] = json!(ttl_distribution["60min"]
                            .as_u64()
                            .unwrap_or(0)
                            .saturating_add(1))
                    }
                }

                let (username, email) = if group_by_api_key {
                    (Value::Null, Value::Null)
                } else if let Some(user) = user_map.get(&group_id) {
                    (
                        json!(user.username.clone()),
                        json!(user.email.clone().unwrap_or_default()),
                    )
                } else {
                    (Value::Null, Value::Null)
                };

                users.push(json!({
                    "group_id": group_id,
                    "username": username,
                    "email": email,
                    "request_count": request_count,
                    "interval_distribution": {
                        "within_5min": within_5min,
                        "within_15min": within_15min,
                        "within_30min": within_30min,
                        "within_60min": within_60min,
                        "over_60min": over_60min,
                    },
                    "interval_percentages": {
                        "within_5min": round_to(within_5min as f64 / request_count as f64 * 100.0, 1),
                        "within_15min": round_to(within_15min as f64 / request_count as f64 * 100.0, 1),
                        "within_30min": round_to(within_30min as f64 / request_count as f64 * 100.0, 1),
                        "within_60min": round_to(within_60min as f64 / request_count as f64 * 100.0, 1),
                        "over_60min": round_to(over_60min as f64 / request_count as f64 * 100.0, 1),
                    },
                    "percentiles": {
                        "p50": p50.map(|value| round_to(value, 2)),
                        "p75": p75.map(|value| round_to(value, 2)),
                        "p90": p90.map(|value| round_to(value, 2)),
                    },
                    "avg_interval_minutes": round_to(avg_interval, 2),
                    "min_interval_minutes": min_interval.map(|value| round_to(value, 2)),
                    "max_interval_minutes": max_interval.map(|value| round_to(value, 2)),
                    "recommended_ttl_minutes": recommended_ttl,
                    "recommendation_reason": admin_usage_ttl_recommendation_reason(recommended_ttl, p75, p90),
                }));
            }

            users.sort_by(|left, right| {
                right["request_count"]
                    .as_u64()
                    .unwrap_or(0)
                    .cmp(&left["request_count"].as_u64().unwrap_or(0))
                    .then_with(|| {
                        left["group_id"]
                            .as_str()
                            .unwrap_or_default()
                            .cmp(right["group_id"].as_str().unwrap_or_default())
                    })
            });

            return Ok(Some(
                Json(json!({
                    "analysis_period_hours": hours,
                    "total_users_analyzed": users.len(),
                    "ttl_distribution": ttl_distribution,
                    "users": users,
                }))
                .into_response(),
            ));
        }
        _ => {}
    }

    Ok(None)
}
