use crate::ai_pipeline::planner::common::{
    apply_local_candidate_evaluation_progress, apply_local_candidate_terminal_plan_reason,
    build_local_runtime_miss_diagnostic,
};
use crate::ai_pipeline::GatewayControlDecision;
use crate::{AppState, LocalExecutionRuntimeMissDiagnostic};

pub(crate) fn set_local_runtime_miss_diagnostic_reason(
    state: &AppState,
    trace_id: &str,
    decision: &GatewayControlDecision,
    plan_kind: &str,
    requested_model: Option<&str>,
    reason: &str,
) {
    state.set_local_execution_runtime_miss_diagnostic(
        trace_id,
        build_local_runtime_miss_diagnostic(decision, plan_kind, requested_model, reason),
    );
}

pub(crate) fn build_local_runtime_execution_exhausted_diagnostic(
    decision: &GatewayControlDecision,
    plan_kind: &str,
    requested_model: Option<&str>,
    candidate_count: usize,
) -> LocalExecutionRuntimeMissDiagnostic {
    let mut diagnostic = build_local_runtime_miss_diagnostic(
        decision,
        plan_kind,
        requested_model,
        "execution_runtime_candidates_exhausted",
    );
    diagnostic.candidate_count = Some(candidate_count);
    diagnostic
}

pub(crate) fn set_local_runtime_execution_exhausted_diagnostic(
    state: &AppState,
    trace_id: &str,
    decision: &GatewayControlDecision,
    plan_kind: &str,
    requested_model: Option<&str>,
    candidate_count: usize,
) {
    state.set_local_execution_runtime_miss_diagnostic(
        trace_id,
        build_local_runtime_execution_exhausted_diagnostic(
            decision,
            plan_kind,
            requested_model,
            candidate_count,
        ),
    );
}

pub(crate) fn build_local_runtime_candidate_evaluation_diagnostic(
    decision: &GatewayControlDecision,
    plan_kind: &str,
    requested_model: Option<&str>,
    candidate_count: usize,
) -> LocalExecutionRuntimeMissDiagnostic {
    let mut diagnostic = build_local_runtime_miss_diagnostic(
        decision,
        plan_kind,
        requested_model,
        "candidate_evaluation_incomplete",
    );
    apply_local_candidate_evaluation_progress(&mut diagnostic, candidate_count);
    diagnostic
}

pub(crate) fn set_local_runtime_candidate_evaluation_diagnostic(
    state: &AppState,
    trace_id: &str,
    decision: &GatewayControlDecision,
    plan_kind: &str,
    requested_model: Option<&str>,
    candidate_count: usize,
) {
    state.set_local_execution_runtime_miss_diagnostic(
        trace_id,
        build_local_runtime_candidate_evaluation_diagnostic(
            decision,
            plan_kind,
            requested_model,
            candidate_count,
        ),
    );
}

pub(crate) fn apply_local_runtime_candidate_evaluation_progress(
    state: &AppState,
    trace_id: &str,
    candidate_count: usize,
) {
    state.mutate_local_execution_runtime_miss_diagnostic(trace_id, |diagnostic| {
        apply_local_candidate_evaluation_progress(diagnostic, candidate_count);
    });
}

pub(crate) fn apply_local_runtime_candidate_evaluation_progress_preserving_candidate_signal(
    state: &AppState,
    trace_id: &str,
    candidate_count: usize,
) {
    let preserve_existing_candidate_signal = candidate_count == 0
        && state.local_execution_runtime_miss_diagnostic_has_candidate_signal(trace_id);
    if preserve_existing_candidate_signal {
        return;
    }
    apply_local_runtime_candidate_evaluation_progress(state, trace_id, candidate_count);
}

pub(crate) fn apply_local_runtime_candidate_terminal_reason(
    state: &AppState,
    trace_id: &str,
    no_plan_reason: &'static str,
) {
    state.mutate_local_execution_runtime_miss_diagnostic(trace_id, |diagnostic| {
        apply_local_candidate_terminal_plan_reason(diagnostic, no_plan_reason);
    });
}

pub(crate) fn record_local_runtime_candidate_skip_reason(
    state: &AppState,
    trace_id: &str,
    skip_reason: &'static str,
) {
    state.mutate_local_execution_runtime_miss_diagnostic(trace_id, |diagnostic| {
        *diagnostic
            .skip_reasons
            .entry(skip_reason.to_string())
            .or_insert(0) += 1;
        *diagnostic.skipped_candidate_count.get_or_insert(0) += 1;
    });
}
