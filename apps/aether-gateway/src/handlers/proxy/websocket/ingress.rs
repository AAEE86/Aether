//! Authenticated public WebSocket upgrade admission shared by AI adapters.

use std::future::Future;
use std::net::SocketAddr;

use axum::body::Body;
use axum::extract::ws::{WebSocket, WebSocketUpgrade};
use axum::http::{HeaderMap, Method, Response, StatusCode, Uri};
use tracing::{info, warn};

use crate::api::response::{
    build_local_auth_rejection_response, build_local_http_error_response,
    build_local_overloaded_response,
};
use crate::control::{
    trusted_auth_local_rejection, GatewayControlDecision, GatewayLocalAuthRejection,
};
use crate::handlers::proxy::websocket::session::{WebSocketSessionLimits, WEBSOCKET_LOG_TRANSPORT};
use crate::handlers::shared::ip_rules_allow;
use crate::headers::{effective_client_ip, extract_or_generate_trace_id};
use crate::router::RequestAdmissionError;
use crate::{AppState, GatewayError};

/// Request facts that survive the HTTP Upgrade and are needed by a protocol
/// adapter for planning, rate limiting, and connection-scoped audit logs.
pub(crate) struct WebSocketRequestContext {
    pub(crate) trace_id: String,
    pub(crate) headers: HeaderMap,
    pub(crate) uri: Uri,
    pub(crate) remote_addr: SocketAddr,
    pub(crate) decision: GatewayControlDecision,
    pub(crate) rpm_bypassed: bool,
    /// Held for the lifetime of the upgraded socket. The Responses session
    /// polls its health and closes the client when a distributed lease is
    /// revoked or expires.
    pub(crate) websocket_connection_permit: Option<aether_runtime::AdmissionPermit>,
}

/// Adapter-specific wording and event identifiers for generic upgrade checks.
#[derive(Clone, Copy)]
pub(crate) struct WebSocketIngressSpec {
    pub(crate) route_unavailable_message: &'static str,
    pub(crate) ip_whitelist_failure_event_name: &'static str,
}

/// Performs the HTTP-only part of an AI WebSocket request.
///
/// The ordinary request permit covers only the HTTP Upgrade window. A
/// dedicated WebSocket connection permit is held for the socket lifetime so
/// idle clients cannot consume capacity reserved for normal HTTP requests.
pub(crate) async fn upgrade_authenticated_ai_websocket<F, Fut>(
    state: AppState,
    remote_addr: SocketAddr,
    ws: WebSocketUpgrade,
    headers: HeaderMap,
    uri: Uri,
    limits: WebSocketSessionLimits,
    spec: WebSocketIngressSpec,
    run_session: F,
) -> Result<Response<Body>, GatewayError>
where
    F: FnOnce(WebSocket, AppState, WebSocketRequestContext) -> Fut + Send + 'static,
    Fut: Future<Output = ()> + Send + 'static,
{
    let trace_id = extract_or_generate_trace_id(&headers);
    let client_ip = effective_client_ip(&headers, &remote_addr);
    if state.admin_security_ip_blacklisted(client_ip).await? {
        return build_local_http_error_response(
            &trace_id,
            None,
            StatusCode::FORBIDDEN,
            "当前 IP 已被禁止访问",
        );
    }

    let request_context = crate::control::resolve_public_request_context(
        &state,
        &Method::GET,
        &uri,
        &headers,
        &trace_id,
    )
    .await?;
    let Some(decision) = request_context.control_decision else {
        return build_local_http_error_response(
            &trace_id,
            None,
            StatusCode::NOT_FOUND,
            spec.route_unavailable_message,
        );
    };
    if let Some(rejection) = trusted_auth_local_rejection(Some(&decision), &headers) {
        return build_local_auth_rejection_response(&trace_id, Some(&decision), &rejection);
    }
    let Some(auth_context) = decision.auth_context.as_ref() else {
        return build_local_auth_rejection_response(
            &trace_id,
            Some(&decision),
            &GatewayLocalAuthRejection::InvalidApiKey,
        );
    };
    if !auth_context.access_allowed {
        return build_local_auth_rejection_response(
            &trace_id,
            Some(&decision),
            &GatewayLocalAuthRejection::InvalidApiKey,
        );
    }
    if !ip_rules_allow(auth_context.ip_rules.as_deref(), client_ip) {
        return build_local_auth_rejection_response(
            &trace_id,
            Some(&decision),
            &GatewayLocalAuthRejection::IpNotAllowed {
                remote_ip: client_ip.to_string(),
            },
        );
    }

    let ip_whitelisted = match state.admin_security_ip_whitelisted(client_ip).await {
        Ok(value) => value,
        Err(error) => {
            warn!(
                event_name = spec.ip_whitelist_failure_event_name,
                log_type = "ops",
                transport = WEBSOCKET_LOG_TRANSPORT,
                websocket = true,
                trace_id = %trace_id,
                client_ip = %client_ip,
                error = ?error,
                "gateway continued with WebSocket rate limiting after IP whitelist check error"
            );
            false
        }
    };
    let request_permit = match state.try_acquire_request_permit().await {
        Ok(permit) => permit,
        Err(error) => return websocket_admission_error_response(&trace_id, &decision, error),
    };
    let websocket_connection_permit = match state.try_acquire_websocket_connection_permit().await {
        Ok(permit) => permit,
        Err(error) => return websocket_admission_error_response(&trace_id, &decision, error),
    };

    let context = WebSocketRequestContext {
        trace_id,
        headers,
        uri,
        remote_addr,
        decision,
        rpm_bypassed: ip_whitelisted,
        websocket_connection_permit,
    };
    Ok(ws
        .max_frame_size(limits.max_frame_size)
        .max_message_size(limits.max_message_size)
        .on_upgrade(move |socket| async move {
            drop(request_permit);
            run_session(socket, state, context).await;
        }))
}

fn websocket_admission_error_response(
    trace_id: &str,
    decision: &GatewayControlDecision,
    error: RequestAdmissionError,
) -> Result<Response<Body>, GatewayError> {
    match error {
        RequestAdmissionError::Local(aether_runtime::ConcurrencyError::Saturated {
            gate,
            limit,
        })
        | RequestAdmissionError::Distributed(
            aether_runtime_state::RuntimeSemaphoreError::Saturated { gate, limit },
        )
        | RequestAdmissionError::Distributed(
            aether_runtime_state::RuntimeSemaphoreError::Unavailable { gate, limit, .. },
        ) => build_local_overloaded_response(trace_id, Some(decision), gate, limit),
        RequestAdmissionError::Local(aether_runtime::ConcurrencyError::Closed { gate }) => Err(
            GatewayError::Internal(format!("gateway concurrency gate {gate} is closed")),
        ),
        RequestAdmissionError::Distributed(
            aether_runtime_state::RuntimeSemaphoreError::InvalidConfiguration(message),
        ) => Err(GatewayError::Internal(message)),
    }
}

/// Connection-level access log fields which are independent of a protocol's
/// per-turn usage lifecycle.
#[derive(Clone, Copy)]
pub(crate) struct WebSocketConnectionLogSpec {
    pub(crate) opened_event_name: &'static str,
    pub(crate) closed_event_name: &'static str,
    pub(crate) opened_message: &'static str,
    pub(crate) closed_message: &'static str,
    pub(crate) execution_path: &'static str,
    pub(crate) provider_type: &'static str,
}

pub(crate) struct WebSocketConnectionLog {
    spec: WebSocketConnectionLogSpec,
    trace_id: String,
    remote_addr: SocketAddr,
    path: String,
    route_class: String,
    user_id: String,
    api_key_id: String,
    started_at: std::time::Instant,
}

impl WebSocketConnectionLog {
    pub(crate) fn new(context: &WebSocketRequestContext, spec: WebSocketConnectionLogSpec) -> Self {
        let auth_context = context.decision.auth_context.as_ref();
        Self {
            spec,
            trace_id: context.trace_id.clone(),
            remote_addr: context.remote_addr,
            path: context.uri.path().to_string(),
            route_class: context
                .decision
                .route_class
                .as_deref()
                .unwrap_or("ai_public")
                .to_string(),
            user_id: auth_context
                .map(|auth_context| auth_context.user_id.clone())
                .unwrap_or_else(|| "-".to_string()),
            api_key_id: auth_context
                .map(|auth_context| auth_context.api_key_id.clone())
                .unwrap_or_else(|| "-".to_string()),
            started_at: std::time::Instant::now(),
        }
    }

    pub(crate) fn log_opened(&self) {
        info!(
            event_name = self.spec.opened_event_name,
            log_type = "access",
            transport = WEBSOCKET_LOG_TRANSPORT,
            websocket = true,
            status = "upgraded",
            status_code = 101u16,
            trace_id = %self.trace_id,
            remote_addr = %self.remote_addr,
            method = "GET",
            path = %self.path,
            user_id = %self.user_id,
            api_key_id = %self.api_key_id,
            route_class = %self.route_class,
            execution_path = self.spec.execution_path,
            provider_type = self.spec.provider_type,
            message = self.spec.opened_message,
        );
    }
}

impl Drop for WebSocketConnectionLog {
    fn drop(&mut self) {
        info!(
            event_name = self.spec.closed_event_name,
            log_type = "access",
            transport = WEBSOCKET_LOG_TRANSPORT,
            websocket = true,
            status = "closed",
            status_code = 101u16,
            trace_id = %self.trace_id,
            remote_addr = %self.remote_addr,
            method = "GET",
            path = %self.path,
            user_id = %self.user_id,
            api_key_id = %self.api_key_id,
            route_class = %self.route_class,
            execution_path = self.spec.execution_path,
            provider_type = self.spec.provider_type,
            elapsed_ms = self.started_at.elapsed().as_millis() as u64,
            message = self.spec.closed_message,
        );
    }
}
