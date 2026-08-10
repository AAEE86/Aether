use std::ops::Deref;
use std::path::PathBuf;

use crate::control::GatewayControlAuthContext;

use super::{LocalVideoTaskFollowUpPlan, VideoTaskTruthSourceMode};

#[derive(Debug)]
pub(crate) struct VideoTaskService(aether_video_tasks_core::VideoTaskService);

impl VideoTaskService {
    pub(crate) fn new(mode: VideoTaskTruthSourceMode) -> Self {
        Self(aether_video_tasks_core::VideoTaskService::new(mode))
    }

    pub(crate) fn with_file_store(
        mode: VideoTaskTruthSourceMode,
        path: impl Into<PathBuf>,
    ) -> std::io::Result<Self> {
        Ok(Self(
            aether_video_tasks_core::VideoTaskService::with_file_store(mode, path)?,
        ))
    }

    pub(crate) fn with_truth_source_mode(&self, mode: VideoTaskTruthSourceMode) -> Self {
        Self(self.0.with_truth_source_mode(mode))
    }

    pub(crate) fn prepare_follow_up_sync_plan(
        &self,
        plan_kind: &str,
        request_path: &str,
        body_json: Option<&serde_json::Value>,
        auth_context: Option<&GatewayControlAuthContext>,
        request_id: &str,
    ) -> Option<LocalVideoTaskFollowUpPlan> {
        self.0.prepare_follow_up_sync_plan(
            plan_kind,
            request_path,
            body_json,
            auth_context.map(|value| value.user_id.as_str()),
            auth_context.map(|value| value.api_key_id.as_str()),
            request_id,
        )
    }
}

impl Deref for VideoTaskService {
    type Target = aether_video_tasks_core::VideoTaskService;

    fn deref(&self) -> &Self::Target {
        &self.0
    }
}
