//! Authenticated public WebSocket upgrade admission shared by AI adapters.

use std::future::Future;
use std::net::SocketAddr;

use axum::body::Body;
use axum::extract::ws::{WebSocket, WebSocketUpgrade};
use axum::http::{HeaderMap, Method, Response, StatusCode, Uri};
use tracing::info;

use crate::api::response::{
    build_local_auth_rejection_response, build_local_http_error_response,
    build_local_overloaded_response,
};
use crate::control::{
    trusted_auth_local_rejection, GatewayControlDecision, GatewayLocalAuthRejection,
};
use crate::handlers::proxy::websocket::session::{WebSocketSessionLimits, WEBSOCKET_LOG_TRANSPORT};
use crate::handlers::shared::ip_rules_allow;
use crate::handlers::shared::strip_query_param;
use crate::headers::{effective_client_ip, extract_or_generate_trace_id};
use crate::router::RequestAdmissionError;
use crate::{AppState, GatewayError};

/// Request facts that survive the HTTP Upgrade and are needed by a protocol
/// adapter for planning, rate limiting, and connection-scoped audit logs.
pub(crate) struct WebSocketRequestContext {
    pub(crate) trace_id: String,
    pub(crate) headers: HeaderMap,
    /// Credential-free URI retained after authentication for planning and
    /// connection logs. In particular, the public `?key=` credential must not
    /// survive into a provider URL.
    pub(crate) uri: Uri,
    pub(crate) remote_addr: SocketAddr,
    pub(crate) decision: GatewayControlDecision,
    /// Held for the lifetime of the upgraded socket. The Responses session
    /// polls its health and closes the client when a distributed lease is
    /// revoked or expires.
    pub(crate) websocket_connection_permit: Option<aether_runtime::AdmissionPermit>,
}

/// Adapter-specific wording and event identifiers for generic upgrade checks.
#[derive(Clone, Copy)]
pub(crate) struct WebSocketIngressSpec {
    pub(crate) route_unavailable_message: &'static str,
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
    // Match the ordinary HTTP front door: admission must cover blacklist and
    // authentication lookups, not start only after those expensive stages.
    let request_permit = match state.try_acquire_request_permit().await {
        Ok(permit) => permit,
        Err(error) => {
            return websocket_admission_error_response(&trace_id, None, Some(uri.path()), error)
        }
    };
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

    // Authentication is the final consumer of public credentials. Everything
    // retained by the upgraded session is safe to pass into request planning.
    let planning_uri = credential_free_websocket_planning_uri(&uri);

    let websocket_connection_permit = match state.try_acquire_websocket_connection_permit().await {
        Ok(permit) => permit,
        Err(error) => {
            return websocket_admission_error_response(
                &trace_id,
                Some(&decision),
                Some(uri.path()),
                error,
            )
        }
    };

    let context = WebSocketRequestContext {
        trace_id,
        headers,
        uri: planning_uri,
        remote_addr,
        decision,
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
    decision: Option<&GatewayControlDecision>,
    request_path: Option<&str>,
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
        ) => build_local_overloaded_response(trace_id, decision, request_path, gate, limit),
        RequestAdmissionError::Local(aether_runtime::ConcurrencyError::Closed { gate }) => Err(
            GatewayError::Internal(format!("gateway concurrency gate {gate} is closed")),
        ),
        RequestAdmissionError::Distributed(
            aether_runtime_state::RuntimeSemaphoreError::InvalidConfiguration(message),
        ) => Err(GatewayError::Internal(message)),
    }
}

/// Removes credentials accepted by the public WebSocket front door before the
/// URI becomes reusable planning input. The original URI is used for auth and
/// is deliberately not retained in [`WebSocketRequestContext`].
pub(crate) fn credential_free_websocket_planning_uri(uri: &Uri) -> Uri {
    let Some(path_and_query) = uri.path_and_query() else {
        return uri.clone();
    };
    let Some(query) = path_and_query.query() else {
        return uri.clone();
    };
    if !url::form_urlencoded::parse(query.as_bytes()).any(|(key, _)| key == "key") {
        return uri.clone();
    }

    let sanitized = strip_query_param(path_and_query.as_str(), "key");
    let mut parts = uri.clone().into_parts();
    parts.path_and_query = Some(
        sanitized
            .parse()
            .expect("a sanitized valid WebSocket URI must have a valid path and query"),
    );
    Uri::from_parts(parts).expect("sanitizing a WebSocket URI must preserve valid URI parts")
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

#[cfg(test)]
mod tests {
    use axum::http::Uri;

    use super::credential_free_websocket_planning_uri;

    #[test]
    fn public_query_credentials_do_not_survive_the_upgrade_context() {
        let original = Uri::from_static(
            "https://gateway.example/v1/responses?key=aether-secret&mode=debug&key=rotated",
        );

        let sanitized = credential_free_websocket_planning_uri(&original);

        assert_eq!(
            sanitized,
            Uri::from_static("https://gateway.example/v1/responses?mode=debug")
        );
    }

    #[test]
    fn credential_free_query_is_left_unchanged() {
        let original = Uri::from_static("/v1/responses?monkey=value&mode=debug");

        assert_eq!(credential_free_websocket_planning_uri(&original), original);
    }
}
