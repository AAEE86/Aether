use serde_json::{Map, Value};

use super::super::super::{LocalVideoTaskStatus, OpenAiVideoTaskSeed};

impl OpenAiVideoTaskSeed {
    pub(crate) fn apply_provider_body(&mut self, provider_body: &Map<String, Value>) {
        let raw_status = provider_body
            .get("status")
            .and_then(Value::as_str)
            .map(str::trim)
            .unwrap_or_default();
        self.status = match raw_status {
            "queued" => LocalVideoTaskStatus::Queued,
            "processing" => LocalVideoTaskStatus::Processing,
            "completed" => LocalVideoTaskStatus::Completed,
            "failed" => LocalVideoTaskStatus::Failed,
            "cancelled" => LocalVideoTaskStatus::Cancelled,
            "expired" => LocalVideoTaskStatus::Expired,
            _ => LocalVideoTaskStatus::Submitted,
        };
        self.progress_percent = provider_body
            .get("progress")
            .and_then(Value::as_u64)
            .and_then(|value| u16::try_from(value).ok())
            .unwrap_or(match self.status {
                LocalVideoTaskStatus::Completed => 100,
                LocalVideoTaskStatus::Processing => 50,
                _ => self.progress_percent,
            });
        self.completed_at_unix_secs = provider_body.get("completed_at").and_then(Value::as_u64);
        self.expires_at_unix_secs = provider_body.get("expires_at").and_then(Value::as_u64);
        let error = provider_body.get("error").and_then(Value::as_object);
        self.error_code = error
            .and_then(|value| value.get("code"))
            .and_then(Value::as_str)
            .map(str::to_string);
        self.error_message = error
            .and_then(|value| value.get("message"))
            .and_then(Value::as_str)
            .map(str::to_string);
        self.video_url = provider_body
            .get("video_url")
            .or_else(|| provider_body.get("url"))
            .or_else(|| provider_body.get("result_url"))
            .and_then(Value::as_str)
            .map(str::to_string);
    }
}
