use super::{AdminStatsComparisonType, AdminStatsTimeRange, AdminStatsUsageFilter};
use crate::gateway::handlers::query_param_value;
use crate::gateway::{AppState, GatewayError};
use chrono::{Datelike, Utc};

pub(super) fn parse_tz_offset_minutes(query: Option<&str>) -> Result<i32, String> {
    query_param_value(query, "tz_offset_minutes")
        .map(|value| {
            value
                .parse::<i32>()
                .map_err(|_| "tz_offset_minutes must be a valid integer".to_string())
        })
        .transpose()
        .map(|value| value.unwrap_or(0))
}

pub(super) fn parse_naive_date(field: &str, value: &str) -> Result<chrono::NaiveDate, String> {
    chrono::NaiveDate::parse_from_str(value, "%Y-%m-%d")
        .map_err(|_| format!("{field} must be a valid date in YYYY-MM-DD format"))
}

pub(crate) fn parse_bounded_u32(
    field: &str,
    value: &str,
    min: u32,
    max: u32,
) -> Result<u32, String> {
    let parsed = value
        .parse::<u32>()
        .map_err(|_| format!("{field} must be a valid integer"))?;
    if parsed < min || parsed > max {
        return Err(format!("{field} must be between {min} and {max}"));
    }
    Ok(parsed)
}

pub(super) fn parse_nonnegative_usize(field: &str, value: &str) -> Result<usize, String> {
    value
        .parse::<usize>()
        .map_err(|_| format!("{field} must be a valid integer"))
}

pub(super) fn admin_usage_default_days() -> usize {
    match std::env::var("ADMIN_USAGE_DEFAULT_DAYS") {
        Ok(value) => value.parse::<usize>().ok().unwrap_or(0),
        Err(_) => match std::env::var("ENVIRONMENT") {
            Ok(value) if !matches!(value.as_str(), "development" | "test" | "testing") => 30,
            _ => 0,
        },
    }
}

pub(super) fn user_today(tz_offset_minutes: i32) -> chrono::NaiveDate {
    (Utc::now() + chrono::Duration::minutes(i64::from(tz_offset_minutes))).date_naive()
}

pub(super) fn resolve_preset_dates(
    preset: &str,
    tz_offset_minutes: i32,
) -> Result<(chrono::NaiveDate, chrono::NaiveDate), String> {
    let user_today = user_today(tz_offset_minutes);
    match preset {
        "today" => Ok((user_today, user_today)),
        "yesterday" => {
            let value = user_today
                .checked_sub_signed(chrono::Duration::days(1))
                .unwrap_or(user_today);
            Ok((value, value))
        }
        "last7days" => Ok((
            user_today
                .checked_sub_signed(chrono::Duration::days(6))
                .unwrap_or(user_today),
            user_today,
        )),
        "last30days" => Ok((
            user_today
                .checked_sub_signed(chrono::Duration::days(29))
                .unwrap_or(user_today),
            user_today,
        )),
        "last90days" => Ok((
            user_today
                .checked_sub_signed(chrono::Duration::days(89))
                .unwrap_or(user_today),
            user_today,
        )),
        "this_week" => {
            let week_start = user_today
                .checked_sub_signed(chrono::Duration::days(i64::from(
                    user_today.weekday().num_days_from_monday(),
                )))
                .unwrap_or(user_today);
            Ok((week_start, user_today))
        }
        "last_week" => {
            let this_week_start = user_today
                .checked_sub_signed(chrono::Duration::days(i64::from(
                    user_today.weekday().num_days_from_monday(),
                )))
                .unwrap_or(user_today);
            let last_week_end = this_week_start
                .checked_sub_signed(chrono::Duration::days(1))
                .unwrap_or(this_week_start);
            let last_week_start = last_week_end
                .checked_sub_signed(chrono::Duration::days(6))
                .unwrap_or(last_week_end);
            Ok((last_week_start, last_week_end))
        }
        "this_month" => {
            let start = user_today.with_day(1).unwrap_or(user_today);
            Ok((start, user_today))
        }
        "last_month" => {
            let first_of_this_month = user_today.with_day(1).unwrap_or(user_today);
            let last_month_end = first_of_this_month
                .checked_sub_signed(chrono::Duration::days(1))
                .unwrap_or(first_of_this_month);
            let last_month_start = last_month_end.with_day(1).unwrap_or(last_month_end);
            Ok((last_month_start, last_month_end))
        }
        "this_year" => {
            let start =
                chrono::NaiveDate::from_ymd_opt(user_today.year(), 1, 1).unwrap_or(user_today);
            Ok((start, user_today))
        }
        _ => Err("Invalid preset".to_string()),
    }
}

pub(super) fn build_time_range_from_days(
    days: u32,
    tz_offset_minutes: i32,
) -> Result<AdminStatsTimeRange, String> {
    let end_date = user_today(tz_offset_minutes);
    let start_date = end_date
        .checked_sub_signed(chrono::Duration::days(i64::from(days.saturating_sub(1))))
        .unwrap_or(end_date);
    Ok(AdminStatsTimeRange {
        start_date,
        end_date,
        tz_offset_minutes,
    })
}

pub(super) fn build_comparison_range(
    current: &AdminStatsTimeRange,
    comparison_type: AdminStatsComparisonType,
) -> Result<AdminStatsTimeRange, String> {
    let comparison = match comparison_type {
        AdminStatsComparisonType::Period => {
            let days = (current.end_date - current.start_date).num_days() + 1;
            let comparison_end = current
                .start_date
                .checked_sub_signed(chrono::Duration::days(1))
                .ok_or_else(|| "comparison range underflow".to_string())?;
            let comparison_start = comparison_end
                .checked_sub_signed(chrono::Duration::days(days - 1))
                .ok_or_else(|| "comparison range underflow".to_string())?;
            (comparison_start, comparison_end)
        }
        AdminStatsComparisonType::Year => (
            safe_year_shift(current.start_date),
            safe_year_shift(current.end_date),
        ),
    };

    Ok(AdminStatsTimeRange {
        start_date: comparison.0,
        end_date: comparison.1,
        tz_offset_minutes: current.tz_offset_minutes,
    })
}

fn safe_year_shift(value: chrono::NaiveDate) -> chrono::NaiveDate {
    value
        .with_year(value.year() - 1)
        .or_else(|| chrono::NaiveDate::from_ymd_opt(value.year() - 1, value.month(), 28))
        .unwrap_or(value)
}

pub(super) async fn list_usage_for_range(
    state: &AppState,
    time_range: &AdminStatsTimeRange,
    filters: &AdminStatsUsageFilter,
) -> Result<Vec<aether_data::repository::usage::StoredRequestUsageAudit>, GatewayError> {
    let Some((created_from_unix_secs, created_until_unix_secs)) = time_range.to_unix_bounds()
    else {
        return Ok(Vec::new());
    };

    state
        .list_usage_audits(&aether_data::repository::usage::UsageAuditListQuery {
            created_from_unix_secs: Some(created_from_unix_secs),
            created_until_unix_secs: Some(created_until_unix_secs),
            user_id: filters.user_id.clone(),
            provider_name: filters.provider_name.clone(),
            model: filters.model.clone(),
        })
        .await
}

pub(crate) async fn list_usage_for_optional_range(
    state: &AppState,
    time_range: Option<&AdminStatsTimeRange>,
    filters: &AdminStatsUsageFilter,
) -> Result<Vec<aether_data::repository::usage::StoredRequestUsageAudit>, GatewayError> {
    match time_range {
        Some(time_range) => list_usage_for_range(state, time_range, filters).await,
        None => {
            state
                .list_usage_audits(&aether_data::repository::usage::UsageAuditListQuery {
                    created_from_unix_secs: None,
                    created_until_unix_secs: None,
                    user_id: filters.user_id.clone(),
                    provider_name: filters.provider_name.clone(),
                    model: filters.model.clone(),
                })
                .await
        }
    }
}
