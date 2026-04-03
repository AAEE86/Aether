use super::super::{
    extract_gemini_short_id_from_path, extract_openai_task_id_from_content_path,
    extract_openai_task_id_from_path, LocalVideoTaskContentAction, LocalVideoTaskReadResponse,
    LocalVideoTaskSnapshot, VideoTaskTruthSourceMode,
};

use super::VideoTaskService;

impl VideoTaskService {
    pub(crate) fn read_response(
        &self,
        route_family: Option<&str>,
        request_path: &str,
    ) -> Option<LocalVideoTaskReadResponse> {
        if self.truth_source_mode != VideoTaskTruthSourceMode::RustAuthoritative {
            return None;
        }
        match route_family {
            Some("openai") => extract_openai_task_id_from_path(request_path)
                .and_then(|task_id| self.store.read_openai(task_id)),
            Some("gemini") => extract_gemini_short_id_from_path(request_path)
                .and_then(|short_id| self.store.read_gemini(short_id)),
            _ => None,
        }
    }

    pub(crate) fn snapshot_for_route(
        &self,
        route_family: Option<&str>,
        request_path: &str,
    ) -> Option<LocalVideoTaskSnapshot> {
        match route_family {
            Some("openai") => extract_openai_task_id_from_path(request_path)
                .and_then(|task_id| self.store.clone_openai(task_id))
                .map(LocalVideoTaskSnapshot::OpenAi),
            Some("gemini") => extract_gemini_short_id_from_path(request_path)
                .and_then(|short_id| self.store.clone_gemini(short_id))
                .map(LocalVideoTaskSnapshot::Gemini),
            _ => None,
        }
    }

    pub(crate) fn prepare_openai_content_stream_action(
        &self,
        request_path: &str,
        query_string: Option<&str>,
        trace_id: &str,
    ) -> Option<LocalVideoTaskContentAction> {
        if self.truth_source_mode != VideoTaskTruthSourceMode::RustAuthoritative {
            return None;
        }
        let task_id = extract_openai_task_id_from_content_path(request_path)?;
        let seed = self.store.clone_openai(task_id)?;
        seed.build_content_stream_action(query_string, trace_id)
    }
}
