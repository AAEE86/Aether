use std::time::{SystemTime, UNIX_EPOCH};

use aether_contracts::ExecutionResult;
use aether_data_contracts::repository::video_tasks::{
    StoredVideoTask, VideoTaskQueryFilter, VideoTaskStatus,
};
use axum::body::Body;
use axum::extract::{Path, Query, State};
use axum::response::IntoResponse;
use axum::response::Redirect;
use axum::Json;
use serde::Deserialize;
use serde_json::{json, Value};

mod cancel;

use super::query::VideoTaskVideoSource;
use super::{
    read_video_task_detail, read_video_task_page, read_video_task_stats,
    read_video_task_video_source,
};
use crate::{AppState, GatewayError};

pub(crate) use self::cancel::{cancel_video_task_record, CancelVideoTaskError};

#[derive(Debug, Deserialize)]
pub(crate) struct ListVideoTasksQuery {
    pub(crate) status: Option<String>,
    pub(crate) user_id: Option<String>,
    pub(crate) model: Option<String>,
    pub(crate) client_api_format: Option<String>,
    pub(crate) page: Option<usize>,
    pub(crate) page_size: Option<usize>,
}

pub(crate) async fn list_video_tasks(
    State(state): State<AppState>,
    Query(query): Query<ListVideoTasksQuery>,
) -> Result<Json<super::query::VideoTaskPageResponse>, axum::response::Response> {
    let filter = parse_filter(&query)?;
    let response = read_video_task_page(
        &state,
        &filter,
        query.page.unwrap_or(1),
        query.page_size.unwrap_or(20),
    )
    .await
    .map_err(IntoResponse::into_response)?;
    Ok(Json(response))
}

pub(crate) async fn get_video_task_stats(
    State(state): State<AppState>,
    Query(query): Query<ListVideoTasksQuery>,
) -> Result<Json<super::query::VideoTaskStatsResponse>, axum::response::Response> {
    let filter = parse_filter(&query)?;
    let response = read_video_task_stats(&state, &filter, current_unix_secs())
        .await
        .map_err(IntoResponse::into_response)?;
    Ok(Json(response))
}

pub(crate) async fn get_video_task_detail(
    State(state): State<AppState>,
    Path(task_id): Path<String>,
) -> Result<Json<StoredVideoTask>, axum::response::Response> {
    let task = read_video_task_detail(&state, &task_id)
        .await
        .map_err(IntoResponse::into_response)?;

    match task {
        Some(task) => Ok(Json(task)),
        None => Err((
            axum::http::StatusCode::NOT_FOUND,
            Json(json!({
                "error": {
                    "message": "Video task not found",
                }
            })),
        )
            .into_response()),
    }
}

pub(crate) async fn cancel_video_task(
    State(state): State<AppState>,
    Path(task_id): Path<String>,
) -> Result<Json<Value>, axum::response::Response> {
    let trace_id = format!("async-task-admin-cancel-{task_id}");
    let execution_request_id =
        crate::execution_identity::ExecutionRequestId::generate().into_string();
    let stored = cancel_video_task_record(&state, &task_id, &trace_id, &execution_request_id)
        .await
        .map_err(|err| match err {
            CancelVideoTaskError::NotFound => (
                axum::http::StatusCode::NOT_FOUND,
                Json(json!({
                    "error": {
                        "message": "Video task not found",
                    }
                })),
            )
                .into_response(),
            CancelVideoTaskError::InvalidStatus(status) => (
                axum::http::StatusCode::BAD_REQUEST,
                Json(json!({
                    "error": {
                        "message": format!(
                            "Cannot cancel task with status: {}",
                            video_task_status_name(status),
                        ),
                    }
                })),
            )
                .into_response(),
            CancelVideoTaskError::Response(response) => response,
            CancelVideoTaskError::Gateway(err) => err.into_response(),
        })?;

    Ok(Json(json!({
        "id": stored.id,
        "status": "cancelled",
        "message": "Task cancelled successfully",
    })))
}

pub(crate) async fn get_video_task_video(
    State(state): State<AppState>,
    Path(task_id): Path<String>,
) -> Result<axum::response::Response, axum::response::Response> {
    let Some(source) = read_video_task_video_source(&state, &task_id)
        .await
        .map_err(IntoResponse::into_response)?
    else {
        return Err((
            axum::http::StatusCode::NOT_FOUND,
            Json(json!({
                "error": {
                    "message": "Video task or video not found",
                }
            })),
        )
            .into_response());
    };

    build_video_task_video_response(&state, &task_id, source)
        .await
        .map_err(IntoResponse::into_response)
}

pub(crate) async fn build_video_task_video_response(
    state: &AppState,
    task_id: &str,
    source: VideoTaskVideoSource,
) -> Result<axum::response::Response, GatewayError> {
    match source {
        VideoTaskVideoSource::Redirect { url } => Ok(Redirect::temporary(&url).into_response()),
        VideoTaskVideoSource::Proxy {
            url,
            header_name,
            header_value,
            filename,
        } => proxy_video_stream(state, task_id, &url, &header_name, &header_value, &filename).await,
    }
}

fn parse_filter(
    query: &ListVideoTasksQuery,
) -> Result<VideoTaskQueryFilter, axum::response::Response> {
    let status = match query.status.as_deref() {
        Some(value) => Some(VideoTaskStatus::from_database(value).map_err(|err| {
            (
                axum::http::StatusCode::BAD_REQUEST,
                Json(json!({
                    "error": {
                        "message": err.to_string(),
                    }
                })),
            )
                .into_response()
        })?),
        None => None,
    };

    Ok(VideoTaskQueryFilter {
        user_id: query.user_id.clone(),
        status,
        model_substring: query.model.clone(),
        client_api_format: query.client_api_format.clone(),
    })
}

fn current_unix_secs() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}

fn video_task_status_name(status: VideoTaskStatus) -> &'static str {
    match status {
        VideoTaskStatus::Pending => "pending",
        VideoTaskStatus::Submitted => "submitted",
        VideoTaskStatus::Queued => "queued",
        VideoTaskStatus::Processing => "processing",
        VideoTaskStatus::Completed => "completed",
        VideoTaskStatus::Failed => "failed",
        VideoTaskStatus::Cancelled => "cancelled",
        VideoTaskStatus::Expired => "expired",
        VideoTaskStatus::Deleted => "deleted",
    }
}

async fn proxy_video_stream(
    state: &AppState,
    task_id: &str,
    url: &str,
    header_name: &str,
    header_value: &str,
    filename: &str,
) -> Result<axum::response::Response, GatewayError> {
    let response = state
        .client
        .get(url)
        .header(header_name, header_value)
        .send()
        .await
        .map_err(|err| GatewayError::UpstreamUnavailable {
            trace_id: task_id.to_string(),
            message: err.to_string(),
        })?;

    if response.status().is_client_error() || response.status().is_server_error() {
        return Err(GatewayError::UpstreamUnavailable {
            trace_id: task_id.to_string(),
            message: format!("video upstream returned HTTP {}", response.status()),
        });
    }

    let status = response.status();
    let content_type = response
        .headers()
        .get(axum::http::header::CONTENT_TYPE)
        .cloned()
        .unwrap_or_else(|| axum::http::HeaderValue::from_static("video/mp4"));
    let content_length = response
        .headers()
        .get(axum::http::header::CONTENT_LENGTH)
        .cloned();
    let cache_control = response
        .headers()
        .get(axum::http::header::CACHE_CONTROL)
        .cloned();
    let body = Body::from_stream(response.bytes_stream());

    let mut outbound = axum::http::Response::builder()
        .status(status)
        .body(body)
        .map_err(|err| GatewayError::Internal(err.to_string()))?;
    outbound
        .headers_mut()
        .insert(axum::http::header::CONTENT_TYPE, content_type);
    outbound.headers_mut().insert(
        axum::http::header::CONTENT_DISPOSITION,
        axum::http::HeaderValue::from_str(&format!("inline; filename=\"{filename}\""))
            .map_err(|err| GatewayError::Internal(err.to_string()))?,
    );
    if let Some(content_length) = content_length {
        outbound
            .headers_mut()
            .insert(axum::http::header::CONTENT_LENGTH, content_length);
    }
    if let Some(cache_control) = cache_control {
        outbound
            .headers_mut()
            .insert(axum::http::header::CACHE_CONTROL, cache_control);
    } else {
        outbound.headers_mut().insert(
            axum::http::header::CACHE_CONTROL,
            axum::http::HeaderValue::from_static("private, max-age=3600"),
        );
    }
    Ok(outbound)
}
