use aether_data::repository::video_tasks::StoredVideoTask;
use serde_json::{Map, Value};

use super::super::{
    extract_gemini_short_id_from_path, extract_openai_task_id_from_path,
    LocalVideoTaskProjectionTarget, LocalVideoTaskReadRefreshPlan, LocalVideoTaskSnapshot,
    VideoTaskTruthSourceMode,
};

use super::VideoTaskService;

impl VideoTaskService {
    pub(crate) fn snapshot_for_refresh_plan(
        &self,
        refresh_plan: &LocalVideoTaskReadRefreshPlan,
    ) -> Option<LocalVideoTaskSnapshot> {
        match &refresh_plan.projection_target {
            LocalVideoTaskProjectionTarget::OpenAi { task_id } => self
                .store
                .clone_openai(task_id)
                .map(LocalVideoTaskSnapshot::OpenAi),
            LocalVideoTaskProjectionTarget::Gemini { short_id } => self
                .store
                .clone_gemini(short_id)
                .map(LocalVideoTaskSnapshot::Gemini),
        }
    }

    pub(crate) fn project_openai_task_response(
        &self,
        task_id: &str,
        provider_body: &Map<String, Value>,
    ) -> bool {
        if self.truth_source_mode != VideoTaskTruthSourceMode::RustAuthoritative {
            return false;
        }
        self.store.project_openai(task_id, provider_body)
    }

    pub(crate) fn project_gemini_task_response(
        &self,
        short_id: &str,
        provider_body: &Map<String, Value>,
    ) -> bool {
        if self.truth_source_mode != VideoTaskTruthSourceMode::RustAuthoritative {
            return false;
        }
        self.store.project_gemini(short_id, provider_body)
    }

    pub(crate) fn prepare_read_refresh_sync_plan(
        &self,
        route_family: Option<&str>,
        request_path: &str,
        trace_id: &str,
    ) -> Option<LocalVideoTaskReadRefreshPlan> {
        if self.truth_source_mode != VideoTaskTruthSourceMode::RustAuthoritative {
            return None;
        }
        match route_family {
            Some("openai") => {
                let task_id = extract_openai_task_id_from_path(request_path)?;
                let seed = self.store.clone_openai(task_id)?;
                Some(LocalVideoTaskReadRefreshPlan {
                    plan: seed.build_get_follow_up_plan(trace_id)?,
                    projection_target: LocalVideoTaskProjectionTarget::OpenAi {
                        task_id: task_id.to_string(),
                    },
                })
            }
            Some("gemini") => {
                let short_id = extract_gemini_short_id_from_path(request_path)?;
                let seed = self.store.clone_gemini(short_id)?;
                Some(LocalVideoTaskReadRefreshPlan {
                    plan: seed.build_get_follow_up_plan(trace_id)?,
                    projection_target: LocalVideoTaskProjectionTarget::Gemini {
                        short_id: short_id.to_string(),
                    },
                })
            }
            _ => None,
        }
    }

    #[cfg(test)]
    pub(crate) fn prepare_poll_refresh_batch(
        &self,
        limit: usize,
        trace_prefix: &str,
    ) -> Vec<LocalVideoTaskReadRefreshPlan> {
        if self.truth_source_mode != VideoTaskTruthSourceMode::RustAuthoritative || limit == 0 {
            return Vec::new();
        }

        self.store
            .list_active_snapshots(limit)
            .into_iter()
            .enumerate()
            .filter_map(|(index, snapshot)| {
                let trace_id = format!("{trace_prefix}-{index}");
                match snapshot {
                    LocalVideoTaskSnapshot::OpenAi(seed) => Some(LocalVideoTaskReadRefreshPlan {
                        plan: seed.build_get_follow_up_plan(&trace_id)?,
                        projection_target: LocalVideoTaskProjectionTarget::OpenAi {
                            task_id: seed.local_task_id.clone(),
                        },
                    }),
                    LocalVideoTaskSnapshot::Gemini(seed) => Some(LocalVideoTaskReadRefreshPlan {
                        plan: seed.build_get_follow_up_plan(&trace_id)?,
                        projection_target: LocalVideoTaskProjectionTarget::Gemini {
                            short_id: seed.local_short_id.clone(),
                        },
                    }),
                }
            })
            .collect()
    }

    pub(crate) fn prepare_poll_refresh_plan_for_stored_task(
        &self,
        task: &StoredVideoTask,
        trace_id: &str,
    ) -> Option<LocalVideoTaskReadRefreshPlan> {
        if self.truth_source_mode != VideoTaskTruthSourceMode::RustAuthoritative {
            return None;
        }

        let snapshot = LocalVideoTaskSnapshot::from_stored_task(task)?;
        match snapshot {
            LocalVideoTaskSnapshot::OpenAi(seed) => Some(LocalVideoTaskReadRefreshPlan {
                plan: seed.build_get_follow_up_plan(trace_id)?,
                projection_target: LocalVideoTaskProjectionTarget::OpenAi {
                    task_id: seed.local_task_id.clone(),
                },
            }),
            LocalVideoTaskSnapshot::Gemini(seed) => Some(LocalVideoTaskReadRefreshPlan {
                plan: seed.build_get_follow_up_plan(trace_id)?,
                projection_target: LocalVideoTaskProjectionTarget::Gemini {
                    short_id: seed.local_short_id.clone(),
                },
            }),
        }
    }

    pub(crate) fn apply_read_refresh_projection(
        &self,
        refresh_plan: &LocalVideoTaskReadRefreshPlan,
        provider_body: &Map<String, Value>,
    ) -> bool {
        match &refresh_plan.projection_target {
            LocalVideoTaskProjectionTarget::OpenAi { task_id } => {
                self.project_openai_task_response(task_id, provider_body)
            }
            LocalVideoTaskProjectionTarget::Gemini { short_id } => {
                self.project_gemini_task_response(short_id, provider_body)
            }
        }
    }
}
