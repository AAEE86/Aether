use serde_json::{Map, Value};

use super::{
    GeminiVideoTaskSeed, LocalVideoTaskReadResponse, LocalVideoTaskRegistryMutation,
    LocalVideoTaskSnapshot, OpenAiVideoTaskSeed,
};

mod backend;
mod registry;

pub(crate) use self::backend::{FileVideoTaskStore, InMemoryVideoTaskStore};
use self::registry::VideoTaskRegistry;

pub(super) trait VideoTaskStore: std::fmt::Debug + Send + Sync {
    fn insert(&self, snapshot: LocalVideoTaskSnapshot);
    fn read_openai(&self, task_id: &str) -> Option<LocalVideoTaskReadResponse>;
    fn read_gemini(&self, short_id: &str) -> Option<LocalVideoTaskReadResponse>;
    fn clone_openai(&self, task_id: &str) -> Option<OpenAiVideoTaskSeed>;
    fn clone_gemini(&self, short_id: &str) -> Option<GeminiVideoTaskSeed>;
    #[cfg(test)]
    fn list_active_snapshots(&self, limit: usize) -> Vec<LocalVideoTaskSnapshot>;
    fn apply_mutation(&self, mutation: LocalVideoTaskRegistryMutation);
    fn project_openai(&self, task_id: &str, provider_body: &Map<String, Value>) -> bool;
    fn project_gemini(&self, short_id: &str, provider_body: &Map<String, Value>) -> bool;
}
