use aether_contracts::ExecutionPlan;
use aether_data_contracts::repository::candidates::{
    RequestCandidateStatus, StoredRequestCandidate, UpsertRequestCandidateRecord,
};
use aether_data_contracts::repository::provider_catalog::{
    StoredProviderCatalogKey, StoredProviderCatalogProvider,
};
use aether_runtime_state::{
    ExecutionConcurrencyReservation, ExecutionReservationError, ExecutionReservationInput,
    ExecutionReservationPermit, ExecutionReservationScope, ExecutionRpmReservation,
};
use aether_scheduler_core::{
    build_execution_request_candidate_seed, build_local_request_candidate_status_record,
    build_report_request_candidate_status_record, count_recent_rpm_requests_for_provider_key_since,
    effective_provider_key_rpm_limit, finalize_execution_request_candidate_report_context,
    parse_request_candidate_report_context,
    resolve_report_request_candidate_slot as resolve_report_request_candidate_slot_from_candidates,
    LocalRequestCandidateStatusRecordInput, ReportRequestCandidateStatusRecordInput,
    SchedulerMinimalCandidateSelectionCandidate, SchedulerRequestCandidateStatusUpdate,
    SchedulerResolvedReportRequestCandidateSlot,
};
use aether_usage_runtime::build_locally_actionable_report_context_from_request_candidate;
use async_trait::async_trait;
use serde_json::Value;
use std::collections::BTreeSet;
use std::sync::{Arc, OnceLock};
use std::time::Duration;
use tracing::{debug, info, warn};
use uuid::Uuid;

use crate::clock::{current_unix_ms, current_unix_secs};
use crate::control::GatewayControlAuthContext;
use crate::execution_runtime::{
    current_stream_candidate_watchdog_progress, StreamCandidateCancellationCause,
    StreamCandidateWatchdogProgress, STREAM_CANDIDATE_WATCHDOG_TIMEOUT_ERROR_TYPE,
    STREAM_CANDIDATE_WATCHDOG_TIMEOUT_MESSAGE,
};
use crate::log_ids::short_request_id;
use crate::{AppState, GatewayError};

const REQUEST_CANDIDATE_PERSISTENCE_ENV: &str = "AETHER_GATEWAY_REQUEST_CANDIDATE_PERSISTENCE";
const REQUEST_CANDIDATE_SEED_WRITE_TIMEOUT_ENV: &str =
    "AETHER_GATEWAY_REQUEST_CANDIDATE_SEED_WRITE_TIMEOUT_MS";
const DEFAULT_REQUEST_CANDIDATE_SEED_WRITE_TIMEOUT_MS: u64 = 10;
const EXECUTION_RESERVATION_ACTIVE_WINDOW_SECS: u64 = 300;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum RequestCandidatePersistenceMode {
    Full,
    Terminal,
    None,
}

fn request_candidate_persistence_mode() -> RequestCandidatePersistenceMode {
    static MODE: OnceLock<RequestCandidatePersistenceMode> = OnceLock::new();
    *MODE.get_or_init(|| {
        match std::env::var(REQUEST_CANDIDATE_PERSISTENCE_ENV)
            .ok()
            .map(|value| value.trim().to_ascii_lowercase())
            .as_deref()
        {
            Some("terminal") | Some("final") | Some("final_only") | Some("final-only") => {
                RequestCandidatePersistenceMode::Terminal
            }
            Some("none") | Some("off") | Some("disabled") | Some("false") | Some("0") => {
                RequestCandidatePersistenceMode::None
            }
            _ => RequestCandidatePersistenceMode::Full,
        }
    })
}

fn request_candidate_status_is_terminal(status: RequestCandidateStatus) -> bool {
    matches!(
        status,
        RequestCandidateStatus::Success
            | RequestCandidateStatus::Failed
            | RequestCandidateStatus::Cancelled
            | RequestCandidateStatus::Skipped
    )
}

fn should_persist_request_candidate_status(status: RequestCandidateStatus) -> bool {
    match request_candidate_persistence_mode() {
        RequestCandidatePersistenceMode::Full => true,
        RequestCandidatePersistenceMode::Terminal => request_candidate_status_is_terminal(status),
        RequestCandidatePersistenceMode::None => false,
    }
}

fn request_candidate_seed_write_timeout() -> Duration {
    static TIMEOUT: OnceLock<Duration> = OnceLock::new();
    *TIMEOUT.get_or_init(|| {
        let millis = std::env::var(REQUEST_CANDIDATE_SEED_WRITE_TIMEOUT_ENV)
            .ok()
            .and_then(|value| value.trim().parse::<u64>().ok())
            .unwrap_or(DEFAULT_REQUEST_CANDIDATE_SEED_WRITE_TIMEOUT_MS);
        Duration::from_millis(millis)
    })
}

#[derive(Debug, Clone)]
pub(crate) struct LocalRequestCandidateStatusSnapshot {
    candidate_id: String,
    request_id: String,
    user_id: Option<String>,
    api_key_id: Option<String>,
    candidate_index: u32,
    retry_index: u32,
    provider_id: String,
    endpoint_id: String,
    key_id: String,
}

/// Synchronous identity plus the optional Pending write for a candidate slot.
/// The identity is installed into the plan before callers perform any await,
/// so a cancellation owner can be armed before persistence starts.
#[derive(Debug)]
pub(crate) struct PreparedExecutionRequestCandidateSlot {
    pending: Option<UpsertRequestCandidateRecord>,
}

#[cfg(test)]
impl PreparedExecutionRequestCandidateSlot {
    pub(crate) fn has_pending_write(&self) -> bool {
        self.pending.is_some()
    }
}

/// Owns a candidate between synchronous slot preparation and the attempt
/// lifecycle handoff. Dropping an armed guard makes the candidate terminal;
/// repository lifecycle rules and the async queue's terminal barrier prevent a
/// late Pending write from reviving it.
pub(crate) struct ExecutionRequestCandidateStartupGuard {
    state: AppState,
    snapshot: Option<LocalRequestCandidateStatusSnapshot>,
    mode: ExecutionRequestCandidateStartupMode,
}

enum ExecutionRequestCandidateStartupMode {
    ResponsesWebSocket,
    HttpStream {
        plan: ExecutionPlan,
        report_context: Option<Value>,
        watchdog_progress: Option<Arc<StreamCandidateWatchdogProgress>>,
    },
}

impl ExecutionRequestCandidateStartupGuard {
    pub(crate) fn new(
        state: &AppState,
        plan: &ExecutionPlan,
        report_context: Option<&Value>,
    ) -> Self {
        Self {
            state: state.clone(),
            snapshot: snapshot_local_request_candidate_status(plan, report_context),
            mode: ExecutionRequestCandidateStartupMode::ResponsesWebSocket,
        }
    }

    pub(crate) fn new_http_stream(
        state: &AppState,
        plan: &ExecutionPlan,
        report_context: Option<&Value>,
    ) -> Self {
        let watchdog_progress = current_stream_candidate_watchdog_progress();
        if let Some(progress) = watchdog_progress.as_ref() {
            // Startup is cancellable before the attempt guard exists. Arm the
            // shared watchdog ownership synchronously so a timeout is handed
            // to this guard instead of the candidate-only fallback.
            progress.mark_attempt_guard_armed();
        }
        Self {
            state: state.clone(),
            snapshot: snapshot_local_request_candidate_status(plan, report_context),
            mode: ExecutionRequestCandidateStartupMode::HttpStream {
                plan: plan.clone(),
                report_context: report_context.cloned(),
                watchdog_progress,
            },
        }
    }

    pub(crate) fn refresh(&mut self, plan: &ExecutionPlan, report_context: Option<&Value>) {
        self.snapshot = snapshot_local_request_candidate_status(plan, report_context);
        if let ExecutionRequestCandidateStartupMode::HttpStream {
            plan: owned_plan,
            report_context: owned_report_context,
            ..
        } = &mut self.mode
        {
            *owned_plan = plan.clone();
            *owned_report_context = report_context.cloned();
        }
    }

    pub(crate) fn disarm(mut self) {
        self.snapshot = None;
        self.mode = ExecutionRequestCandidateStartupMode::ResponsesWebSocket;
    }

    pub(crate) fn cancel(mut self) {
        self.terminalize();
    }

    fn terminalize(&mut self) {
        let mode = std::mem::replace(
            &mut self.mode,
            ExecutionRequestCandidateStartupMode::ResponsesWebSocket,
        );
        match mode {
            ExecutionRequestCandidateStartupMode::ResponsesWebSocket => {
                let Some(snapshot) = self.snapshot.take() else {
                    return;
                };
                let status_update = SchedulerRequestCandidateStatusUpdate {
                    status: RequestCandidateStatus::Cancelled,
                    status_code: Some(499),
                    error_type: Some("websocket_cancelled".to_string()),
                    error_message: Some(
                        "Responses WebSocket turn was cancelled before startup completed"
                            .to_string(),
                    ),
                    latency_ms: None,
                    started_at_unix_ms: None,
                    finished_at_unix_ms: Some(current_unix_ms()),
                };
                let record = match try_enqueue_local_request_candidate_status_snapshot(
                    &self.state,
                    &snapshot,
                    status_update,
                ) {
                    Ok(()) => return,
                    Err(record) => record,
                };
                let state = self.state.clone();
                if let Ok(handle) = tokio::runtime::Handle::try_current() {
                    handle.spawn(async move {
                        persist_local_request_candidate_status_record(&state, record).await;
                    });
                }
            }
            ExecutionRequestCandidateStartupMode::HttpStream {
                plan,
                report_context,
                watchdog_progress,
            } => {
                if watchdog_progress
                    .as_ref()
                    .is_some_and(|progress| !progress.try_claim_terminal())
                {
                    self.snapshot = None;
                    return;
                }
                let cancellation_cause = watchdog_progress
                    .as_ref()
                    .and_then(|progress| progress.cancellation_cause());
                let (status, status_code, error_type, error_message) = if matches!(
                    cancellation_cause,
                    Some(StreamCandidateCancellationCause::WatchdogTimeout)
                ) {
                    (
                        RequestCandidateStatus::Failed,
                        http::StatusCode::GATEWAY_TIMEOUT.as_u16(),
                        STREAM_CANDIDATE_WATCHDOG_TIMEOUT_ERROR_TYPE,
                        STREAM_CANDIDATE_WATCHDOG_TIMEOUT_MESSAGE,
                    )
                } else {
                    (
                        RequestCandidateStatus::Cancelled,
                        499,
                        "local_stream_startup_cancelled",
                        "Local HTTP stream attempt was cancelled before startup completed",
                    )
                };
                let record = self.snapshot.take().map(|snapshot| {
                    build_local_request_candidate_status_snapshot_record(
                        &snapshot,
                        SchedulerRequestCandidateStatusUpdate {
                            status,
                            status_code: Some(status_code),
                            error_type: Some(error_type.to_string()),
                            error_message: Some(error_message.to_string()),
                            latency_ms: None,
                            started_at_unix_ms: None,
                            finished_at_unix_ms: Some(current_unix_ms()),
                        },
                    )
                });
                let state = self.state.clone();
                if let Ok(handle) = tokio::runtime::Handle::try_current() {
                    handle.spawn(async move {
                        if let Some(record) = record {
                            persist_local_request_candidate_status_record(&state, record).await;
                        }
                        crate::orchestration::release_local_pool_key_lease(
                            &state,
                            crate::orchestration::LocalExecutionEffectContext {
                                plan: &plan,
                                report_context: report_context.as_ref(),
                            },
                        )
                        .await;
                        if let Some(progress) = watchdog_progress {
                            progress.mark_terminal_completed();
                        }
                    });
                } else if let Some(progress) = watchdog_progress {
                    // The watchdog must never wait forever merely because the
                    // cancellation happened outside a Tokio runtime.
                    progress.mark_terminal_completed();
                }
            }
        }
    }
}

impl Drop for ExecutionRequestCandidateStartupGuard {
    fn drop(&mut self) {
        self.terminalize();
    }
}

#[async_trait]
pub(crate) trait RequestCandidateRuntimeReader {
    async fn read_request_candidates_by_request_id(
        &self,
        request_id: &str,
    ) -> Result<Vec<StoredRequestCandidate>, GatewayError>;
}

#[async_trait]
pub(crate) trait RequestCandidateRuntimeWriter: Sync {
    fn has_request_candidate_data_writer(&self) -> bool;

    async fn upsert_request_candidate(
        &self,
        candidate: UpsertRequestCandidateRecord,
    ) -> Result<Option<StoredRequestCandidate>, GatewayError>;

    async fn enqueue_request_candidate_status(
        &self,
        candidate: UpsertRequestCandidateRecord,
    ) -> Result<Option<()>, GatewayError> {
        self.upsert_request_candidate(candidate)
            .await
            .map(|stored| stored.map(|_| ()))
    }

    fn try_enqueue_request_candidate_status(
        &self,
        candidate: UpsertRequestCandidateRecord,
    ) -> Result<(), UpsertRequestCandidateRecord> {
        Err(candidate)
    }
}

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub(crate) enum ExecutionRequestReservationError {
    #[error("execution request reservation {scope:?} is saturated at {limit}")]
    Saturated {
        scope: ExecutionReservationScope,
        limit: usize,
    },
    #[error("execution request reservation is unavailable: {message}")]
    Unavailable { message: String },
}

/// Acquires the execution capacity selected by a plan from strongly read limits.
///
/// `Ok(None)` means none of the provider, provider-key, API-key, or provider-key
/// RPM scopes currently has a positive limit. Catalog and candidate snapshot
/// failures are still fail-closed so callers never execute against stale limits.
pub(crate) async fn acquire_execution_request_reservation(
    state: &AppState,
    plan: &ExecutionPlan,
    auth_context: Option<&GatewayControlAuthContext>,
) -> Result<Option<ExecutionReservationPermit>, ExecutionRequestReservationError> {
    let provider_id = required_execution_reservation_id("provider_id", &plan.provider_id)?;
    let key_id = required_execution_reservation_id("key_id", &plan.key_id)?;
    let candidate_id = required_execution_reservation_candidate_id(plan)?;
    let provider_ids = vec![provider_id.to_string()];
    let key_ids = vec![key_id.to_string()];

    let (providers, keys) = tokio::join!(
        state.read_provider_catalog_providers_by_ids_strong(&provider_ids),
        state.read_provider_catalog_keys_by_ids_strong(&key_ids),
    );
    let provider = providers
        .map_err(|err| ExecutionRequestReservationError::Unavailable {
            message: format!("strong provider limit read failed: {err:?}"),
        })?
        .into_iter()
        .find(|provider| provider.id == provider_id)
        .ok_or_else(|| ExecutionRequestReservationError::Unavailable {
            message: format!("planned provider {provider_id} is missing from the strong catalog"),
        })?;
    let key = keys
        .map_err(|err| ExecutionRequestReservationError::Unavailable {
            message: format!("strong provider-key limit read failed: {err:?}"),
        })?
        .into_iter()
        .find(|key| key.id == key_id)
        .ok_or_else(|| ExecutionRequestReservationError::Unavailable {
            message: format!("planned provider key {key_id} is missing from the strong catalog"),
        })?;
    if key.provider_id != provider_id {
        return Err(ExecutionRequestReservationError::Unavailable {
            message: format!(
                "planned provider key {key_id} belongs to provider {}, not {provider_id}",
                key.provider_id
            ),
        });
    }

    let now_unix_secs = current_unix_secs();
    let limits = ExecutionRequestReservationLimits::from_catalog_and_auth(
        &provider,
        &key,
        auth_context,
        now_unix_secs,
    )?;
    if limits.is_empty() {
        return Ok(None);
    }

    let api_key_ids = limits
        .api_key
        .as_ref()
        .map(|(api_key_id, _)| vec![api_key_id.clone()])
        .unwrap_or_default();
    let provider_scope_ids = if limits.provider.is_some() {
        vec![provider_id.to_string()]
    } else {
        Vec::new()
    };
    let key_scope_ids = if limits.provider_key.is_some() || limits.provider_key_rpm.is_some() {
        vec![key_id.to_string()]
    } else {
        Vec::new()
    };
    let recent_candidates = state
        .read_runtime_scoped_request_candidates_since(
            &provider_scope_ids,
            &key_scope_ids,
            &api_key_ids,
            now_unix_secs.saturating_sub(EXECUTION_RESERVATION_ACTIVE_WINDOW_SECS),
        )
        .await
        .map_err(|err| ExecutionRequestReservationError::Unavailable {
            message: format!("scoped request candidate read failed: {err:?}"),
        })?;
    let input = build_execution_request_reservation_input(
        candidate_id,
        &limits,
        &recent_candidates,
        now_unix_secs,
        state.provider_key_rpm_reset_at(key_id, now_unix_secs),
    );

    state
        .runtime_state
        .try_acquire_execution_reservation(input)
        .await
        .map(Some)
        .map_err(map_execution_reservation_error)
}

#[derive(Debug, Default, PartialEq, Eq)]
struct ExecutionRequestReservationLimits {
    provider: Option<(String, usize)>,
    provider_key: Option<(String, usize)>,
    api_key: Option<(String, usize)>,
    provider_key_rpm: Option<(String, usize)>,
}

impl ExecutionRequestReservationLimits {
    fn from_catalog_and_auth(
        provider: &StoredProviderCatalogProvider,
        key: &StoredProviderCatalogKey,
        auth_context: Option<&GatewayControlAuthContext>,
        now_unix_secs: u64,
    ) -> Result<Self, ExecutionRequestReservationError> {
        let api_key = auth_context
            .and_then(|auth| {
                positive_i32_limit(auth.api_key_concurrent_limit).map(|limit| (auth, limit))
            })
            .map(|(auth, limit)| {
                let api_key_id = required_execution_reservation_id("api_key_id", &auth.api_key_id)?;
                Ok((api_key_id.to_string(), limit))
            })
            .transpose()?;

        Ok(Self {
            provider: positive_i32_limit(provider.concurrent_limit)
                .map(|limit| (provider.id.clone(), limit)),
            provider_key: positive_i32_limit(key.concurrent_limit)
                .map(|limit| (key.id.clone(), limit)),
            api_key,
            provider_key_rpm: effective_provider_key_rpm_limit(key, now_unix_secs)
                .map(|limit| (key.id.clone(), limit)),
        })
    }

    fn is_empty(&self) -> bool {
        self.provider.is_none()
            && self.provider_key.is_none()
            && self.api_key.is_none()
            && self.provider_key_rpm.is_none()
    }
}

fn build_execution_request_reservation_input(
    candidate_id: &str,
    limits: &ExecutionRequestReservationLimits,
    recent_candidates: &[StoredRequestCandidate],
    now_unix_secs: u64,
    provider_key_rpm_reset_after_unix_secs: Option<u64>,
) -> ExecutionReservationInput {
    let concurrency_reservation =
        |scope_id: &(String, usize), matches_scope: fn(&StoredRequestCandidate, &str) -> bool| {
            ExecutionConcurrencyReservation {
                key: scope_id.0.clone(),
                limit: scope_id.1,
                observed_candidate_ids: recent_active_candidate_ids(
                    recent_candidates,
                    scope_id.0.as_str(),
                    now_unix_secs,
                    matches_scope,
                ),
            }
        };

    let provider_key_rpm =
        limits
            .provider_key_rpm
            .as_ref()
            .map(|(key_id, limit)| ExecutionRpmReservation {
                key: key_id.clone(),
                limit: *limit,
                observed_candidate_ids: recent_rpm_candidate_ids(
                    recent_candidates,
                    key_id,
                    now_unix_secs,
                    provider_key_rpm_reset_after_unix_secs,
                ),
                observed_count_floor: count_recent_rpm_requests_for_provider_key_since(
                    recent_candidates,
                    key_id,
                    now_unix_secs,
                    provider_key_rpm_reset_after_unix_secs,
                ),
                reset_after_unix_secs: provider_key_rpm_reset_after_unix_secs,
            });

    ExecutionReservationInput {
        candidate_id: candidate_id.to_string(),
        provider: limits.provider.as_ref().map(|scope| {
            concurrency_reservation(scope, |candidate, id| {
                candidate.provider_id.as_deref() == Some(id)
            })
        }),
        provider_key: limits.provider_key.as_ref().map(|scope| {
            concurrency_reservation(scope, |candidate, id| {
                candidate.key_id.as_deref() == Some(id)
            })
        }),
        api_key: limits.api_key.as_ref().map(|scope| {
            concurrency_reservation(scope, |candidate, id| {
                candidate.api_key_id.as_deref() == Some(id)
            })
        }),
        provider_key_rpm,
    }
}

fn recent_active_candidate_ids(
    recent_candidates: &[StoredRequestCandidate],
    scope_id: &str,
    now_unix_secs: u64,
    matches_scope: fn(&StoredRequestCandidate, &str) -> bool,
) -> Vec<String> {
    recent_candidates
        .iter()
        .filter(|candidate| matches_scope(candidate, scope_id))
        .filter(|candidate| candidate.finished_at_unix_ms.is_none())
        .filter(|candidate| {
            matches!(
                candidate.status,
                RequestCandidateStatus::Pending | RequestCandidateStatus::Streaming
            )
        })
        .filter(|candidate| {
            now_unix_secs.saturating_sub(candidate_observed_at_unix_secs(candidate))
                <= EXECUTION_RESERVATION_ACTIVE_WINDOW_SECS
        })
        .filter_map(|candidate| nonempty_candidate_id(candidate))
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect()
}

fn recent_rpm_candidate_ids(
    recent_candidates: &[StoredRequestCandidate],
    key_id: &str,
    now_unix_secs: u64,
    reset_after_unix_secs: Option<u64>,
) -> Vec<String> {
    recent_candidates
        .iter()
        .filter(|candidate| candidate.key_id.as_deref() == Some(key_id))
        .filter(|candidate| candidate.status.is_attempted(candidate.started_at_unix_ms))
        .filter(|candidate| {
            let observed_at = candidate_observed_at_unix_secs(candidate);
            now_unix_secs.saturating_sub(observed_at)
                <= aether_scheduler_core::PROVIDER_KEY_RPM_WINDOW_SECS
                && reset_after_unix_secs.is_none_or(|reset_after| observed_at > reset_after)
        })
        .filter_map(|candidate| nonempty_candidate_id(candidate))
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect()
}

fn candidate_observed_at_unix_secs(candidate: &StoredRequestCandidate) -> u64 {
    candidate
        .started_at_unix_ms
        .unwrap_or(candidate.created_at_unix_ms)
        / 1_000
}

fn nonempty_candidate_id(candidate: &StoredRequestCandidate) -> Option<String> {
    let candidate_id = candidate.id.trim();
    (!candidate_id.is_empty()).then(|| candidate_id.to_string())
}

fn required_execution_reservation_candidate_id(
    plan: &ExecutionPlan,
) -> Result<&str, ExecutionRequestReservationError> {
    let candidate_id = plan.candidate_id.as_deref().unwrap_or_default();
    required_execution_reservation_id("candidate_id", candidate_id)
}

fn required_execution_reservation_id<'a>(
    field: &str,
    value: &'a str,
) -> Result<&'a str, ExecutionRequestReservationError> {
    let value = value.trim();
    if value.is_empty() {
        return Err(ExecutionRequestReservationError::Unavailable {
            message: format!("execution plan {field} is missing"),
        });
    }
    Ok(value)
}

fn positive_i32_limit(limit: Option<i32>) -> Option<usize> {
    limit
        .and_then(|limit| usize::try_from(limit).ok())
        .filter(|limit| *limit > 0)
}

fn map_execution_reservation_error(
    error: ExecutionReservationError,
) -> ExecutionRequestReservationError {
    match error {
        ExecutionReservationError::Rejected { scope, limit } => {
            ExecutionRequestReservationError::Saturated { scope, limit }
        }
        ExecutionReservationError::Unavailable { message } => {
            ExecutionRequestReservationError::Unavailable { message }
        }
        ExecutionReservationError::InvalidConfiguration(message) => {
            ExecutionRequestReservationError::Unavailable { message }
        }
    }
}

#[async_trait]
pub(crate) trait RequestCandidateRuntimeCapabilityReader {
    async fn read_request_candidate_user_model_capability_settings(
        &self,
        user_id: &str,
    ) -> Result<Option<Value>, GatewayError>;

    async fn read_request_candidate_api_key_force_capabilities(
        &self,
        user_id: &str,
        api_key_id: &str,
    ) -> Result<Option<Value>, GatewayError>;
}

pub(crate) async fn resolve_request_candidate_required_capabilities(
    state: &(impl RequestCandidateRuntimeCapabilityReader + ?Sized),
    user_id: &str,
    api_key_id: &str,
    requested_model: Option<&str>,
    explicit_required_capabilities: Option<&Value>,
    model_directive_base_model: Option<&str>,
) -> Option<Value> {
    let mut merged = serde_json::Map::new();

    match state
        .read_request_candidate_user_model_capability_settings(user_id)
        .await
    {
        Ok(settings) => merge_capability_object(
            &mut merged,
            select_requested_model_capabilities(
                settings.as_ref(),
                requested_model,
                model_directive_base_model,
            ),
        ),
        Err(error) => {
            warn!(
                user_id = %user_id,
                api_key_id = %api_key_id,
                requested_model = requested_model.unwrap_or_default(),
                error = ?error,
                "gateway request candidate user model capabilities lookup failed"
            );
        }
    }

    match state
        .read_request_candidate_api_key_force_capabilities(user_id, api_key_id)
        .await
    {
        Ok(force_capabilities) => {
            merge_capability_object(&mut merged, force_capabilities.as_ref());
        }
        Err(error) => {
            warn!(
                user_id = %user_id,
                api_key_id = %api_key_id,
                requested_model = requested_model.unwrap_or_default(),
                error = ?error,
                "gateway request candidate api key capabilities lookup failed"
            );
        }
    }

    merge_capability_object(&mut merged, explicit_required_capabilities);

    (!merged.is_empty()).then_some(Value::Object(merged))
}

fn merge_capability_object(target: &mut serde_json::Map<String, Value>, source: Option<&Value>) {
    let Some(source) = source.and_then(Value::as_object) else {
        return;
    };

    for (capability, value) in source {
        if capability.trim().is_empty() {
            continue;
        }
        target.insert(capability.clone(), value.clone());
    }
}

fn select_requested_model_capabilities<'a>(
    settings: Option<&'a Value>,
    requested_model: Option<&str>,
    model_directive_base_model: Option<&str>,
) -> Option<&'a Value> {
    let requested_model = requested_model
        .map(str::trim)
        .filter(|value| !value.is_empty())?;
    let settings = settings?.as_object()?;

    find_model_capabilities(settings, requested_model).or_else(|| {
        model_directive_base_model
            .map(str::trim)
            .filter(|base_model| !base_model.is_empty() && *base_model != requested_model)
            .and_then(|base_model| find_model_capabilities(settings, base_model))
    })
}

fn find_model_capabilities<'a>(
    settings: &'a serde_json::Map<String, Value>,
    requested_model: &str,
) -> Option<&'a Value> {
    settings.get(requested_model).or_else(|| {
        settings.iter().find_map(|(model_name, capabilities)| {
            model_name
                .trim()
                .eq_ignore_ascii_case(requested_model)
                .then_some(capabilities)
        })
    })
}

fn request_candidate_status_label(status: RequestCandidateStatus) -> &'static str {
    match status {
        RequestCandidateStatus::Available => "available",
        RequestCandidateStatus::Unused => "unused",
        RequestCandidateStatus::Pending => "pending",
        RequestCandidateStatus::Streaming => "streaming",
        RequestCandidateStatus::Success => "success",
        RequestCandidateStatus::Failed => "failed",
        RequestCandidateStatus::Cancelled => "cancelled",
        RequestCandidateStatus::Skipped => "skipped",
    }
}

pub(crate) fn snapshot_local_request_candidate_status(
    plan: &ExecutionPlan,
    report_context: Option<&Value>,
) -> Option<LocalRequestCandidateStatusSnapshot> {
    let candidate_id = plan
        .candidate_id
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())?;
    let metadata = parse_request_candidate_report_context(report_context);
    let candidate_index = metadata
        .as_ref()
        .and_then(|metadata| metadata.candidate_index)
        .unwrap_or(0);

    Some(LocalRequestCandidateStatusSnapshot {
        candidate_id: candidate_id.to_string(),
        request_id: metadata
            .as_ref()
            .and_then(|metadata| metadata.request_id.clone())
            .filter(|request_id| !request_id.trim().is_empty())
            .unwrap_or_else(|| plan.request_id.clone()),
        user_id: metadata
            .as_ref()
            .and_then(|metadata| metadata.user_id.clone()),
        api_key_id: metadata
            .as_ref()
            .and_then(|metadata| metadata.api_key_id.clone()),
        candidate_index,
        retry_index: metadata
            .as_ref()
            .map(|metadata| metadata.retry_index)
            .unwrap_or(0),
        provider_id: plan.provider_id.clone(),
        endpoint_id: plan.endpoint_id.clone(),
        key_id: plan.key_id.clone(),
    })
}

pub(crate) async fn persist_local_request_candidate_status_record(
    state: &(impl RequestCandidateRuntimeWriter + ?Sized),
    record: UpsertRequestCandidateRecord,
) {
    let candidate_id = record.id.clone();
    let request_id = short_request_id(record.request_id.as_str());
    let candidate_index = record.candidate_index;
    let retry_index = record.retry_index;
    let status = record.status;

    if !should_persist_request_candidate_status(status) {
        debug!(
            event_name = "request_candidate_status_persistence_skipped",
            log_type = "event",
            request_id = %request_id,
            candidate_id = %candidate_id,
            candidate_index,
            retry_index,
            status = request_candidate_status_label(status),
            source = "local_status",
            "gateway skipped request candidate status update due to persistence mode"
        );
        return;
    }

    match state.enqueue_request_candidate_status(record).await {
        Ok(Some(())) => {
            debug!(
                event_name = "request_candidate_status_persisted",
                log_type = "event",
                request_id = %request_id,
                candidate_id = %candidate_id,
                candidate_index,
                retry_index,
                status = request_candidate_status_label(status),
                source = "local_status",
                "gateway persisted request candidate status update"
            );
        }
        Ok(None) => {
            warn!(
                event_name = "request_candidate_writer_unavailable",
                log_type = "event",
                request_id = %request_id,
                candidate_id = %candidate_id,
                candidate_index,
                retry_index,
                status = request_candidate_status_label(status),
                source = "local_status",
                "gateway skipped request candidate persistence because writer is unavailable"
            );
        }
        Err(err) => {
            warn!(
                event_name = "request_candidate_status_persist_failed",
                log_type = "event",
                request_id = %request_id,
                candidate_id = %candidate_id,
                error = ?err,
                "gateway failed to persist request candidate status update"
            );
        }
    }
}

pub(crate) async fn record_local_request_candidate_status(
    state: &(impl RequestCandidateRuntimeWriter + ?Sized),
    plan: &ExecutionPlan,
    report_context: Option<&Value>,
    status_update: SchedulerRequestCandidateStatusUpdate,
) {
    let Some(record) =
        build_local_request_candidate_status_record(LocalRequestCandidateStatusRecordInput {
            plan,
            report_context,
            status_update,
        })
    else {
        return;
    };
    persist_local_request_candidate_status_record(state, record).await;
}

pub(crate) async fn record_local_request_candidate_extra_data(
    state: &(impl RequestCandidateRuntimeWriter + ?Sized),
    plan: &ExecutionPlan,
    report_context: Option<&Value>,
    status: RequestCandidateStatus,
    status_code: Option<u16>,
    latency_ms: Option<u64>,
    extra_data: Value,
) {
    let Some(snapshot) = snapshot_local_request_candidate_status(plan, report_context) else {
        return;
    };
    let record = UpsertRequestCandidateRecord {
        id: snapshot.candidate_id.clone(),
        request_id: snapshot.request_id.clone(),
        user_id: snapshot.user_id.clone(),
        api_key_id: snapshot.api_key_id.clone(),
        username: None,
        api_key_name: None,
        candidate_index: snapshot.candidate_index,
        retry_index: snapshot.retry_index,
        provider_id: Some(snapshot.provider_id.clone()),
        endpoint_id: Some(snapshot.endpoint_id.clone()),
        key_id: Some(snapshot.key_id.clone()),
        status,
        skip_reason: None,
        is_cached: None,
        status_code,
        error_type: None,
        error_message: None,
        latency_ms,
        concurrent_requests: None,
        extra_data: Some(extra_data),
        required_capabilities: None,
        created_at_unix_ms: None,
        started_at_unix_ms: None,
        finished_at_unix_ms: None,
    };
    persist_local_request_candidate_status_record(state, record).await;
}

fn build_local_request_candidate_status_snapshot_record(
    snapshot: &LocalRequestCandidateStatusSnapshot,
    status_update: SchedulerRequestCandidateStatusUpdate,
) -> UpsertRequestCandidateRecord {
    let SchedulerRequestCandidateStatusUpdate {
        status,
        status_code,
        error_type,
        error_message,
        latency_ms,
        started_at_unix_ms,
        finished_at_unix_ms,
    } = status_update;
    UpsertRequestCandidateRecord {
        id: snapshot.candidate_id.clone(),
        request_id: snapshot.request_id.clone(),
        user_id: snapshot.user_id.clone(),
        api_key_id: snapshot.api_key_id.clone(),
        username: None,
        api_key_name: None,
        candidate_index: snapshot.candidate_index,
        retry_index: snapshot.retry_index,
        provider_id: Some(snapshot.provider_id.clone()),
        endpoint_id: Some(snapshot.endpoint_id.clone()),
        key_id: Some(snapshot.key_id.clone()),
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
        created_at_unix_ms: None,
        started_at_unix_ms,
        finished_at_unix_ms,
    }
}

pub(crate) fn try_enqueue_local_request_candidate_status_snapshot(
    state: &(impl RequestCandidateRuntimeWriter + ?Sized),
    snapshot: &LocalRequestCandidateStatusSnapshot,
    status_update: SchedulerRequestCandidateStatusUpdate,
) -> Result<(), UpsertRequestCandidateRecord> {
    let record = build_local_request_candidate_status_snapshot_record(snapshot, status_update);
    if !should_persist_request_candidate_status(record.status) {
        return Ok(());
    }
    state.try_enqueue_request_candidate_status(record)
}

pub(crate) async fn record_local_request_candidate_status_snapshot(
    state: &(impl RequestCandidateRuntimeWriter + ?Sized),
    snapshot: &LocalRequestCandidateStatusSnapshot,
    status_update: SchedulerRequestCandidateStatusUpdate,
) {
    let record = build_local_request_candidate_status_snapshot_record(snapshot, status_update);
    persist_local_request_candidate_status_record(state, record).await;
}

pub(crate) async fn record_report_request_candidate_status(
    state: &(impl RequestCandidateRuntimeReader + RequestCandidateRuntimeWriter + ?Sized),
    report_context: Option<&Value>,
    status_update: SchedulerRequestCandidateStatusUpdate,
) {
    if matches!(
        request_candidate_persistence_mode(),
        RequestCandidatePersistenceMode::None
    ) {
        return;
    }
    let Some(slot) = resolve_report_request_candidate_slot(state, report_context).await else {
        return;
    };
    let request_id = slot.request_id.clone();
    let request_id_for_log = short_request_id(request_id.as_str());
    let candidate_index = slot.candidate_index;
    let retry_index = slot.retry_index;
    let record =
        build_report_request_candidate_status_record(ReportRequestCandidateStatusRecordInput {
            slot,
            status_update,
            now_unix_ms: current_unix_ms(),
        });
    let candidate_id = record.id.clone();
    let status = record.status;

    if !should_persist_request_candidate_status(status) {
        debug!(
            event_name = "request_candidate_report_status_persistence_skipped",
            log_type = "event",
            request_id = %request_id_for_log,
            candidate_id = %candidate_id,
            candidate_index,
            retry_index,
            status = request_candidate_status_label(status),
            source = "report_status",
            "gateway skipped report-driven request candidate status update due to persistence mode"
        );
        return;
    }

    match state.enqueue_request_candidate_status(record).await {
        Ok(Some(())) => {
            debug!(
                event_name = "request_candidate_report_status_persisted",
                log_type = "event",
                request_id = %request_id_for_log,
                candidate_id = %candidate_id,
                candidate_index,
                retry_index,
                status = request_candidate_status_label(status),
                source = "report_status",
                "gateway persisted report-driven request candidate status update"
            );
        }
        Ok(None) => {
            warn!(
                event_name = "request_candidate_writer_unavailable",
                log_type = "event",
                request_id = %request_id_for_log,
                candidate_id = %candidate_id,
                candidate_index,
                retry_index,
                status = request_candidate_status_label(status),
                source = "report_status",
                "gateway skipped request candidate persistence because writer is unavailable"
            );
        }
        Err(err) => {
            warn!(
                event_name = "request_candidate_report_status_persist_failed",
                log_type = "event",
                request_id = %request_id_for_log,
                candidate_index,
                retry_index,
                error = ?err,
                "gateway failed to persist report-driven request candidate status update"
            );
        }
    }
}

pub(crate) async fn prepare_execution_request_candidate_slot(
    state: &(impl RequestCandidateRuntimeReader + RequestCandidateRuntimeWriter + ?Sized),
    plan: &mut ExecutionPlan,
    report_context: &mut Option<Value>,
) -> Result<PreparedExecutionRequestCandidateSlot, GatewayError> {
    install_execution_request_candidate_identity(plan, report_context);
    let writer_available = state.has_request_candidate_data_writer();
    let metadata = parse_request_candidate_report_context(report_context.as_ref());
    let request_id = metadata
        .as_ref()
        .and_then(|metadata| metadata.request_id.as_deref())
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .unwrap_or(plan.request_id.as_str());
    let candidate_index = metadata
        .as_ref()
        .and_then(|metadata| metadata.candidate_index)
        .unwrap_or(0);
    let retry_index = metadata
        .as_ref()
        .map(|metadata| metadata.retry_index)
        .unwrap_or(0);
    let stored_candidate_id = state
        .read_request_candidates_by_request_id(request_id)
        .await?
        .into_iter()
        .find(|candidate| {
            candidate.candidate_index == candidate_index && candidate.retry_index == retry_index
        })
        .map(|candidate| candidate.id);
    if let Some(stored_candidate_id) = stored_candidate_id {
        plan.candidate_id = Some(stored_candidate_id.clone());
        let context = report_context
            .take()
            .unwrap_or_else(|| Value::Object(Default::default()));
        *report_context = Some(finalize_execution_request_candidate_report_context(
            context,
            &stored_candidate_id,
        ));
        return Ok(PreparedExecutionRequestCandidateSlot { pending: None });
    }
    let deterministic_candidate_id =
        deterministic_request_candidate_id(request_id, candidate_index, retry_index);
    let mut seed_report_context = report_context.clone();
    if let Some(context) = seed_report_context.as_mut().and_then(Value::as_object_mut) {
        context.remove("candidate_id");
    }
    let seed = build_execution_request_candidate_seed(
        plan,
        seed_report_context.as_ref(),
        current_unix_ms(),
        deterministic_candidate_id,
    );
    let generated_candidate_id = seed.upsert_record.id.clone();
    let request_id = short_request_id(plan.request_id.as_str());

    plan.candidate_id = Some(generated_candidate_id.clone());
    *report_context = Some(finalize_execution_request_candidate_report_context(
        seed.report_context,
        &generated_candidate_id,
    ));

    if !writer_available {
        warn!(
            event_name = "request_candidate_writer_unavailable",
            log_type = "event",
            request_id = %request_id,
            candidate_id = %generated_candidate_id,
            provider_id = %plan.provider_id,
            endpoint_id = %plan.endpoint_id,
            key_id = %plan.key_id,
            source = "seed",
            "gateway fixed request candidate identity without persistence because writer is unavailable"
        );
        return Ok(PreparedExecutionRequestCandidateSlot { pending: None });
    }

    if !should_persist_request_candidate_status(seed.upsert_record.status) {
        debug!(
            event_name = "request_candidate_slot_seed_persistence_skipped",
            log_type = "event",
            request_id = %request_id,
            candidate_id = %generated_candidate_id,
            provider_id = %plan.provider_id,
            endpoint_id = %plan.endpoint_id,
            key_id = %plan.key_id,
            source = "seed",
            "gateway skipped request candidate seed due to persistence mode"
        );
        return Ok(PreparedExecutionRequestCandidateSlot { pending: None });
    }

    Ok(PreparedExecutionRequestCandidateSlot {
        pending: Some(seed.upsert_record),
    })
}

/// Installs the deterministic identity for an execution candidate without
/// performing I/O. Callers that own cancellation cleanup can invoke this
/// before their first await and then let the async preparation reconcile an
/// already-persisted slot.
pub(crate) fn install_execution_request_candidate_identity(
    plan: &mut ExecutionPlan,
    report_context: &mut Option<Value>,
) {
    let metadata = parse_request_candidate_report_context(report_context.as_ref());
    let request_id = metadata
        .as_ref()
        .and_then(|metadata| metadata.request_id.as_deref())
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .unwrap_or(plan.request_id.as_str());
    let candidate_index = metadata
        .as_ref()
        .and_then(|metadata| metadata.candidate_index)
        .unwrap_or(0);
    let retry_index = metadata
        .as_ref()
        .map(|metadata| metadata.retry_index)
        .unwrap_or(0);
    let candidate_id = metadata
        .as_ref()
        .and_then(|metadata| metadata.candidate_id.clone())
        .filter(|candidate_id| !candidate_id.trim().is_empty())
        .or_else(|| {
            plan.candidate_id
                .clone()
                .filter(|candidate_id| !candidate_id.trim().is_empty())
        })
        .unwrap_or_else(|| {
            deterministic_request_candidate_id(request_id, candidate_index, retry_index)
        });

    plan.candidate_id = Some(candidate_id.clone());
    let context = report_context
        .take()
        .unwrap_or_else(|| Value::Object(Default::default()));
    *report_context = Some(finalize_execution_request_candidate_report_context(
        context,
        candidate_id.as_str(),
    ));
}

fn deterministic_request_candidate_id(
    request_id: &str,
    candidate_index: u32,
    retry_index: u32,
) -> String {
    let slot = format!(
        "aether:request-candidate:{}:{request_id}:{candidate_index}:{retry_index}",
        request_id.len()
    );
    Uuid::new_v5(&Uuid::NAMESPACE_URL, slot.as_bytes()).to_string()
}

pub(crate) async fn persist_execution_request_candidate_slot(
    state: &(impl RequestCandidateRuntimeWriter + ?Sized),
    plan: &mut ExecutionPlan,
    report_context: &mut Option<Value>,
    prepared: PreparedExecutionRequestCandidateSlot,
) -> Result<(), GatewayError> {
    let Some(seed_upsert_record) = prepared.pending else {
        return Ok(());
    };
    let generated_candidate_id = seed_upsert_record.id.clone();
    let request_id = short_request_id(plan.request_id.as_str());
    let candidate_id = match tokio::time::timeout(
        request_candidate_seed_write_timeout(),
        state.upsert_request_candidate(seed_upsert_record),
    )
    .await
    {
        Ok(Ok(Some(stored))) => {
            info!(
                event_name = "request_candidate_slot_seeded",
                log_type = "event",
                request_id = %request_id,
                candidate_id = %stored.id,
                provider_id = %plan.provider_id,
                endpoint_id = %plan.endpoint_id,
                key_id = %plan.key_id,
                source = "seed",
                "gateway seeded execution request candidate slot"
            );
            stored.id
        }
        Ok(Ok(None)) => {
            warn!(
                event_name = "request_candidate_writer_unavailable",
                log_type = "event",
                request_id = %request_id,
                candidate_id = %generated_candidate_id,
                provider_id = %plan.provider_id,
                endpoint_id = %plan.endpoint_id,
                key_id = %plan.key_id,
                source = "seed",
                "gateway skipped request candidate seed because writer is unavailable"
            );
            generated_candidate_id.clone()
        }
        Ok(Err(err)) => {
            warn!(
                event_name = "request_candidate_slot_seed_failed",
                log_type = "event",
                request_id = %request_id,
                error = ?err,
                "gateway failed to seed execution request candidate slot"
            );
            generated_candidate_id.clone()
        }
        Err(_) => {
            let timeout_ms = request_candidate_seed_write_timeout().as_millis() as u64;
            warn!(
                event_name = "request_candidate_slot_seed_timed_out",
                log_type = "event",
                request_id = %request_id,
                candidate_id = %generated_candidate_id,
                provider_id = %plan.provider_id,
                endpoint_id = %plan.endpoint_id,
                key_id = %plan.key_id,
                source = "seed",
                timeout_ms,
                "gateway skipped blocking request candidate seed after timeout"
            );
            generated_candidate_id.clone()
        }
    };

    if candidate_id != generated_candidate_id {
        return Err(GatewayError::Internal(format!(
            "request candidate slot identity changed from {generated_candidate_id} to {candidate_id}"
        )));
    }
    Ok(())
}

pub(crate) async fn ensure_execution_request_candidate_slot(
    state: &(impl RequestCandidateRuntimeReader + RequestCandidateRuntimeWriter + ?Sized),
    plan: &mut ExecutionPlan,
    report_context: &mut Option<Value>,
) -> Result<(), GatewayError> {
    let prepared = prepare_execution_request_candidate_slot(state, plan, report_context).await?;
    persist_execution_request_candidate_slot(state, plan, report_context, prepared).await
}

pub(crate) async fn persist_available_local_candidate(
    state: &(impl RequestCandidateRuntimeWriter + ?Sized),
    trace_id: &str,
    user_id: &str,
    api_key_id: &str,
    candidate: &SchedulerMinimalCandidateSelectionCandidate,
    candidate_index: u32,
    retry_index: u32,
    candidate_id: &str,
    required_capabilities: Option<&Value>,
    extra_data: Option<serde_json::Value>,
    created_at_unix_ms: u64,
    error_context: &'static str,
) -> String {
    if !should_persist_request_candidate_status(RequestCandidateStatus::Available) {
        return candidate_id.to_string();
    }
    match state
        .upsert_request_candidate(UpsertRequestCandidateRecord {
            id: candidate_id.to_string(),
            request_id: trace_id.to_string(),
            user_id: Some(user_id.to_string()),
            api_key_id: Some(api_key_id.to_string()),
            username: None,
            api_key_name: None,
            candidate_index,
            retry_index,
            provider_id: Some(candidate.provider_id.clone()),
            endpoint_id: Some(candidate.endpoint_id.clone()),
            key_id: Some(candidate.key_id.clone()),
            status: RequestCandidateStatus::Available,
            skip_reason: None,
            is_cached: Some(false),
            status_code: None,
            error_type: None,
            error_message: None,
            latency_ms: None,
            concurrent_requests: None,
            extra_data,
            required_capabilities: required_capabilities.cloned(),
            created_at_unix_ms: Some(created_at_unix_ms),
            started_at_unix_ms: None,
            finished_at_unix_ms: None,
        })
        .await
    {
        Ok(Some(stored)) => {
            debug!(
                event_name = "request_candidate_status_persisted",
                log_type = "event",
                request_id = %short_request_id(trace_id),
                candidate_id = %stored.id,
                candidate_index,
                retry_index,
                status = "available",
                source = "planner_available",
                provider_id = %candidate.provider_id,
                endpoint_id = %candidate.endpoint_id,
                key_id = %candidate.key_id,
                has_required_capabilities = required_capabilities.is_some(),
                "gateway persisted available local request candidate"
            );
            stored.id
        }
        Ok(None) => {
            warn!(
                event_name = "request_candidate_writer_unavailable",
                log_type = "event",
                request_id = %short_request_id(trace_id),
                candidate_id = %candidate_id,
                candidate_index,
                retry_index,
                status = "available",
                source = "planner_available",
                provider_id = %candidate.provider_id,
                endpoint_id = %candidate.endpoint_id,
                key_id = %candidate.key_id,
                "gateway skipped request candidate persistence because writer is unavailable"
            );
            candidate_id.to_string()
        }
        Err(err) => {
            warn!(
                trace_id = %trace_id,
                candidate_id = %candidate_id,
                error = ?err,
                "{error_context}"
            );
            candidate_id.to_string()
        }
    }
}

pub(crate) async fn persist_skipped_local_candidate(
    state: &(impl RequestCandidateRuntimeWriter + ?Sized),
    trace_id: &str,
    user_id: &str,
    api_key_id: &str,
    candidate: &SchedulerMinimalCandidateSelectionCandidate,
    candidate_index: u32,
    retry_index: u32,
    candidate_id: &str,
    required_capabilities: Option<&Value>,
    skip_reason: &str,
    extra_data: Option<serde_json::Value>,
    finished_at_unix_ms: u64,
    error_context: &'static str,
) {
    if !should_persist_request_candidate_status(RequestCandidateStatus::Skipped) {
        return;
    }
    match state
        .upsert_request_candidate(UpsertRequestCandidateRecord {
            id: candidate_id.to_string(),
            request_id: trace_id.to_string(),
            user_id: Some(user_id.to_string()),
            api_key_id: Some(api_key_id.to_string()),
            username: None,
            api_key_name: None,
            candidate_index,
            retry_index,
            provider_id: Some(candidate.provider_id.clone()),
            endpoint_id: Some(candidate.endpoint_id.clone()),
            key_id: Some(candidate.key_id.clone()),
            status: RequestCandidateStatus::Skipped,
            skip_reason: Some(skip_reason.to_string()),
            is_cached: Some(false),
            status_code: None,
            error_type: None,
            error_message: None,
            latency_ms: None,
            concurrent_requests: None,
            extra_data,
            required_capabilities: required_capabilities.cloned(),
            created_at_unix_ms: None,
            started_at_unix_ms: None,
            finished_at_unix_ms: Some(finished_at_unix_ms),
        })
        .await
    {
        Ok(Some(stored)) => {
            debug!(
                event_name = "request_candidate_status_persisted",
                log_type = "event",
                request_id = %short_request_id(trace_id),
                candidate_id = %stored.id,
                candidate_index,
                retry_index,
                status = "skipped",
                skip_reason,
                source = "planner_skipped",
                provider_id = %candidate.provider_id,
                endpoint_id = %candidate.endpoint_id,
                key_id = %candidate.key_id,
                has_required_capabilities = required_capabilities.is_some(),
                "gateway persisted skipped local request candidate"
            );
        }
        Ok(None) => {
            warn!(
                event_name = "request_candidate_writer_unavailable",
                log_type = "event",
                request_id = %short_request_id(trace_id),
                candidate_id = %candidate_id,
                candidate_index,
                retry_index,
                status = "skipped",
                skip_reason,
                source = "planner_skipped",
                provider_id = %candidate.provider_id,
                endpoint_id = %candidate.endpoint_id,
                key_id = %candidate.key_id,
                "gateway skipped request candidate persistence because writer is unavailable"
            );
        }
        Err(err) => {
            warn!(
                trace_id = %trace_id,
                candidate_id = %candidate_id,
                skip_reason,
                error = ?err,
                "{error_context}"
            );
        }
    }
}

pub(crate) async fn resolve_locally_actionable_request_candidate_report_context(
    state: &(impl RequestCandidateRuntimeReader + ?Sized),
    context: &Value,
) -> Option<Value> {
    let request_id = context
        .get("request_id")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())?;
    let existing_candidates = state
        .read_request_candidates_by_request_id(request_id)
        .await
        .ok()?;
    if existing_candidates.len() != 1 {
        return None;
    }

    build_locally_actionable_report_context_from_request_candidate(context, &existing_candidates[0])
}

async fn resolve_report_request_candidate_slot(
    state: &(impl RequestCandidateRuntimeReader + ?Sized),
    report_context: Option<&Value>,
) -> Option<SchedulerResolvedReportRequestCandidateSlot> {
    let metadata = parse_request_candidate_report_context(report_context)?;
    if metadata
        .request_id
        .as_deref()
        .map(str::trim)
        .is_some_and(|value| !value.is_empty())
        && metadata
            .candidate_id
            .as_deref()
            .map(str::trim)
            .is_some_and(|value| !value.is_empty())
    {
        return resolve_report_request_candidate_slot_from_candidates(
            &[],
            metadata,
            current_unix_ms(),
            Uuid::new_v4().to_string(),
        );
    }

    let request_id = metadata.request_id.clone()?;
    let existing_candidates = state
        .read_request_candidates_by_request_id(request_id.as_str())
        .await
        .ok()
        .unwrap_or_default();
    resolve_report_request_candidate_slot_from_candidates(
        &existing_candidates,
        metadata,
        current_unix_ms(),
        Uuid::new_v4().to_string(),
    )
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;
    use std::sync::{Arc, Mutex};
    use std::time::Duration;

    use aether_contracts::{ExecutionPlan, RequestBody};
    use aether_data::repository::auth::{
        InMemoryAuthApiKeySnapshotRepository, StoredAuthApiKeyExportRecord,
    };
    use aether_data::repository::candidates::InMemoryRequestCandidateRepository;
    use aether_data::repository::usage::InMemoryUsageReadRepository;
    use aether_data_contracts::repository::candidates::{
        RequestCandidateReadRepository, RequestCandidateStatus, StoredRequestCandidate,
        UpsertRequestCandidateRecord,
    };
    use aether_data_contracts::repository::provider_catalog::{
        StoredProviderCatalogKey, StoredProviderCatalogProvider,
    };
    use aether_runtime_state::{ExecutionReservationError, ExecutionReservationScope};
    use aether_scheduler_core::SchedulerMinimalCandidateSelectionCandidate;
    use serde_json::{json, Value};

    use super::{
        build_execution_request_reservation_input, ensure_execution_request_candidate_slot,
        install_execution_request_candidate_identity, map_execution_reservation_error,
        persist_available_local_candidate, persist_execution_request_candidate_slot,
        prepare_execution_request_candidate_slot, record_local_request_candidate_status,
        record_report_request_candidate_status, resolve_request_candidate_required_capabilities,
        select_requested_model_capabilities, snapshot_local_request_candidate_status,
        try_enqueue_local_request_candidate_status_snapshot, ExecutionRequestCandidateStartupGuard,
        ExecutionRequestReservationError, ExecutionRequestReservationLimits,
        RequestCandidateRuntimeReader, RequestCandidateRuntimeWriter,
        SchedulerRequestCandidateStatusUpdate,
    };
    use crate::control::GatewayControlAuthContext;
    use crate::data::GatewayDataState;
    use crate::execution_runtime::{
        StreamCandidateWatchdogProgress, STREAM_CANDIDATE_WATCHDOG_TIMEOUT_ERROR_TYPE,
    };
    use crate::AppState;

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

    fn build_test_state_with_auth(
        repository: Arc<InMemoryRequestCandidateRepository>,
        auth_repository: Arc<InMemoryAuthApiKeySnapshotRepository>,
    ) -> AppState {
        AppState::new()
            .expect("gateway state should build")
            .with_data_state_for_tests(
                GatewayDataState::with_request_candidate_and_usage_repository_for_tests(
                    repository,
                    Arc::new(InMemoryUsageReadRepository::default()),
                )
                .with_auth_api_key_reader(auth_repository),
            )
    }

    async fn wait_for_candidate_status(
        repository: &InMemoryRequestCandidateRepository,
        request_id: &str,
        status: RequestCandidateStatus,
    ) -> StoredRequestCandidate {
        tokio::time::timeout(Duration::from_secs(1), async {
            loop {
                let stored = repository
                    .list_by_request_id(request_id)
                    .await
                    .expect("request candidates should read");
                if let Some(candidate) = stored
                    .into_iter()
                    .find(|candidate| candidate.status == status)
                {
                    return candidate;
                }
                tokio::task::yield_now().await;
            }
        })
        .await
        .expect("candidate should reach the expected status")
    }

    #[derive(Default)]
    struct SynchronousStatusWriter {
        records: Mutex<Vec<UpsertRequestCandidateRecord>>,
    }

    #[async_trait::async_trait]
    impl RequestCandidateRuntimeWriter for SynchronousStatusWriter {
        fn has_request_candidate_data_writer(&self) -> bool {
            true
        }

        async fn upsert_request_candidate(
            &self,
            _candidate: UpsertRequestCandidateRecord,
        ) -> Result<Option<StoredRequestCandidate>, crate::GatewayError> {
            panic!("synchronous status fast path must not call the async writer")
        }

        fn try_enqueue_request_candidate_status(
            &self,
            candidate: UpsertRequestCandidateRecord,
        ) -> Result<(), UpsertRequestCandidateRecord> {
            self.records
                .lock()
                .expect("synchronous status records lock")
                .push(candidate);
            Ok(())
        }
    }

    struct UnavailableStatusWriter;

    #[async_trait::async_trait]
    impl RequestCandidateRuntimeReader for UnavailableStatusWriter {
        async fn read_request_candidates_by_request_id(
            &self,
            _request_id: &str,
        ) -> Result<Vec<StoredRequestCandidate>, crate::GatewayError> {
            Ok(Vec::new())
        }
    }

    #[async_trait::async_trait]
    impl RequestCandidateRuntimeWriter for UnavailableStatusWriter {
        fn has_request_candidate_data_writer(&self) -> bool {
            false
        }

        async fn upsert_request_candidate(
            &self,
            _candidate: UpsertRequestCandidateRecord,
        ) -> Result<Option<StoredRequestCandidate>, crate::GatewayError> {
            panic!("unavailable writer must not be called")
        }
    }

    struct BlockingCandidateReader {
        entered: Arc<tokio::sync::Notify>,
        release: Arc<tokio::sync::Notify>,
    }

    #[async_trait::async_trait]
    impl RequestCandidateRuntimeReader for BlockingCandidateReader {
        async fn read_request_candidates_by_request_id(
            &self,
            _request_id: &str,
        ) -> Result<Vec<StoredRequestCandidate>, crate::GatewayError> {
            self.entered.notify_one();
            self.release.notified().await;
            Ok(Vec::new())
        }
    }

    #[async_trait::async_trait]
    impl RequestCandidateRuntimeWriter for BlockingCandidateReader {
        fn has_request_candidate_data_writer(&self) -> bool {
            false
        }

        async fn upsert_request_candidate(
            &self,
            _candidate: UpsertRequestCandidateRecord,
        ) -> Result<Option<StoredRequestCandidate>, crate::GatewayError> {
            panic!("blocking reader has no writer")
        }
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
            transport_profile: None,
            timeouts: None,
        }
    }

    #[test]
    fn installs_candidate_identity_before_async_preparation() {
        let mut first_plan = sample_plan();
        first_plan.request_id = "request-sync-identity".to_string();
        let mut first_context = Some(json!({
            "request_id": "request-sync-identity",
            "candidate_index": 4,
            "retry_index": 2,
        }));
        let mut second_plan = first_plan.clone();
        let mut second_context = first_context.clone();

        install_execution_request_candidate_identity(&mut first_plan, &mut first_context);
        install_execution_request_candidate_identity(&mut second_plan, &mut second_context);

        assert_eq!(first_plan.candidate_id, second_plan.candidate_id);
        assert_eq!(
            first_context
                .as_ref()
                .and_then(|context| context.get("candidate_id")),
            second_context
                .as_ref()
                .and_then(|context| context.get("candidate_id")),
        );
    }

    #[test]
    fn synchronous_identity_preserves_materialized_candidate_id() {
        let mut plan = sample_plan();
        plan.candidate_id = Some("persisted-random-candidate".to_string());
        let mut report_context = Some(json!({
            "request_id": plan.request_id,
            "candidate_id": "persisted-random-candidate",
            "candidate_index": 1,
            "retry_index": 0,
        }));

        install_execution_request_candidate_identity(&mut plan, &mut report_context);

        assert_eq!(
            plan.candidate_id.as_deref(),
            Some("persisted-random-candidate")
        );
        assert_eq!(
            report_context
                .as_ref()
                .and_then(|context| context.get("candidate_id"))
                .and_then(Value::as_str),
            Some("persisted-random-candidate"),
        );
    }

    #[tokio::test]
    async fn installed_identity_is_visible_while_async_prepare_is_blocked() {
        let entered = Arc::new(tokio::sync::Notify::new());
        let release = Arc::new(tokio::sync::Notify::new());
        let reader = BlockingCandidateReader {
            entered: Arc::clone(&entered),
            release,
        };
        let mut plan = sample_plan();
        plan.request_id = "request-blocked-prepare".to_string();
        let mut report_context = Some(json!({
            "request_id": "request-blocked-prepare",
            "candidate_index": 2,
            "retry_index": 1,
        }));
        install_execution_request_candidate_identity(&mut plan, &mut report_context);
        let installed_id = plan.candidate_id.clone().expect("installed candidate id");

        let mut prepare = Box::pin(prepare_execution_request_candidate_slot(
            &reader,
            &mut plan,
            &mut report_context,
        ));
        tokio::select! {
            _ = entered.notified() => {}
            result = &mut prepare => panic!("prepare unexpectedly completed: {result:?}"),
        }
        drop(prepare);

        assert_eq!(plan.candidate_id.as_deref(), Some(installed_id.as_str()));
        assert_eq!(
            report_context
                .as_ref()
                .and_then(|context| context.get("candidate_id"))
                .and_then(Value::as_str),
            Some(installed_id.as_str()),
        );
    }

    fn sample_provider(concurrent_limit: Option<i32>) -> StoredProviderCatalogProvider {
        StoredProviderCatalogProvider::new(
            "provider-request-candidate-seed-123".to_string(),
            "Provider".to_string(),
            Some("https://example.com".to_string()),
            "openai".to_string(),
        )
        .expect("provider should build")
        .with_transport_fields(
            true,
            false,
            false,
            concurrent_limit,
            None,
            None,
            None,
            None,
            None,
        )
    }

    fn sample_provider_key(
        concurrent_limit: Option<i32>,
        rpm_limit: Option<u32>,
    ) -> StoredProviderCatalogKey {
        StoredProviderCatalogKey::new(
            "key-request-candidate-seed-123".to_string(),
            "provider-request-candidate-seed-123".to_string(),
            "Provider key".to_string(),
            "api_key".to_string(),
            None,
            true,
        )
        .expect("provider key should build")
        .with_rate_limit_fields(
            rpm_limit,
            concurrent_limit,
            None,
            None,
            None,
            None,
            None,
            None,
            None,
        )
    }

    fn sample_auth_context(api_key_concurrent_limit: Option<i32>) -> GatewayControlAuthContext {
        GatewayControlAuthContext {
            user_id: "user-1".to_string(),
            api_key_id: "api-key-1".to_string(),
            username: None,
            api_key_name: None,
            balance_remaining: None,
            access_allowed: true,
            user_rate_limit: None,
            api_key_rate_limit: None,
            api_key_concurrent_limit,
            api_key_is_standalone: false,
            admin_bypass_limits: false,
            local_rejection: None,
            allowed_models: None,
            ip_rules: None,
        }
    }

    fn runtime_candidate(
        id: &str,
        status: RequestCandidateStatus,
        observed_at_unix_secs: u64,
        finished: bool,
        concurrent_requests: Option<u32>,
    ) -> StoredRequestCandidate {
        StoredRequestCandidate {
            id: id.to_string(),
            request_id: format!("request-{id}"),
            user_id: Some("user-1".to_string()),
            api_key_id: Some("api-key-1".to_string()),
            username: None,
            api_key_name: None,
            candidate_index: 0,
            retry_index: 0,
            provider_id: Some("provider-request-candidate-seed-123".to_string()),
            endpoint_id: Some("endpoint-request-candidate-seed-123".to_string()),
            key_id: Some("key-request-candidate-seed-123".to_string()),
            status,
            skip_reason: None,
            is_cached: false,
            status_code: None,
            error_type: None,
            error_message: None,
            latency_ms: None,
            concurrent_requests,
            extra_data: None,
            required_capabilities: None,
            created_at_unix_ms: observed_at_unix_secs * 1_000,
            started_at_unix_ms: Some(observed_at_unix_secs * 1_000),
            finished_at_unix_ms: finished.then_some(observed_at_unix_secs * 1_000),
        }
    }

    #[tokio::test]
    async fn fixes_candidate_identity_even_when_writer_is_unavailable() {
        let mut plan = sample_plan();
        let mut report_context = None;

        let prepared = prepare_execution_request_candidate_slot(
            &UnavailableStatusWriter,
            &mut plan,
            &mut report_context,
        )
        .await
        .expect("candidate slot should be prepared");

        assert!(prepared.pending.is_none());
        let candidate_id = plan
            .candidate_id
            .as_deref()
            .expect("candidate id should be fixed");
        assert!(!candidate_id.trim().is_empty());
        assert_eq!(
            report_context
                .as_ref()
                .and_then(|context| context.get("candidate_id"))
                .and_then(serde_json::Value::as_str),
            Some(candidate_id),
        );
        assert_eq!(
            report_context
                .as_ref()
                .and_then(|context| context.get("provider_id"))
                .and_then(serde_json::Value::as_str),
            Some("provider-request-candidate-seed-123"),
        );
    }

    #[tokio::test]
    async fn preparing_candidate_slots_does_not_publish_pending_before_admission() {
        let repository = Arc::new(InMemoryRequestCandidateRepository::default());
        let state = build_test_state(Arc::clone(&repository));
        let mut first_plan = sample_plan();
        first_plan.request_id = "request-before-admission-1".to_string();
        let mut first_context = None;
        let mut second_plan = sample_plan();
        second_plan.request_id = "request-before-admission-2".to_string();
        let mut second_context = None;

        let _first =
            prepare_execution_request_candidate_slot(&state, &mut first_plan, &mut first_context)
                .await
                .expect("first candidate slot should be prepared");
        let _second =
            prepare_execution_request_candidate_slot(&state, &mut second_plan, &mut second_context)
                .await
                .expect("second candidate slot should be prepared");

        assert_ne!(first_plan.candidate_id, second_plan.candidate_id);
        assert!(repository
            .list_by_request_id("request-before-admission-1")
            .await
            .expect("first request candidates should read")
            .is_empty());
        assert!(repository
            .list_by_request_id("request-before-admission-2")
            .await
            .expect("second request candidates should read")
            .is_empty());
    }

    #[tokio::test]
    async fn concurrent_first_preparations_use_one_canonical_slot_identity() {
        let repository = Arc::new(InMemoryRequestCandidateRepository::default());
        let state = build_test_state(Arc::clone(&repository));
        let mut first_plan = sample_plan();
        first_plan.candidate_id = Some("caller-candidate-first".to_string());
        let mut first_context = Some(json!({
            "request_id": "request-canonical-concurrent",
            "candidate_id": "caller-candidate-first",
            "candidate_index": 3,
            "retry_index": 2,
        }));
        let mut second_plan = first_plan.clone();
        second_plan.candidate_id = Some("caller-candidate-second".to_string());
        let mut second_context = Some(json!({
            "request_id": "request-canonical-concurrent",
            "candidate_id": "caller-candidate-second",
            "candidate_index": 3,
            "retry_index": 2,
        }));

        let (first, second) = tokio::join!(
            prepare_execution_request_candidate_slot(&state, &mut first_plan, &mut first_context,),
            prepare_execution_request_candidate_slot(&state, &mut second_plan, &mut second_context,),
        );
        first.expect("first candidate slot should be prepared");
        second.expect("second candidate slot should be prepared");

        assert_eq!(first_plan.candidate_id, second_plan.candidate_id);
        assert_ne!(
            first_plan.candidate_id.as_deref(),
            Some("caller-candidate-first")
        );
        assert_ne!(
            second_plan.candidate_id.as_deref(),
            Some("caller-candidate-second")
        );
        let canonical_id = first_plan
            .candidate_id
            .as_deref()
            .expect("canonical candidate id");
        assert_eq!(
            first_context
                .as_ref()
                .and_then(|context| context.get("candidate_id"))
                .and_then(serde_json::Value::as_str),
            Some(canonical_id),
        );
        assert_eq!(
            second_context
                .as_ref()
                .and_then(|context| context.get("candidate_id"))
                .and_then(serde_json::Value::as_str),
            Some(canonical_id),
        );
    }

    #[tokio::test]
    async fn persisted_slot_identity_overrides_caller_candidate_ids() {
        let repository = Arc::new(InMemoryRequestCandidateRepository::default());
        let state = build_test_state(Arc::clone(&repository));
        let mut first_plan = sample_plan();
        first_plan.request_id = "request-persisted-canonical".to_string();
        let mut first_context = Some(json!({
            "request_id": "request-persisted-canonical",
            "candidate_index": 4,
            "retry_index": 1,
        }));
        ensure_execution_request_candidate_slot(&state, &mut first_plan, &mut first_context)
            .await
            .expect("first candidate slot should persist");
        let canonical_id = first_plan
            .candidate_id
            .clone()
            .expect("canonical candidate id");

        let mut repeated_plan = sample_plan();
        repeated_plan.request_id = "request-persisted-canonical".to_string();
        repeated_plan.candidate_id = Some("caller-repeated-id".to_string());
        let mut repeated_context = Some(json!({
            "request_id": "request-persisted-canonical",
            "candidate_id": "caller-repeated-id",
            "candidate_index": 4,
            "retry_index": 1,
        }));
        let prepared = prepare_execution_request_candidate_slot(
            &state,
            &mut repeated_plan,
            &mut repeated_context,
        )
        .await
        .expect("persisted candidate slot should resolve");

        assert!(prepared.pending.is_none());
        assert_eq!(
            repeated_plan.candidate_id.as_deref(),
            Some(canonical_id.as_str())
        );
        assert_eq!(
            repeated_context
                .as_ref()
                .and_then(|context| context.get("candidate_id"))
                .and_then(serde_json::Value::as_str),
            Some(canonical_id.as_str()),
        );
    }

    #[tokio::test]
    async fn async_prepare_accepts_legacy_non_deterministic_persisted_slot() {
        let mut legacy = runtime_candidate(
            "legacy-random-candidate-id",
            RequestCandidateStatus::Available,
            crate::clock::current_unix_secs(),
            false,
            None,
        );
        legacy.request_id = "request-legacy-canonical".to_string();
        legacy.candidate_index = 5;
        legacy.retry_index = 3;
        let repository = Arc::new(InMemoryRequestCandidateRepository::seed([legacy]));
        let state = build_test_state(repository);
        let mut plan = sample_plan();
        plan.request_id = "request-legacy-canonical".to_string();
        plan.candidate_id = None;
        let mut report_context = Some(json!({
            "request_id": "request-legacy-canonical",
            "candidate_index": 5,
            "retry_index": 3,
        }));
        install_execution_request_candidate_identity(&mut plan, &mut report_context);
        let startup_identity = plan.candidate_id.clone().expect("startup identity");

        let prepared =
            prepare_execution_request_candidate_slot(&state, &mut plan, &mut report_context)
                .await
                .expect("legacy slot should remain usable");

        assert!(prepared.pending.is_none());
        assert_ne!(startup_identity, "legacy-random-candidate-id");
        assert_eq!(
            plan.candidate_id.as_deref(),
            Some("legacy-random-candidate-id")
        );
        assert_eq!(
            report_context
                .as_ref()
                .and_then(|context| context.get("candidate_id"))
                .and_then(Value::as_str),
            Some("legacy-random-candidate-id"),
        );
    }

    #[tokio::test]
    async fn rejected_candidate_is_skipped_without_counting_as_an_attempt() {
        let repository = Arc::new(InMemoryRequestCandidateRepository::default());
        let state = build_test_state(Arc::clone(&repository));
        let mut plan = sample_plan();
        plan.request_id = "request-reservation-rejected".to_string();
        let mut report_context = None;
        let _prepared =
            prepare_execution_request_candidate_slot(&state, &mut plan, &mut report_context)
                .await
                .expect("candidate slot should be prepared");

        record_local_request_candidate_status(
            &state,
            &plan,
            report_context.as_ref(),
            SchedulerRequestCandidateStatusUpdate {
                status: RequestCandidateStatus::Skipped,
                status_code: Some(429),
                error_type: Some("execution_reservation_rejected".to_string()),
                error_message: Some("Gateway execution capacity is busy".to_string()),
                latency_ms: None,
                started_at_unix_ms: None,
                finished_at_unix_ms: Some(crate::clock::current_unix_ms()),
            },
        )
        .await;

        let stored = repository
            .list_by_request_id("request-reservation-rejected")
            .await
            .expect("rejected request candidate should read");
        assert_eq!(stored.len(), 1);
        let rejected = &stored[0];
        assert_eq!(rejected.status, RequestCandidateStatus::Skipped);
        assert_eq!(rejected.started_at_unix_ms, None);
        assert!(rejected.finished_at_unix_ms.is_some());
        assert!(!rejected.status.is_attempted(rejected.started_at_unix_ms));

        let limits = ExecutionRequestReservationLimits::from_catalog_and_auth(
            &sample_provider(Some(1)),
            &sample_provider_key(Some(1), Some(1)),
            Some(&sample_auth_context(Some(1))),
            crate::clock::current_unix_secs(),
        )
        .expect("reservation limits should build");
        let input = build_execution_request_reservation_input(
            "next-candidate",
            &limits,
            &stored,
            crate::clock::current_unix_secs(),
            None,
        );

        assert!(input
            .provider
            .expect("provider concurrency scope")
            .observed_candidate_ids
            .is_empty());
        assert!(input
            .provider_key
            .expect("provider-key concurrency scope")
            .observed_candidate_ids
            .is_empty());
        assert!(input
            .api_key
            .expect("API-key concurrency scope")
            .observed_candidate_ids
            .is_empty());
        let rpm = input.provider_key_rpm.expect("provider-key RPM scope");
        assert!(rpm.observed_candidate_ids.is_empty());
        assert_eq!(rpm.observed_count_floor, 0);
    }

    #[test]
    fn builds_all_execution_reservation_scopes_with_matching_time_windows() {
        let provider = sample_provider(Some(3));
        let key = sample_provider_key(Some(2), Some(9));
        let auth = sample_auth_context(Some(4));
        let limits = ExecutionRequestReservationLimits::from_catalog_and_auth(
            &provider,
            &key,
            Some(&auth),
            1_000,
        )
        .expect("limits should build");
        let candidates = vec![
            runtime_candidate(
                "active",
                RequestCandidateStatus::Pending,
                999,
                false,
                Some(7),
            ),
            runtime_candidate("finished", RequestCandidateStatus::Success, 998, true, None),
            runtime_candidate(
                "at-reset",
                RequestCandidateStatus::Streaming,
                995,
                false,
                None,
            ),
            runtime_candidate("stale", RequestCandidateStatus::Pending, 699, false, None),
        ];

        let input = build_execution_request_reservation_input(
            "candidate-new",
            &limits,
            &candidates,
            1_000,
            Some(995),
        );

        assert_eq!(input.candidate_id, "candidate-new");
        assert_eq!(input.provider.as_ref().map(|scope| scope.limit), Some(3));
        assert_eq!(
            input.provider_key.as_ref().map(|scope| scope.limit),
            Some(2)
        );
        assert_eq!(input.api_key.as_ref().map(|scope| scope.limit), Some(4));
        for scope in [&input.provider, &input.provider_key, &input.api_key] {
            assert_eq!(
                scope
                    .as_ref()
                    .map(|scope| scope.observed_candidate_ids.as_slice()),
                Some(["active".to_string(), "at-reset".to_string()].as_slice()),
            );
        }
        let rpm = input.provider_key_rpm.expect("RPM scope should exist");
        assert_eq!(rpm.limit, 9);
        assert_eq!(
            rpm.observed_candidate_ids,
            vec!["active".to_string(), "finished".to_string()]
        );
        assert_eq!(rpm.observed_count_floor, 7);
        assert_eq!(rpm.reset_after_unix_secs, Some(995));
    }

    #[test]
    fn preserves_saturation_scope_and_limit_for_public_mapping() {
        assert_eq!(
            map_execution_reservation_error(ExecutionReservationError::Rejected {
                scope: ExecutionReservationScope::ProviderKeyRpm,
                limit: 60,
            }),
            ExecutionRequestReservationError::Saturated {
                scope: ExecutionReservationScope::ProviderKeyRpm,
                limit: 60,
            },
        );
    }

    #[test]
    fn streaming_snapshot_uses_synchronous_status_enqueue_fast_path() {
        let mut plan = sample_plan();
        plan.request_id = "attempt-streaming-fast-path".to_string();
        plan.candidate_id = Some("candidate-streaming-fast-path".to_string());
        let report_context = json!({"request_id": "root-streaming-fast-path"});
        let snapshot = snapshot_local_request_candidate_status(&plan, Some(&report_context))
            .expect("candidate snapshot should build");
        let writer = SynchronousStatusWriter::default();

        try_enqueue_local_request_candidate_status_snapshot(
            &writer,
            &snapshot,
            SchedulerRequestCandidateStatusUpdate {
                status: RequestCandidateStatus::Streaming,
                status_code: Some(200),
                error_type: None,
                error_message: None,
                latency_ms: None,
                started_at_unix_ms: Some(123),
                finished_at_unix_ms: None,
            },
        )
        .expect("streaming status should use the synchronous enqueue path");

        let records = writer
            .records
            .lock()
            .expect("synchronous status records lock");
        assert_eq!(records.len(), 1);
        assert_eq!(records[0].request_id, "root-streaming-fast-path");
        assert_eq!(records[0].status, RequestCandidateStatus::Streaming);
        assert_eq!(records[0].status_code, Some(200));
    }

    fn sample_minimal_candidate() -> SchedulerMinimalCandidateSelectionCandidate {
        SchedulerMinimalCandidateSelectionCandidate {
            provider_id: "provider-1".to_string(),
            provider_name: "Provider".to_string(),
            provider_type: "custom".to_string(),
            provider_priority: 0,
            endpoint_id: "endpoint-1".to_string(),
            endpoint_api_format: "openai:chat".to_string(),
            key_id: "provider-key-1".to_string(),
            key_name: "provider-key-1".to_string(),
            key_auth_type: "api_key".to_string(),
            key_internal_priority: 0,
            key_global_priority_for_format: Some(0),
            key_capabilities: Some(json!({"provider_only_capability": true})),
            model_id: "model-1".to_string(),
            global_model_id: "global-model-1".to_string(),
            global_model_name: "gpt-5".to_string(),
            selected_provider_model_name: "gpt-5".to_string(),
            supports_streaming: true,
            mapping_matched_model: None,
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

        ensure_execution_request_candidate_slot(&state, &mut plan, &mut report_context)
            .await
            .expect("candidate slot should be seeded");

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
    async fn dropped_startup_guard_terminalizes_a_seeded_candidate() {
        let repository = Arc::new(InMemoryRequestCandidateRepository::default());
        let state = build_test_state(Arc::clone(&repository));
        let mut plan = sample_plan();
        let mut report_context = Some(json!({
            "request_id": "req-request-candidate-seed-123",
            "client_api_format": "openai:chat"
        }));
        let prepared =
            prepare_execution_request_candidate_slot(&state, &mut plan, &mut report_context)
                .await
                .expect("candidate slot should be prepared");
        let guard =
            ExecutionRequestCandidateStartupGuard::new(&state, &plan, report_context.as_ref());

        persist_execution_request_candidate_slot(&state, &mut plan, &mut report_context, prepared)
            .await
            .expect("candidate slot should be persisted");
        drop(guard);

        let stored = wait_for_candidate_status(
            repository.as_ref(),
            "req-request-candidate-seed-123",
            RequestCandidateStatus::Cancelled,
        )
        .await;
        assert_eq!(stored.status_code, Some(499));
        assert_eq!(stored.error_type.as_deref(), Some("websocket_cancelled"));
        assert!(stored.finished_at_unix_ms.is_some());
    }

    #[tokio::test]
    async fn late_pending_seed_cannot_revive_a_cancelled_startup_candidate() {
        let repository = Arc::new(InMemoryRequestCandidateRepository::default());
        let state = build_test_state(Arc::clone(&repository));
        let mut plan = sample_plan();
        let mut report_context = Some(json!({
            "request_id": "req-request-candidate-seed-123",
            "client_api_format": "openai:chat"
        }));
        let prepared =
            prepare_execution_request_candidate_slot(&state, &mut plan, &mut report_context)
                .await
                .expect("candidate slot should be prepared");
        let guard =
            ExecutionRequestCandidateStartupGuard::new(&state, &plan, report_context.as_ref());

        drop(guard);
        wait_for_candidate_status(
            repository.as_ref(),
            "req-request-candidate-seed-123",
            RequestCandidateStatus::Cancelled,
        )
        .await;
        persist_execution_request_candidate_slot(&state, &mut plan, &mut report_context, prepared)
            .await
            .expect("late candidate seed should preserve identity");

        let stored = repository
            .list_by_request_id("req-request-candidate-seed-123")
            .await
            .expect("request candidates should read");
        assert_eq!(stored.len(), 1);
        assert_eq!(stored[0].status, RequestCandidateStatus::Cancelled);
        assert_eq!(stored[0].status_code, Some(499));
    }

    #[tokio::test]
    async fn http_stream_startup_guard_owns_watchdog_timeout_terminal() {
        let repository = Arc::new(InMemoryRequestCandidateRepository::default());
        let state = build_test_state(Arc::clone(&repository));
        let mut plan = sample_plan();
        plan.request_id = "req-stream-startup-watchdog-timeout".to_string();
        let mut report_context = Some(json!({
            "request_id": "req-stream-startup-watchdog-timeout",
            "client_api_format": "openai:chat"
        }));
        install_execution_request_candidate_identity(&mut plan, &mut report_context);

        let progress = StreamCandidateWatchdogProgress::shared();
        progress
            .clone()
            .scope({
                let progress = Arc::clone(&progress);
                async move {
                    let guard = ExecutionRequestCandidateStartupGuard::new_http_stream(
                        &state,
                        &plan,
                        report_context.as_ref(),
                    );
                    assert!(progress.attempt_guard_armed());
                    progress.mark_watchdog_timeout();
                    drop(guard);
                }
            })
            .await;

        assert!(progress.terminal_owner_claimed());
        tokio::time::timeout(
            Duration::from_secs(1),
            progress.wait_for_terminal_completion(),
        )
        .await
        .expect("startup timeout owner should signal terminal completion");
        let stored = wait_for_candidate_status(
            repository.as_ref(),
            "req-stream-startup-watchdog-timeout",
            RequestCandidateStatus::Failed,
        )
        .await;
        assert_eq!(
            stored.status_code,
            Some(http::StatusCode::GATEWAY_TIMEOUT.as_u16())
        );
        assert_eq!(
            stored.error_type.as_deref(),
            Some(STREAM_CANDIDATE_WATCHDOG_TIMEOUT_ERROR_TYPE)
        );
        let stored = repository
            .list_by_request_id("req-stream-startup-watchdog-timeout")
            .await
            .expect("request candidates should read");
        assert_eq!(stored.len(), 1);
        assert_eq!(stored[0].status, RequestCandidateStatus::Failed);
    }

    #[tokio::test]
    async fn canonicalizes_matching_plan_and_report_candidate_ids_for_a_new_slot() {
        let repository = Arc::new(InMemoryRequestCandidateRepository::default());
        let state = build_test_state(Arc::clone(&repository));
        let mut plan = sample_plan();
        plan.candidate_id = Some("cand-existing-123".to_string());
        let mut report_context = Some(json!({
            "request_id": "req-request-candidate-seed-123",
            "candidate_id": "cand-existing-123"
        }));

        ensure_execution_request_candidate_slot(&state, &mut plan, &mut report_context)
            .await
            .expect("existing candidate slot should resolve");

        let canonical_id = plan
            .candidate_id
            .as_deref()
            .expect("canonical candidate id")
            .to_string();
        assert_ne!(canonical_id, "cand-existing-123");
        let stored = repository
            .list_by_request_id("req-request-candidate-seed-123")
            .await
            .expect("request candidates should read");
        assert_eq!(stored.len(), 1);
        assert_eq!(stored[0].id, canonical_id);
        assert_eq!(
            report_context
                .as_ref()
                .and_then(|value| value.get("candidate_id"))
                .and_then(|value| value.as_str()),
            Some(canonical_id.as_str())
        );
    }

    #[tokio::test]
    async fn seeds_execution_request_candidate_slot_when_plan_candidate_id_lacks_report_context() {
        let repository = Arc::new(InMemoryRequestCandidateRepository::default());
        let state = build_test_state(Arc::clone(&repository));
        let mut plan = sample_plan();
        plan.candidate_id = Some("cand-existing-123".to_string());
        let mut report_context = None;

        ensure_execution_request_candidate_slot(&state, &mut plan, &mut report_context)
            .await
            .expect("candidate slot should be seeded");

        let canonical_id = plan
            .candidate_id
            .as_deref()
            .expect("canonical candidate id")
            .to_string();
        assert_ne!(canonical_id, "cand-existing-123");
        let report_context = report_context.expect("report context should be populated");
        assert_eq!(
            report_context
                .get("candidate_id")
                .and_then(|value| value.as_str()),
            Some(canonical_id.as_str())
        );
        let stored = repository
            .list_by_request_id("req-request-candidate-seed-123")
            .await
            .expect("request candidates should read");
        assert_eq!(stored.len(), 1);
        assert_eq!(stored[0].id, canonical_id);
        assert_eq!(stored[0].status, RequestCandidateStatus::Pending);
    }

    #[tokio::test]
    async fn records_report_request_candidate_status_for_existing_slot() {
        let repository = Arc::new(InMemoryRequestCandidateRepository::seed(vec![
            StoredRequestCandidate::new(
                "cand-report-123".to_string(),
                "req-report-123".to_string(),
                Some("user-1".to_string()),
                Some("api-key-1".to_string()),
                None,
                None,
                0,
                0,
                Some("provider-report-123".to_string()),
                Some("endpoint-report-123".to_string()),
                Some("key-report-123".to_string()),
                RequestCandidateStatus::Pending,
                None,
                false,
                None,
                None,
                None,
                None,
                None,
                None,
                None,
                100_000,
                Some(100_000),
                None,
            )
            .expect("request candidate should build"),
        ]));
        let state = build_test_state(Arc::clone(&repository));
        let report_context = json!({
            "request_id": "req-report-123",
            "candidate_id": "cand-report-123",
            "candidate_index": 0,
            "retry_index": 0,
            "provider_id": "provider-report-123",
            "endpoint_id": "endpoint-report-123",
            "key_id": "key-report-123"
        });

        record_report_request_candidate_status(
            &state,
            Some(&report_context),
            SchedulerRequestCandidateStatusUpdate {
                status: RequestCandidateStatus::Success,
                status_code: Some(200),
                error_type: None,
                error_message: None,
                latency_ms: Some(25),
                started_at_unix_ms: Some(101),
                finished_at_unix_ms: Some(102),
            },
        )
        .await;

        let stored = repository
            .list_by_request_id("req-report-123")
            .await
            .expect("request candidates should read");
        assert_eq!(stored.len(), 1);
        assert_eq!(stored[0].id, "cand-report-123");
        assert_eq!(stored[0].status, RequestCandidateStatus::Success);
        assert_eq!(stored[0].status_code, Some(200));
        assert_eq!(stored[0].latency_ms, Some(25));
        assert_eq!(stored[0].started_at_unix_ms, Some(101));
        assert_eq!(stored[0].finished_at_unix_ms, Some(102));
    }

    #[tokio::test]
    async fn resolves_request_candidate_required_capabilities_from_user_model_and_api_key() {
        let repository = Arc::new(InMemoryRequestCandidateRepository::default());
        let auth_repository = Arc::new(
            InMemoryAuthApiKeySnapshotRepository::default().with_export_records(vec![
                StoredAuthApiKeyExportRecord::new(
                    "user-1".to_string(),
                    "api-key-1".to_string(),
                    "hash-1".to_string(),
                    None,
                    Some("default".to_string()),
                    None,
                    None,
                    None,
                    None,
                    None,
                    Some(json!({"cache_1h": false, "context_1m": true})),
                    true,
                    None,
                    false,
                    0,
                    0,
                    0.0,
                    false,
                )
                .expect("export record should build"),
            ]),
        );
        let state = build_test_state_with_auth(repository, auth_repository)
            .with_auth_user_model_capability_settings_for_tests(
                "user-1",
                json!({
                    "gpt-5": {
                        "cache_1h": true,
                        "context_1m": false
                    }
                }),
            );
        let explicit_required_capabilities = json!({"gemini_files": true});

        let required_capabilities = resolve_request_candidate_required_capabilities(
            &state,
            "user-1",
            "api-key-1",
            Some("gpt-5"),
            Some(&explicit_required_capabilities),
            None,
        )
        .await
        .expect("required capabilities should resolve");

        assert_eq!(required_capabilities["cache_1h"], json!(false));
        assert_eq!(required_capabilities["context_1m"], json!(true));
        assert_eq!(required_capabilities["gemini_files"], json!(true));
    }

    #[test]
    fn requested_model_capabilities_use_the_policy_resolved_base_model() {
        let base_only = json!({
            "deployment-alias": {
                "context_1m": true
            }
        });
        assert_eq!(
            select_requested_model_capabilities(
                Some(&base_only),
                Some("deployment-alias-VendorFuture"),
                Some("deployment-alias"),
            ),
            Some(&base_only["deployment-alias"])
        );
        assert_eq!(
            select_requested_model_capabilities(
                Some(&base_only),
                Some("deployment-alias-VendorFuture"),
                None,
            ),
            None
        );

        let exact_and_base = json!({
            "deployment-alias-VendorFuture": {
                "cache_1h": true
            },
            "deployment-alias": {
                "context_1m": true
            }
        });
        assert_eq!(
            select_requested_model_capabilities(
                Some(&exact_and_base),
                Some("deployment-alias-VendorFuture"),
                Some("deployment-alias"),
            ),
            Some(&exact_and_base["deployment-alias-VendorFuture"])
        );
    }

    #[tokio::test]
    async fn persists_request_required_capabilities_instead_of_provider_key_capabilities() {
        let repository = Arc::new(InMemoryRequestCandidateRepository::default());
        let state = build_test_state(Arc::clone(&repository));
        let required_capabilities = json!({"cache_1h": true});

        persist_available_local_candidate(
            &state,
            "req-runtime-cap-123",
            "user-1",
            "api-key-1",
            &sample_minimal_candidate(),
            0,
            0,
            "cand-runtime-cap-123",
            Some(&required_capabilities),
            None,
            100_000,
            "request candidate persist should succeed",
        )
        .await;

        let stored = repository
            .list_by_request_id("req-runtime-cap-123")
            .await
            .expect("request candidates should read");
        assert_eq!(stored.len(), 1);
        assert_eq!(
            stored[0].required_capabilities,
            Some(required_capabilities.clone())
        );
        assert_ne!(
            stored[0].required_capabilities,
            sample_minimal_candidate().key_capabilities
        );
    }
}
