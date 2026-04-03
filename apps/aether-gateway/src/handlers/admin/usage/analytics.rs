use super::super::{parse_bounded_u32, round_to};
use crate::gateway::handlers::{query_param_value, unix_secs_to_rfc3339};
use crate::gateway::{AppState, GatewayError};
use serde_json::{json, Value};
use std::collections::{BTreeMap, BTreeSet};

pub(super) fn admin_usage_total_tokens(
    item: &aether_data::repository::usage::StoredRequestUsageAudit,
) -> u64 {
    item.input_tokens
        .saturating_add(item.output_tokens)
        .saturating_add(item.cache_creation_input_tokens)
        .saturating_add(item.cache_read_input_tokens)
}

pub(super) fn admin_usage_parse_limit(query: Option<&str>) -> Result<usize, String> {
    match query_param_value(query, "limit") {
        None => Ok(100),
        Some(value) => {
            let parsed = value
                .parse::<usize>()
                .map_err(|_| "limit must be a positive integer".to_string())?;
            if parsed == 0 || parsed > 500 {
                return Err("limit must be between 1 and 500".to_string());
            }
            Ok(parsed)
        }
    }
}

pub(super) fn admin_usage_parse_offset(query: Option<&str>) -> Result<usize, String> {
    match query_param_value(query, "offset") {
        None => Ok(0),
        Some(value) => value
            .parse::<usize>()
            .map_err(|_| "offset must be a non-negative integer".to_string()),
    }
}

pub(super) fn admin_usage_parse_ids(query: Option<&str>) -> Option<BTreeSet<String>> {
    let ids = query_param_value(query, "ids")?;
    let parsed: BTreeSet<String> = ids
        .split(',')
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(ToOwned::to_owned)
        .collect();
    Some(parsed)
}

pub(super) fn admin_usage_parse_recent_hours(
    query: Option<&str>,
    default: u32,
) -> Result<u32, String> {
    match query_param_value(query, "hours") {
        Some(value) => parse_bounded_u32("hours", &value, 1, 720),
        None => Ok(default),
    }
}

pub(super) fn admin_usage_parse_timeline_limit(query: Option<&str>) -> Result<usize, String> {
    match query_param_value(query, "limit") {
        Some(value) => {
            let parsed = value
                .parse::<usize>()
                .map_err(|_| "limit must be an integer between 100 and 50000".to_string())?;
            if (100..=50_000).contains(&parsed) {
                Ok(parsed)
            } else {
                Err("limit must be an integer between 100 and 50000".to_string())
            }
        }
        None => Ok(3_000),
    }
}

pub(super) fn admin_usage_parse_aggregation_limit(query: Option<&str>) -> Result<usize, String> {
    match query_param_value(query, "limit") {
        Some(value) => {
            let parsed = value
                .parse::<usize>()
                .map_err(|_| "limit must be an integer between 1 and 100".to_string())?;
            if (1..=100).contains(&parsed) {
                Ok(parsed)
            } else {
                Err("limit must be an integer between 1 and 100".to_string())
            }
        }
        None => Ok(20),
    }
}

pub(super) fn admin_usage_token_cache_hit_rate(input_tokens: u64, cache_read_tokens: u64) -> f64 {
    let total_input_context = input_tokens.saturating_add(cache_read_tokens);
    if total_input_context == 0 {
        0.0
    } else {
        round_to(
            cache_read_tokens as f64 / total_input_context as f64 * 100.0,
            2,
        )
    }
}

pub(super) fn admin_usage_aggregation_by_model_json(
    usage: &[aether_data::repository::usage::StoredRequestUsageAudit],
    limit: usize,
) -> serde_json::Value {
    let mut grouped: BTreeMap<String, (u64, u64, u64, u64, f64, f64)> = BTreeMap::new();
    for item in usage {
        let key = item.model.clone();
        let entry = grouped.entry(key).or_insert((0, 0, 0, 0, 0.0, 0.0));
        entry.0 = entry.0.saturating_add(1);
        entry.1 = entry.1.saturating_add(item.total_tokens);
        entry.2 = entry.2.saturating_add(item.input_tokens);
        entry.3 = entry.3.saturating_add(item.cache_read_input_tokens);
        entry.4 += item.total_cost_usd;
        entry.5 += item.actual_total_cost_usd;
    }

    let mut items: Vec<serde_json::Value> = grouped
        .into_iter()
        .map(
            |(model, (request_count, total_tokens, input_tokens, cache_read_tokens, total_cost, actual_cost))| {
                json!({
                    "model": model,
                    "request_count": request_count,
                    "total_tokens": total_tokens,
                    "total_input_context": input_tokens.saturating_add(cache_read_tokens),
                    "output_tokens": total_tokens.saturating_sub(input_tokens),
                    "total_cost": round_to(total_cost, 6),
                    "actual_cost": round_to(actual_cost, 6),
                    "cache_read_tokens": cache_read_tokens,
                    "cache_creation_tokens": 0,
                    "cache_hit_rate": admin_usage_token_cache_hit_rate(input_tokens, cache_read_tokens),
                })
            },
        )
        .collect();
    items.sort_by(|left, right| {
        right["request_count"]
            .as_u64()
            .unwrap_or_default()
            .cmp(&left["request_count"].as_u64().unwrap_or_default())
            .then_with(|| {
                left["model"]
                    .as_str()
                    .unwrap_or_default()
                    .cmp(right["model"].as_str().unwrap_or_default())
            })
    });
    items.truncate(limit);
    json!(items)
}

pub(super) async fn admin_usage_aggregation_by_user_json(
    state: &AppState,
    usage: &[aether_data::repository::usage::StoredRequestUsageAudit],
    limit: usize,
) -> Result<serde_json::Value, GatewayError> {
    let mut grouped: BTreeMap<String, (u64, u64, f64)> = BTreeMap::new();
    for item in usage {
        let Some(user_id) = item.user_id.as_ref() else {
            continue;
        };
        let entry = grouped.entry(user_id.clone()).or_insert((0, 0, 0.0));
        entry.0 = entry.0.saturating_add(1);
        entry.1 = entry.1.saturating_add(item.total_tokens);
        entry.2 += item.total_cost_usd;
    }

    let usernames = if state.has_user_data_reader() && !grouped.is_empty() {
        state
            .list_users_by_ids(&grouped.keys().cloned().collect::<Vec<_>>())
            .await?
            .into_iter()
            .map(|user| (user.id, (user.email, user.username)))
            .collect::<BTreeMap<_, _>>()
    } else {
        BTreeMap::new()
    };

    let mut items: Vec<serde_json::Value> = grouped
        .into_iter()
        .map(|(user_id, (request_count, total_tokens, total_cost))| {
            let (email, username) = usernames
                .get(&user_id)
                .cloned()
                .unwrap_or((None, String::new()));
            json!({
                "user_id": user_id,
                "email": email,
                "username": if username.is_empty() { serde_json::Value::Null } else { json!(username) },
                "request_count": request_count,
                "total_tokens": total_tokens,
                "total_cost": round_to(total_cost, 6),
            })
        })
        .collect();
    items.sort_by(|left, right| {
        right["request_count"]
            .as_u64()
            .unwrap_or_default()
            .cmp(&left["request_count"].as_u64().unwrap_or_default())
            .then_with(|| {
                left["user_id"]
                    .as_str()
                    .unwrap_or_default()
                    .cmp(right["user_id"].as_str().unwrap_or_default())
            })
    });
    items.truncate(limit);
    Ok(json!(items))
}

pub(super) fn admin_usage_aggregation_by_provider_json(
    usage: &[aether_data::repository::usage::StoredRequestUsageAudit],
    limit: usize,
) -> serde_json::Value {
    let mut grouped: BTreeMap<String, (u64, u64, u64, u64, f64, f64, u64, u64)> = BTreeMap::new();
    for item in usage {
        let key = item
            .provider_id
            .clone()
            .unwrap_or_else(|| "unknown".to_string());
        let entry = grouped.entry(key).or_insert((0, 0, 0, 0, 0.0, 0.0, 0, 0));
        entry.0 = entry.0.saturating_add(1);
        entry.1 = entry.1.saturating_add(item.total_tokens);
        entry.2 = entry.2.saturating_add(item.input_tokens);
        entry.3 = entry.3.saturating_add(item.cache_read_input_tokens);
        entry.4 += item.total_cost_usd;
        entry.5 += item.actual_total_cost_usd;
        entry.6 = entry
            .6
            .saturating_add(item.response_time_ms.unwrap_or_default());
        entry.7 = entry
            .7
            .saturating_add(if admin_usage_is_success(item) { 1 } else { 0 });
    }

    let mut items: Vec<serde_json::Value> = grouped
        .into_iter()
        .map(
            |(provider_id, (request_count, total_tokens, input_tokens, cache_read_tokens, total_cost, actual_cost, response_time_ms_sum, success_count))| {
                let avg_response_time_ms = if request_count == 0 {
                    0.0
                } else {
                    round_to(response_time_ms_sum as f64 / request_count as f64, 2)
                };
                let error_count = request_count.saturating_sub(success_count);
                let success_rate = if request_count == 0 {
                    0.0
                } else {
                    round_to(success_count as f64 / request_count as f64 * 100.0, 2)
                };
                json!({
                    "provider_id": provider_id,
                    "provider": serde_json::Value::Null,
                    "request_count": request_count,
                    "total_tokens": total_tokens,
                    "total_input_context": input_tokens.saturating_add(cache_read_tokens),
                    "output_tokens": total_tokens.saturating_sub(input_tokens),
                    "total_cost": round_to(total_cost, 6),
                    "actual_cost": round_to(actual_cost, 6),
                    "avg_response_time_ms": avg_response_time_ms,
                    "success_rate": success_rate,
                    "error_count": error_count,
                    "cache_read_tokens": cache_read_tokens,
                    "cache_creation_tokens": 0,
                    "cache_hit_rate": admin_usage_token_cache_hit_rate(input_tokens, cache_read_tokens),
                })
            },
        )
        .collect();
    items.sort_by(|left, right| {
        right["request_count"]
            .as_u64()
            .unwrap_or_default()
            .cmp(&left["request_count"].as_u64().unwrap_or_default())
            .then_with(|| {
                left["provider_id"]
                    .as_str()
                    .unwrap_or_default()
                    .cmp(right["provider_id"].as_str().unwrap_or_default())
            })
    });
    items.truncate(limit);
    json!(items)
}

pub(super) fn admin_usage_aggregation_by_api_format_json(
    usage: &[aether_data::repository::usage::StoredRequestUsageAudit],
    limit: usize,
) -> serde_json::Value {
    let mut grouped: BTreeMap<String, (u64, u64, u64, u64, f64, f64, u64)> = BTreeMap::new();
    for item in usage {
        let key = item
            .api_format
            .clone()
            .unwrap_or_else(|| "unknown".to_string());
        let entry = grouped.entry(key).or_insert((0, 0, 0, 0, 0.0, 0.0, 0));
        entry.0 = entry.0.saturating_add(1);
        entry.1 = entry.1.saturating_add(item.total_tokens);
        entry.2 = entry.2.saturating_add(item.input_tokens);
        entry.3 = entry.3.saturating_add(item.cache_read_input_tokens);
        entry.4 += item.total_cost_usd;
        entry.5 += item.actual_total_cost_usd;
        entry.6 = entry
            .6
            .saturating_add(item.response_time_ms.unwrap_or_default());
    }

    let mut items: Vec<serde_json::Value> = grouped
        .into_iter()
        .map(
            |(api_format, (request_count, total_tokens, input_tokens, cache_read_tokens, total_cost, actual_cost, response_time_ms_sum))| {
                let avg_response_time_ms = if request_count == 0 {
                    0.0
                } else {
                    round_to(response_time_ms_sum as f64 / request_count as f64, 2)
                };
                json!({
                    "api_format": api_format,
                    "request_count": request_count,
                    "total_tokens": total_tokens,
                    "total_input_context": input_tokens.saturating_add(cache_read_tokens),
                    "output_tokens": total_tokens.saturating_sub(input_tokens),
                    "total_cost": round_to(total_cost, 6),
                    "actual_cost": round_to(actual_cost, 6),
                    "avg_response_time_ms": avg_response_time_ms,
                    "cache_read_tokens": cache_read_tokens,
                    "cache_creation_tokens": 0,
                    "cache_hit_rate": admin_usage_token_cache_hit_rate(input_tokens, cache_read_tokens),
                })
            },
        )
        .collect();
    items.sort_by(|left, right| {
        right["request_count"]
            .as_u64()
            .unwrap_or_default()
            .cmp(&left["request_count"].as_u64().unwrap_or_default())
            .then_with(|| {
                left["api_format"]
                    .as_str()
                    .unwrap_or_default()
                    .cmp(right["api_format"].as_str().unwrap_or_default())
            })
    });
    items.truncate(limit);
    json!(items)
}

pub(super) fn admin_usage_heatmap_json(
    usage: &[aether_data::repository::usage::StoredRequestUsageAudit],
) -> serde_json::Value {
    let mut grouped: BTreeMap<String, (u64, u64, f64, f64, u64, u64)> = BTreeMap::new();
    for item in usage {
        let Ok(created_at_unix_secs) = i64::try_from(item.created_at_unix_secs) else {
            continue;
        };
        let Some(created_at) =
            chrono::DateTime::<chrono::Utc>::from_timestamp(created_at_unix_secs, 0)
        else {
            continue;
        };
        let date_key = created_at.format("%Y-%m-%d").to_string();
        let entry = grouped.entry(date_key).or_insert((0, 0, 0.0, 0.0, 0, 0));
        entry.0 = entry.0.saturating_add(1);
        entry.1 = entry.1.saturating_add(item.total_tokens);
        entry.2 += item.total_cost_usd;
        entry.3 += item.actual_total_cost_usd;
        entry.4 = entry.4.saturating_add(item.cache_read_input_tokens);
        entry.5 = entry.5.saturating_add(item.cache_creation_input_tokens);
    }
    let items = grouped
        .into_iter()
        .map(
            |(
                date,
                (
                    request_count,
                    total_tokens,
                    total_cost,
                    actual_total_cost,
                    cache_read_tokens,
                    cache_creation_tokens,
                ),
            )| {
                json!({
                    "date": date,
                    "request_count": request_count,
                    "total_tokens": total_tokens,
                    "total_cost": round_to(total_cost, 6),
                    "actual_total_cost": round_to(actual_total_cost, 6),
                    "cache_read_tokens": cache_read_tokens,
                    "cache_creation_tokens": cache_creation_tokens,
                })
            },
        )
        .collect::<Vec<_>>();
    json!(items)
}

pub(super) fn admin_usage_is_success(
    item: &aether_data::repository::usage::StoredRequestUsageAudit,
) -> bool {
    matches!(
        item.status.as_str(),
        "completed" | "success" | "ok" | "billed" | "settled"
    ) && item.status_code.is_none_or(|code| code < 400)
}

pub(super) fn admin_usage_matches_optional_id(value: Option<&str>, expected: Option<&str>) -> bool {
    let Some(expected) = expected.map(str::trim).filter(|value| !value.is_empty()) else {
        return true;
    };
    value.is_some_and(|candidate| candidate == expected)
}

pub(super) async fn list_recent_completed_usage_for_cache_affinity(
    state: &AppState,
    hours: u32,
    user_id: Option<&str>,
) -> Result<Vec<aether_data::repository::usage::StoredRequestUsageAudit>, GatewayError> {
    let now_unix_secs = u64::try_from(chrono::Utc::now().timestamp()).unwrap_or_default();
    let created_from_unix_secs = now_unix_secs.saturating_sub(u64::from(hours) * 3600);
    let mut items = state
        .list_usage_audits(&aether_data::repository::usage::UsageAuditListQuery {
            created_from_unix_secs: Some(created_from_unix_secs),
            created_until_unix_secs: None,
            user_id: user_id.map(ToOwned::to_owned),
            provider_name: None,
            model: None,
        })
        .await?;
    items.retain(|item| item.status == "completed");
    items.sort_by(|left, right| {
        left.created_at_unix_secs
            .cmp(&right.created_at_unix_secs)
            .then_with(|| left.id.cmp(&right.id))
    });
    Ok(items)
}

pub(super) fn admin_usage_group_completed_by_user(
    items: &[aether_data::repository::usage::StoredRequestUsageAudit],
) -> BTreeMap<String, Vec<aether_data::repository::usage::StoredRequestUsageAudit>> {
    let mut grouped = BTreeMap::new();
    for item in items.iter().filter(|item| item.user_id.is_some()) {
        grouped
            .entry(item.user_id.clone().unwrap_or_default())
            .or_insert_with(Vec::new)
            .push(item.clone());
    }
    grouped
}

pub(super) fn admin_usage_group_completed_by_api_key(
    items: &[aether_data::repository::usage::StoredRequestUsageAudit],
    api_key_id: Option<&str>,
) -> BTreeMap<String, Vec<aether_data::repository::usage::StoredRequestUsageAudit>> {
    let mut grouped = BTreeMap::new();
    for item in items.iter().filter(|item| item.api_key_id.is_some()) {
        if !admin_usage_matches_optional_id(item.api_key_id.as_deref(), api_key_id) {
            continue;
        }
        grouped
            .entry(item.api_key_id.clone().unwrap_or_default())
            .or_insert_with(Vec::new)
            .push(item.clone());
    }
    grouped
}

pub(super) fn admin_usage_collect_request_intervals_minutes(
    items: &[aether_data::repository::usage::StoredRequestUsageAudit],
) -> Vec<f64> {
    let mut previous_created_at_unix_secs = None;
    let mut intervals = Vec::new();
    for item in items {
        if let Some(previous) = previous_created_at_unix_secs {
            intervals.push(item.created_at_unix_secs.saturating_sub(previous) as f64 / 60.0);
        }
        previous_created_at_unix_secs = Some(item.created_at_unix_secs);
    }
    intervals
}

pub(super) fn admin_usage_percentile_cont(values: &[f64], percentile: f64) -> Option<f64> {
    if values.is_empty() {
        return None;
    }
    if values.len() == 1 {
        return Some(values[0]);
    }
    let position = percentile.clamp(0.0, 1.0) * (values.len() - 1) as f64;
    let lower_index = position.floor() as usize;
    let upper_index = position.ceil() as usize;
    let lower = values[lower_index];
    let upper = values[upper_index];
    Some(lower + (upper - lower) * (position - lower_index as f64))
}

pub(super) fn admin_usage_calculate_recommended_ttl(
    p75_interval: Option<f64>,
    p90_interval: Option<f64>,
) -> u64 {
    let Some(p75_interval) = p75_interval else {
        return 5;
    };
    let Some(p90_interval) = p90_interval else {
        return 5;
    };

    if p90_interval <= 5.0 {
        5
    } else if p75_interval <= 15.0 {
        15
    } else if p75_interval <= 30.0 {
        30
    } else {
        60
    }
}

pub(super) fn admin_usage_ttl_recommendation_reason(
    ttl: u64,
    p75_interval: Option<f64>,
    p90_interval: Option<f64>,
) -> String {
    let Some(p75_interval) = p75_interval else {
        return "数据不足，使用默认值".to_string();
    };
    let Some(p90_interval) = p90_interval else {
        return "数据不足，使用默认值".to_string();
    };

    match ttl {
        5 => format!("高频用户：90% 的请求间隔在 {:.1} 分钟内", p90_interval),
        15 => format!("中高频用户：75% 的请求间隔在 {:.1} 分钟内", p75_interval),
        30 => format!("中频用户：75% 的请求间隔在 {:.1} 分钟内", p75_interval),
        _ => format!(
            "低频用户：75% 的请求间隔为 {:.1} 分钟，建议使用长 TTL",
            p75_interval
        ),
    }
}

pub(super) fn admin_usage_proportional_limits(
    grouped: &BTreeMap<String, Vec<serde_json::Value>>,
    limit: usize,
    total_points: usize,
) -> BTreeMap<String, usize> {
    let mut limits = BTreeMap::new();
    for (group_id, items) in grouped {
        let computed = if total_points <= limit || total_points == 0 {
            items.len()
        } else {
            let scaled =
                ((items.len() as f64 * limit as f64) / total_points as f64).ceil() as usize;
            std::cmp::max(scaled, 1)
        };
        limits.insert(group_id.clone(), computed);
    }
    limits
}

pub(super) fn admin_usage_point_sort_key(
    left: &serde_json::Value,
    right: &serde_json::Value,
) -> std::cmp::Ordering {
    left["x"]
        .as_str()
        .unwrap_or_default()
        .cmp(right["x"].as_str().unwrap_or_default())
        .then_with(|| {
            left["user_id"]
                .as_str()
                .unwrap_or_default()
                .cmp(right["user_id"].as_str().unwrap_or_default())
        })
}

pub(super) fn admin_usage_matches_search(
    item: &aether_data::repository::usage::StoredRequestUsageAudit,
    search: Option<&str>,
) -> bool {
    let Some(search) = search.map(str::trim).filter(|value| !value.is_empty()) else {
        return true;
    };
    let haystack = [
        item.username.as_deref(),
        item.api_key_name.as_deref(),
        Some(item.model.as_str()),
        Some(item.provider_name.as_str()),
    ];
    search.split_whitespace().all(|keyword| {
        let keyword = keyword.to_ascii_lowercase();
        haystack
            .iter()
            .flatten()
            .any(|value| value.to_ascii_lowercase().contains(keyword.as_str()))
    })
}

pub(super) fn admin_usage_matches_username(
    item: &aether_data::repository::usage::StoredRequestUsageAudit,
    username: Option<&str>,
) -> bool {
    let Some(username) = username.map(str::trim).filter(|value| !value.is_empty()) else {
        return true;
    };
    item.username
        .as_deref()
        .unwrap_or_default()
        .to_ascii_lowercase()
        .contains(username.to_ascii_lowercase().as_str())
}

pub(super) fn admin_usage_matches_eq(value: &str, query: Option<&str>) -> bool {
    let Some(query) = query
        .map(str::trim)
        .filter(|candidate| !candidate.is_empty())
    else {
        return true;
    };
    value.eq_ignore_ascii_case(query)
}

pub(super) fn admin_usage_matches_api_format(
    item: &aether_data::repository::usage::StoredRequestUsageAudit,
    api_format: Option<&str>,
) -> bool {
    let Some(api_format) = api_format.map(str::trim).filter(|value| !value.is_empty()) else {
        return true;
    };
    item.api_format
        .as_deref()
        .is_some_and(|value| value.eq_ignore_ascii_case(api_format))
}

pub(super) fn admin_usage_matches_status(
    item: &aether_data::repository::usage::StoredRequestUsageAudit,
    status: Option<&str>,
) -> bool {
    let Some(status) = status.map(str::trim).filter(|value| !value.is_empty()) else {
        return true;
    };
    match status {
        "stream" => item.is_stream,
        "standard" => !item.is_stream,
        "error" => {
            item.status_code.is_some_and(|value| value >= 400) || item.error_message.is_some()
        }
        "pending" | "streaming" | "completed" | "cancelled" => item.status == status,
        "failed" => {
            item.status == "failed"
                || item.status_code.is_some_and(|value| value >= 400)
                || item.error_message.is_some()
        }
        "active" => matches!(item.status.as_str(), "pending" | "streaming"),
        _ => true,
    }
}

pub(super) fn admin_usage_record_json(
    item: &aether_data::repository::usage::StoredRequestUsageAudit,
    users_by_id: &BTreeMap<String, aether_data::repository::users::StoredUserSummary>,
) -> Value {
    let user = item
        .user_id
        .as_ref()
        .and_then(|user_id| users_by_id.get(user_id));
    let username = user
        .map(|value| value.username.clone())
        .or_else(|| item.username.clone())
        .unwrap_or_else(|| "已删除用户".to_string());
    let user_email = user
        .and_then(|value| value.email.clone())
        .unwrap_or_else(|| "已删除用户".to_string());

    json!({
        "id": item.id,
        "user_id": item.user_id,
        "user_email": user_email,
        "username": username,
        "api_key": item.api_key_id.as_ref().map(|api_key_id| json!({
            "id": api_key_id,
            "name": item.api_key_name.clone(),
            "display": item.api_key_name.clone().unwrap_or_else(|| api_key_id.clone()),
        })),
        "provider": item.provider_name,
        "model": item.model,
        "target_model": item.target_model,
        "input_tokens": item.input_tokens,
        "output_tokens": item.output_tokens,
        "cache_creation_input_tokens": item.cache_creation_input_tokens,
        "cache_read_input_tokens": item.cache_read_input_tokens,
        "total_tokens": admin_usage_total_tokens(item),
        "cost": round_to(item.total_cost_usd, 6),
        "actual_cost": round_to(item.actual_total_cost_usd, 6),
        "rate_multiplier": Value::Null,
        "response_time_ms": item.response_time_ms,
        "first_byte_time_ms": item.first_byte_time_ms,
        "created_at": unix_secs_to_rfc3339(item.created_at_unix_secs),
        "is_stream": item.is_stream,
        "input_price_per_1m": Value::Null,
        "output_price_per_1m": item.output_price_per_1m,
        "cache_creation_price_per_1m": Value::Null,
        "cache_read_price_per_1m": Value::Null,
        "status_code": item.status_code,
        "error_message": item.error_message,
        "status": item.status,
        "has_fallback": false,
        "has_retry": false,
        "has_rectified": false,
        "api_format": item.api_format,
        "endpoint_api_format": item.endpoint_api_format,
        "has_format_conversion": item.has_format_conversion,
        "api_key_name": item.api_key_name,
        "model_version": Value::Null,
    })
}
