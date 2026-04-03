use crate::gateway::constants::{
    CONTROL_ENDPOINT_SIGNATURE_HEADER, CONTROL_EXECUTION_RUNTIME_HEADER,
    CONTROL_ROUTE_CLASS_HEADER, CONTROL_ROUTE_FAMILY_HEADER, CONTROL_ROUTE_KIND_HEADER,
    DEPENDENCY_REASON_HEADER, EXECUTION_PATH_HEADER, LOCAL_EXECUTION_RUNTIME_MISS_REASON_HEADER,
    TRACE_ID_HEADER,
};
use crate::gateway::{
    record_shadow_result_non_blocking, AppState, GatewayControlDecision,
    GatewayPublicRequestContext,
};
use aether_runtime::{maybe_hold_axum_response_permit, AdmissionPermit};
use axum::body::{Body, Bytes};
use axum::http::{self, header::HeaderName, header::HeaderValue, Response};
use std::time::Instant;
use tracing::info;

pub(super) fn request_wants_stream(
    request_context: &GatewayPublicRequestContext,
    body: &axum::body::Bytes,
) -> bool {
    if request_context
        .request_path
        .contains(":streamGenerateContent")
    {
        return true;
    }
    if !request_context
        .request_content_type
        .as_deref()
        .map(|value| value.to_ascii_lowercase().contains("application/json"))
        .unwrap_or(false)
        || body.is_empty()
    {
        return false;
    }
    serde_json::from_slice::<serde_json::Value>(body)
        .ok()
        .and_then(|value| value.get("stream").and_then(|stream| stream.as_bool()))
        .unwrap_or(false)
}

pub(super) fn finalize_gateway_response(
    state: &AppState,
    mut response: Response<Body>,
    trace_id: &str,
    remote_addr: &std::net::SocketAddr,
    method: &http::Method,
    path_and_query: &str,
    control_decision: Option<&GatewayControlDecision>,
    execution_path: &'static str,
    started_at: &Instant,
    request_permit: Option<AdmissionPermit>,
) -> Response<Body> {
    attach_control_decision_headers(&mut response, control_decision);
    if !response.headers().contains_key(TRACE_ID_HEADER) {
        response.headers_mut().insert(
            HeaderName::from_static(TRACE_ID_HEADER),
            HeaderValue::from_str(trace_id).expect("trace id should be a valid header value"),
        );
    }
    response.headers_mut().insert(
        HeaderName::from_static(EXECUTION_PATH_HEADER),
        HeaderValue::from_static(execution_path),
    );

    let elapsed_ms = started_at.elapsed().as_millis() as u64;
    let dependency_reason = response
        .headers()
        .get(DEPENDENCY_REASON_HEADER)
        .and_then(|value| value.to_str().ok())
        .unwrap_or("none");
    let local_execution_runtime_miss_reason = response
        .headers()
        .get(LOCAL_EXECUTION_RUNTIME_MISS_REASON_HEADER)
        .and_then(|value| value.to_str().ok())
        .unwrap_or("none");
    info!(
        trace_id = %trace_id,
        remote_addr = %remote_addr,
        method = %method,
        path = %path_and_query,
        route_class = control_decision
            .and_then(|decision| decision.route_class.as_deref())
            .unwrap_or("passthrough"),
        execution_path,
        dependency_reason,
        local_execution_runtime_miss_reason,
        status = response.status().as_u16(),
        elapsed_ms,
        "gateway completed request"
    );

    record_shadow_result_non_blocking(
        state.clone(),
        trace_id,
        method,
        path_and_query,
        control_decision,
        execution_path,
        &response,
    );

    maybe_hold_axum_response_permit(response, request_permit)
}

fn attach_control_decision_headers(
    response: &mut Response<Body>,
    control_decision: Option<&GatewayControlDecision>,
) {
    let Some(control_decision) = control_decision else {
        return;
    };
    if !response.headers().contains_key(CONTROL_ROUTE_CLASS_HEADER) {
        response.headers_mut().insert(
            HeaderName::from_static(CONTROL_ROUTE_CLASS_HEADER),
            HeaderValue::from_str(
                control_decision
                    .route_class
                    .as_deref()
                    .unwrap_or("passthrough"),
            )
            .expect("route class should be a valid header value"),
        );
    }
    if !response
        .headers()
        .contains_key(CONTROL_EXECUTION_RUNTIME_HEADER)
    {
        response.headers_mut().insert(
            HeaderName::from_static(CONTROL_EXECUTION_RUNTIME_HEADER),
            HeaderValue::from_static(if control_decision.is_execution_runtime_candidate() {
                "true"
            } else {
                "false"
            }),
        );
    }
    if let Some(route_family) = control_decision.route_family.as_deref() {
        if !response.headers().contains_key(CONTROL_ROUTE_FAMILY_HEADER) {
            response.headers_mut().insert(
                HeaderName::from_static(CONTROL_ROUTE_FAMILY_HEADER),
                HeaderValue::from_str(route_family)
                    .expect("route family should be a valid header value"),
            );
        }
    }
    if let Some(route_kind) = control_decision.route_kind.as_deref() {
        if !response.headers().contains_key(CONTROL_ROUTE_KIND_HEADER) {
            response.headers_mut().insert(
                HeaderName::from_static(CONTROL_ROUTE_KIND_HEADER),
                HeaderValue::from_str(route_kind)
                    .expect("route kind should be a valid header value"),
            );
        }
    }
    if let Some(endpoint_signature) = control_decision.auth_endpoint_signature.as_deref() {
        if !response
            .headers()
            .contains_key(CONTROL_ENDPOINT_SIGNATURE_HEADER)
        {
            response.headers_mut().insert(
                HeaderName::from_static(CONTROL_ENDPOINT_SIGNATURE_HEADER),
                HeaderValue::from_str(endpoint_signature)
                    .expect("endpoint signature should be a valid header value"),
            );
        }
    }
}

pub(super) fn finalize_gateway_response_with_context(
    state: &AppState,
    response: Response<Body>,
    remote_addr: &std::net::SocketAddr,
    request_context: &GatewayPublicRequestContext,
    execution_path: &'static str,
    started_at: &Instant,
    request_permit: Option<AdmissionPermit>,
) -> Response<Body> {
    finalize_gateway_response(
        state,
        response,
        &request_context.trace_id,
        remote_addr,
        &request_context.request_method,
        &request_context.request_path_and_query(),
        request_context.control_decision.as_ref(),
        execution_path,
        started_at,
        request_permit,
    )
}
