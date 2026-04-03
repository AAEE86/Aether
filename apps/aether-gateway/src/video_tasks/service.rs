use std::path::PathBuf;
use std::sync::Arc;

use super::store::{FileVideoTaskStore, InMemoryVideoTaskStore, VideoTaskStore};
use super::VideoTaskTruthSourceMode;

mod follow_up;
mod lifecycle;
mod read;
mod refresh;

#[derive(Debug)]
pub(crate) struct VideoTaskService {
    truth_source_mode: VideoTaskTruthSourceMode,
    store: Arc<dyn VideoTaskStore>,
}

impl VideoTaskService {
    pub(crate) fn new(mode: VideoTaskTruthSourceMode) -> Self {
        Self::with_store(mode, Arc::new(InMemoryVideoTaskStore::default()))
    }

    pub(crate) fn with_file_store(
        mode: VideoTaskTruthSourceMode,
        path: impl Into<PathBuf>,
    ) -> std::io::Result<Self> {
        Ok(Self::with_store(
            mode,
            Arc::new(FileVideoTaskStore::new(path)?),
        ))
    }

    fn with_store(mode: VideoTaskTruthSourceMode, store: Arc<dyn VideoTaskStore>) -> Self {
        Self {
            truth_source_mode: mode,
            store,
        }
    }

    pub(crate) fn with_truth_source_mode(&self, mode: VideoTaskTruthSourceMode) -> Self {
        Self {
            truth_source_mode: mode,
            store: self.store.clone(),
        }
    }

    pub(crate) fn is_rust_authoritative(&self) -> bool {
        self.truth_source_mode == VideoTaskTruthSourceMode::RustAuthoritative
    }

    pub(crate) fn truth_source_mode(&self) -> VideoTaskTruthSourceMode {
        self.truth_source_mode
    }
}
