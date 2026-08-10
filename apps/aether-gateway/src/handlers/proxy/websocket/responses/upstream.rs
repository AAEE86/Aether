//! Physical upstream WebSocket binding and transport helpers.

use std::time::Duration;

use serde_json::Value;

use super::adapter::{responses_public_wire_codec, ResponsesProviderObserver};
use super::backend::NativeResponsesWebSocketBackend;
use super::binding::{UpstreamBindingIdentity, UpstreamBindingIdentityError};
use super::redaction::ResponsesWebSocketRedactionRestorer;
use super::request::planned_response_create_event;
use super::state::{BoundResponsesConnection, ExhaustedResponsesWebSocketExclusions};
use super::turn_state::ResponsesTurnState;
use crate::ai_serving::{AiExecutionDecision, ResponsesWebSocketBodyNormalization};
use aether_scheduler_core::SchedulerMinimalCandidateSelectionCandidate;

/// 上游 WebSocket 握手的默认绝对 deadline（30 秒）。
/// 覆盖 DNS → TCP connect → TLS → HTTP 101 Upgrade → 发送首条 event 的完整链路。
/// 如果 decision 配置了更短的 first_byte_ms 或 total_ms，取其与此值的较小者。
const DEFAULT_UPSTREAM_HANDSHAKE_DEADLINE_MS: u64 = 30_000;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum ResponsesUpstreamBindError {
    ExecutionReservationLost,
    Upstream(&'static str),
}

/// 从 decision.timeouts 推导实际 handshake 绝对 deadline。
/// 取 first_byte_ms / total_ms / DEFAULT 三者中的最小正值。
pub(super) fn resolve_upstream_handshake_deadline(decision: &AiExecutionDecision) -> Duration {
    let mut deadline_ms = DEFAULT_UPSTREAM_HANDSHAKE_DEADLINE_MS;
    if let Some(timeouts) = decision.timeouts.as_ref() {
        if let Some(first_byte_ms) = timeouts.first_byte_ms.filter(|v| *v > 0) {
            deadline_ms = deadline_ms.min(first_byte_ms);
        }
        if let Some(total_ms) = timeouts.total_ms.filter(|v| *v > 0) {
            deadline_ms = deadline_ms.min(total_ms);
        }
    }
    Duration::from_millis(deadline_ms)
}

/// Removes protocol negotiation that would change Aether's stable public
/// Responses contract. The canonical decision is the single source used for
/// binding identity, physical open, stored state, and reuse comparison.
pub(super) fn canonicalize_responses_websocket_decision(
    mut decision: AiExecutionDecision,
) -> AiExecutionDecision {
    decision
        .provider_request_headers
        .retain(|name, _| !name.trim().eq_ignore_ascii_case("openai-beta"));
    decision
        .extra_headers
        .retain(|name, _| !name.trim().eq_ignore_ascii_case("openai-beta"));
    if let Some(headers) = decision
        .report_context
        .as_mut()
        .and_then(|context| context.get_mut("provider_request_headers"))
        .and_then(Value::as_object_mut)
    {
        headers.retain(|name, _| !name.trim().eq_ignore_ascii_case("openai-beta"));
    }
    decision
}

pub(super) async fn bind_responses_upstream<F>(
    decision: &AiExecutionDecision,
    bound_candidate: SchedulerMinimalCandidateSelectionCandidate,
    credential_binding_fingerprint: String,
    normalization: ResponsesWebSocketBodyNormalization,
    initial_event: &Value,
    backend: &'static dyn NativeResponsesWebSocketBackend,
    provider_observer: &'static dyn ResponsesProviderObserver,
    execution_reservation_is_healthy: F,
) -> Result<BoundResponsesConnection, ResponsesUpstreamBindError>
where
    F: Fn() -> bool,
{
    let decision = canonicalize_responses_websocket_decision(decision.clone());
    if !execution_reservation_is_healthy() {
        return Err(ResponsesUpstreamBindError::ExecutionReservationLost);
    }
    // 绝对 deadline：从此刻起必须在限定时间内完成握手 + 首条事件发送，
    // 防止慢 TLS / 慢 HTTP Upgrade 无限占用 connection permit。
    let handshake_deadline = resolve_upstream_handshake_deadline(&decision);
    let result = tokio::time::timeout(
        handshake_deadline,
        bind_responses_upstream_inner(
            &decision,
            bound_candidate,
            credential_binding_fingerprint,
            normalization,
            initial_event,
            backend,
            provider_observer,
            &execution_reservation_is_healthy,
        ),
    )
    .await;
    match result {
        Ok(Err(ResponsesUpstreamBindError::Upstream(_))) if !execution_reservation_is_healthy() => {
            Err(ResponsesUpstreamBindError::ExecutionReservationLost)
        }
        Ok(result) => result,
        Err(_) if !execution_reservation_is_healthy() => {
            Err(ResponsesUpstreamBindError::ExecutionReservationLost)
        }
        Err(_) => Err(ResponsesUpstreamBindError::Upstream(
            "responses_websocket_upstream_handshake_timeout",
        )),
    }
}

/// 实际执行握手 + 首条事件发送的内部函数，由外层 timeout 包裹。
async fn bind_responses_upstream_inner<F>(
    decision: &AiExecutionDecision,
    bound_candidate: SchedulerMinimalCandidateSelectionCandidate,
    credential_binding_fingerprint: String,
    normalization: ResponsesWebSocketBodyNormalization,
    initial_event: &Value,
    backend: &'static dyn NativeResponsesWebSocketBackend,
    provider_observer: &'static dyn ResponsesProviderObserver,
    execution_reservation_is_healthy: &F,
) -> Result<BoundResponsesConnection, ResponsesUpstreamBindError>
where
    F: Fn() -> bool,
{
    let binding_identity =
        UpstreamBindingIdentity::from_decision(backend, decision, &credential_binding_fingerprint)
            .map_err(|error| {
                ResponsesUpstreamBindError::Upstream(match error {
                    UpstreamBindingIdentityError::MissingUpstreamUrl => {
                        backend.upstream_errors().upstream_url_missing
                    }
                    UpstreamBindingIdentityError::InvalidUpstreamUrl => {
                        backend.upstream_errors().upstream_url_invalid
                    }
                    UpstreamBindingIdentityError::InvalidHandshakeHeaders => {
                        backend.upstream_errors().headers_invalid
                    }
                })
            })?;
    if !execution_reservation_is_healthy() {
        return Err(ResponsesUpstreamBindError::ExecutionReservationLost);
    }
    let mut opened = backend
        .open_session(decision)
        .await
        .map_err(ResponsesUpstreamBindError::Upstream)?;
    let first_event = planned_response_create_event(decision, initial_event)
        .map_err(ResponsesUpstreamBindError::Upstream)?;
    let first_event = serde_json::from_str::<Value>(&first_event).map_err(|_| {
        ResponsesUpstreamBindError::Upstream("responses_websocket_initial_send_failed")
    })?;
    if !execution_reservation_is_healthy() {
        opened.session.close().await;
        return Err(ResponsesUpstreamBindError::ExecutionReservationLost);
    }
    opened
        .session
        .send_response_create(&first_event)
        .await
        .map_err(|_| {
            ResponsesUpstreamBindError::Upstream("responses_websocket_initial_send_failed")
        })?;

    let client_model = initial_event
        .get("model")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .ok_or(ResponsesUpstreamBindError::Upstream(
            "responses_websocket_model_missing",
        ))?
        .to_string();
    let provider_model = decision
        .provider_request_body
        .as_ref()
        .and_then(|body| body.get("model"))
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .or_else(|| {
            decision
                .mapped_model
                .as_deref()
                .map(str::trim)
                .filter(|value| !value.is_empty())
        })
        .ok_or(ResponsesUpstreamBindError::Upstream(
            "responses_websocket_mapped_model_missing",
        ))?
        .to_string();

    Ok(BoundResponsesConnection {
        backend_session: opened.session,
        backend,
        public_codec: responses_public_wire_codec(),
        public_event_state: Default::default(),
        provider_observer,
        client_model,
        provider_model,
        decision_template: decision.clone(),
        bound_candidate,
        body_normalization: normalization,
        binding_identity,
        // 首条 response.create 已经发出，但这一轮的 logical turn 和 attempt 由调用方
        // 通过 `ResponsesTurnState::begin` 装上：绑定本身不持有记账状态。
        turn_state: ResponsesTurnState::Idle,
        public_event_sequence: Default::default(),
        public_teardown: Default::default(),
        latest_public_response_id: None,
        // 同理，这一轮的 mask session 也由调用方登记：绑定看不到脱敏链路。
        redaction_restorer: ResponsesWebSocketRedactionRestorer::default(),
        next_turn_index: 2,
        upstream_response_headers: opened.response_headers,
        pending_provider_drain: None,
        pending_provider_observation: None,
        exhausted_exclusions: ExhaustedResponsesWebSocketExclusions::default(),
        pending_turn_finalization: None,
        session_termination: Default::default(),
    })
}

pub(super) async fn close_bound_upstream(bound: &mut BoundResponsesConnection) {
    bound.backend_session.close().await;
}

pub(super) fn decision_reuses_bound_upstream(
    bound: &BoundResponsesConnection,
    backend: &'static dyn NativeResponsesWebSocketBackend,
    decision: &AiExecutionDecision,
    credential_binding_fingerprint: &str,
) -> bool {
    let decision = canonicalize_responses_websocket_decision(decision.clone());
    bound.backend_session.is_bound()
        && bound
            .binding_identity
            .matches_turn_decision(backend, &decision, credential_binding_fingerprint)
            .unwrap_or(false)
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::Arc;
    use std::time::Duration;

    use aether_contracts::ExecutionTimeouts;
    use aether_scheduler_core::SchedulerMinimalCandidateSelectionCandidate;
    use axum::extract::ws::{Message, WebSocketUpgrade};
    use axum::response::IntoResponse;
    use axum::routing::get;
    use axum::Router;
    use futures_util::StreamExt;
    use tokio::sync::{oneshot, Mutex};

    use crate::ai_serving::AiExecutionDecision;

    use super::{
        canonicalize_responses_websocket_decision, resolve_upstream_handshake_deadline,
        ResponsesUpstreamBindError, DEFAULT_UPSTREAM_HANDSHAKE_DEADLINE_MS,
    };

    fn sample_decision() -> AiExecutionDecision {
        AiExecutionDecision {
            action: "local".to_string(),
            decision_kind: None,
            execution_strategy: None,
            conversion_mode: None,
            request_id: None,
            candidate_id: None,
            provider_name: None,
            provider_type: Some("custom".to_string()),
            provider_id: None,
            endpoint_id: None,
            key_id: None,
            upstream_base_url: None,
            upstream_url: Some("https://example.test/v1/responses".to_string()),
            provider_request_method: None,
            auth_header: None,
            auth_value: None,
            provider_api_format: Some("openai:responses".to_string()),
            client_api_format: Some("openai:responses".to_string()),
            provider_contract: None,
            client_contract: None,
            model_name: None,
            mapped_model: Some("provider-model".to_string()),
            prompt_cache_key: None,
            extra_headers: std::collections::BTreeMap::new(),
            provider_request_headers: std::collections::BTreeMap::new(),
            provider_request_body: None,
            provider_request_body_base64: None,
            content_type: None,
            content_encoding: None,
            request_gzip: None,
            proxy: None,
            transport_profile: None,
            timeouts: None,
            upstream_is_stream: true,
            report_kind: None,
            report_context: None,
            auth_context: None,
        }
    }

    #[test]
    fn stable_backend_decision_removes_beta_negotiation_case_insensitively() {
        let mut decision = sample_decision();
        decision.provider_request_headers = std::collections::BTreeMap::from([
            (
                "OpEnAI-BeTa".to_string(),
                "responses_multi_agent=2026-05-01".to_string(),
            ),
            ("x-provider-header".to_string(), "keep".to_string()),
        ]);
        decision.extra_headers.insert(
            "OPENAI-BETA".to_string(),
            "responses_multi_agent=2026-05-01".to_string(),
        );
        decision.report_context = Some(serde_json::json!({
            "provider_request_headers": {
                "OpenAI-Beta": "responses_multi_agent=v1",
                "x-provider-header": "keep"
            }
        }));

        let canonical = canonicalize_responses_websocket_decision(decision);

        assert!(canonical
            .provider_request_headers
            .keys()
            .all(|name| !name.eq_ignore_ascii_case("openai-beta")));
        assert!(canonical
            .extra_headers
            .keys()
            .all(|name| !name.eq_ignore_ascii_case("openai-beta")));
        assert_eq!(
            canonical.provider_request_headers.get("x-provider-header"),
            Some(&"keep".to_string())
        );
        let captured_headers = canonical
            .report_context
            .as_ref()
            .and_then(|context| context.get("provider_request_headers"))
            .and_then(serde_json::Value::as_object)
            .expect("captured provider headers should remain an object");
        assert!(captured_headers
            .keys()
            .all(|name| !name.eq_ignore_ascii_case("openai-beta")));
        assert_eq!(captured_headers["x-provider-header"], "keep");
    }

    #[test]
    fn planned_initial_event_rejects_multi_agent_injected_after_public_decoding() {
        let fallback = serde_json::json!({
            "type": "response.create",
            "model": "public-model",
            "input": "hello",
        });
        for provider_body in [
            serde_json::json!({
                "model": "provider-model",
                "multi_agent": {"enabled": true},
                "input": "hello",
            }),
            serde_json::json!({
                "model": "provider-model",
                "input": [{
                    "type": "message",
                    "caller": {"type": "multi_agent"},
                }],
            }),
        ] {
            let mut decision = sample_decision();
            decision.provider_request_body = Some(provider_body);

            assert_eq!(
                super::planned_response_create_event(&decision, &fallback),
                Err("responses_websocket_multi_agent_unsupported")
            );
        }
    }

    fn sample_bound_candidate(model: &str) -> SchedulerMinimalCandidateSelectionCandidate {
        SchedulerMinimalCandidateSelectionCandidate {
            provider_id: "provider-1".to_string(),
            provider_name: "Provider".to_string(),
            provider_type: "custom".to_string(),
            provider_priority: 0,
            endpoint_id: "endpoint-1".to_string(),
            endpoint_api_format: "openai:responses".to_string(),
            key_id: "key-1".to_string(),
            key_name: "key-1".to_string(),
            key_auth_type: "api_key".to_string(),
            key_internal_priority: 0,
            key_global_priority_for_format: None,
            key_capabilities: None,
            model_id: "model-1".to_string(),
            global_model_id: "global-model-1".to_string(),
            global_model_name: model.to_string(),
            selected_provider_model_name: model.to_string(),
            supports_streaming: true,
            mapping_matched_model: None,
        }
    }

    #[test]
    fn handshake_deadline_defaults_to_30s_without_configured_timeouts() {
        let decision = sample_decision();
        let deadline = resolve_upstream_handshake_deadline(&decision);
        assert_eq!(
            deadline,
            Duration::from_millis(DEFAULT_UPSTREAM_HANDSHAKE_DEADLINE_MS)
        );
    }

    #[test]
    fn handshake_deadline_uses_first_byte_ms_when_shorter_than_default() {
        let mut decision = sample_decision();
        decision.timeouts = Some(ExecutionTimeouts {
            first_byte_ms: Some(10_000),
            total_ms: Some(60_000),
            ..ExecutionTimeouts::default()
        });
        let deadline = resolve_upstream_handshake_deadline(&decision);
        assert_eq!(deadline, Duration::from_millis(10_000));
    }

    #[test]
    fn handshake_deadline_uses_total_ms_when_shorter_than_first_byte_and_default() {
        let mut decision = sample_decision();
        decision.timeouts = Some(ExecutionTimeouts {
            first_byte_ms: Some(25_000),
            total_ms: Some(8_000),
            ..ExecutionTimeouts::default()
        });
        let deadline = resolve_upstream_handshake_deadline(&decision);
        assert_eq!(deadline, Duration::from_millis(8_000));
    }

    #[test]
    fn handshake_deadline_ignores_zero_values() {
        let mut decision = sample_decision();
        decision.timeouts = Some(ExecutionTimeouts {
            first_byte_ms: Some(0),
            total_ms: Some(0),
            ..ExecutionTimeouts::default()
        });
        let deadline = resolve_upstream_handshake_deadline(&decision);
        assert_eq!(
            deadline,
            Duration::from_millis(DEFAULT_UPSTREAM_HANDSHAKE_DEADLINE_MS)
        );
    }

    #[test]
    fn handshake_deadline_does_not_exceed_default_even_with_larger_configured_values() {
        let mut decision = sample_decision();
        decision.timeouts = Some(ExecutionTimeouts {
            first_byte_ms: Some(120_000),
            total_ms: Some(600_000),
            ..ExecutionTimeouts::default()
        });
        let deadline = resolve_upstream_handshake_deadline(&decision);
        assert_eq!(
            deadline,
            Duration::from_millis(DEFAULT_UPSTREAM_HANDSHAKE_DEADLINE_MS)
        );
    }

    #[tokio::test]
    async fn unhealthy_reservation_short_circuits_before_upstream_validation() {
        use super::bind_responses_upstream;
        use crate::ai_serving::ResponsesWebSocketBodyNormalization;
        use crate::handlers::proxy::websocket::responses::adapter::resolve_responses_provider_observer;
        use crate::handlers::proxy::websocket::responses::backend::resolve_native_responses_websocket_backend;
        use serde_json::json;

        let mut decision = sample_decision();
        decision.upstream_url = Some("not a valid upstream URL".to_string());
        decision.provider_request_body = Some(json!({"model": "test-model"}));
        let result = bind_responses_upstream(
            &decision,
            sample_bound_candidate("test-model"),
            "credential-1".to_string(),
            ResponsesWebSocketBodyNormalization::for_tests("test-model"),
            &json!({"type": "response.create", "model": "test-model"}),
            resolve_native_responses_websocket_backend(
                crate::orchestration::ResponsesWebSocketBackendKind::NativeResponsesWebSocket,
            ),
            resolve_responses_provider_observer(
                crate::orchestration::ResponsesProviderObserverKind::Standard,
            ),
            || false,
        )
        .await;

        assert_eq!(
            result.err(),
            Some(ResponsesUpstreamBindError::ExecutionReservationLost)
        );
    }

    #[tokio::test]
    async fn reservation_lost_after_handshake_prevents_initial_provider_send() {
        use super::bind_responses_upstream;
        use crate::ai_serving::ResponsesWebSocketBodyNormalization;
        use crate::handlers::proxy::websocket::responses::adapter::resolve_responses_provider_observer;
        use crate::handlers::proxy::websocket::responses::backend::resolve_native_responses_websocket_backend;
        use serde_json::json;

        let (provider_event_tx, provider_event_rx) = oneshot::channel();
        let provider_event_tx = Arc::new(Mutex::new(Some(provider_event_tx)));
        let app = Router::new().route(
            "/v1/responses",
            get(move |ws: WebSocketUpgrade| {
                let provider_event_tx = Arc::clone(&provider_event_tx);
                async move {
                    ws.on_upgrade(move |mut socket| async move {
                        while let Some(Ok(message)) = socket.next().await {
                            match message {
                                Message::Text(_) => {
                                    if let Some(sender) = provider_event_tx.lock().await.take() {
                                        let _ = sender.send(());
                                    }
                                    break;
                                }
                                Message::Close(_) => break,
                                _ => {}
                            }
                        }
                    })
                    .into_response()
                }
            }),
        );
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("mock listener should bind");
        let address = listener
            .local_addr()
            .expect("listener should expose address");
        let server = tokio::spawn(async move {
            axum::serve(listener, app)
                .await
                .expect("mock server should run");
        });

        let mut decision = sample_decision();
        decision.upstream_url = Some(format!("http://{address}/v1/responses"));
        decision.provider_request_body = Some(json!({"model": "test-model"}));
        let health_checks = AtomicUsize::new(0);
        let result = bind_responses_upstream(
            &decision,
            sample_bound_candidate("test-model"),
            "credential-1".to_string(),
            ResponsesWebSocketBodyNormalization::for_tests("test-model"),
            &json!({"type": "response.create", "model": "test-model"}),
            resolve_native_responses_websocket_backend(
                crate::orchestration::ResponsesWebSocketBackendKind::NativeResponsesWebSocket,
            ),
            resolve_responses_provider_observer(
                crate::orchestration::ResponsesProviderObserverKind::Standard,
            ),
            || health_checks.fetch_add(1, Ordering::SeqCst) < 2,
        )
        .await;

        assert_eq!(
            result.err(),
            Some(ResponsesUpstreamBindError::ExecutionReservationLost)
        );
        assert_eq!(health_checks.load(Ordering::SeqCst), 3);
        assert!(
            tokio::time::timeout(Duration::from_millis(100), provider_event_rx)
                .await
                .is_err(),
            "provider must not receive response.create after reservation loss"
        );
        server.abort();
    }

    #[tokio::test]
    async fn bind_responses_upstream_times_out_against_stalled_server() {
        use super::bind_responses_upstream;
        use crate::ai_serving::ResponsesWebSocketBodyNormalization;
        use crate::handlers::proxy::websocket::responses::adapter::resolve_responses_provider_observer;
        use crate::handlers::proxy::websocket::responses::backend::resolve_native_responses_websocket_backend;
        use serde_json::json;

        // 启动一个接受 TCP 连接但永不完成 HTTP Upgrade 的 mock 服务器
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("mock listener should bind");
        let addr = listener.local_addr().expect("should have local addr");
        let _server = tokio::spawn(async move {
            loop {
                let (socket, _) = listener.accept().await.unwrap();
                // 接受连接但不发送任何 HTTP 响应，模拟 stalled handshake
                tokio::spawn(async move {
                    let _hold = socket;
                    tokio::time::sleep(Duration::from_secs(300)).await;
                });
            }
        });

        let mut decision = sample_decision();
        decision.upstream_url = Some(format!("http://{addr}/v1/responses"));
        // 设置极短的 deadline 以便测试快速完成
        decision.timeouts = Some(ExecutionTimeouts {
            first_byte_ms: Some(100),
            total_ms: Some(200),
            ..ExecutionTimeouts::default()
        });
        decision.provider_request_body = Some(json!({"model": "test-model"}));

        let backend = resolve_native_responses_websocket_backend(
            crate::orchestration::ResponsesWebSocketBackendKind::NativeResponsesWebSocket,
        );
        let provider_observer = resolve_responses_provider_observer(
            crate::orchestration::ResponsesProviderObserverKind::Standard,
        );
        let result = bind_responses_upstream(
            &decision,
            sample_bound_candidate("test-model"),
            "credential-1".to_string(),
            ResponsesWebSocketBodyNormalization::for_tests("test-model"),
            &json!({"type": "response.create", "model": "test-model"}),
            backend,
            provider_observer,
            || true,
        )
        .await;

        assert_eq!(
            result.err().expect("bind should fail with timeout"),
            ResponsesUpstreamBindError::Upstream("responses_websocket_upstream_handshake_timeout")
        );
    }
}
