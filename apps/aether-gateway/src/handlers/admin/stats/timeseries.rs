use super::{
    round_to, AdminStatsAggregate, AdminStatsGranularity, AdminStatsTimeRange,
    AdminStatsTimeSeriesBucket, MIN_PERCENTILE_SAMPLES,
};
use chrono::{Datelike, Utc};
use serde_json::json;

pub(super) fn build_time_series_payload(
    time_range: &AdminStatsTimeRange,
    granularity: AdminStatsGranularity,
    items: &[aether_data::repository::usage::StoredRequestUsageAudit],
) -> Vec<serde_json::Value> {
    match granularity {
        AdminStatsGranularity::Hour => build_hourly_time_series_payload(time_range, items),
        AdminStatsGranularity::Day => build_daily_time_series_payload(time_range, items),
        AdminStatsGranularity::Week => build_weekly_time_series_payload(time_range, items),
        AdminStatsGranularity::Month => build_monthly_time_series_payload(time_range, items),
    }
}

pub(super) fn build_daily_time_series_buckets(
    time_range: &AdminStatsTimeRange,
    items: &[aether_data::repository::usage::StoredRequestUsageAudit],
) -> std::collections::BTreeMap<chrono::NaiveDate, AdminStatsTimeSeriesBucket> {
    let mut buckets: std::collections::BTreeMap<chrono::NaiveDate, AdminStatsTimeSeriesBucket> =
        time_range
            .local_dates()
            .into_iter()
            .map(|date| (date, AdminStatsTimeSeriesBucket::default()))
            .collect();

    for item in items {
        let Some(local_day) = time_range.local_date_for_unix_secs(item.created_at_unix_secs) else {
            continue;
        };
        let Some(bucket) = buckets.get_mut(&local_day) else {
            continue;
        };
        bucket.add_usage(item);
    }

    buckets
}

fn build_daily_time_series_payload(
    time_range: &AdminStatsTimeRange,
    items: &[aether_data::repository::usage::StoredRequestUsageAudit],
) -> Vec<serde_json::Value> {
    build_daily_time_series_buckets(time_range, items)
        .into_iter()
        .map(|(date, bucket)| bucket.to_json_with_avg(date.to_string()))
        .collect()
}

fn build_weekly_time_series_payload(
    time_range: &AdminStatsTimeRange,
    items: &[aether_data::repository::usage::StoredRequestUsageAudit],
) -> Vec<serde_json::Value> {
    let mut weekly: std::collections::BTreeMap<
        (i32, u32),
        (chrono::NaiveDate, AdminStatsTimeSeriesBucket),
    > = std::collections::BTreeMap::new();

    for (date, bucket) in build_daily_time_series_buckets(time_range, items) {
        let iso = date.iso_week();
        let entry = weekly
            .entry((iso.year(), iso.week()))
            .or_insert_with(|| (date, AdminStatsTimeSeriesBucket::default()));
        entry.0 = entry.0.min(date);
        entry.1.merge(&bucket);
    }

    weekly
        .into_values()
        .map(|(date, bucket)| bucket.to_json_with_avg(date.to_string()))
        .collect()
}

fn build_monthly_time_series_payload(
    time_range: &AdminStatsTimeRange,
    items: &[aether_data::repository::usage::StoredRequestUsageAudit],
) -> Vec<serde_json::Value> {
    let mut monthly: std::collections::BTreeMap<
        (i32, u32),
        (chrono::NaiveDate, AdminStatsTimeSeriesBucket),
    > = std::collections::BTreeMap::new();

    for (date, bucket) in build_daily_time_series_buckets(time_range, items) {
        let Some(month_start) = chrono::NaiveDate::from_ymd_opt(date.year(), date.month(), 1)
        else {
            continue;
        };
        let entry = monthly
            .entry((date.year(), date.month()))
            .or_insert_with(|| (month_start, AdminStatsTimeSeriesBucket::default()));
        entry.1.merge(&bucket);
    }

    monthly
        .into_values()
        .map(|(date, bucket)| bucket.to_json_with_avg(date.to_string()))
        .collect()
}

fn build_hourly_time_series_payload(
    time_range: &AdminStatsTimeRange,
    items: &[aether_data::repository::usage::StoredRequestUsageAudit],
) -> Vec<serde_json::Value> {
    let Some((mut current, end)) = time_range.to_utc_datetime_bounds() else {
        return Vec::new();
    };
    let offset = chrono::Duration::minutes(i64::from(time_range.tz_offset_minutes));
    let mut buckets: std::collections::BTreeMap<String, AdminStatsTimeSeriesBucket> =
        std::collections::BTreeMap::new();

    while current < end {
        let label = (current + offset)
            .format("%Y-%m-%dT%H:00:00+00:00")
            .to_string();
        buckets.insert(label, AdminStatsTimeSeriesBucket::default());
        let Some(next) = current.checked_add_signed(chrono::Duration::hours(1)) else {
            break;
        };
        current = next;
    }

    for item in items {
        let Some(unix_secs) = i64::try_from(item.created_at_unix_secs).ok() else {
            continue;
        };
        let Some(timestamp) = chrono::DateTime::<Utc>::from_timestamp(unix_secs, 0) else {
            continue;
        };
        let Some(local) = timestamp.checked_add_signed(offset) else {
            continue;
        };
        let label = local.format("%Y-%m-%dT%H:00:00+00:00").to_string();
        let Some(bucket) = buckets.get_mut(&label) else {
            continue;
        };
        bucket.add_usage(item);
    }

    buckets
        .into_iter()
        .map(|(date, bucket)| bucket.to_json_without_avg(date))
        .collect()
}

pub(crate) fn aggregate_usage_stats(
    items: &[aether_data::repository::usage::StoredRequestUsageAudit],
) -> AdminStatsAggregate {
    let mut aggregate = AdminStatsAggregate::default();
    for item in items {
        aggregate.total_requests = aggregate.total_requests.saturating_add(1);
        aggregate.total_tokens = aggregate.total_tokens.saturating_add(item.total_tokens);
        aggregate.total_cost += item.total_cost_usd;
        aggregate.actual_total_cost += item.actual_total_cost_usd;
        aggregate.total_response_time_ms += item.response_time_ms.unwrap_or(0) as f64;
        if item.status_code.is_some_and(|value| value >= 400) || item.error_message.is_some() {
            aggregate.error_requests = aggregate.error_requests.saturating_add(1);
        }
    }
    aggregate
}

pub(super) fn percentile_cont(values: &mut [u64], percentile: f64) -> Option<u64> {
    if values.len() < MIN_PERCENTILE_SAMPLES {
        return None;
    }
    values.sort_unstable();

    let position = percentile * (values.len().saturating_sub(1)) as f64;
    let lower = position.floor() as usize;
    let upper = position.ceil() as usize;
    let lower_value = values[lower] as f64;
    let upper_value = values[upper] as f64;
    Some((lower_value + (upper_value - lower_value) * (position - lower as f64)).trunc() as u64)
}

pub(super) fn pct_change_value(current: f64, previous: f64) -> serde_json::Value {
    if previous == 0.0 {
        if current == 0.0 {
            json!(0.0)
        } else {
            serde_json::Value::Null
        }
    } else {
        json!(round_to((current - previous) / previous * 100.0, 2))
    }
}

pub(super) fn linear_regression(values: &[f64]) -> (f64, f64) {
    let n = values.len();
    if n <= 1 {
        return (0.0, values.first().copied().unwrap_or(0.0));
    }
    let sum_x: f64 = (0..n).map(|value| value as f64).sum();
    let sum_y: f64 = values.iter().sum();
    let sum_x2: f64 = (0..n).map(|value| (value * value) as f64).sum();
    let sum_xy: f64 = values
        .iter()
        .enumerate()
        .map(|(index, value)| index as f64 * *value)
        .sum();
    let n = n as f64;
    let denom = n * sum_x2 - sum_x * sum_x;
    if denom == 0.0 {
        return (0.0, values.last().copied().unwrap_or(0.0));
    }
    let slope = (n * sum_xy - sum_x * sum_y) / denom;
    let intercept = (sum_y - slope * sum_x) / n;
    (slope, intercept)
}
