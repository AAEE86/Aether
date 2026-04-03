use super::admin_stats_provider_quota_usage_empty_response;
use crate::gateway::handlers::unix_secs_to_rfc3339;
use crate::gateway::{AppState, GatewayControlDecision, GatewayError, GatewayPublicRequestContext};
use axum::{
    body::Body,
    http,
    response::{IntoResponse, Response},
    Json,
};
use chrono::{Datelike, Utc};
use serde_json::json;

pub(super) async fn maybe_build_local_admin_stats_provider_quota_response(
    state: &AppState,
    request_context: &GatewayPublicRequestContext,
    decision: &GatewayControlDecision,
) -> Result<Option<Response<Body>>, GatewayError> {
    if decision.route_kind.as_deref() != Some("provider_quota_usage")
        || request_context.request_method != http::Method::GET
        || !matches!(
            request_context.request_path.as_str(),
            "/api/admin/stats/providers/quota-usage" | "/api/admin/stats/providers/quota-usage/"
        )
    {
        return Ok(None);
    }

    if !state.has_provider_catalog_data_reader() {
        return Ok(Some(admin_stats_provider_quota_usage_empty_response()));
    }

    let now = Utc::now();
    let now_unix_secs = now.timestamp().max(0) as u64;
    let now_day = u64::from(now.day());
    let mut providers = state.list_provider_catalog_providers(false).await?;
    providers.retain(|provider| {
        provider.billing_type.as_deref() == Some("monthly_quota")
            || provider.monthly_quota_usd.is_some()
    });

    let mut payload: Vec<serde_json::Value> = providers
        .into_iter()
        .map(|provider| {
            let quota = provider.monthly_quota_usd.unwrap_or(0.0);
            let used = provider.monthly_used_usd.unwrap_or(0.0);
            let remaining = (quota - used).max(0.0);
            let usage_percent = if quota > 0.0 {
                ((used / quota) * 10_000.0).round() / 100.0
            } else {
                0.0
            };

            let days_elapsed = provider
                .quota_last_reset_at_unix_secs
                .map(|reset_at| ((now_unix_secs.saturating_sub(reset_at)) / 86_400).max(1))
                .unwrap_or_else(|| now_day.saturating_sub(1).max(1));

            let daily_rate = if used > 0.0 {
                used / days_elapsed as f64
            } else {
                0.0
            };
            let estimated_exhaust_at_unix_secs = if daily_rate > 0.0 && remaining > 0.0 {
                let estimated = now_unix_secs
                    .saturating_add(((remaining / daily_rate) * 86_400.0).max(0.0) as u64);
                Some(
                    provider
                        .quota_expires_at_unix_secs
                        .map(|quota_expires_at| quota_expires_at.min(estimated))
                        .unwrap_or(estimated),
                )
            } else {
                provider.quota_expires_at_unix_secs
            };

            json!({
                "id": provider.id,
                "name": provider.name,
                "quota_usd": quota,
                "used_usd": used,
                "remaining_usd": remaining,
                "usage_percent": usage_percent,
                "quota_expires_at": provider.quota_expires_at_unix_secs.and_then(unix_secs_to_rfc3339),
                "estimated_exhaust_at": estimated_exhaust_at_unix_secs.and_then(unix_secs_to_rfc3339),
            })
        })
        .collect();

    payload.sort_by(|left, right| {
        let left_value = left
            .get("usage_percent")
            .and_then(serde_json::Value::as_f64)
            .unwrap_or(0.0);
        let right_value = right
            .get("usage_percent")
            .and_then(serde_json::Value::as_f64)
            .unwrap_or(0.0);
        right_value.total_cmp(&left_value)
    });

    Ok(Some(Json(json!({ "providers": payload })).into_response()))
}
