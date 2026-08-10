use aether_ai_serving::{
    apply_ai_runtime_candidate_evaluation_progress,
    apply_ai_runtime_candidate_evaluation_progress_preserving_candidate_signal,
    apply_ai_runtime_candidate_evaluation_progress_to_diagnostic,
    apply_ai_runtime_candidate_terminal_plan_reason_to_diagnostic,
    apply_ai_runtime_candidate_terminal_reason, build_ai_runtime_candidate_evaluation_diagnostic,
    build_ai_runtime_execution_exhausted_diagnostic, record_ai_runtime_candidate_skip_reason,
    record_ai_runtime_candidate_skip_reason_on_diagnostic,
    set_ai_runtime_candidate_evaluation_diagnostic, set_ai_runtime_execution_exhausted_diagnostic,
    set_ai_runtime_miss_diagnostic_reason, AiRuntimeMissDiagnosticFields,
    AiRuntimeMissDiagnosticPort,
};

use crate::ai_serving::GatewayControlDecision;
use crate::{AppState, LocalExecutionRuntimeMissDiagnostic};

struct GatewayRuntimeMissDiagnosticPort<'a> {
    state: Option<&'a AppState>,
}

/// Removes a request-scoped diagnostic on every exit path, including task
/// cancellation and panic unwinding.
pub(crate) struct RuntimeMissDiagnosticCleanupGuard {
    state: AppState,
    request_id: String,
}

impl RuntimeMissDiagnosticCleanupGuard {
    pub(crate) fn new(state: &AppState, request_id: impl Into<String>) -> Self {
        Self {
            state: state.clone(),
            request_id: request_id.into(),
        }
    }
}

impl Drop for RuntimeMissDiagnosticCleanupGuard {
    fn drop(&mut self) {
        self.state
            .clear_local_execution_runtime_miss_diagnostic(&self.request_id);
    }
}

/// Selects the server-owned key used by the per-request runtime-miss state
/// machine. The public trace remains available to callers for logging only.
pub(crate) fn runtime_miss_diagnostic_key_from_parts<'a>(
    parts: &'a http::request::Parts,
    public_trace_id: &'a str,
) -> &'a str {
    crate::execution_identity::execution_request_id_from_parts(parts, public_trace_id)
}

impl AiRuntimeMissDiagnosticFields for LocalExecutionRuntimeMissDiagnostic {
    fn set_reason(&mut self, reason: String) {
        self.reason = reason;
    }

    fn set_candidate_count(&mut self, candidate_count: usize) {
        self.candidate_count = Some(candidate_count);
    }

    fn candidate_count(&self) -> Option<usize> {
        self.candidate_count
    }

    fn skipped_candidate_count(&self) -> Option<usize> {
        self.skipped_candidate_count
    }

    fn skip_reason_count(&self, skip_reason: &str) -> usize {
        self.skip_reasons.get(skip_reason).copied().unwrap_or(0)
    }

    fn skip_reason_len(&self) -> usize {
        self.skip_reasons.len()
    }

    fn record_skip_reason(&mut self, skip_reason: &'static str) {
        *self
            .skip_reasons
            .entry(skip_reason.to_string())
            .or_insert(0) += 1;
        *self.skipped_candidate_count.get_or_insert(0) += 1;
    }
}

impl AiRuntimeMissDiagnosticPort for GatewayRuntimeMissDiagnosticPort<'_> {
    type Decision = GatewayControlDecision;
    type Diagnostic = LocalExecutionRuntimeMissDiagnostic;

    fn build_runtime_miss_diagnostic(
        &self,
        decision: &Self::Decision,
        plan_kind: &str,
        requested_model: Option<&str>,
        reason: &str,
    ) -> Self::Diagnostic {
        LocalExecutionRuntimeMissDiagnostic {
            reason: reason.to_string(),
            route_family: decision.route_family.clone(),
            route_kind: decision.route_kind.clone(),
            public_path: Some(decision.public_path.clone()),
            plan_kind: Some(plan_kind.to_string()),
            requested_model: requested_model.map(ToOwned::to_owned),
            candidate_count: None,
            skipped_candidate_count: None,
            skip_reasons: std::collections::BTreeMap::new(),
        }
    }

    fn set_candidate_count(&self, diagnostic: &mut Self::Diagnostic, candidate_count: usize) {
        AiRuntimeMissDiagnosticFields::set_candidate_count(diagnostic, candidate_count);
    }

    fn apply_candidate_evaluation_progress(
        &self,
        diagnostic: &mut Self::Diagnostic,
        candidate_count: usize,
    ) {
        apply_ai_runtime_candidate_evaluation_progress_to_diagnostic(diagnostic, candidate_count);
    }

    fn apply_candidate_terminal_plan_reason(
        &self,
        diagnostic: &mut Self::Diagnostic,
        no_plan_reason: &'static str,
    ) {
        apply_ai_runtime_candidate_terminal_plan_reason_to_diagnostic(diagnostic, no_plan_reason);
    }

    fn record_candidate_skip_reason(
        &self,
        diagnostic: &mut Self::Diagnostic,
        skip_reason: &'static str,
    ) {
        record_ai_runtime_candidate_skip_reason_on_diagnostic(diagnostic, skip_reason);
    }

    fn set_runtime_miss_diagnostic(&self, trace_id: &str, diagnostic: Self::Diagnostic) {
        self.state
            .expect("runtime miss diagnostic setter requires gateway state")
            .set_local_execution_runtime_miss_diagnostic(trace_id, diagnostic);
    }

    fn mutate_runtime_miss_diagnostic<F>(&self, trace_id: &str, apply: F)
    where
        F: FnOnce(&mut Self::Diagnostic) + Send,
    {
        self.state
            .expect("runtime miss diagnostic mutator requires gateway state")
            .mutate_local_execution_runtime_miss_diagnostic(trace_id, apply);
    }

    fn runtime_miss_diagnostic_has_candidate_signal(&self, trace_id: &str) -> bool {
        self.state
            .expect("runtime miss diagnostic signal check requires gateway state")
            .local_execution_runtime_miss_diagnostic_has_candidate_signal(trace_id)
    }
}

pub(crate) fn set_local_runtime_miss_diagnostic_reason(
    state: &AppState,
    trace_id: &str,
    decision: &GatewayControlDecision,
    plan_kind: &str,
    requested_model: Option<&str>,
    reason: &str,
) {
    let port = GatewayRuntimeMissDiagnosticPort { state: Some(state) };
    set_ai_runtime_miss_diagnostic_reason(
        &port,
        trace_id,
        decision,
        plan_kind,
        requested_model,
        reason,
    );
}

pub(crate) fn build_local_runtime_execution_exhausted_diagnostic(
    decision: &GatewayControlDecision,
    plan_kind: &str,
    requested_model: Option<&str>,
    candidate_count: usize,
) -> LocalExecutionRuntimeMissDiagnostic {
    let port = GatewayRuntimeMissDiagnosticPort { state: None };
    build_ai_runtime_execution_exhausted_diagnostic(
        &port,
        decision,
        plan_kind,
        requested_model,
        candidate_count,
    )
}

pub(crate) fn set_local_runtime_execution_exhausted_diagnostic(
    state: &AppState,
    trace_id: &str,
    decision: &GatewayControlDecision,
    plan_kind: &str,
    requested_model: Option<&str>,
    candidate_count: usize,
) {
    let port = GatewayRuntimeMissDiagnosticPort { state: Some(state) };
    set_ai_runtime_execution_exhausted_diagnostic(
        &port,
        trace_id,
        decision,
        plan_kind,
        requested_model,
        candidate_count,
    );
}

pub(crate) fn build_local_runtime_candidate_evaluation_diagnostic(
    decision: &GatewayControlDecision,
    plan_kind: &str,
    requested_model: Option<&str>,
    candidate_count: usize,
) -> LocalExecutionRuntimeMissDiagnostic {
    let port = GatewayRuntimeMissDiagnosticPort { state: None };
    build_ai_runtime_candidate_evaluation_diagnostic(
        &port,
        decision,
        plan_kind,
        requested_model,
        candidate_count,
    )
}

pub(crate) fn set_local_runtime_candidate_evaluation_diagnostic(
    state: &AppState,
    trace_id: &str,
    decision: &GatewayControlDecision,
    plan_kind: &str,
    requested_model: Option<&str>,
    candidate_count: usize,
) {
    let port = GatewayRuntimeMissDiagnosticPort { state: Some(state) };
    set_ai_runtime_candidate_evaluation_diagnostic(
        &port,
        trace_id,
        decision,
        plan_kind,
        requested_model,
        candidate_count,
    );
}

pub(crate) fn apply_local_runtime_candidate_evaluation_progress(
    state: &AppState,
    trace_id: &str,
    candidate_count: usize,
) {
    let port = GatewayRuntimeMissDiagnosticPort { state: Some(state) };
    apply_ai_runtime_candidate_evaluation_progress(&port, trace_id, candidate_count);
}

pub(crate) fn apply_local_runtime_candidate_evaluation_progress_preserving_candidate_signal(
    state: &AppState,
    trace_id: &str,
    candidate_count: usize,
) {
    let port = GatewayRuntimeMissDiagnosticPort { state: Some(state) };
    apply_ai_runtime_candidate_evaluation_progress_preserving_candidate_signal(
        &port,
        trace_id,
        candidate_count,
    );
}

pub(crate) fn apply_local_runtime_candidate_terminal_reason(
    state: &AppState,
    trace_id: &str,
    no_plan_reason: &'static str,
) {
    let port = GatewayRuntimeMissDiagnosticPort { state: Some(state) };
    apply_ai_runtime_candidate_terminal_reason(&port, trace_id, no_plan_reason);
}

pub(crate) fn record_local_runtime_candidate_skip_reason(
    state: &AppState,
    trace_id: &str,
    skip_reason: &'static str,
) {
    let port = GatewayRuntimeMissDiagnosticPort { state: Some(state) };
    record_ai_runtime_candidate_skip_reason(&port, trace_id, skip_reason);
}

#[cfg(test)]
mod tests {
    use super::{
        runtime_miss_diagnostic_key_from_parts, set_local_runtime_miss_diagnostic_reason,
        RuntimeMissDiagnosticCleanupGuard,
    };
    use crate::control::GatewayControlDecision;
    use crate::execution_identity::ExecutionRequestId;
    use crate::AppState;

    #[test]
    fn identical_public_traces_keep_runtime_miss_diagnostics_isolated() {
        let state = AppState::new().expect("state should build");
        let public_trace_id = "shared-client-trace";
        let mut first = http::Request::new(()).into_parts().0;
        let mut second = http::Request::new(()).into_parts().0;
        first.extensions.insert(ExecutionRequestId::server_owned(
            "execution-request-1".to_string(),
        ));
        second.extensions.insert(ExecutionRequestId::server_owned(
            "execution-request-2".to_string(),
        ));
        let first_key = runtime_miss_diagnostic_key_from_parts(&first, public_trace_id).to_string();
        let second_key =
            runtime_miss_diagnostic_key_from_parts(&second, public_trace_id).to_string();
        let decision = GatewayControlDecision::synthetic(
            "/v1/responses",
            Some("ai_public".to_string()),
            Some("openai".to_string()),
            Some("responses".to_string()),
            Some("openai:responses".to_string()),
        );

        set_local_runtime_miss_diagnostic_reason(
            &state,
            &first_key,
            &decision,
            "openai_responses_sync",
            Some("gpt-5"),
            "first_request_reason",
        );
        set_local_runtime_miss_diagnostic_reason(
            &state,
            &second_key,
            &decision,
            "openai_responses_sync",
            Some("gpt-5"),
            "second_request_reason",
        );

        assert!(state
            .take_local_execution_runtime_miss_diagnostic(public_trace_id)
            .is_none());
        assert_eq!(
            state
                .take_local_execution_runtime_miss_diagnostic(&first_key)
                .expect("first diagnostic")
                .reason,
            "first_request_reason"
        );
        assert_eq!(
            state
                .take_local_execution_runtime_miss_diagnostic(&second_key)
                .expect("second diagnostic")
                .reason,
            "second_request_reason"
        );
    }

    #[test]
    fn cleanup_guard_removes_request_scoped_diagnostic() {
        let state = AppState::new().expect("state should build");
        let request_id = "execution-request-cleanup";
        let decision = GatewayControlDecision::synthetic(
            "/v1/responses",
            Some("ai_public".to_string()),
            Some("openai".to_string()),
            Some("responses".to_string()),
            Some("openai:responses".to_string()),
        );
        {
            let _cleanup = RuntimeMissDiagnosticCleanupGuard::new(&state, request_id);
            set_local_runtime_miss_diagnostic_reason(
                &state,
                request_id,
                &decision,
                "openai_responses_sync",
                Some("gpt-5"),
                "candidate_evaluation_incomplete",
            );
        }

        assert!(state
            .take_local_execution_runtime_miss_diagnostic(request_id)
            .is_none());
    }
}
