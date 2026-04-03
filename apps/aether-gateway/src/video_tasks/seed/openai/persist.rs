use aether_data::repository::video_tasks::UpsertVideoTask;
use serde_json::json;

use super::super::super::{
    current_unix_timestamp_secs, request_body_string, request_body_u32, LocalVideoTaskSnapshot,
    LocalVideoTaskStatus, OpenAiVideoTaskSeed, DEFAULT_VIDEO_TASK_MAX_POLL_COUNT,
    DEFAULT_VIDEO_TASK_POLL_INTERVAL_SECONDS,
};

impl OpenAiVideoTaskSeed {
    pub(crate) fn to_upsert_record(&self) -> UpsertVideoTask {
        let now_unix_secs = current_unix_timestamp_secs();
        let next_poll_at_unix_secs = match self.status {
            LocalVideoTaskStatus::Submitted
            | LocalVideoTaskStatus::Queued
            | LocalVideoTaskStatus::Processing => Some(
                self.created_at_unix_secs
                    .saturating_add(u64::from(DEFAULT_VIDEO_TASK_POLL_INTERVAL_SECONDS)),
            ),
            _ => None,
        };
        UpsertVideoTask {
            id: self.local_task_id.clone(),
            short_id: None,
            request_id: self.persistence.request_id.clone(),
            user_id: self.user_id.clone(),
            api_key_id: self.api_key_id.clone(),
            username: self.persistence.username.clone(),
            api_key_name: self.persistence.api_key_name.clone(),
            external_task_id: Some(self.upstream_task_id.clone()),
            provider_id: Some(self.transport.provider_id.clone()),
            endpoint_id: Some(self.transport.endpoint_id.clone()),
            key_id: Some(self.transport.key_id.clone()),
            client_api_format: Some(self.persistence.client_api_format.clone()),
            provider_api_format: Some(self.persistence.provider_api_format.clone()),
            format_converted: self.persistence.format_converted,
            model: self.model.clone().or_else(|| Some(String::new())),
            prompt: self.prompt.clone().or_else(|| Some(String::new())),
            original_request_body: Some(self.persistence.original_request_body.clone()),
            duration_seconds: request_body_u32(&self.persistence.original_request_body, "seconds"),
            resolution: request_body_string(&self.persistence.original_request_body, "resolution"),
            aspect_ratio: request_body_string(
                &self.persistence.original_request_body,
                "aspect_ratio",
            ),
            size: self.size.clone(),
            status: self.status.as_database_status(),
            progress_percent: self.progress_percent,
            progress_message: None,
            retry_count: 0,
            poll_interval_seconds: DEFAULT_VIDEO_TASK_POLL_INTERVAL_SECONDS,
            next_poll_at_unix_secs,
            poll_count: 0,
            max_poll_count: DEFAULT_VIDEO_TASK_MAX_POLL_COUNT,
            created_at_unix_secs: self.created_at_unix_secs,
            submitted_at_unix_secs: Some(self.created_at_unix_secs),
            completed_at_unix_secs: self.completed_at_unix_secs,
            updated_at_unix_secs: self.completed_at_unix_secs.unwrap_or(now_unix_secs),
            error_code: self.error_code.clone(),
            error_message: self.error_message.clone(),
            video_url: self.video_url.clone(),
            request_metadata: Some(json!({
                "rust_owner": "async_task",
                "rust_local_snapshot": LocalVideoTaskSnapshot::OpenAi(self.clone()),
            })),
        }
    }
}
