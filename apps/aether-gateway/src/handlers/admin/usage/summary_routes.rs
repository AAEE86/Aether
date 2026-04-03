use super::super::{
    aggregate_usage_stats, list_usage_for_optional_range, round_to, AdminStatsTimeRange,
    AdminStatsUsageFilter,
};
use super::{
    admin_usage_bad_request_response, admin_usage_data_unavailable_response,
    admin_usage_matches_api_format, admin_usage_matches_eq, admin_usage_matches_search,
    admin_usage_matches_status, admin_usage_matches_username, admin_usage_parse_ids,
    admin_usage_parse_limit, admin_usage_parse_offset, admin_usage_record_json,
    admin_usage_total_tokens, ADMIN_USAGE_DATA_UNAVAILABLE_DETAIL,
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
use std::collections::{BTreeMap, BTreeSet};

pub(super) async fn maybe_build_local_admin_usage_summary_response(
    state: &AppState,
    request_context: &GatewayPublicRequestContext,
) -> Result<Option<Response<Body>>, GatewayError> {
    let route_kind = request_context
        .control_decision
        .as_ref()
        .and_then(|decision| decision.route_kind.as_deref());

    match route_kind {
        Some("stats")
            if request_context.request_method == http::Method::GET
                && matches!(
                    request_context.request_path.as_str(),
                    "/api/admin/usage/stats" | "/api/admin/usage/stats/"
                ) =>
        {
            if !state.has_usage_data_reader() {
                return Ok(Some(admin_usage_data_unavailable_response(
                    ADMIN_USAGE_DATA_UNAVAILABLE_DETAIL,
                )));
            }

            let query = request_context.request_query_string.as_deref();
            let time_range = match AdminStatsTimeRange::resolve_optional(query) {
                Ok(value) => value,
                Err(detail) => return Ok(Some(admin_usage_bad_request_response(detail))),
            };
            let usage = list_usage_for_optional_range(
                state,
                time_range.as_ref(),
                &AdminStatsUsageFilter::default(),
            )
            .await?;
            let aggregate = aggregate_usage_stats(&usage);
            let cache_creation_tokens: u64 = usage
                .iter()
                .map(|item| item.cache_creation_input_tokens)
                .sum();
            let cache_read_tokens: u64 =
                usage.iter().map(|item| item.cache_read_input_tokens).sum();
            let cache_creation_cost: f64 =
                usage.iter().map(|item| item.cache_creation_cost_usd).sum();
            let cache_read_cost: f64 = usage.iter().map(|item| item.cache_read_cost_usd).sum();
            let total_tokens: u64 = usage.iter().map(admin_usage_total_tokens).sum();
            let avg_response_time = round_to(aggregate.avg_response_time_ms() / 1000.0, 2);
            let error_rate = if aggregate.total_requests == 0 {
                0.0
            } else {
                round_to(
                    (aggregate.error_requests as f64 / aggregate.total_requests as f64) * 100.0,
                    2,
                )
            };

            return Ok(Some(
                Json(json!({
                    "total_requests": aggregate.total_requests,
                    "total_tokens": total_tokens,
                    "total_cost": round_to(aggregate.total_cost, 6),
                    "total_actual_cost": round_to(aggregate.actual_total_cost, 6),
                    "avg_response_time": avg_response_time,
                    "error_count": aggregate.error_requests,
                    "error_rate": error_rate,
                    "cache_stats": {
                        "cache_creation_tokens": cache_creation_tokens,
                        "cache_read_tokens": cache_read_tokens,
                        "cache_creation_cost": round_to(cache_creation_cost, 6),
                        "cache_read_cost": round_to(cache_read_cost, 6),
                    }
                }))
                .into_response(),
            ));
        }
        Some("active")
            if request_context.request_method == http::Method::GET
                && matches!(
                    request_context.request_path.as_str(),
                    "/api/admin/usage/active" | "/api/admin/usage/active/"
                ) =>
        {
            if !state.has_usage_data_reader() {
                return Ok(Some(admin_usage_data_unavailable_response(
                    ADMIN_USAGE_DATA_UNAVAILABLE_DETAIL,
                )));
            }

            let query = request_context.request_query_string.as_deref();
            let requested_ids = admin_usage_parse_ids(query);
            let usage = state
                .list_usage_audits(&aether_data::repository::usage::UsageAuditListQuery::default())
                .await?;
            let mut items: Vec<_> = usage
                .into_iter()
                .filter(|item| match requested_ids.as_ref() {
                    Some(ids) => ids.contains(&item.id),
                    None => matches!(item.status.as_str(), "pending" | "streaming"),
                })
                .collect();
            items.sort_by(|left, right| {
                right
                    .created_at_unix_secs
                    .cmp(&left.created_at_unix_secs)
                    .then_with(|| left.id.cmp(&right.id))
            });
            if requested_ids.is_none() && items.len() > 50 {
                items.truncate(50);
            }

            let payload: Vec<_> = items
                .into_iter()
                .map(|item| {
                    let mut value = json!({
                        "id": item.id,
                        "status": item.status,
                        "input_tokens": item.input_tokens,
                        "output_tokens": item.output_tokens,
                        "cache_creation_input_tokens": item.cache_creation_input_tokens,
                        "cache_read_input_tokens": item.cache_read_input_tokens,
                        "cost": round_to(item.total_cost_usd, 6),
                        "actual_cost": round_to(item.actual_total_cost_usd, 6),
                        "response_time_ms": item.response_time_ms,
                        "first_byte_time_ms": item.first_byte_time_ms,
                        "provider": item.provider_name,
                        "api_key_name": item.api_key_name,
                    });
                    if let Some(api_format) = item.api_format {
                        value["api_format"] = json!(api_format);
                    }
                    if let Some(endpoint_api_format) = item.endpoint_api_format {
                        value["endpoint_api_format"] = json!(endpoint_api_format);
                    }
                    value["has_format_conversion"] = json!(item.has_format_conversion);
                    if let Some(target_model) = item.target_model {
                        value["target_model"] = json!(target_model);
                    }
                    value
                })
                .collect();

            return Ok(Some(Json(json!({ "requests": payload })).into_response()));
        }
        Some("records")
            if request_context.request_method == http::Method::GET
                && matches!(
                    request_context.request_path.as_str(),
                    "/api/admin/usage/records" | "/api/admin/usage/records/"
                ) =>
        {
            if !state.has_usage_data_reader() {
                return Ok(Some(admin_usage_data_unavailable_response(
                    ADMIN_USAGE_DATA_UNAVAILABLE_DETAIL,
                )));
            }

            let query = request_context.request_query_string.as_deref();
            let time_range = match AdminStatsTimeRange::resolve_optional(query) {
                Ok(value) => value,
                Err(detail) => return Ok(Some(admin_usage_bad_request_response(detail))),
            };
            let filters = AdminStatsUsageFilter {
                user_id: query_param_value(query, "user_id"),
                provider_name: None,
                model: None,
            };
            let mut usage =
                list_usage_for_optional_range(state, time_range.as_ref(), &filters).await?;

            let search = query_param_value(query, "search");
            let username_filter = query_param_value(query, "username");
            let model_filter = query_param_value(query, "model");
            let provider_filter = query_param_value(query, "provider");
            let api_format_filter = query_param_value(query, "api_format");
            let status_filter = query_param_value(query, "status");
            let limit = match admin_usage_parse_limit(query) {
                Ok(value) => value,
                Err(detail) => return Ok(Some(admin_usage_bad_request_response(detail))),
            };
            let offset = match admin_usage_parse_offset(query) {
                Ok(value) => value,
                Err(detail) => return Ok(Some(admin_usage_bad_request_response(detail))),
            };

            usage.retain(|item| {
                admin_usage_matches_search(item, search.as_deref())
                    && admin_usage_matches_username(item, username_filter.as_deref())
                    && admin_usage_matches_eq(item.model.as_str(), model_filter.as_deref())
                    && admin_usage_matches_eq(
                        item.provider_name.as_str(),
                        provider_filter.as_deref(),
                    )
                    && admin_usage_matches_api_format(item, api_format_filter.as_deref())
                    && admin_usage_matches_status(item, status_filter.as_deref())
            });
            usage.sort_by(|left, right| {
                right
                    .created_at_unix_secs
                    .cmp(&left.created_at_unix_secs)
                    .then_with(|| left.id.cmp(&right.id))
            });
            let total = usage.len();

            let user_ids: Vec<String> = usage
                .iter()
                .filter_map(|item| item.user_id.clone())
                .collect::<BTreeSet<_>>()
                .into_iter()
                .collect();
            let users_by_id: BTreeMap<String, aether_data::repository::users::StoredUserSummary> =
                if state.has_user_data_reader() && !user_ids.is_empty() {
                    state
                        .list_users_by_ids(&user_ids)
                        .await?
                        .into_iter()
                        .map(|user| (user.id.clone(), user))
                        .collect()
                } else {
                    BTreeMap::new()
                };

            let records: Vec<_> = usage
                .into_iter()
                .skip(offset)
                .take(limit)
                .map(|item| admin_usage_record_json(&item, &users_by_id))
                .collect();

            return Ok(Some(
                Json(json!({
                    "records": records,
                    "total": total,
                    "limit": limit,
                    "offset": offset,
                }))
                .into_response(),
            ));
        }
        _ => {}
    }

    Ok(None)
}
