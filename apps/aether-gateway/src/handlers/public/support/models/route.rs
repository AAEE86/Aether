use std::time::{SystemTime, UNIX_EPOCH};

use axum::{body::Body, response::Response};

use super::models_responses::{
    build_claude_model_detail_response, build_claude_models_list_response,
    build_empty_models_list_response, build_gemini_model_detail_response,
    build_gemini_models_list_response, build_models_auth_error_response,
    build_models_not_found_response, build_openai_model_detail_response,
    build_openai_models_list_response,
};
use super::models_shared::{filter_rows_for_models, models_api_format, models_detail_id};
use super::{query_param_value, AppState, GatewayPublicRequestContext};

pub(super) async fn maybe_build_local_models_route_response(
    state: &AppState,
    request_context: &GatewayPublicRequestContext,
) -> Option<Response<Body>> {
    let decision = request_context.control_decision.as_ref()?;
    if decision.route_family.as_deref() != Some("models") {
        return None;
    }
    let api_format = models_api_format(request_context)?;
    if !state.has_minimal_candidate_selection_reader() {
        return None;
    }

    let auth_context = decision.auth_context.as_ref()?;
    let now_unix_secs = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();
    let auth_snapshot = state
        .read_auth_api_key_snapshot(
            &auth_context.user_id,
            &auth_context.api_key_id,
            now_unix_secs,
        )
        .await
        .ok()
        .flatten();
    let auth_snapshot = auth_snapshot.as_ref();

    match decision.route_kind.as_deref() {
        Some("list") => {
            let rows = state
                .list_minimal_candidate_selection_rows_for_api_format(api_format)
                .await
                .ok()?;
            let rows = filter_rows_for_models(rows, auth_snapshot, api_format);
            if rows.is_empty() {
                return Some(build_empty_models_list_response(api_format));
            }
            let response = match api_format {
                "claude:chat" => {
                    let before_id = query_param_value(
                        request_context.request_query_string.as_deref(),
                        "before_id",
                    );
                    let after_id = query_param_value(
                        request_context.request_query_string.as_deref(),
                        "after_id",
                    );
                    let limit =
                        query_param_value(request_context.request_query_string.as_deref(), "limit")
                            .and_then(|value| value.parse::<usize>().ok())
                            .filter(|value| *value > 0)
                            .unwrap_or(20);
                    build_claude_models_list_response(
                        &rows,
                        before_id.as_deref(),
                        after_id.as_deref(),
                        limit,
                    )
                }
                "gemini:chat" => {
                    let page_size = query_param_value(
                        request_context.request_query_string.as_deref(),
                        "pageSize",
                    )
                    .and_then(|value| value.parse::<usize>().ok())
                    .filter(|value| *value > 0)
                    .unwrap_or(50);
                    let page_token = query_param_value(
                        request_context.request_query_string.as_deref(),
                        "pageToken",
                    );
                    build_gemini_models_list_response(&rows, page_size, page_token.as_deref())
                }
                _ => build_openai_models_list_response(&rows),
            };
            Some(response)
        }
        Some("detail") => {
            let model_id = models_detail_id(&request_context.request_path)?;
            let rows = state
                .list_minimal_candidate_selection_rows_for_api_format_and_global_model(
                    api_format, &model_id,
                )
                .await
                .ok()?;
            let rows = filter_rows_for_models(rows, auth_snapshot, api_format);
            let Some(row) = rows.first() else {
                return Some(build_models_not_found_response(&model_id, api_format));
            };
            let response = match api_format {
                "claude:chat" => build_claude_model_detail_response(row),
                "gemini:chat" => build_gemini_model_detail_response(row),
                _ => build_openai_model_detail_response(row),
            };
            Some(response)
        }
        _ => Some(build_models_auth_error_response(api_format)),
    }
}
