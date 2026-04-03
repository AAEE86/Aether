use std::collections::BTreeMap;

use aether_contracts::{ExecutionPlan, ExecutionTimeouts, ProxySnapshot};
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum VideoTaskSyncReportMode {
    InlineSync,
    Background,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum VideoTaskTruthSourceMode {
    #[default]
    PythonSyncReport,
    #[allow(dead_code)] // reserved for the next migration step when Rust owns video_tasks writes
    RustAuthoritative,
}

#[derive(Debug, Clone, PartialEq)]
pub(crate) struct LocalVideoTaskSuccessPlan {
    pub(super) seed: LocalVideoTaskSeed,
    pub(super) report_mode: VideoTaskSyncReportMode,
}

#[derive(Debug, Clone, PartialEq)]
pub(crate) struct LocalVideoTaskFollowUpPlan {
    pub(crate) plan: ExecutionPlan,
    pub(crate) report_kind: Option<String>,
    pub(crate) report_context: Option<Value>,
}

#[derive(Debug, Clone, PartialEq)]
pub(crate) struct LocalVideoTaskReadRefreshPlan {
    pub(crate) plan: ExecutionPlan,
    pub(super) projection_target: LocalVideoTaskProjectionTarget,
}

#[derive(Debug, Clone, PartialEq)]
pub(crate) enum LocalVideoTaskContentAction {
    Immediate { status_code: u16, body_json: Value },
    StreamPlan(ExecutionPlan),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) enum LocalVideoTaskProjectionTarget {
    OpenAi { task_id: String },
    Gemini { short_id: String },
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub(crate) enum LocalVideoTaskSnapshot {
    OpenAi(OpenAiVideoTaskSeed),
    Gemini(GeminiVideoTaskSeed),
}

#[allow(dead_code)] // projection states are staged for upcoming poller/store wiring
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) enum LocalVideoTaskStatus {
    Submitted,
    Queued,
    Processing,
    Completed,
    Failed,
    Cancelled,
    Expired,
    Deleted,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct LocalVideoTaskReadResponse {
    pub(crate) status_code: u16,
    pub(crate) body_json: Value,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum LocalVideoTaskRegistryMutation {
    OpenAiCancelled { task_id: String },
    OpenAiDeleted { task_id: String },
    GeminiCancelled { short_id: String },
}

#[derive(Debug, Clone, PartialEq)]
pub(crate) enum LocalVideoTaskSeed {
    OpenAiCreate(OpenAiVideoTaskSeed),
    OpenAiRemix(OpenAiVideoTaskSeed),
    GeminiCreate(GeminiVideoTaskSeed),
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub(crate) struct LocalVideoTaskTransport {
    pub(super) upstream_base_url: String,
    pub(super) provider_name: Option<String>,
    pub(super) provider_id: String,
    pub(super) endpoint_id: String,
    pub(super) key_id: String,
    pub(super) headers: BTreeMap<String, String>,
    pub(super) content_type: Option<String>,
    pub(super) model_name: Option<String>,
    pub(super) proxy: Option<ProxySnapshot>,
    pub(super) tls_profile: Option<String>,
    pub(super) timeouts: Option<ExecutionTimeouts>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub(crate) struct LocalVideoTaskPersistence {
    pub(super) request_id: String,
    pub(super) username: Option<String>,
    pub(super) api_key_name: Option<String>,
    pub(super) client_api_format: String,
    pub(super) provider_api_format: String,
    pub(super) original_request_body: Value,
    pub(super) format_converted: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub(crate) struct OpenAiVideoTaskSeed {
    pub(super) local_task_id: String,
    pub(super) upstream_task_id: String,
    pub(super) created_at_unix_secs: u64,
    pub(super) user_id: Option<String>,
    pub(super) api_key_id: Option<String>,
    pub(super) model: Option<String>,
    pub(super) prompt: Option<String>,
    pub(super) size: Option<String>,
    pub(super) seconds: Option<String>,
    pub(super) remixed_from_video_id: Option<String>,
    pub(super) status: LocalVideoTaskStatus,
    pub(super) progress_percent: u16,
    pub(super) completed_at_unix_secs: Option<u64>,
    pub(super) expires_at_unix_secs: Option<u64>,
    pub(super) error_code: Option<String>,
    pub(super) error_message: Option<String>,
    pub(super) video_url: Option<String>,
    pub(super) persistence: LocalVideoTaskPersistence,
    pub(super) transport: LocalVideoTaskTransport,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub(crate) struct GeminiVideoTaskSeed {
    pub(super) local_short_id: String,
    pub(super) upstream_operation_name: String,
    pub(super) user_id: Option<String>,
    pub(super) api_key_id: Option<String>,
    pub(super) model: String,
    pub(super) status: LocalVideoTaskStatus,
    pub(super) progress_percent: u16,
    pub(super) error_code: Option<String>,
    pub(super) error_message: Option<String>,
    pub(super) metadata: Value,
    pub(super) persistence: LocalVideoTaskPersistence,
    pub(super) transport: LocalVideoTaskTransport,
}
