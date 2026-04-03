use aether_data::repository::video_tasks::{StoredVideoTask, UpsertVideoTask};
use serde_json::{json, Map, Value};

use crate::gateway::provider_transport::GatewayProviderTransportSnapshot;

use super::super::{
    local_status_from_stored, non_empty_owned, request_body_string, GeminiVideoTaskSeed,
    LocalVideoTaskPersistence, LocalVideoTaskReadResponse, LocalVideoTaskSnapshot,
    LocalVideoTaskStatus, LocalVideoTaskTransport, OpenAiVideoTaskSeed,
};

impl LocalVideoTaskSnapshot {
    pub(crate) fn to_upsert_record(&self) -> UpsertVideoTask {
        match self {
            Self::OpenAi(seed) => seed.to_upsert_record(),
            Self::Gemini(seed) => seed.to_upsert_record(),
        }
    }

    pub(crate) fn from_stored_task(task: &StoredVideoTask) -> Option<Self> {
        task.request_metadata
            .as_ref()
            .and_then(|metadata| metadata.get("rust_local_snapshot"))
            .cloned()
            .and_then(|value| serde_json::from_value::<LocalVideoTaskSnapshot>(value).ok())
    }

    pub(crate) fn from_stored_task_with_transport(
        task: &StoredVideoTask,
        transport: &GatewayProviderTransportSnapshot,
    ) -> Option<Self> {
        let provider_api_format = task.provider_api_format.as_deref()?.trim();
        let persistence = LocalVideoTaskPersistence::from_stored_task(task)?;
        let transport = LocalVideoTaskTransport::from_provider_transport(
            transport,
            provider_api_format,
            non_empty_owned(task.model.as_ref()),
        )?;

        match provider_api_format {
            "openai:video" => {
                let upstream_task_id = non_empty_owned(task.external_task_id.as_ref())?;
                Some(Self::OpenAi(OpenAiVideoTaskSeed {
                    local_task_id: task.id.clone(),
                    upstream_task_id,
                    created_at_unix_secs: task.created_at_unix_secs,
                    user_id: task.user_id.clone(),
                    api_key_id: task.api_key_id.clone(),
                    model: non_empty_owned(task.model.as_ref()),
                    prompt: non_empty_owned(task.prompt.as_ref()).or_else(|| {
                        request_body_string(&persistence.original_request_body, "prompt")
                    }),
                    size: non_empty_owned(task.size.as_ref()).or_else(|| {
                        request_body_string(&persistence.original_request_body, "size")
                    }),
                    seconds: task
                        .duration_seconds
                        .map(|value| value.to_string())
                        .or_else(|| {
                            request_body_string(&persistence.original_request_body, "seconds")
                        }),
                    remixed_from_video_id: request_body_string(
                        &persistence.original_request_body,
                        "remix_video_id",
                    )
                    .or_else(|| {
                        request_body_string(
                            &persistence.original_request_body,
                            "remixed_from_video_id",
                        )
                    }),
                    status: local_status_from_stored(task.status),
                    progress_percent: task.progress_percent,
                    completed_at_unix_secs: task.completed_at_unix_secs,
                    expires_at_unix_secs: None,
                    error_code: task.error_code.clone(),
                    error_message: task.error_message.clone(),
                    video_url: non_empty_owned(task.video_url.as_ref()),
                    persistence,
                    transport,
                }))
            }
            "gemini:video" => {
                let local_short_id =
                    non_empty_owned(task.short_id.as_ref()).unwrap_or_else(|| task.id.clone());
                let upstream_operation_name = non_empty_owned(task.external_task_id.as_ref())?;
                let model = non_empty_owned(task.model.as_ref())?;
                Some(Self::Gemini(GeminiVideoTaskSeed {
                    local_short_id,
                    upstream_operation_name,
                    user_id: task.user_id.clone(),
                    api_key_id: task.api_key_id.clone(),
                    model,
                    status: local_status_from_stored(task.status),
                    progress_percent: task.progress_percent,
                    error_code: task.error_code.clone(),
                    error_message: task.error_message.clone(),
                    metadata: Value::Object(Map::new()),
                    persistence,
                    transport,
                }))
            }
            _ => None,
        }
    }

    pub(crate) fn read_response(&self) -> LocalVideoTaskReadResponse {
        match self {
            Self::OpenAi(seed) => match seed.status {
                LocalVideoTaskStatus::Cancelled => LocalVideoTaskReadResponse {
                    status_code: 404,
                    body_json: json!({"detail": "Video task was cancelled"}),
                },
                LocalVideoTaskStatus::Deleted => LocalVideoTaskReadResponse {
                    status_code: 404,
                    body_json: json!({"detail": "Video task not found"}),
                },
                _ => LocalVideoTaskReadResponse {
                    status_code: 200,
                    body_json: seed.client_body_json(),
                },
            },
            Self::Gemini(seed) => match seed.status {
                LocalVideoTaskStatus::Cancelled => LocalVideoTaskReadResponse {
                    status_code: 404,
                    body_json: json!({"detail": "Video task was cancelled"}),
                },
                LocalVideoTaskStatus::Deleted => LocalVideoTaskReadResponse {
                    status_code: 404,
                    body_json: json!({"detail": "Video task not found"}),
                },
                _ => LocalVideoTaskReadResponse {
                    status_code: 200,
                    body_json: seed.client_body_json(),
                },
            },
        }
    }

    #[cfg(test)]
    pub(crate) fn is_active_for_refresh(&self) -> bool {
        match self {
            Self::OpenAi(seed) => matches!(
                seed.status,
                LocalVideoTaskStatus::Submitted
                    | LocalVideoTaskStatus::Queued
                    | LocalVideoTaskStatus::Processing
            ),
            Self::Gemini(seed) => matches!(
                seed.status,
                LocalVideoTaskStatus::Submitted
                    | LocalVideoTaskStatus::Queued
                    | LocalVideoTaskStatus::Processing
            ),
        }
    }

    pub(crate) fn apply_provider_body(&mut self, provider_body: &Map<String, Value>) {
        match self {
            Self::OpenAi(seed) => seed.apply_provider_body(provider_body),
            Self::Gemini(seed) => seed.apply_provider_body(provider_body),
        }
    }

    pub(crate) fn provider_name(&self) -> Option<&str> {
        match self {
            Self::OpenAi(seed) => seed.transport.provider_name.as_deref(),
            Self::Gemini(seed) => seed.transport.provider_name.as_deref(),
        }
    }
}
