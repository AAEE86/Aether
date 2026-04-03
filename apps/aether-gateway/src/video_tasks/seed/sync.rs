use aether_contracts::ExecutionPlan;
use serde_json::{json, Map, Value};
use uuid::Uuid;

use super::super::{
    context_text, context_u64, current_unix_timestamp_secs, generate_local_short_id,
    request_body_text, GeminiVideoTaskSeed, LocalVideoTaskPersistence, LocalVideoTaskSeed,
    LocalVideoTaskSnapshot, LocalVideoTaskStatus, LocalVideoTaskSuccessPlan,
    LocalVideoTaskTransport, OpenAiVideoTaskSeed, VideoTaskSyncReportMode,
    VideoTaskTruthSourceMode,
};

impl LocalVideoTaskSeed {
    pub(crate) fn from_sync_finalize(
        report_kind: &str,
        provider_body: &Map<String, Value>,
        report_context: &Map<String, Value>,
        plan: &ExecutionPlan,
    ) -> Option<Self> {
        let transport = LocalVideoTaskTransport::from_plan(plan)?;
        let persistence = LocalVideoTaskPersistence::from_report_context(report_context, plan);
        match report_kind {
            "openai_video_create_sync_finalize" => {
                let upstream_id = provider_body.get("id").and_then(Value::as_str)?.trim();
                if upstream_id.is_empty() {
                    return None;
                }

                Some(Self::OpenAiCreate(OpenAiVideoTaskSeed {
                    local_task_id: context_text(report_context, "local_task_id")
                        .unwrap_or_else(|| Uuid::new_v4().to_string()),
                    upstream_task_id: upstream_id.to_string(),
                    created_at_unix_secs: context_u64(report_context, "local_created_at")
                        .unwrap_or_else(current_unix_timestamp_secs),
                    user_id: context_text(report_context, "user_id"),
                    api_key_id: context_text(report_context, "api_key_id"),
                    model: context_text(report_context, "model")
                        .or_else(|| request_body_text(report_context, "model")),
                    prompt: request_body_text(report_context, "prompt"),
                    size: request_body_text(report_context, "size"),
                    seconds: request_body_text(report_context, "seconds"),
                    remixed_from_video_id: None,
                    status: LocalVideoTaskStatus::Submitted,
                    progress_percent: 0,
                    completed_at_unix_secs: None,
                    expires_at_unix_secs: None,
                    error_code: None,
                    error_message: None,
                    video_url: None,
                    persistence: persistence.clone(),
                    transport: transport.clone(),
                }))
            }
            "openai_video_remix_sync_finalize" => {
                let upstream_id = provider_body.get("id").and_then(Value::as_str)?.trim();
                if upstream_id.is_empty() {
                    return None;
                }

                Some(Self::OpenAiRemix(OpenAiVideoTaskSeed {
                    local_task_id: context_text(report_context, "local_task_id")
                        .unwrap_or_else(|| Uuid::new_v4().to_string()),
                    upstream_task_id: upstream_id.to_string(),
                    created_at_unix_secs: context_u64(report_context, "local_created_at")
                        .unwrap_or_else(current_unix_timestamp_secs),
                    user_id: context_text(report_context, "user_id"),
                    api_key_id: context_text(report_context, "api_key_id"),
                    model: context_text(report_context, "model")
                        .or_else(|| request_body_text(report_context, "model")),
                    prompt: request_body_text(report_context, "prompt"),
                    size: request_body_text(report_context, "size"),
                    seconds: request_body_text(report_context, "seconds"),
                    remixed_from_video_id: context_text(report_context, "task_id")
                        .or_else(|| request_body_text(report_context, "remix_video_id")),
                    status: LocalVideoTaskStatus::Submitted,
                    progress_percent: 0,
                    completed_at_unix_secs: None,
                    expires_at_unix_secs: None,
                    error_code: None,
                    error_message: None,
                    video_url: None,
                    persistence: persistence.clone(),
                    transport: transport.clone(),
                }))
            }
            "gemini_video_create_sync_finalize" => {
                let operation_name = provider_body
                    .get("name")
                    .or_else(|| provider_body.get("id"))
                    .and_then(Value::as_str)
                    .map(str::trim)
                    .filter(|value| !value.is_empty());
                operation_name?;

                Some(Self::GeminiCreate(GeminiVideoTaskSeed {
                    local_short_id: context_text(report_context, "local_short_id")
                        .unwrap_or_else(generate_local_short_id),
                    upstream_operation_name: operation_name?.to_string(),
                    user_id: context_text(report_context, "user_id"),
                    api_key_id: context_text(report_context, "api_key_id"),
                    model: context_text(report_context, "model")
                        .or_else(|| context_text(report_context, "model_name"))
                        .unwrap_or_else(|| "unknown".to_string()),
                    status: LocalVideoTaskStatus::Submitted,
                    progress_percent: 0,
                    error_code: None,
                    error_message: None,
                    metadata: json!({}),
                    persistence,
                    transport,
                }))
            }
            _ => None,
        }
    }

    pub(crate) fn success_report_kind(&self) -> &'static str {
        match self {
            Self::OpenAiCreate(_) => "openai_video_create_sync_success",
            Self::OpenAiRemix(_) => "openai_video_remix_sync_success",
            Self::GeminiCreate(_) => "gemini_video_create_sync_success",
        }
    }

    pub(crate) fn apply_to_report_context(&self, report_context: &mut Map<String, Value>) {
        match self {
            Self::OpenAiCreate(seed) | Self::OpenAiRemix(seed) => {
                report_context.insert(
                    "local_task_id".to_string(),
                    Value::String(seed.local_task_id.clone()),
                );
                report_context.insert(
                    "local_created_at".to_string(),
                    Value::Number(seed.created_at_unix_secs.into()),
                );
            }
            Self::GeminiCreate(seed) => {
                report_context.insert(
                    "local_short_id".to_string(),
                    Value::String(seed.local_short_id.clone()),
                );
            }
        }
    }

    pub(crate) fn client_body_json(&self) -> Value {
        match self {
            Self::OpenAiCreate(seed) | Self::OpenAiRemix(seed) => seed.client_body_json(),
            Self::GeminiCreate(seed) => seed.client_body_json(),
        }
    }
}

impl VideoTaskTruthSourceMode {
    pub(crate) fn prepare_sync_success(
        self,
        report_kind: &str,
        provider_body: &Map<String, Value>,
        report_context: &Map<String, Value>,
        plan: &ExecutionPlan,
    ) -> Option<LocalVideoTaskSuccessPlan> {
        let seed = LocalVideoTaskSeed::from_sync_finalize(
            report_kind,
            provider_body,
            report_context,
            plan,
        )?;
        let report_mode = match self {
            Self::PythonSyncReport => VideoTaskSyncReportMode::InlineSync,
            Self::RustAuthoritative => VideoTaskSyncReportMode::Background,
        };
        Some(LocalVideoTaskSuccessPlan { seed, report_mode })
    }
}

impl LocalVideoTaskSuccessPlan {
    pub(crate) fn success_report_kind(&self) -> &'static str {
        self.seed.success_report_kind()
    }

    pub(crate) fn report_mode(&self) -> VideoTaskSyncReportMode {
        self.report_mode
    }

    pub(crate) fn apply_to_report_context(&self, report_context: &mut Map<String, Value>) {
        self.seed.apply_to_report_context(report_context);
        if matches!(self.report_mode, VideoTaskSyncReportMode::Background) {
            report_context.insert("rust_video_task_persisted".to_string(), Value::Bool(true));
        }
    }

    pub(crate) fn client_body_json(&self) -> Value {
        self.seed.client_body_json()
    }

    pub(crate) fn to_snapshot(&self) -> LocalVideoTaskSnapshot {
        match &self.seed {
            LocalVideoTaskSeed::OpenAiCreate(seed) | LocalVideoTaskSeed::OpenAiRemix(seed) => {
                LocalVideoTaskSnapshot::OpenAi(seed.clone())
            }
            LocalVideoTaskSeed::GeminiCreate(seed) => LocalVideoTaskSnapshot::Gemini(seed.clone()),
        }
    }
}
