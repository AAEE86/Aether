use super::{
    admin_stats_bad_request_response, admin_stats_leaderboard_empty_response,
    build_api_key_leaderboard_items, build_model_leaderboard_items, build_user_leaderboard_items,
    compare_leaderboard_items, compute_dense_rank, list_usage_for_optional_range,
    load_user_leaderboard_metadata, parse_bounded_u32, parse_nonnegative_usize, round_to,
    AdminStatsLeaderboardMetric, AdminStatsSortOrder, AdminStatsTimeRange, AdminStatsUsageFilter,
};
use crate::gateway::handlers::{query_param_bool, query_param_value};
use crate::gateway::{AppState, GatewayError, GatewayPublicRequestContext};
use axum::{
    body::Body,
    http,
    response::{IntoResponse, Response},
    Json,
};
use serde_json::json;

pub(super) async fn maybe_build_local_admin_stats_leaderboard_response(
    state: &AppState,
    request_context: &GatewayPublicRequestContext,
) -> Result<Option<Response<Body>>, GatewayError> {
    let query = request_context.request_query_string.as_deref();

    if request_context
        .control_decision
        .as_ref()
        .and_then(|decision| decision.route_kind.as_deref())
        == Some("leaderboard_models")
        && request_context.request_method == http::Method::GET
        && matches!(
            request_context.request_path.as_str(),
            "/api/admin/stats/leaderboard/models" | "/api/admin/stats/leaderboard/models/"
        )
    {
        let time_range = match AdminStatsTimeRange::resolve_optional(query) {
            Ok(value) => value,
            Err(detail) => return Ok(Some(admin_stats_bad_request_response(detail))),
        };
        let metric = match AdminStatsLeaderboardMetric::parse(query) {
            Ok(value) => value,
            Err(detail) => return Ok(Some(admin_stats_bad_request_response(detail))),
        };
        let order = match AdminStatsSortOrder::parse(query) {
            Ok(value) => value,
            Err(detail) => return Ok(Some(admin_stats_bad_request_response(detail))),
        };
        let limit = match query_param_value(query, "limit")
            .map(|value| parse_bounded_u32("limit", &value, 1, 100))
            .transpose()
        {
            Ok(Some(value)) => value as usize,
            Ok(None) => 10,
            Err(detail) => return Ok(Some(admin_stats_bad_request_response(detail))),
        };
        let offset = match query_param_value(query, "offset")
            .map(|value| parse_nonnegative_usize("offset", &value))
            .transpose()
        {
            Ok(Some(value)) => value,
            Ok(None) => 0,
            Err(detail) => return Ok(Some(admin_stats_bad_request_response(detail))),
        };
        if !state.has_usage_data_reader() {
            return Ok(Some(admin_stats_leaderboard_empty_response(
                metric,
                time_range.as_ref(),
            )));
        }
        let filters = AdminStatsUsageFilter::from_query(query);
        let usage = list_usage_for_optional_range(state, time_range.as_ref(), &filters).await?;
        let mut leaderboard = build_model_leaderboard_items(&usage);
        leaderboard.sort_by(|left, right| compare_leaderboard_items(metric, order, left, right));

        let total = leaderboard.len();
        let items: Vec<_> = leaderboard
            .iter()
            .enumerate()
            .skip(offset)
            .take(limit)
            .map(|(index, item)| {
                let rank = compute_dense_rank(metric, &leaderboard, index);
                let value = match metric {
                    AdminStatsLeaderboardMetric::Requests => json!(item.requests),
                    AdminStatsLeaderboardMetric::Tokens => json!(item.tokens),
                    AdminStatsLeaderboardMetric::Cost => json!(round_to(item.cost, 6)),
                };
                json!({
                    "rank": rank,
                    "id": item.id,
                    "name": item.id,
                    "value": value,
                    "requests": item.requests,
                    "tokens": item.tokens,
                    "cost": round_to(item.cost, 6),
                })
            })
            .collect();

        return Ok(Some(
            Json(json!({
                "items": items,
                "total": total,
                "metric": metric.as_str(),
                "start_date": time_range.as_ref().map(|value| value.start_date.to_string()),
                "end_date": time_range.as_ref().map(|value| value.end_date.to_string()),
            }))
            .into_response(),
        ));
    }

    if request_context
        .control_decision
        .as_ref()
        .and_then(|decision| decision.route_kind.as_deref())
        == Some("leaderboard_api_keys")
        && request_context.request_method == http::Method::GET
        && matches!(
            request_context.request_path.as_str(),
            "/api/admin/stats/leaderboard/api-keys" | "/api/admin/stats/leaderboard/api-keys/"
        )
    {
        let time_range = match AdminStatsTimeRange::resolve_optional(query) {
            Ok(value) => value,
            Err(detail) => return Ok(Some(admin_stats_bad_request_response(detail))),
        };
        let metric = match AdminStatsLeaderboardMetric::parse(query) {
            Ok(value) => value,
            Err(detail) => return Ok(Some(admin_stats_bad_request_response(detail))),
        };
        let order = match AdminStatsSortOrder::parse(query) {
            Ok(value) => value,
            Err(detail) => return Ok(Some(admin_stats_bad_request_response(detail))),
        };
        let limit = match query_param_value(query, "limit")
            .map(|value| parse_bounded_u32("limit", &value, 1, 100))
            .transpose()
        {
            Ok(Some(value)) => value as usize,
            Ok(None) => 10,
            Err(detail) => return Ok(Some(admin_stats_bad_request_response(detail))),
        };
        let offset = match query_param_value(query, "offset")
            .map(|value| parse_nonnegative_usize("offset", &value))
            .transpose()
        {
            Ok(Some(value)) => value,
            Ok(None) => 0,
            Err(detail) => return Ok(Some(admin_stats_bad_request_response(detail))),
        };
        if !state.has_usage_data_reader() {
            return Ok(Some(admin_stats_leaderboard_empty_response(
                metric,
                time_range.as_ref(),
            )));
        }
        let include_inactive = query_param_bool(query, "include_inactive", false);
        let exclude_admin = query_param_bool(query, "exclude_admin", false);
        let filters = AdminStatsUsageFilter::from_query(query);
        let usage = list_usage_for_optional_range(state, time_range.as_ref(), &filters).await?;
        let snapshots = if state.has_auth_api_key_data_reader() {
            let api_key_ids: Vec<String> = usage
                .iter()
                .filter_map(|item| item.api_key_id.clone())
                .collect::<std::collections::BTreeSet<_>>()
                .into_iter()
                .collect();
            Some(
                state
                    .read_auth_api_key_snapshots_by_ids(&api_key_ids)
                    .await?,
            )
        } else {
            None
        };
        let mut leaderboard = build_api_key_leaderboard_items(
            &usage,
            snapshots.as_deref(),
            include_inactive,
            exclude_admin,
        );
        leaderboard.sort_by(|left, right| compare_leaderboard_items(metric, order, left, right));

        let total = leaderboard.len();
        let items: Vec<_> = leaderboard
            .iter()
            .enumerate()
            .skip(offset)
            .take(limit)
            .map(|(index, item)| {
                let rank = compute_dense_rank(metric, &leaderboard, index);
                let value = match metric {
                    AdminStatsLeaderboardMetric::Requests => json!(item.requests),
                    AdminStatsLeaderboardMetric::Tokens => json!(item.tokens),
                    AdminStatsLeaderboardMetric::Cost => json!(round_to(item.cost, 6)),
                };
                json!({
                    "rank": rank,
                    "id": item.id,
                    "name": item.name,
                    "value": value,
                    "requests": item.requests,
                    "tokens": item.tokens,
                    "cost": round_to(item.cost, 6),
                })
            })
            .collect();

        return Ok(Some(
            Json(json!({
                "items": items,
                "total": total,
                "metric": metric.as_str(),
                "start_date": time_range.as_ref().map(|value| value.start_date.to_string()),
                "end_date": time_range.as_ref().map(|value| value.end_date.to_string()),
            }))
            .into_response(),
        ));
    }

    if request_context
        .control_decision
        .as_ref()
        .and_then(|decision| decision.route_kind.as_deref())
        == Some("leaderboard_users")
        && request_context.request_method == http::Method::GET
        && matches!(
            request_context.request_path.as_str(),
            "/api/admin/stats/leaderboard/users" | "/api/admin/stats/leaderboard/users/"
        )
    {
        let time_range = match AdminStatsTimeRange::resolve_optional(query) {
            Ok(value) => value,
            Err(detail) => return Ok(Some(admin_stats_bad_request_response(detail))),
        };
        let metric = match AdminStatsLeaderboardMetric::parse(query) {
            Ok(value) => value,
            Err(detail) => return Ok(Some(admin_stats_bad_request_response(detail))),
        };
        let order = match AdminStatsSortOrder::parse(query) {
            Ok(value) => value,
            Err(detail) => return Ok(Some(admin_stats_bad_request_response(detail))),
        };
        let limit = match query_param_value(query, "limit")
            .map(|value| parse_bounded_u32("limit", &value, 1, 100))
            .transpose()
        {
            Ok(Some(value)) => value as usize,
            Ok(None) => 10,
            Err(detail) => return Ok(Some(admin_stats_bad_request_response(detail))),
        };
        let offset = match query_param_value(query, "offset")
            .map(|value| parse_nonnegative_usize("offset", &value))
            .transpose()
        {
            Ok(Some(value)) => value,
            Ok(None) => 0,
            Err(detail) => return Ok(Some(admin_stats_bad_request_response(detail))),
        };
        if !state.has_usage_data_reader() {
            return Ok(Some(admin_stats_leaderboard_empty_response(
                metric,
                time_range.as_ref(),
            )));
        }
        let include_inactive = query_param_bool(query, "include_inactive", false);
        let exclude_admin = query_param_bool(query, "exclude_admin", false);
        let filters = AdminStatsUsageFilter::from_query(query);
        let usage = list_usage_for_optional_range(state, time_range.as_ref(), &filters).await?;
        let user_ids: Vec<String> = usage
            .iter()
            .filter_map(|item| item.user_id.clone())
            .collect::<std::collections::BTreeSet<_>>()
            .into_iter()
            .collect();
        let user_metadata = load_user_leaderboard_metadata(state, &user_ids).await?;
        let mut leaderboard =
            build_user_leaderboard_items(&usage, &user_metadata, include_inactive, exclude_admin);
        leaderboard.sort_by(|left, right| compare_leaderboard_items(metric, order, left, right));

        let total = leaderboard.len();
        let items: Vec<_> = leaderboard
            .iter()
            .enumerate()
            .skip(offset)
            .take(limit)
            .map(|(index, item)| {
                let rank = compute_dense_rank(metric, &leaderboard, index);
                let value = match metric {
                    AdminStatsLeaderboardMetric::Requests => json!(item.requests),
                    AdminStatsLeaderboardMetric::Tokens => json!(item.tokens),
                    AdminStatsLeaderboardMetric::Cost => json!(round_to(item.cost, 6)),
                };
                json!({
                    "rank": rank,
                    "id": item.id,
                    "name": item.name,
                    "value": value,
                    "requests": item.requests,
                    "tokens": item.tokens,
                    "cost": round_to(item.cost, 6),
                })
            })
            .collect();

        return Ok(Some(
            Json(json!({
                "items": items,
                "total": total,
                "metric": metric.as_str(),
                "start_date": time_range.as_ref().map(|value| value.start_date.to_string()),
                "end_date": time_range.as_ref().map(|value| value.end_date.to_string()),
            }))
            .into_response(),
        ));
    }

    Ok(None)
}
