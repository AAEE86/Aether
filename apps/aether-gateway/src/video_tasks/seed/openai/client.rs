use std::collections::BTreeMap;

use aether_contracts::{ExecutionPlan, RequestBody};
use serde_json::{json, Value};

use super::super::super::{
    map_openai_task_status, parse_video_content_variant, LocalVideoTaskContentAction,
    LocalVideoTaskStatus, OpenAiVideoTaskSeed,
};

impl OpenAiVideoTaskSeed {
    pub(crate) fn build_content_stream_action(
        &self,
        query_string: Option<&str>,
        trace_id: &str,
    ) -> Option<LocalVideoTaskContentAction> {
        match self.status {
            LocalVideoTaskStatus::Submitted
            | LocalVideoTaskStatus::Queued
            | LocalVideoTaskStatus::Processing => {
                return Some(LocalVideoTaskContentAction::Immediate {
                    status_code: 202,
                    body_json: json!({
                        "detail": format!(
                            "Video is still processing (status: {})",
                            map_openai_task_status(self.status)
                        )
                    }),
                });
            }
            LocalVideoTaskStatus::Failed | LocalVideoTaskStatus::Expired => {
                return Some(LocalVideoTaskContentAction::Immediate {
                    status_code: 422,
                    body_json: json!({
                        "detail": format!(
                            "Video generation failed: {}",
                            self.error_message
                                .clone()
                                .unwrap_or_else(|| "Unknown error".to_string())
                        )
                    }),
                });
            }
            LocalVideoTaskStatus::Cancelled => {
                return Some(LocalVideoTaskContentAction::Immediate {
                    status_code: 404,
                    body_json: json!({"detail": "Video task was cancelled"}),
                });
            }
            LocalVideoTaskStatus::Deleted => {
                return Some(LocalVideoTaskContentAction::Immediate {
                    status_code: 404,
                    body_json: json!({"detail": "Video task not found"}),
                });
            }
            LocalVideoTaskStatus::Completed => {}
        }

        let variant = parse_video_content_variant(query_string)?;
        let (url, headers) = if variant == "video" {
            if let Some(video_url) = self
                .video_url
                .clone()
                .filter(|value| value.starts_with("http://") || value.starts_with("https://"))
            {
                (video_url, BTreeMap::new())
            } else {
                let mut headers = self.transport.headers.clone();
                headers.remove("content-type");
                headers.remove("content-length");
                (
                    format!(
                        "{}/v1/videos/{}/content",
                        self.transport.upstream_base_url.trim_end_matches('/'),
                        self.upstream_task_id
                    ),
                    headers,
                )
            }
        } else {
            let mut headers = self.transport.headers.clone();
            headers.remove("content-type");
            headers.remove("content-length");
            (
                format!(
                    "{}/v1/videos/{}/content?variant={variant}",
                    self.transport.upstream_base_url.trim_end_matches('/'),
                    self.upstream_task_id
                ),
                headers,
            )
        };

        Some(LocalVideoTaskContentAction::StreamPlan(ExecutionPlan {
            request_id: trace_id.to_string(),
            candidate_id: None,
            provider_name: self.transport.provider_name.clone(),
            provider_id: self.transport.provider_id.clone(),
            endpoint_id: self.transport.endpoint_id.clone(),
            key_id: self.transport.key_id.clone(),
            method: "GET".to_string(),
            url,
            headers,
            content_type: None,
            content_encoding: None,
            body: RequestBody {
                json_body: None,
                body_bytes_b64: None,
                body_ref: None,
            },
            stream: true,
            client_api_format: "openai:video".to_string(),
            provider_api_format: "openai:video".to_string(),
            model_name: self
                .model
                .clone()
                .or_else(|| self.transport.model_name.clone()),
            proxy: self.transport.proxy.clone(),
            tls_profile: self.transport.tls_profile.clone(),
            timeouts: self.transport.timeouts.clone(),
        }))
    }

    pub(crate) fn client_body_json(&self) -> Value {
        let mut body = json!({
            "id": self.local_task_id,
            "object": "video",
            "status": map_openai_task_status(self.status),
            "progress": self.progress_percent,
            "created_at": self.created_at_unix_secs,
        });

        if let Some(model) = &self.model {
            body["model"] = Value::String(model.clone());
        }
        if let Some(prompt) = &self.prompt {
            body["prompt"] = Value::String(prompt.clone());
        }
        if let Some(size) = &self.size {
            body["size"] = Value::String(size.clone());
        }
        if let Some(seconds) = &self.seconds {
            body["seconds"] = Value::String(seconds.clone());
        }
        if let Some(remixed_from_video_id) = &self.remixed_from_video_id {
            body["remixed_from_video_id"] = Value::String(remixed_from_video_id.clone());
        }
        if let Some(completed_at) = self.completed_at_unix_secs {
            body["completed_at"] = Value::Number(completed_at.into());
        }
        if let Some(expires_at) = self.expires_at_unix_secs {
            body["expires_at"] = Value::Number(expires_at.into());
        }
        if self.status == LocalVideoTaskStatus::Failed
            || self.status == LocalVideoTaskStatus::Expired
        {
            body["error"] = json!({
                "code": self.error_code.clone().unwrap_or_else(|| "unknown".to_string()),
                "message": self
                    .error_message
                    .clone()
                    .unwrap_or_else(|| "Video generation failed".to_string()),
            });
        }

        body
    }
}
