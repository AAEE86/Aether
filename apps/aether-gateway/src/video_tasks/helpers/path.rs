use std::time::{SystemTime, UNIX_EPOCH};

use aether_data::repository::video_tasks::VideoTaskStatus as StoredVideoTaskStatus;
use uuid::Uuid;

use super::super::{
    LocalVideoTaskRegistryMutation, LocalVideoTaskStatus, VideoTaskTruthSourceMode,
};

pub(crate) fn extract_openai_task_id_from_path(path: &str) -> Option<&str> {
    let suffix = path.strip_prefix("/v1/videos/")?;
    if suffix.is_empty()
        || suffix.contains('/')
        || suffix.ends_with(":cancel")
        || suffix.ends_with(":delete")
    {
        return None;
    }
    Some(suffix)
}

pub(crate) fn extract_gemini_short_id_from_path(path: &str) -> Option<&str> {
    let operations_index = path.find("/operations/")?;
    let suffix = &path[(operations_index + "/operations/".len())..];
    if suffix.is_empty() || suffix.contains('/') || suffix.ends_with(":cancel") {
        return None;
    }
    Some(suffix)
}

pub(crate) fn extract_openai_task_id_from_cancel_path(path: &str) -> Option<&str> {
    let suffix = path.strip_prefix("/v1/videos/")?;
    suffix
        .strip_suffix("/cancel")
        .filter(|value| !value.is_empty())
}

pub(crate) fn extract_openai_task_id_from_remix_path(path: &str) -> Option<&str> {
    let suffix = path.strip_prefix("/v1/videos/")?;
    suffix
        .strip_suffix("/remix")
        .filter(|value| !value.is_empty())
}

pub(crate) fn extract_openai_task_id_from_content_path(path: &str) -> Option<&str> {
    let suffix = path.strip_prefix("/v1/videos/")?;
    suffix
        .strip_suffix("/content")
        .filter(|value| !value.is_empty())
}

pub(crate) fn extract_gemini_short_id_from_cancel_path(path: &str) -> Option<&str> {
    let operations_index = path.find("/operations/")?;
    let suffix = &path[(operations_index + "/operations/".len())..];
    let short_id = suffix.strip_suffix(":cancel")?;
    if short_id.is_empty() || short_id.contains('/') {
        return None;
    }
    Some(short_id)
}

pub(crate) fn resolve_local_video_registry_mutation(
    truth_source_mode: VideoTaskTruthSourceMode,
    request_path: &str,
    report_kind: &str,
) -> Option<LocalVideoTaskRegistryMutation> {
    if truth_source_mode != VideoTaskTruthSourceMode::RustAuthoritative {
        return None;
    }

    match report_kind {
        "openai_video_delete_sync_finalize" => {
            let task_id = extract_openai_task_id_from_path(request_path)?;
            Some(LocalVideoTaskRegistryMutation::OpenAiDeleted {
                task_id: task_id.to_string(),
            })
        }
        "openai_video_cancel_sync_finalize" => {
            let task_id = extract_openai_task_id_from_cancel_path(request_path)?;
            Some(LocalVideoTaskRegistryMutation::OpenAiCancelled {
                task_id: task_id.to_string(),
            })
        }
        "gemini_video_cancel_sync_finalize" => {
            let short_id = extract_gemini_short_id_from_cancel_path(request_path)?;
            Some(LocalVideoTaskRegistryMutation::GeminiCancelled {
                short_id: short_id.to_string(),
            })
        }
        _ => None,
    }
}

pub(crate) fn current_unix_timestamp_secs() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}

pub(crate) fn local_status_from_stored(status: StoredVideoTaskStatus) -> LocalVideoTaskStatus {
    match status {
        StoredVideoTaskStatus::Pending | StoredVideoTaskStatus::Submitted => {
            LocalVideoTaskStatus::Submitted
        }
        StoredVideoTaskStatus::Queued => LocalVideoTaskStatus::Queued,
        StoredVideoTaskStatus::Processing => LocalVideoTaskStatus::Processing,
        StoredVideoTaskStatus::Completed => LocalVideoTaskStatus::Completed,
        StoredVideoTaskStatus::Failed => LocalVideoTaskStatus::Failed,
        StoredVideoTaskStatus::Cancelled => LocalVideoTaskStatus::Cancelled,
        StoredVideoTaskStatus::Expired => LocalVideoTaskStatus::Expired,
        StoredVideoTaskStatus::Deleted => LocalVideoTaskStatus::Deleted,
    }
}

pub(crate) fn generate_local_short_id() -> String {
    Uuid::new_v4()
        .simple()
        .to_string()
        .chars()
        .take(12)
        .collect()
}
