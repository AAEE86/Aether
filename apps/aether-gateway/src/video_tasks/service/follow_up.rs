use serde_json::Value;

use super::super::{
    extract_gemini_short_id_from_cancel_path, extract_openai_task_id_from_cancel_path,
    extract_openai_task_id_from_path, extract_openai_task_id_from_remix_path,
    LocalVideoTaskFollowUpPlan,
};
use crate::gateway::GatewayControlAuthContext;

use super::VideoTaskService;

impl VideoTaskService {
    pub(crate) fn prepare_follow_up_sync_plan(
        &self,
        plan_kind: &str,
        request_path: &str,
        body_json: Option<&Value>,
        auth_context: Option<&GatewayControlAuthContext>,
        trace_id: &str,
    ) -> Option<LocalVideoTaskFollowUpPlan> {
        match plan_kind {
            "openai_video_remix_sync" => {
                let task_id = extract_openai_task_id_from_remix_path(request_path)?;
                let seed = self.store.clone_openai(task_id)?;
                seed.build_remix_follow_up_plan(body_json?, auth_context, trace_id)
            }
            "openai_video_delete_sync" => {
                let task_id = extract_openai_task_id_from_path(request_path)?;
                let seed = self.store.clone_openai(task_id)?;
                seed.build_delete_follow_up_plan(auth_context, trace_id)
            }
            "openai_video_cancel_sync" => {
                let task_id = extract_openai_task_id_from_cancel_path(request_path)?;
                let seed = self.store.clone_openai(task_id)?;
                seed.build_cancel_follow_up_plan(auth_context, trace_id)
            }
            "gemini_video_cancel_sync" => {
                let short_id = extract_gemini_short_id_from_cancel_path(request_path)?;
                let seed = self.store.clone_gemini(short_id)?;
                seed.build_cancel_follow_up_plan(auth_context, trace_id)
            }
            _ => None,
        }
    }
}
