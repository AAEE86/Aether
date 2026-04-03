use super::{
    admin_stats_bad_request_response, admin_stats_comparison_empty_response,
    admin_stats_error_distribution_empty_response,
    admin_stats_performance_percentiles_empty_response, admin_stats_time_series_empty_response,
    aggregate_usage_stats, build_comparison_range, build_time_series_payload, list_usage_for_range,
    pct_change_value, percentile_cont, round_to, AdminStatsComparisonType, AdminStatsGranularity,
    AdminStatsTimeRange, AdminStatsUsageFilter,
};
use crate::gateway::handlers::query_param_value;
use crate::gateway::{AppState, GatewayControlDecision, GatewayError, GatewayPublicRequestContext};
use axum::{
    body::Body,
    http,
    response::{IntoResponse, Response},
    Json,
};
use serde_json::json;

pub(super) async fn maybe_build_local_admin_stats_analytics_response(
    state: &AppState,
    request_context: &GatewayPublicRequestContext,
    decision: &GatewayControlDecision,
) -> Result<Option<Response<Body>>, GatewayError> {
    if decision.route_kind.as_deref() == Some("comparison")
        && request_context.request_method == http::Method::GET
        && matches!(
            request_context.request_path.as_str(),
            "/api/admin/stats/comparison" | "/api/admin/stats/comparison/"
        )
    {
        let current_range = match AdminStatsTimeRange::resolve_required(
            request_context.request_query_string.as_deref(),
            "current_start",
            "current_end",
        ) {
            Ok(value) => value,
            Err(detail) => return Ok(Some(admin_stats_bad_request_response(detail))),
        };
        let comparison_type = match query_param_value(
            request_context.request_query_string.as_deref(),
            "comparison_type",
        )
        .as_deref()
        {
            None | Some("period") => AdminStatsComparisonType::Period,
            Some("year") => AdminStatsComparisonType::Year,
            Some(_) => {
                return Ok(Some(admin_stats_bad_request_response(
                    "comparison_type must be 'period' or 'year'".to_string(),
                )));
            }
        };

        let comparison_range = match build_comparison_range(&current_range, comparison_type) {
            Ok(value) => value,
            Err(detail) => return Ok(Some(admin_stats_bad_request_response(detail))),
        };
        if !state.has_usage_data_reader() {
            return Ok(Some(admin_stats_comparison_empty_response(
                &current_range,
                &comparison_range,
            )));
        }
        let current_usage =
            list_usage_for_range(state, &current_range, &AdminStatsUsageFilter::default()).await?;
        let comparison_usage =
            list_usage_for_range(state, &comparison_range, &AdminStatsUsageFilter::default())
                .await?;
        let current = aggregate_usage_stats(&current_usage);
        let comparison = aggregate_usage_stats(&comparison_usage);

        return Ok(Some(
            Json(json!({
                "current": {
                    "total_requests": current.total_requests,
                    "total_tokens": current.total_tokens,
                    "total_cost": round_to(current.total_cost, 6),
                    "actual_total_cost": round_to(current.actual_total_cost, 6),
                    "avg_response_time_ms": round_to(current.avg_response_time_ms(), 2),
                    "error_requests": current.error_requests,
                },
                "comparison": {
                    "total_requests": comparison.total_requests,
                    "total_tokens": comparison.total_tokens,
                    "total_cost": round_to(comparison.total_cost, 6),
                    "actual_total_cost": round_to(comparison.actual_total_cost, 6),
                    "avg_response_time_ms": round_to(comparison.avg_response_time_ms(), 2),
                    "error_requests": comparison.error_requests,
                },
                "change_percent": {
                    "total_requests": pct_change_value(current.total_requests as f64, comparison.total_requests as f64),
                    "total_tokens": pct_change_value(current.total_tokens as f64, comparison.total_tokens as f64),
                    "total_cost": pct_change_value(current.total_cost, comparison.total_cost),
                    "actual_total_cost": pct_change_value(current.actual_total_cost, comparison.actual_total_cost),
                    "avg_response_time_ms": pct_change_value(current.avg_response_time_ms(), comparison.avg_response_time_ms()),
                    "error_requests": pct_change_value(current.error_requests as f64, comparison.error_requests as f64),
                },
                "current_start": current_range.start_date.to_string(),
                "current_end": current_range.end_date.to_string(),
                "comparison_start": comparison_range.start_date.to_string(),
                "comparison_end": comparison_range.end_date.to_string(),
            }))
            .into_response(),
        ));
    }

    if decision.route_kind.as_deref() == Some("error_distribution")
        && request_context.request_method == http::Method::GET
        && matches!(
            request_context.request_path.as_str(),
            "/api/admin/stats/errors/distribution" | "/api/admin/stats/errors/distribution/"
        )
    {
        let Some(time_range) = (match AdminStatsTimeRange::resolve_optional(
            request_context.request_query_string.as_deref(),
        ) {
            Ok(value) => value,
            Err(detail) => return Ok(Some(admin_stats_bad_request_response(detail))),
        }) else {
            return Ok(Some(
                Json(json!({
                    "distribution": [],
                    "trend": [],
                }))
                .into_response(),
            ));
        };
        if !state.has_usage_data_reader() {
            return Ok(Some(admin_stats_error_distribution_empty_response()));
        }

        let usage =
            list_usage_for_range(state, &time_range, &AdminStatsUsageFilter::default()).await?;
        let mut distribution: std::collections::BTreeMap<String, u64> =
            std::collections::BTreeMap::new();
        let mut trend: std::collections::BTreeMap<String, std::collections::BTreeMap<String, u64>> =
            std::collections::BTreeMap::new();

        for item in usage {
            let Some(category) = item
                .error_category
                .as_ref()
                .filter(|value| !value.trim().is_empty())
            else {
                continue;
            };
            let Some(local_day) =
                time_range.local_date_string_for_unix_secs(item.created_at_unix_secs)
            else {
                continue;
            };
            *distribution.entry(category.clone()).or_default() += 1;
            *trend
                .entry(local_day)
                .or_default()
                .entry(category.clone())
                .or_default() += 1;
        }

        let mut distribution_items: Vec<_> = distribution
            .into_iter()
            .map(|(category, count)| json!({ "category": category, "count": count }))
            .collect();
        distribution_items.sort_by(|left, right| {
            let left_count = left
                .get("count")
                .and_then(serde_json::Value::as_u64)
                .unwrap_or(0);
            let right_count = right
                .get("count")
                .and_then(serde_json::Value::as_u64)
                .unwrap_or(0);
            right_count.cmp(&left_count).then_with(|| {
                left.get("category")
                    .and_then(serde_json::Value::as_str)
                    .cmp(&right.get("category").and_then(serde_json::Value::as_str))
            })
        });

        let trend_items: Vec<_> = trend
            .into_iter()
            .map(|(date, categories)| {
                let total: u64 = categories.values().copied().sum();
                json!({
                    "date": date,
                    "total": total,
                    "categories": categories,
                })
            })
            .collect();

        return Ok(Some(
            Json(json!({
                "distribution": distribution_items,
                "trend": trend_items,
            }))
            .into_response(),
        ));
    }

    if decision.route_kind.as_deref() == Some("performance_percentiles")
        && request_context.request_method == http::Method::GET
        && matches!(
            request_context.request_path.as_str(),
            "/api/admin/stats/performance/percentiles"
                | "/api/admin/stats/performance/percentiles/"
        )
    {
        let Some(time_range) = (match AdminStatsTimeRange::resolve_optional(
            request_context.request_query_string.as_deref(),
        ) {
            Ok(value) => value,
            Err(detail) => return Ok(Some(admin_stats_bad_request_response(detail))),
        }) else {
            return Ok(Some(Json(json!([])).into_response()));
        };
        if !state.has_usage_data_reader() {
            return Ok(Some(admin_stats_performance_percentiles_empty_response()));
        }

        let usage =
            list_usage_for_range(state, &time_range, &AdminStatsUsageFilter::default()).await?;
        let mut by_day: std::collections::BTreeMap<String, (Vec<u64>, Vec<u64>)> = time_range
            .local_date_strings()
            .into_iter()
            .map(|date| (date, (Vec::new(), Vec::new())))
            .collect();

        for item in usage {
            if item.status != "completed" {
                continue;
            }
            let Some(local_day) =
                time_range.local_date_string_for_unix_secs(item.created_at_unix_secs)
            else {
                continue;
            };
            let Some((response_times, first_byte_times)) = by_day.get_mut(&local_day) else {
                continue;
            };
            if let Some(response_time_ms) = item.response_time_ms {
                response_times.push(response_time_ms);
            }
            if let Some(first_byte_time_ms) = item.first_byte_time_ms {
                first_byte_times.push(first_byte_time_ms);
            }
        }

        let payload: Vec<_> = by_day
            .into_iter()
            .map(|(date, (mut response_times, mut first_byte_times))| {
                json!({
                    "date": date,
                    "p50_response_time_ms": percentile_cont(&mut response_times, 0.5),
                    "p90_response_time_ms": percentile_cont(&mut response_times, 0.9),
                    "p99_response_time_ms": percentile_cont(&mut response_times, 0.99),
                    "p50_first_byte_time_ms": percentile_cont(&mut first_byte_times, 0.5),
                    "p90_first_byte_time_ms": percentile_cont(&mut first_byte_times, 0.9),
                    "p99_first_byte_time_ms": percentile_cont(&mut first_byte_times, 0.99),
                })
            })
            .collect();

        return Ok(Some(
            Json(serde_json::Value::Array(payload)).into_response(),
        ));
    }

    if decision.route_kind.as_deref() == Some("time_series")
        && request_context.request_method == http::Method::GET
        && matches!(
            request_context.request_path.as_str(),
            "/api/admin/stats/time-series" | "/api/admin/stats/time-series/"
        )
    {
        let granularity =
            match AdminStatsGranularity::parse(request_context.request_query_string.as_deref()) {
                Ok(value) => value,
                Err(detail) => return Ok(Some(admin_stats_bad_request_response(detail))),
            };
        let Some(time_range) = (match AdminStatsTimeRange::resolve_optional(
            request_context.request_query_string.as_deref(),
        ) {
            Ok(value) => value,
            Err(detail) => return Ok(Some(admin_stats_bad_request_response(detail))),
        }) else {
            return Ok(Some(Json(json!([])).into_response()));
        };
        if let Err(detail) = time_range.validate_for_time_series(granularity) {
            return Ok(Some(admin_stats_bad_request_response(detail)));
        }
        if !state.has_usage_data_reader() {
            return Ok(Some(admin_stats_time_series_empty_response()));
        }

        let filters =
            AdminStatsUsageFilter::from_query(request_context.request_query_string.as_deref());
        let usage = list_usage_for_range(state, &time_range, &filters).await?;
        let payload = build_time_series_payload(&time_range, granularity, &usage);

        return Ok(Some(
            Json(serde_json::Value::Array(payload)).into_response(),
        ));
    }

    Ok(None)
}
