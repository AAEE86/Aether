use crate::gateway::handlers::query_param_value;
use crate::gateway::{AppState, GatewayError, GatewayPublicRequestContext};
use axum::{body::Body, response::Response};
use chrono::Utc;
use serde_json::json;

const MIN_PERCENTILE_SAMPLES: usize = 10;

#[path = "stats/analytics_routes.rs"]
mod analytics_routes;
#[path = "stats/cost_routes.rs"]
mod cost_routes;
#[path = "stats/leaderboard.rs"]
mod leaderboard;
#[path = "stats/leaderboard_routes.rs"]
mod leaderboard_routes;
#[path = "stats/provider_quota_routes.rs"]
mod provider_quota_routes;
#[path = "stats/range.rs"]
mod range;
#[path = "stats/responses.rs"]
mod responses;
#[path = "stats/timeseries.rs"]
mod timeseries;

use self::leaderboard::{
    build_api_key_leaderboard_items, build_model_leaderboard_items, build_user_leaderboard_items,
    compare_leaderboard_items, compute_dense_rank, load_user_leaderboard_metadata,
};
use self::range::{
    admin_usage_default_days, build_comparison_range, build_time_range_from_days,
    list_usage_for_range, parse_naive_date, parse_nonnegative_usize, parse_tz_offset_minutes,
    resolve_preset_dates, user_today,
};
pub(crate) use self::range::{list_usage_for_optional_range, parse_bounded_u32};
pub(crate) use self::responses::admin_stats_bad_request_response;
use self::responses::{
    admin_stats_comparison_empty_response, admin_stats_cost_forecast_empty_response,
    admin_stats_cost_savings_empty_response, admin_stats_error_distribution_empty_response,
    admin_stats_leaderboard_empty_response, admin_stats_performance_percentiles_empty_response,
    admin_stats_provider_quota_usage_empty_response, admin_stats_time_series_empty_response,
};
pub(crate) use self::timeseries::aggregate_usage_stats;
use self::timeseries::{
    build_daily_time_series_buckets, build_time_series_payload, linear_regression,
    pct_change_value, percentile_cont,
};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum AdminStatsComparisonType {
    Period,
    Year,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum AdminStatsGranularity {
    Hour,
    Day,
    Week,
    Month,
}

#[derive(Clone, Debug)]
pub(crate) struct AdminStatsTimeRange {
    pub(crate) start_date: chrono::NaiveDate,
    pub(crate) end_date: chrono::NaiveDate,
    pub(crate) tz_offset_minutes: i32,
}

#[derive(Clone, Debug, Default)]
pub(crate) struct AdminStatsUsageFilter {
    pub(crate) user_id: Option<String>,
    pub(crate) provider_name: Option<String>,
    pub(crate) model: Option<String>,
}

#[derive(Clone, Debug, Default)]
pub(crate) struct AdminStatsAggregate {
    pub(crate) total_requests: u64,
    pub(crate) total_tokens: u64,
    pub(crate) total_cost: f64,
    pub(crate) actual_total_cost: f64,
    pub(crate) total_response_time_ms: f64,
    pub(crate) error_requests: u64,
}

#[derive(Clone, Debug)]
struct AdminStatsForecastPoint {
    date: chrono::NaiveDate,
    total_cost: f64,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum AdminStatsLeaderboardMetric {
    Requests,
    Tokens,
    Cost,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum AdminStatsSortOrder {
    Asc,
    Desc,
}

#[derive(Clone, Debug)]
struct AdminStatsLeaderboardItem {
    id: String,
    name: String,
    requests: u64,
    tokens: u64,
    cost: f64,
}

#[derive(Clone, Debug)]
struct AdminStatsUserMetadata {
    name: String,
    role: String,
    is_active: bool,
    is_deleted: bool,
}

impl AdminStatsAggregate {
    pub(crate) fn avg_response_time_ms(&self) -> f64 {
        if self.total_requests == 0 {
            0.0
        } else {
            self.total_response_time_ms / self.total_requests as f64
        }
    }
}

#[derive(Clone, Debug, Default)]
struct AdminStatsTimeSeriesBucket {
    total_requests: u64,
    input_tokens: u64,
    output_tokens: u64,
    cache_creation_tokens: u64,
    cache_read_tokens: u64,
    total_cost: f64,
    total_response_time_ms: f64,
}

impl AdminStatsGranularity {
    fn parse(query: Option<&str>) -> Result<Self, String> {
        match query_param_value(query, "granularity").as_deref() {
            None | Some("day") => Ok(Self::Day),
            Some("hour") => Ok(Self::Hour),
            Some("week") => Ok(Self::Week),
            Some("month") => Ok(Self::Month),
            Some(_) => Err("granularity must be one of: hour, day, week, month".to_string()),
        }
    }
}

impl AdminStatsLeaderboardMetric {
    fn parse(query: Option<&str>) -> Result<Self, String> {
        match query_param_value(query, "metric").as_deref() {
            None | Some("requests") => Ok(Self::Requests),
            Some("tokens") => Ok(Self::Tokens),
            Some("cost") => Ok(Self::Cost),
            Some(_) => Err("metric must be one of: requests, tokens, cost".to_string()),
        }
    }

    fn as_str(self) -> &'static str {
        match self {
            Self::Requests => "requests",
            Self::Tokens => "tokens",
            Self::Cost => "cost",
        }
    }
}

impl AdminStatsSortOrder {
    fn parse(query: Option<&str>) -> Result<Self, String> {
        match query_param_value(query, "order").as_deref() {
            None | Some("desc") => Ok(Self::Desc),
            Some("asc") => Ok(Self::Asc),
            Some(_) => Err("order must be one of: asc, desc".to_string()),
        }
    }
}

impl AdminStatsUsageFilter {
    fn from_query(query: Option<&str>) -> Self {
        Self {
            user_id: query_param_value(query, "user_id"),
            provider_name: query_param_value(query, "provider_name"),
            model: query_param_value(query, "model"),
        }
    }
}

impl AdminStatsTimeSeriesBucket {
    fn add_usage(&mut self, item: &aether_data::repository::usage::StoredRequestUsageAudit) {
        self.total_requests = self.total_requests.saturating_add(1);
        self.input_tokens = self.input_tokens.saturating_add(item.input_tokens);
        self.output_tokens = self.output_tokens.saturating_add(item.output_tokens);
        self.cache_creation_tokens = self
            .cache_creation_tokens
            .saturating_add(item.cache_creation_input_tokens);
        self.cache_read_tokens = self
            .cache_read_tokens
            .saturating_add(item.cache_read_input_tokens);
        self.total_cost += item.total_cost_usd;
        self.total_response_time_ms += item.response_time_ms.unwrap_or(0) as f64;
    }

    fn merge(&mut self, other: &Self) {
        self.total_requests = self.total_requests.saturating_add(other.total_requests);
        self.input_tokens = self.input_tokens.saturating_add(other.input_tokens);
        self.output_tokens = self.output_tokens.saturating_add(other.output_tokens);
        self.cache_creation_tokens = self
            .cache_creation_tokens
            .saturating_add(other.cache_creation_tokens);
        self.cache_read_tokens = self
            .cache_read_tokens
            .saturating_add(other.cache_read_tokens);
        self.total_cost += other.total_cost;
        self.total_response_time_ms += other.total_response_time_ms;
    }

    fn avg_response_time_ms(&self) -> f64 {
        if self.total_requests == 0 {
            0.0
        } else {
            self.total_response_time_ms / self.total_requests as f64
        }
    }

    fn to_json_with_avg(&self, date: String) -> serde_json::Value {
        json!({
            "date": date,
            "total_requests": self.total_requests,
            "input_tokens": self.input_tokens,
            "output_tokens": self.output_tokens,
            "cache_creation_tokens": self.cache_creation_tokens,
            "cache_read_tokens": self.cache_read_tokens,
            "total_cost": round_to(self.total_cost, 6),
            "avg_response_time_ms": round_to(self.avg_response_time_ms(), 2),
        })
    }

    fn to_json_without_avg(&self, date: String) -> serde_json::Value {
        json!({
            "date": date,
            "total_requests": self.total_requests,
            "input_tokens": self.input_tokens,
            "output_tokens": self.output_tokens,
            "cache_creation_tokens": self.cache_creation_tokens,
            "cache_read_tokens": self.cache_read_tokens,
            "total_cost": round_to(self.total_cost, 6),
        })
    }
}

impl AdminStatsTimeRange {
    pub(crate) fn resolve_optional(query: Option<&str>) -> Result<Option<Self>, String> {
        let tz_offset_minutes = parse_tz_offset_minutes(query)?;
        let start_date = query_param_value(query, "start_date")
            .map(|value| parse_naive_date("start_date", &value))
            .transpose()?;
        let end_date = query_param_value(query, "end_date")
            .map(|value| parse_naive_date("end_date", &value))
            .transpose()?;
        let preset = query_param_value(query, "preset");

        if preset.is_none() && start_date.is_none() && end_date.is_none() {
            let default_days = admin_usage_default_days();
            if default_days == 0 {
                return Ok(None);
            }
            let end_date = user_today(tz_offset_minutes);
            let start_date = end_date
                .checked_sub_signed(chrono::Duration::days(
                    i64::try_from(default_days.saturating_sub(1)).unwrap_or(0),
                ))
                .unwrap_or(end_date);
            return Ok(Some(Self {
                start_date,
                end_date,
                tz_offset_minutes,
            }));
        }

        let (start_date, end_date) = match (preset.as_deref(), start_date, end_date) {
            (Some(preset), None, None) => resolve_preset_dates(preset, tz_offset_minutes)?,
            (None, Some(start_date), Some(end_date)) => (start_date, end_date),
            (Some(_), Some(_), _) | (Some(_), _, Some(_)) => {
                return Err("preset cannot be combined with start_date or end_date".to_string());
            }
            _ => {
                return Err(
                    "Either preset or both start_date and end_date must be provided".to_string(),
                );
            }
        };

        if start_date > end_date {
            return Err("start_date must be <= end_date".to_string());
        }

        let days = (end_date - start_date).num_days();
        if days > 365 {
            return Err("Query range cannot exceed 365 days".to_string());
        }

        Ok(Some(Self {
            start_date,
            end_date,
            tz_offset_minutes,
        }))
    }

    fn resolve_required(
        query: Option<&str>,
        start_key: &str,
        end_key: &str,
    ) -> Result<Self, String> {
        let tz_offset_minutes = parse_tz_offset_minutes(query)?;
        let start_date = query_param_value(query, start_key)
            .ok_or_else(|| format!("{start_key} is required"))
            .and_then(|value| parse_naive_date(start_key, &value))?;
        let end_date = query_param_value(query, end_key)
            .ok_or_else(|| format!("{end_key} is required"))
            .and_then(|value| parse_naive_date(end_key, &value))?;

        if start_date > end_date {
            return Err(format!("{start_key} must be <= {end_key}"));
        }

        Ok(Self {
            start_date,
            end_date,
            tz_offset_minutes,
        })
    }

    fn to_unix_bounds(&self) -> Option<(u64, u64)> {
        let offset = chrono::Duration::minutes(i64::from(self.tz_offset_minutes));
        let start_local = self.start_date.and_hms_opt(0, 0, 0)?;
        let end_local = self
            .end_date
            .checked_add_signed(chrono::Duration::days(1))?
            .and_hms_opt(0, 0, 0)?;
        let start_utc =
            chrono::DateTime::<Utc>::from_naive_utc_and_offset(start_local - offset, Utc)
                .timestamp();
        let end_utc =
            chrono::DateTime::<Utc>::from_naive_utc_and_offset(end_local - offset, Utc).timestamp();
        if start_utc < 0 || end_utc <= 0 {
            return None;
        }
        Some((start_utc as u64, end_utc as u64))
    }

    fn to_utc_datetime_bounds(&self) -> Option<(chrono::DateTime<Utc>, chrono::DateTime<Utc>)> {
        let offset = chrono::Duration::minutes(i64::from(self.tz_offset_minutes));
        let start_local = self.start_date.and_hms_opt(0, 0, 0)?;
        let end_local = self
            .end_date
            .checked_add_signed(chrono::Duration::days(1))?
            .and_hms_opt(0, 0, 0)?;
        Some((
            chrono::DateTime::<Utc>::from_naive_utc_and_offset(start_local - offset, Utc),
            chrono::DateTime::<Utc>::from_naive_utc_and_offset(end_local - offset, Utc),
        ))
    }

    fn validate_for_time_series(&self, granularity: AdminStatsGranularity) -> Result<(), String> {
        if granularity == AdminStatsGranularity::Hour && self.start_date != self.end_date {
            return Err("Hour granularity only supports single day query".to_string());
        }
        let days_inclusive = (self.end_date - self.start_date).num_days() + 1;
        if days_inclusive > 90 {
            return Err(format!(
                "Time series query range cannot exceed 90 days (requested {days_inclusive} days). For longer ranges, use aggregated statistics instead."
            ));
        }
        Ok(())
    }

    fn local_dates(&self) -> Vec<chrono::NaiveDate> {
        let mut current = self.start_date;
        let mut dates = Vec::new();
        while current <= self.end_date {
            dates.push(current);
            let Some(next) = current.checked_add_signed(chrono::Duration::days(1)) else {
                break;
            };
            current = next;
        }
        dates
    }

    fn local_date_strings(&self) -> Vec<String> {
        self.local_dates()
            .into_iter()
            .map(|date| date.to_string())
            .collect()
    }

    fn local_date_for_unix_secs(&self, unix_secs: u64) -> Option<chrono::NaiveDate> {
        let timestamp = chrono::DateTime::<Utc>::from_timestamp(i64::try_from(unix_secs).ok()?, 0)?;
        let local = timestamp
            .checked_add_signed(chrono::Duration::minutes(i64::from(self.tz_offset_minutes)))?;
        Some(local.date_naive())
    }

    fn local_date_string_for_unix_secs(&self, unix_secs: u64) -> Option<String> {
        Some(self.local_date_for_unix_secs(unix_secs)?.to_string())
    }
}

pub(crate) async fn maybe_build_local_admin_stats_response(
    state: &AppState,
    request_context: &GatewayPublicRequestContext,
) -> Result<Option<Response<Body>>, GatewayError> {
    let Some(decision) = request_context.control_decision.as_ref() else {
        return Ok(None);
    };
    if decision.route_family.as_deref() != Some("stats_manage") {
        return Ok(None);
    }

    if let Some(response) =
        provider_quota_routes::maybe_build_local_admin_stats_provider_quota_response(
            state,
            request_context,
            decision,
        )
        .await?
    {
        return Ok(Some(response));
    }

    if let Some(response) = analytics_routes::maybe_build_local_admin_stats_analytics_response(
        state,
        request_context,
        decision,
    )
    .await?
    {
        return Ok(Some(response));
    }

    if let Some(response) =
        cost_routes::maybe_build_local_admin_stats_cost_response(state, request_context).await?
    {
        return Ok(Some(response));
    }

    if let Some(response) = leaderboard_routes::maybe_build_local_admin_stats_leaderboard_response(
        state,
        request_context,
    )
    .await?
    {
        return Ok(Some(response));
    }

    Ok(None)
}

pub(crate) fn round_to(value: f64, decimals: u32) -> f64 {
    let factor = 10_f64.powi(i32::try_from(decimals).unwrap_or(0));
    (value * factor).round() / factor
}
