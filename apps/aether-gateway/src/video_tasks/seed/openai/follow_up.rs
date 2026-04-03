use aether_contracts::{ExecutionPlan, RequestBody};
use serde_json::Value;

use crate::gateway::GatewayControlAuthContext;

use super::super::super::{
    build_video_follow_up_report_context, resolve_follow_up_auth, LocalVideoTaskFollowUpPlan,
    LocalVideoTaskStatus, OpenAiVideoTaskSeed,
};

impl OpenAiVideoTaskSeed {
    pub(crate) fn build_delete_follow_up_plan(
        &self,
        auth_context: Option<&GatewayControlAuthContext>,
        trace_id: &str,
    ) -> Option<LocalVideoTaskFollowUpPlan> {
        if !matches!(
            self.status,
            LocalVideoTaskStatus::Completed | LocalVideoTaskStatus::Failed
        ) {
            return None;
        }
        let (user_id, api_key_id) = resolve_follow_up_auth(
            self.user_id.as_deref(),
            self.api_key_id.as_deref(),
            auth_context,
        )?;

        let mut headers = self.transport.headers.clone();
        headers.remove("content-type");
        headers.remove("content-length");

        Some(LocalVideoTaskFollowUpPlan {
            plan: ExecutionPlan {
                request_id: trace_id.to_string(),
                candidate_id: None,
                provider_name: self.transport.provider_name.clone(),
                provider_id: self.transport.provider_id.clone(),
                endpoint_id: self.transport.endpoint_id.clone(),
                key_id: self.transport.key_id.clone(),
                method: "DELETE".to_string(),
                url: format!(
                    "{}/v1/videos/{}",
                    self.transport.upstream_base_url.trim_end_matches('/'),
                    self.upstream_task_id
                ),
                headers,
                content_type: None,
                content_encoding: None,
                body: RequestBody {
                    json_body: None,
                    body_bytes_b64: None,
                    body_ref: None,
                },
                stream: false,
                client_api_format: "openai:video".to_string(),
                provider_api_format: "openai:video".to_string(),
                model_name: self
                    .model
                    .clone()
                    .or_else(|| self.transport.model_name.clone()),
                proxy: self.transport.proxy.clone(),
                tls_profile: self.transport.tls_profile.clone(),
                timeouts: self.transport.timeouts.clone(),
            },
            report_kind: Some("openai_video_delete_sync_finalize".to_string()),
            report_context: Some(build_video_follow_up_report_context(
                &self.persistence.request_id,
                &user_id,
                &api_key_id,
                &self.local_task_id,
                self.model
                    .clone()
                    .or_else(|| self.transport.model_name.clone()),
                &self.transport,
                "openai:video",
                "openai:video",
            )),
        })
    }

    pub(crate) fn build_get_follow_up_plan(&self, trace_id: &str) -> Option<ExecutionPlan> {
        if !matches!(
            self.status,
            LocalVideoTaskStatus::Submitted
                | LocalVideoTaskStatus::Queued
                | LocalVideoTaskStatus::Processing
        ) {
            return None;
        }

        let mut headers = self.transport.headers.clone();
        headers.remove("content-type");
        headers.remove("content-length");

        Some(ExecutionPlan {
            request_id: trace_id.to_string(),
            candidate_id: None,
            provider_name: self.transport.provider_name.clone(),
            provider_id: self.transport.provider_id.clone(),
            endpoint_id: self.transport.endpoint_id.clone(),
            key_id: self.transport.key_id.clone(),
            method: "GET".to_string(),
            url: format!(
                "{}/v1/videos/{}",
                self.transport.upstream_base_url.trim_end_matches('/'),
                self.upstream_task_id
            ),
            headers,
            content_type: None,
            content_encoding: None,
            body: RequestBody {
                json_body: None,
                body_bytes_b64: None,
                body_ref: None,
            },
            stream: false,
            client_api_format: "openai:video".to_string(),
            provider_api_format: "openai:video".to_string(),
            model_name: self
                .model
                .clone()
                .or_else(|| self.transport.model_name.clone()),
            proxy: self.transport.proxy.clone(),
            tls_profile: self.transport.tls_profile.clone(),
            timeouts: self.transport.timeouts.clone(),
        })
    }

    pub(crate) fn build_cancel_follow_up_plan(
        &self,
        auth_context: Option<&GatewayControlAuthContext>,
        trace_id: &str,
    ) -> Option<LocalVideoTaskFollowUpPlan> {
        if !matches!(
            self.status,
            LocalVideoTaskStatus::Submitted
                | LocalVideoTaskStatus::Queued
                | LocalVideoTaskStatus::Processing
        ) {
            return None;
        }
        let (user_id, api_key_id) = resolve_follow_up_auth(
            self.user_id.as_deref(),
            self.api_key_id.as_deref(),
            auth_context,
        )?;

        let mut headers = self.transport.headers.clone();
        headers.remove("content-type");
        headers.remove("content-length");

        Some(LocalVideoTaskFollowUpPlan {
            plan: ExecutionPlan {
                request_id: trace_id.to_string(),
                candidate_id: None,
                provider_name: self.transport.provider_name.clone(),
                provider_id: self.transport.provider_id.clone(),
                endpoint_id: self.transport.endpoint_id.clone(),
                key_id: self.transport.key_id.clone(),
                method: "DELETE".to_string(),
                url: format!(
                    "{}/v1/videos/{}",
                    self.transport.upstream_base_url.trim_end_matches('/'),
                    self.upstream_task_id
                ),
                headers,
                content_type: None,
                content_encoding: None,
                body: RequestBody {
                    json_body: None,
                    body_bytes_b64: None,
                    body_ref: None,
                },
                stream: false,
                client_api_format: "openai:video".to_string(),
                provider_api_format: "openai:video".to_string(),
                model_name: self
                    .model
                    .clone()
                    .or_else(|| self.transport.model_name.clone()),
                proxy: self.transport.proxy.clone(),
                tls_profile: self.transport.tls_profile.clone(),
                timeouts: self.transport.timeouts.clone(),
            },
            report_kind: Some("openai_video_cancel_sync_finalize".to_string()),
            report_context: Some(build_video_follow_up_report_context(
                &self.persistence.request_id,
                &user_id,
                &api_key_id,
                &self.local_task_id,
                self.model
                    .clone()
                    .or_else(|| self.transport.model_name.clone()),
                &self.transport,
                "openai:video",
                "openai:video",
            )),
        })
    }

    pub(crate) fn build_remix_follow_up_plan(
        &self,
        body_json: &Value,
        auth_context: Option<&GatewayControlAuthContext>,
        trace_id: &str,
    ) -> Option<LocalVideoTaskFollowUpPlan> {
        if !matches!(self.status, LocalVideoTaskStatus::Completed) {
            return None;
        }
        if body_json.is_null() {
            return None;
        }
        let (user_id, api_key_id) = resolve_follow_up_auth(
            self.user_id.as_deref(),
            self.api_key_id.as_deref(),
            auth_context,
        )?;

        let mut headers = self.transport.headers.clone();
        headers.remove("content-length");
        let content_type = self
            .transport
            .content_type
            .clone()
            .unwrap_or_else(|| "application/json".to_string());
        headers
            .entry("content-type".to_string())
            .or_insert_with(|| content_type.clone());

        let mut report_context = build_video_follow_up_report_context(
            &self.persistence.request_id,
            &user_id,
            &api_key_id,
            &self.local_task_id,
            self.model
                .clone()
                .or_else(|| self.transport.model_name.clone()),
            &self.transport,
            "openai:video",
            "openai:video",
        );
        if let Some(report_context_object) = report_context.as_object_mut() {
            report_context_object.insert("original_request_body".to_string(), body_json.clone());
        }

        Some(LocalVideoTaskFollowUpPlan {
            plan: ExecutionPlan {
                request_id: trace_id.to_string(),
                candidate_id: None,
                provider_name: self.transport.provider_name.clone(),
                provider_id: self.transport.provider_id.clone(),
                endpoint_id: self.transport.endpoint_id.clone(),
                key_id: self.transport.key_id.clone(),
                method: "POST".to_string(),
                url: format!(
                    "{}/v1/videos/{}/remix",
                    self.transport.upstream_base_url.trim_end_matches('/'),
                    self.upstream_task_id
                ),
                headers,
                content_type: Some(content_type),
                content_encoding: None,
                body: RequestBody::from_json(body_json.clone()),
                stream: false,
                client_api_format: "openai:video".to_string(),
                provider_api_format: "openai:video".to_string(),
                model_name: self
                    .model
                    .clone()
                    .or_else(|| self.transport.model_name.clone()),
                proxy: self.transport.proxy.clone(),
                tls_profile: self.transport.tls_profile.clone(),
                timeouts: self.transport.timeouts.clone(),
            },
            report_kind: Some("openai_video_remix_sync_finalize".to_string()),
            report_context: Some(report_context),
        })
    }
}
