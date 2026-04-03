use std::time::{SystemTime, UNIX_EPOCH};

use aether_contracts::{ExecutionError, ExecutionPlan};
use aether_data::repository::candidates::{
    RequestCandidateStatus, StoredRequestCandidate, UpsertRequestCandidateRecord,
};
use serde_json::Value;
use tracing::warn;
use uuid::Uuid;

use crate::gateway::AppState;

pub(crate) async fn record_local_request_candidate_status(
    state: &AppState,
    plan: &ExecutionPlan,
    report_context: Option<&Value>,
    status: RequestCandidateStatus,
    status_code: Option<u16>,
    error_type: Option<String>,
    error_message: Option<String>,
    latency_ms: Option<u64>,
    started_at_unix_secs: Option<u64>,
    finished_at_unix_secs: Option<u64>,
) {
    let Some(candidate_id) = plan
        .candidate_id
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
    else {
        return;
    };
    let Some(metadata) = parse_report_context(report_context) else {
        return;
    };
    let Some(candidate_index) = metadata.candidate_index else {
        return;
    };

    if let Err(err) = state
        .upsert_request_candidate(UpsertRequestCandidateRecord {
            id: candidate_id.to_string(),
            request_id: plan.request_id.clone(),
            user_id: metadata.user_id,
            api_key_id: metadata.api_key_id,
            username: None,
            api_key_name: None,
            candidate_index,
            retry_index: metadata.retry_index,
            provider_id: Some(plan.provider_id.clone()),
            endpoint_id: Some(plan.endpoint_id.clone()),
            key_id: Some(plan.key_id.clone()),
            status,
            skip_reason: None,
            is_cached: None,
            status_code,
            error_type,
            error_message,
            latency_ms,
            concurrent_requests: None,
            extra_data: None,
            required_capabilities: None,
            created_at_unix_secs: None,
            started_at_unix_secs,
            finished_at_unix_secs,
        })
        .await
    {
        warn!(
            request_id = %plan.request_id,
            candidate_id = %candidate_id,
            error = ?err,
            "gateway failed to persist request candidate status update"
        );
    }
}

pub(crate) async fn record_report_request_candidate_status(
    state: &AppState,
    report_context: Option<&Value>,
    status: RequestCandidateStatus,
    status_code: Option<u16>,
    error_type: Option<String>,
    error_message: Option<String>,
    latency_ms: Option<u64>,
    started_at_unix_secs: Option<u64>,
    finished_at_unix_secs: Option<u64>,
) {
    let Some(slot) = resolve_report_request_candidate_slot(state, report_context).await else {
        return;
    };

    let terminal_unix_secs = finished_at_unix_secs.unwrap_or_else(current_unix_secs);
    let started_at_unix_secs = started_at_unix_secs
        .or(slot.started_at_unix_secs)
        .or_else(|| status.is_attempted(None).then_some(terminal_unix_secs));
    let finished_at_unix_secs = finished_at_unix_secs
        .or(slot.finished_at_unix_secs)
        .or_else(|| is_terminal_candidate_status(status).then_some(terminal_unix_secs));

    if let Err(err) = state
        .upsert_request_candidate(UpsertRequestCandidateRecord {
            id: slot.id,
            request_id: slot.request_id.clone(),
            user_id: slot.user_id,
            api_key_id: slot.api_key_id,
            username: None,
            api_key_name: None,
            candidate_index: slot.candidate_index,
            retry_index: slot.retry_index,
            provider_id: slot.provider_id,
            endpoint_id: slot.endpoint_id,
            key_id: slot.key_id,
            status,
            skip_reason: None,
            is_cached: None,
            status_code,
            error_type,
            error_message,
            latency_ms,
            concurrent_requests: None,
            extra_data: slot.extra_data,
            required_capabilities: None,
            created_at_unix_secs: Some(slot.created_at_unix_secs),
            started_at_unix_secs,
            finished_at_unix_secs,
        })
        .await
    {
        warn!(
            request_id = %slot.request_id,
            candidate_index = slot.candidate_index,
            retry_index = slot.retry_index,
            error = ?err,
            "gateway failed to persist report-driven request candidate status update"
        );
    }
}

pub(crate) async fn ensure_execution_request_candidate_slot(
    state: &AppState,
    plan: &mut ExecutionPlan,
    report_context: &mut Option<Value>,
) {
    if !state.has_request_candidate_data_writer() {
        return;
    }
    if plan
        .candidate_id
        .as_deref()
        .map(str::trim)
        .is_some_and(|value| !value.is_empty())
    {
        return;
    }

    let mut context = report_context
        .as_ref()
        .and_then(Value::as_object)
        .cloned()
        .unwrap_or_default();
    let request_id = context
        .get("request_id")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .unwrap_or(plan.request_id.as_str())
        .to_string();
    let candidate_index = context
        .get("candidate_index")
        .and_then(Value::as_u64)
        .and_then(|value| u32::try_from(value).ok())
        .unwrap_or(0);
    let retry_index = context
        .get("retry_index")
        .and_then(Value::as_u64)
        .and_then(|value| u32::try_from(value).ok())
        .unwrap_or(0);
    let generated_candidate_id = context
        .get("candidate_id")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(ToOwned::to_owned)
        .unwrap_or_else(|| Uuid::new_v4().to_string());
    let started_at_unix_secs = current_unix_secs();

    let candidate_id = match state
        .upsert_request_candidate(UpsertRequestCandidateRecord {
            id: generated_candidate_id.clone(),
            request_id: request_id.clone(),
            user_id: context
                .get("user_id")
                .and_then(Value::as_str)
                .map(str::trim)
                .filter(|value| !value.is_empty())
                .map(ToOwned::to_owned),
            api_key_id: context
                .get("api_key_id")
                .and_then(Value::as_str)
                .map(str::trim)
                .filter(|value| !value.is_empty())
                .map(ToOwned::to_owned),
            username: None,
            api_key_name: None,
            candidate_index,
            retry_index,
            provider_id: Some(plan.provider_id.clone()),
            endpoint_id: Some(plan.endpoint_id.clone()),
            key_id: Some(plan.key_id.clone()),
            status: RequestCandidateStatus::Pending,
            skip_reason: None,
            is_cached: Some(false),
            status_code: None,
            error_type: None,
            error_message: None,
            latency_ms: None,
            concurrent_requests: None,
            extra_data: None,
            required_capabilities: None,
            created_at_unix_secs: Some(started_at_unix_secs),
            started_at_unix_secs: Some(started_at_unix_secs),
            finished_at_unix_secs: None,
        })
        .await
    {
        Ok(Some(stored)) => stored.id,
        Ok(None) => generated_candidate_id,
        Err(err) => {
            warn!(
                request_id = %plan.request_id,
                error = ?err,
                "gateway failed to seed execution request candidate slot"
            );
            return;
        }
    };

    plan.candidate_id = Some(candidate_id.clone());
    context.insert("request_id".to_string(), Value::String(request_id));
    context.insert("candidate_id".to_string(), Value::String(candidate_id));
    context.insert(
        "candidate_index".to_string(),
        Value::Number(candidate_index.into()),
    );
    context.insert(
        "provider_id".to_string(),
        Value::String(plan.provider_id.clone()),
    );
    context.insert(
        "endpoint_id".to_string(),
        Value::String(plan.endpoint_id.clone()),
    );
    context.insert("key_id".to_string(), Value::String(plan.key_id.clone()));
    *report_context = Some(Value::Object(context));
}

pub(crate) fn execution_error_details(
    error: Option<&ExecutionError>,
    body_json: Option<&Value>,
) -> (Option<String>, Option<String>) {
    match error {
        Some(error) => (
            Some(format!("{:?}", error.kind)),
            Some(error.message.trim().to_string()).filter(|value| !value.is_empty()),
        ),
        None => (
            None,
            body_json
                .and_then(extract_error_message)
                .map(str::trim)
                .filter(|value| !value.is_empty())
                .map(ToOwned::to_owned),
        ),
    }
}

fn extract_error_message(body_json: &Value) -> Option<&str> {
    body_json
        .get("error")
        .and_then(|error| {
            error
                .get("message")
                .and_then(Value::as_str)
                .or_else(|| error.as_str())
        })
        .or_else(|| body_json.get("message").and_then(Value::as_str))
}

pub(crate) fn current_unix_secs() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}

#[derive(Debug, Clone)]
struct RequestCandidateReportContext {
    request_id: Option<String>,
    candidate_id: Option<String>,
    user_id: Option<String>,
    api_key_id: Option<String>,
    candidate_index: Option<u32>,
    retry_index: u32,
    provider_id: Option<String>,
    endpoint_id: Option<String>,
    key_id: Option<String>,
    client_api_format: Option<String>,
    provider_api_format: Option<String>,
}

fn parse_report_context(report_context: Option<&Value>) -> Option<RequestCandidateReportContext> {
    let report_context = report_context?;
    let retry_index = report_context
        .get("retry_index")
        .and_then(|value| value.as_u64())
        .unwrap_or_default();
    Some(RequestCandidateReportContext {
        request_id: report_context
            .get("request_id")
            .and_then(|value| value.as_str())
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(ToOwned::to_owned),
        candidate_id: report_context
            .get("candidate_id")
            .and_then(|value| value.as_str())
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(ToOwned::to_owned),
        user_id: report_context
            .get("user_id")
            .and_then(|value| value.as_str())
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(ToOwned::to_owned),
        api_key_id: report_context
            .get("api_key_id")
            .and_then(|value| value.as_str())
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(ToOwned::to_owned),
        candidate_index: report_context
            .get("candidate_index")
            .and_then(|value| value.as_u64())
            .map(|value| value as u32),
        retry_index: retry_index as u32,
        provider_id: report_context
            .get("provider_id")
            .and_then(|value| value.as_str())
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(ToOwned::to_owned),
        endpoint_id: report_context
            .get("endpoint_id")
            .and_then(|value| value.as_str())
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(ToOwned::to_owned),
        key_id: report_context
            .get("key_id")
            .and_then(|value| value.as_str())
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(ToOwned::to_owned),
        client_api_format: report_context
            .get("client_api_format")
            .and_then(|value| value.as_str())
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(ToOwned::to_owned),
        provider_api_format: report_context
            .get("provider_api_format")
            .and_then(|value| value.as_str())
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(ToOwned::to_owned),
    })
}

#[derive(Debug, Clone)]
struct ResolvedReportRequestCandidateSlot {
    id: String,
    request_id: String,
    user_id: Option<String>,
    api_key_id: Option<String>,
    candidate_index: u32,
    retry_index: u32,
    provider_id: Option<String>,
    endpoint_id: Option<String>,
    key_id: Option<String>,
    extra_data: Option<Value>,
    created_at_unix_secs: u64,
    started_at_unix_secs: Option<u64>,
    finished_at_unix_secs: Option<u64>,
}

async fn resolve_report_request_candidate_slot(
    state: &AppState,
    report_context: Option<&Value>,
) -> Option<ResolvedReportRequestCandidateSlot> {
    let metadata = parse_report_context(report_context)?;
    let request_id = metadata.request_id.clone()?;
    let existing_candidates = state
        .read_request_candidates_by_request_id(request_id.as_str())
        .await
        .ok()
        .unwrap_or_default();
    let matched_candidate = match_existing_report_candidate(&existing_candidates, &metadata);
    let synthesized_extra_data = build_report_candidate_extra_data(&metadata);
    let created_at_unix_secs = matched_candidate
        .as_ref()
        .map(|candidate| candidate.created_at_unix_secs)
        .unwrap_or_else(current_unix_secs);
    let candidate_index = matched_candidate
        .as_ref()
        .map(|candidate| candidate.candidate_index)
        .or(metadata.candidate_index)
        .unwrap_or_else(|| next_candidate_index(&existing_candidates));
    let retry_index = matched_candidate
        .as_ref()
        .map(|candidate| candidate.retry_index)
        .unwrap_or(metadata.retry_index);

    Some(ResolvedReportRequestCandidateSlot {
        id: matched_candidate
            .as_ref()
            .map(|candidate| candidate.id.clone())
            .or(metadata.candidate_id)
            .unwrap_or_else(|| Uuid::new_v4().to_string()),
        request_id,
        user_id: matched_candidate
            .as_ref()
            .and_then(|candidate| candidate.user_id.clone())
            .or(metadata.user_id),
        api_key_id: matched_candidate
            .as_ref()
            .and_then(|candidate| candidate.api_key_id.clone())
            .or(metadata.api_key_id),
        candidate_index,
        retry_index,
        provider_id: matched_candidate
            .as_ref()
            .and_then(|candidate| candidate.provider_id.clone())
            .or(metadata.provider_id),
        endpoint_id: matched_candidate
            .as_ref()
            .and_then(|candidate| candidate.endpoint_id.clone())
            .or(metadata.endpoint_id),
        key_id: matched_candidate
            .as_ref()
            .and_then(|candidate| candidate.key_id.clone())
            .or(metadata.key_id),
        extra_data: matched_candidate
            .as_ref()
            .and_then(|candidate| candidate.extra_data.clone())
            .or(synthesized_extra_data),
        created_at_unix_secs,
        started_at_unix_secs: matched_candidate
            .as_ref()
            .and_then(|candidate| candidate.started_at_unix_secs),
        finished_at_unix_secs: matched_candidate
            .as_ref()
            .and_then(|candidate| candidate.finished_at_unix_secs),
    })
}

fn match_existing_report_candidate<'a>(
    candidates: &'a [StoredRequestCandidate],
    metadata: &RequestCandidateReportContext,
) -> Option<&'a StoredRequestCandidate> {
    if let Some(candidate_id) = metadata.candidate_id.as_deref() {
        if let Some(candidate) = candidates
            .iter()
            .find(|candidate| candidate.id == candidate_id)
        {
            return Some(candidate);
        }
    }

    if let Some(candidate_index) = metadata.candidate_index {
        if let Some(candidate) = candidates.iter().find(|candidate| {
            candidate.candidate_index == candidate_index
                && candidate.retry_index == metadata.retry_index
        }) {
            return Some(candidate);
        }
    }

    candidates
        .iter()
        .filter(|candidate| {
            candidate.provider_id.as_deref() == metadata.provider_id.as_deref()
                && candidate.endpoint_id.as_deref() == metadata.endpoint_id.as_deref()
                && candidate.key_id.as_deref() == metadata.key_id.as_deref()
        })
        .max_by_key(|candidate| {
            (
                candidate.retry_index,
                candidate.candidate_index,
                candidate.created_at_unix_secs,
            )
        })
}

fn next_candidate_index(candidates: &[StoredRequestCandidate]) -> u32 {
    candidates
        .iter()
        .map(|candidate| candidate.candidate_index)
        .max()
        .map(|value| value.saturating_add(1))
        .unwrap_or_default()
}

fn build_report_candidate_extra_data(metadata: &RequestCandidateReportContext) -> Option<Value> {
    let mut extra_data = serde_json::Map::new();
    extra_data.insert("gateway_execution_runtime".to_string(), Value::Bool(true));
    extra_data.insert("phase".to_string(), Value::String("3c_trial".to_string()));
    if let Some(client_api_format) = metadata.client_api_format.clone() {
        extra_data.insert(
            "client_api_format".to_string(),
            Value::String(client_api_format),
        );
    }
    if let Some(provider_api_format) = metadata.provider_api_format.clone() {
        extra_data.insert(
            "provider_api_format".to_string(),
            Value::String(provider_api_format),
        );
    }
    (!extra_data.is_empty()).then_some(Value::Object(extra_data))
}

fn is_terminal_candidate_status(status: RequestCandidateStatus) -> bool {
    matches!(
        status,
        RequestCandidateStatus::Unused
            | RequestCandidateStatus::Success
            | RequestCandidateStatus::Failed
            | RequestCandidateStatus::Cancelled
            | RequestCandidateStatus::Skipped
    )
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;
    use std::sync::Arc;

    use aether_contracts::{ExecutionPlan, RequestBody};
    use aether_data::repository::candidates::{
        InMemoryRequestCandidateRepository, RequestCandidateReadRepository, RequestCandidateStatus,
    };
    use aether_data::repository::usage::InMemoryUsageReadRepository;
    use serde_json::json;

    use super::ensure_execution_request_candidate_slot;
    use crate::gateway::gateway_data::GatewayDataState;
    use crate::gateway::AppState;

    fn build_test_state(repository: Arc<InMemoryRequestCandidateRepository>) -> AppState {
        AppState::new()
            .expect("gateway state should build")
            .with_data_state_for_tests(
                GatewayDataState::with_request_candidate_and_usage_repository_for_tests(
                    repository,
                    Arc::new(InMemoryUsageReadRepository::default()),
                ),
            )
    }

    fn sample_plan() -> ExecutionPlan {
        ExecutionPlan {
            request_id: "req-request-candidate-seed-123".to_string(),
            candidate_id: None,
            provider_name: Some("openai".to_string()),
            provider_id: "provider-request-candidate-seed-123".to_string(),
            endpoint_id: "endpoint-request-candidate-seed-123".to_string(),
            key_id: "key-request-candidate-seed-123".to_string(),
            method: "POST".to_string(),
            url: "https://api.openai.example/v1/chat/completions".to_string(),
            headers: BTreeMap::new(),
            content_type: Some("application/json".to_string()),
            content_encoding: None,
            body: RequestBody::from_json(json!({"model": "gpt-5", "messages": []})),
            stream: false,
            client_api_format: "openai:chat".to_string(),
            provider_api_format: "openai:chat".to_string(),
            model_name: Some("gpt-5".to_string()),
            proxy: None,
            tls_profile: None,
            timeouts: None,
        }
    }

    #[tokio::test]
    async fn seeds_execution_request_candidate_slot_for_plan_without_candidate_id() {
        let repository = Arc::new(InMemoryRequestCandidateRepository::default());
        let state = build_test_state(Arc::clone(&repository));
        let mut plan = sample_plan();
        let mut report_context = Some(json!({
            "request_id": "req-request-candidate-seed-123",
            "client_api_format": "openai:chat"
        }));

        ensure_execution_request_candidate_slot(&state, &mut plan, &mut report_context).await;

        let candidate_id = plan
            .candidate_id
            .clone()
            .expect("candidate id should be seeded");
        let report_context = report_context.expect("report context should be populated");
        assert_eq!(
            report_context
                .get("candidate_id")
                .and_then(|value| value.as_str()),
            Some(candidate_id.as_str())
        );
        assert_eq!(
            report_context
                .get("candidate_index")
                .and_then(|value| value.as_u64()),
            Some(0)
        );
        assert_eq!(
            report_context
                .get("provider_id")
                .and_then(|value| value.as_str()),
            Some("provider-request-candidate-seed-123")
        );

        let stored = repository
            .list_by_request_id("req-request-candidate-seed-123")
            .await
            .expect("request candidates should read");
        assert_eq!(stored.len(), 1);
        assert_eq!(stored[0].id, candidate_id);
        assert_eq!(stored[0].status, RequestCandidateStatus::Pending);
        assert_eq!(
            stored[0].provider_id.as_deref(),
            Some("provider-request-candidate-seed-123")
        );
        assert_eq!(
            stored[0].endpoint_id.as_deref(),
            Some("endpoint-request-candidate-seed-123")
        );
        assert_eq!(
            stored[0].key_id.as_deref(),
            Some("key-request-candidate-seed-123")
        );
    }

    #[tokio::test]
    async fn does_not_reseed_execution_request_candidate_slot_when_plan_already_has_candidate_id() {
        let repository = Arc::new(InMemoryRequestCandidateRepository::default());
        let state = build_test_state(Arc::clone(&repository));
        let mut plan = sample_plan();
        plan.candidate_id = Some("cand-existing-123".to_string());
        let mut report_context = Some(json!({
            "request_id": "req-request-candidate-seed-123"
        }));

        ensure_execution_request_candidate_slot(&state, &mut plan, &mut report_context).await;

        assert_eq!(plan.candidate_id.as_deref(), Some("cand-existing-123"));
        let stored = repository
            .list_by_request_id("req-request-candidate-seed-123")
            .await
            .expect("request candidates should read");
        assert!(stored.is_empty());
        assert_eq!(
            report_context
                .as_ref()
                .and_then(|value| value.get("candidate_id"))
                .and_then(|value| value.as_str()),
            None
        );
    }
}
