use super::{
    admin_stats_bad_request_response, admin_stats_cost_forecast_empty_response,
    admin_stats_cost_savings_empty_response, build_daily_time_series_buckets,
    build_time_range_from_days, linear_regression, list_usage_for_range, parse_bounded_u32,
    parse_tz_offset_minutes, round_to, AdminStatsForecastPoint, AdminStatsGranularity,
    AdminStatsTimeRange, AdminStatsUsageFilter,
};
use crate::gateway::handlers::query_param_value;
use crate::gateway::{AppState, GatewayError, GatewayPublicRequestContext};
use axum::{
    body::Body,
    http,
    response::{IntoResponse, Response},
    Json,
};
use serde_json::json;

pub(super) async fn maybe_build_local_admin_stats_cost_response(
    state: &AppState,
    request_context: &GatewayPublicRequestContext,
) -> Result<Option<Response<Body>>, GatewayError> {
    let query = request_context.request_query_string.as_deref();

    if request_context
        .control_decision
        .as_ref()
        .and_then(|decision| decision.route_kind.as_deref())
        == Some("cost_forecast")
        && request_context.request_method == http::Method::GET
        && matches!(
            request_context.request_path.as_str(),
            "/api/admin/stats/cost/forecast" | "/api/admin/stats/cost/forecast/"
        )
    {
        if !state.has_usage_data_reader() {
            return Ok(Some(admin_stats_cost_forecast_empty_response()));
        }

        let forecast_days = match query_param_value(query, "forecast_days")
            .map(|value| parse_bounded_u32("forecast_days", &value, 1, 90))
            .transpose()
        {
            Ok(Some(value)) => value,
            Ok(None) => 7,
            Err(detail) => return Ok(Some(admin_stats_bad_request_response(detail))),
        };
        let tz_offset_minutes = match parse_tz_offset_minutes(query) {
            Ok(value) => value,
            Err(detail) => return Ok(Some(admin_stats_bad_request_response(detail))),
        };
        let time_range = match AdminStatsTimeRange::resolve_optional(query) {
            Ok(Some(value)) => value,
            Ok(None) => {
                let days = match query_param_value(query, "days")
                    .map(|value| parse_bounded_u32("days", &value, 7, 365))
                    .transpose()
                {
                    Ok(Some(value)) => value,
                    Ok(None) => 30,
                    Err(detail) => return Ok(Some(admin_stats_bad_request_response(detail))),
                };
                match build_time_range_from_days(days, tz_offset_minutes) {
                    Ok(value) => value,
                    Err(detail) => return Ok(Some(admin_stats_bad_request_response(detail))),
                }
            }
            Err(detail) => return Ok(Some(admin_stats_bad_request_response(detail))),
        };
        if let Err(detail) = time_range.validate_for_time_series(AdminStatsGranularity::Day) {
            return Ok(Some(admin_stats_bad_request_response(detail)));
        }

        let usage =
            list_usage_for_range(state, &time_range, &AdminStatsUsageFilter::default()).await?;
        let daily = build_daily_time_series_buckets(&time_range, &usage);
        let history: Vec<AdminStatsForecastPoint> = daily
            .into_iter()
            .map(|(date, bucket)| AdminStatsForecastPoint {
                date,
                total_cost: bucket.total_cost,
            })
            .collect();
        let values: Vec<f64> = history.iter().map(|item| item.total_cost).collect();
        let (slope, intercept) = linear_regression(&values);
        let last_date = history
            .last()
            .map(|item| item.date)
            .unwrap_or(time_range.end_date);
        let forecast: Vec<_> = (0..forecast_days)
            .map(|index| {
                let idx = values.len() + index as usize;
                let predicted = (slope * idx as f64 + intercept).max(0.0);
                json!({
                    "date": last_date
                        .checked_add_signed(chrono::Duration::days(i64::from(index + 1)))
                        .unwrap_or(last_date)
                        .to_string(),
                    "total_cost": round_to(predicted, 4),
                })
            })
            .collect();

        return Ok(Some(
            Json(json!({
                "history": history.into_iter().map(|item| json!({
                    "date": item.date.to_string(),
                    "total_cost": round_to(item.total_cost, 6),
                })).collect::<Vec<_>>(),
                "forecast": forecast,
                "slope": round_to(slope, 6),
                "intercept": round_to(intercept, 6),
                "start_date": time_range.start_date.to_string(),
                "end_date": time_range.end_date.to_string(),
            }))
            .into_response(),
        ));
    }

    if request_context
        .control_decision
        .as_ref()
        .and_then(|decision| decision.route_kind.as_deref())
        == Some("cost_savings")
        && request_context.request_method == http::Method::GET
        && matches!(
            request_context.request_path.as_str(),
            "/api/admin/stats/cost/savings" | "/api/admin/stats/cost/savings/"
        )
    {
        let time_range = match AdminStatsTimeRange::resolve_optional(query) {
            Ok(value) => value,
            Err(detail) => return Ok(Some(admin_stats_bad_request_response(detail))),
        };
        if time_range.is_none() {
            return Ok(Some(admin_stats_cost_savings_empty_response()));
        }
        if !state.has_usage_data_reader() {
            return Ok(Some(admin_stats_cost_savings_empty_response()));
        }

        let filters = AdminStatsUsageFilter {
            user_id: None,
            provider_name: query_param_value(query, "provider_name"),
            model: query_param_value(query, "model"),
        };
        let usage = list_usage_for_range(
            state,
            time_range.as_ref().expect("time range exists"),
            &filters,
        )
        .await?;

        let cache_read_tokens: u64 = usage.iter().map(|item| item.cache_read_input_tokens).sum();
        let cache_read_cost: f64 = usage.iter().map(|item| item.cache_read_cost_usd).sum();
        let cache_creation_cost: f64 = usage.iter().map(|item| item.cache_creation_cost_usd).sum();
        let mut estimated_full_cost: f64 = usage
            .iter()
            .map(|item| {
                item.output_price_per_1m.unwrap_or(0.0) * item.cache_read_input_tokens as f64
                    / 1_000_000.0
            })
            .sum();
        if estimated_full_cost <= 0.0 && cache_read_cost > 0.0 {
            estimated_full_cost = cache_read_cost * 10.0;
        }
        let cache_savings = estimated_full_cost - cache_read_cost;

        return Ok(Some(
            Json(json!({
                "cache_read_tokens": cache_read_tokens,
                "cache_read_cost": round_to(cache_read_cost, 6),
                "cache_creation_cost": round_to(cache_creation_cost, 6),
                "estimated_full_cost": round_to(estimated_full_cost, 6),
                "cache_savings": round_to(cache_savings, 6),
            }))
            .into_response(),
        ));
    }

    Ok(None)
}
