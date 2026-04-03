use aether_contracts::ExecutionPlan;
use aether_data::repository::video_tasks::StoredVideoTask;
use serde_json::{Map, Value};

use super::super::{
    resolve_local_video_registry_mutation, LocalVideoTaskSnapshot, LocalVideoTaskSuccessPlan,
};
use super::VideoTaskService;

impl VideoTaskService {
    pub(crate) fn prepare_sync_success(
        &self,
        report_kind: &str,
        provider_body: &Map<String, Value>,
        report_context: &Map<String, Value>,
        plan: &ExecutionPlan,
    ) -> Option<LocalVideoTaskSuccessPlan> {
        self.truth_source_mode.prepare_sync_success(
            report_kind,
            provider_body,
            report_context,
            plan,
        )
    }

    pub(crate) fn record_snapshot(&self, snapshot: LocalVideoTaskSnapshot) {
        self.store.insert(snapshot);
    }

    pub(crate) fn hydrate_from_stored_task(&self, task: &StoredVideoTask) -> bool {
        let Some(snapshot) = LocalVideoTaskSnapshot::from_stored_task(task) else {
            return false;
        };
        self.store.insert(snapshot);
        true
    }

    pub(crate) fn apply_finalize_mutation(&self, request_path: &str, report_kind: &str) {
        let Some(mutation) = resolve_local_video_registry_mutation(
            self.truth_source_mode,
            request_path,
            report_kind,
        ) else {
            return;
        };
        self.store.apply_mutation(mutation);
    }
}
