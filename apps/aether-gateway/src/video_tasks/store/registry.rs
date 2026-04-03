use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

use super::super::{
    GeminiVideoTaskSeed, LocalVideoTaskReadResponse, LocalVideoTaskRegistryMutation,
    LocalVideoTaskSnapshot, LocalVideoTaskStatus, OpenAiVideoTaskSeed,
};

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub(super) struct VideoTaskRegistry {
    openai: BTreeMap<String, LocalVideoTaskSnapshot>,
    gemini: BTreeMap<String, LocalVideoTaskSnapshot>,
}

impl VideoTaskRegistry {
    pub(super) fn insert(&mut self, snapshot: LocalVideoTaskSnapshot) {
        match &snapshot {
            LocalVideoTaskSnapshot::OpenAi(seed) => {
                self.openai.insert(seed.local_task_id.clone(), snapshot);
            }
            LocalVideoTaskSnapshot::Gemini(seed) => {
                self.gemini.insert(seed.local_short_id.clone(), snapshot);
            }
        }
    }

    pub(super) fn read_openai(&self, task_id: &str) -> Option<LocalVideoTaskReadResponse> {
        self.openai
            .get(task_id)
            .map(LocalVideoTaskSnapshot::read_response)
    }

    pub(super) fn read_gemini(&self, short_id: &str) -> Option<LocalVideoTaskReadResponse> {
        self.gemini
            .get(short_id)
            .map(LocalVideoTaskSnapshot::read_response)
    }

    pub(super) fn clone_openai(&self, task_id: &str) -> Option<OpenAiVideoTaskSeed> {
        let LocalVideoTaskSnapshot::OpenAi(seed) = self.openai.get(task_id)?.clone() else {
            return None;
        };
        Some(seed)
    }

    pub(super) fn clone_gemini(&self, short_id: &str) -> Option<GeminiVideoTaskSeed> {
        let LocalVideoTaskSnapshot::Gemini(seed) = self.gemini.get(short_id)?.clone() else {
            return None;
        };
        Some(seed)
    }

    #[cfg(test)]
    pub(super) fn list_active_snapshots(&self, limit: usize) -> Vec<LocalVideoTaskSnapshot> {
        self.openai
            .values()
            .chain(self.gemini.values())
            .filter(|snapshot| snapshot.is_active_for_refresh())
            .take(limit)
            .cloned()
            .collect()
    }

    pub(super) fn apply_mutation(&mut self, mutation: LocalVideoTaskRegistryMutation) {
        match mutation {
            LocalVideoTaskRegistryMutation::OpenAiCancelled { task_id } => {
                if let Some(LocalVideoTaskSnapshot::OpenAi(seed)) = self.openai.get_mut(&task_id) {
                    seed.status = LocalVideoTaskStatus::Cancelled;
                }
            }
            LocalVideoTaskRegistryMutation::OpenAiDeleted { task_id } => {
                if let Some(LocalVideoTaskSnapshot::OpenAi(seed)) = self.openai.get_mut(&task_id) {
                    seed.status = LocalVideoTaskStatus::Deleted;
                }
            }
            LocalVideoTaskRegistryMutation::GeminiCancelled { short_id } => {
                if let Some(LocalVideoTaskSnapshot::Gemini(seed)) = self.gemini.get_mut(&short_id) {
                    seed.status = LocalVideoTaskStatus::Cancelled;
                }
            }
        }
    }

    #[allow(dead_code)]
    pub(super) fn project_openai(
        &mut self,
        task_id: &str,
        provider_body: &Map<String, Value>,
    ) -> bool {
        let Some(LocalVideoTaskSnapshot::OpenAi(seed)) = self.openai.get_mut(task_id) else {
            return false;
        };
        seed.apply_provider_body(provider_body);
        true
    }

    #[allow(dead_code)]
    pub(super) fn project_gemini(
        &mut self,
        short_id: &str,
        provider_body: &Map<String, Value>,
    ) -> bool {
        let Some(LocalVideoTaskSnapshot::Gemini(seed)) = self.gemini.get_mut(short_id) else {
            return false;
        };
        seed.apply_provider_body(provider_body);
        true
    }
}
