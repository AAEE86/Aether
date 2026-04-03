use super::{
    admin_monitoring_bad_request_response, admin_monitoring_not_found_response,
    parse_admin_monitoring_limit,
};
use crate::gateway::handlers::{query_param_value, unix_secs_to_rfc3339};
use crate::gateway::{AppState, GatewayError, GatewayPublicRequestContext};
use axum::{
    body::Body,
    response::{IntoResponse, Response},
    Json,
};
use serde_json::json;

fn admin_monitoring_trace_request_id_from_path(request_path: &str) -> Option<String> {
    let value = request_path
        .strip_prefix("/api/admin/monitoring/trace/")?
        .trim()
        .trim_matches('/')
        .to_string();
    if value.is_empty() || value.contains('/') {
        None
    } else {
        Some(value)
    }
}

fn admin_monitoring_trace_provider_id_from_path(request_path: &str) -> Option<String> {
    let value = request_path
        .strip_prefix("/api/admin/monitoring/trace/stats/provider/")?
        .trim()
        .trim_matches('/')
        .to_string();
    if value.is_empty() || value.contains('/') {
        None
    } else {
        Some(value)
    }
}

fn parse_admin_monitoring_attempted_only(query: Option<&str>) -> Result<bool, String> {
    match query_param_value(query, "attempted_only") {
        None => Ok(false),
        Some(value) => match value.trim().to_ascii_lowercase().as_str() {
            "true" | "1" | "yes" => Ok(true),
            "false" | "0" | "no" => Ok(false),
            _ => Err("attempted_only must be a boolean".to_string()),
        },
    }
}

pub(super) async fn build_admin_monitoring_trace_request_response(
    state: &AppState,
    request_context: &GatewayPublicRequestContext,
) -> Result<Response<Body>, GatewayError> {
    let Some(request_id) =
        admin_monitoring_trace_request_id_from_path(&request_context.request_path)
    else {
        return Ok(admin_monitoring_bad_request_response("缺少 request_id"));
    };
    let attempted_only = match parse_admin_monitoring_attempted_only(
        request_context.request_query_string.as_deref(),
    ) {
        Ok(value) => value,
        Err(detail) => return Ok(admin_monitoring_bad_request_response(detail)),
    };

    let Some(trace) = state
        .read_decision_trace(&request_id, attempted_only)
        .await?
    else {
        return Ok(admin_monitoring_not_found_response("Request not found"));
    };

    let candidates = trace
        .candidates
        .iter()
        .map(|item| {
            let candidate = &item.candidate;
            json!({
                "id": candidate.id,
                "request_id": candidate.request_id,
                "candidate_index": candidate.candidate_index,
                "retry_index": candidate.retry_index,
                "provider_id": candidate.provider_id,
                "provider_name": item.provider_name,
                "provider_website": item.provider_website,
                "endpoint_id": candidate.endpoint_id,
                "endpoint_name": item.endpoint_api_format,
                "key_id": candidate.key_id,
                "key_name": item.provider_key_name,
                "key_account_label": serde_json::Value::Null,
                "key_preview": serde_json::Value::Null,
                "key_auth_type": item.provider_key_auth_type,
                "key_oauth_plan_type": serde_json::Value::Null,
                "key_capabilities": item.provider_key_capabilities,
                "required_capabilities": candidate.required_capabilities,
                "status": candidate.status,
                "skip_reason": candidate.skip_reason,
                "is_cached": candidate.is_cached,
                "status_code": candidate.status_code,
                "error_type": candidate.error_type,
                "error_message": candidate.error_message,
                "latency_ms": candidate.latency_ms,
                "concurrent_requests": candidate.concurrent_requests,
                "extra_data": candidate.extra_data,
                "created_at": unix_secs_to_rfc3339(candidate.created_at_unix_secs),
                "started_at": candidate.started_at_unix_secs.and_then(unix_secs_to_rfc3339),
                "finished_at": candidate.finished_at_unix_secs.and_then(unix_secs_to_rfc3339),
            })
        })
        .collect::<Vec<_>>();

    Ok(Json(json!({
        "request_id": trace.request_id,
        "total_candidates": trace.total_candidates,
        "final_status": trace.final_status,
        "total_latency_ms": trace.total_latency_ms,
        "candidates": candidates,
    }))
    .into_response())
}

pub(super) async fn build_admin_monitoring_trace_provider_stats_response(
    state: &AppState,
    request_context: &GatewayPublicRequestContext,
) -> Result<Response<Body>, GatewayError> {
    let Some(provider_id) =
        admin_monitoring_trace_provider_id_from_path(&request_context.request_path)
    else {
        return Ok(admin_monitoring_bad_request_response("缺少 provider_id"));
    };
    let limit = match parse_admin_monitoring_limit(request_context.request_query_string.as_deref())
    {
        Ok(value) => value,
        Err(detail) => return Ok(admin_monitoring_bad_request_response(detail)),
    };

    let candidates = state
        .read_request_candidates_by_provider_id(&provider_id, limit)
        .await?;
    let total_attempts = candidates.len();
    let success_count = candidates
        .iter()
        .filter(|item| {
            item.status == aether_data::repository::candidates::RequestCandidateStatus::Success
        })
        .count();
    let failed_count = candidates
        .iter()
        .filter(|item| {
            item.status == aether_data::repository::candidates::RequestCandidateStatus::Failed
        })
        .count();
    let cancelled_count = candidates
        .iter()
        .filter(|item| {
            item.status == aether_data::repository::candidates::RequestCandidateStatus::Cancelled
        })
        .count();
    let skipped_count = candidates
        .iter()
        .filter(|item| {
            item.status == aether_data::repository::candidates::RequestCandidateStatus::Skipped
        })
        .count();
    let pending_count = candidates
        .iter()
        .filter(|item| {
            item.status == aether_data::repository::candidates::RequestCandidateStatus::Pending
        })
        .count();
    let available_count = candidates
        .iter()
        .filter(|item| {
            item.status == aether_data::repository::candidates::RequestCandidateStatus::Available
        })
        .count();
    let unused_count = candidates
        .iter()
        .filter(|item| {
            item.status == aether_data::repository::candidates::RequestCandidateStatus::Unused
        })
        .count();
    let completed_count = success_count + failed_count;
    let failure_rate = if completed_count == 0 {
        0.0
    } else {
        ((failed_count as f64 / completed_count as f64) * 10000.0).round() / 100.0
    };
    let latency_values = candidates
        .iter()
        .filter_map(|item| item.latency_ms.map(|value| value as f64))
        .collect::<Vec<_>>();
    let avg_latency_ms = if latency_values.is_empty() {
        0.0
    } else {
        let total = latency_values.iter().sum::<f64>();
        ((total / latency_values.len() as f64) * 100.0).round() / 100.0
    };

    Ok(Json(json!({
        "provider_id": provider_id,
        "total_attempts": total_attempts,
        "success_count": success_count,
        "failed_count": failed_count,
        "cancelled_count": cancelled_count,
        "skipped_count": skipped_count,
        "pending_count": pending_count,
        "available_count": available_count,
        "unused_count": unused_count,
        "failure_rate": failure_rate,
        "avg_latency_ms": avg_latency_ms,
    }))
    .into_response())
}
