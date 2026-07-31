//! Responses WebSocket 请求侧 PII 脱敏。
//!
//! HTTP 路径在前门建 `RedactionSessionSlot` 并塞进 `parts.extensions`，planner
//! 只有拿到这个 slot 才会脱敏。WS 的 planning Parts 是合成的：四个规划入口
//! （首轮、换模型 re-plan、独立轮、配额透明重试）靠 `build_planning_parts` 注入
//! slot 就能复用 planner 的脱敏；但复用已绑定 upstream 的 continuation 根本不进
//! planner，必须在这里先把客户端事件脱敏，再交给协议归一化、上游发送和审计。
//!
//! 因此约定：**进入任何下游用途之前，客户端 `response.create` 只在这里脱敏一次**，
//! 之后所有路径都只看脱敏后的事件。

use serde_json::Value;

use crate::ai_serving::{
    resolve_local_decision_execution_runtime_auth_context, resolve_provider_chat_pii_redaction,
};
use crate::control::GatewayControlDecision;
use crate::{AppState, GatewayError};

/// Responses WebSocket 只承载 `openai:responses`，脱敏规则按这个客户端格式选取。
const RESPONSES_WEBSOCKET_CLIENT_API_FORMAT: &str = "openai:responses";

/// WS 在选出候选之前就要脱敏，所以脱敏 session 先记在这个固定 key 下。
///
/// slot 是 per-turn 的（见 `build_planning_parts`），这一轮之后即随 slot 一起丢弃；
/// planner 后续用真实 candidate_id 再取一次配置时，body 已是脱敏态、不会重复写入。
const WEBSOCKET_TURN_REDACTION_CANDIDATE_ID: &str = "responses_websocket_turn";

/// 对一条客户端 `response.create` 做请求侧脱敏。
///
/// 返回 `Some(脱敏后的事件)` 仅当脱敏真正命中；`None` 表示未启用或没有命中，调用方
/// 继续用原事件即可（避免未开启脱敏时多一次整包 clone）。
///
/// 脱敏只改写 `instructions` / `input`（见 `privacy::mask_openai_responses_request_value`），
/// `type` / `model` / `previous_response_id` / `generate` 等协议字段原样保留，所以脱敏后的
/// 事件仍可直接用于协议归一化和上游发送。
///
/// 出错必须让这一轮失败：脱敏已启用却读不到配置或加密密钥时，把原文发上游就是
/// 静默旁路，正是本次要修的问题。
pub(super) async fn redact_responses_websocket_client_event(
    state: &AppState,
    parts: &http::request::Parts,
    control_decision: &GatewayControlDecision,
    client_event: &Value,
) -> Result<Option<Value>, GatewayError> {
    let Some(auth_context) =
        resolve_local_decision_execution_runtime_auth_context(control_decision)
    else {
        return Ok(None);
    };
    let redaction = resolve_provider_chat_pii_redaction(
        state,
        parts,
        client_event,
        &auth_context,
        RESPONSES_WEBSOCKET_CLIENT_API_FORMAT,
        WEBSOCKET_TURN_REDACTION_CANDIDATE_ID,
    )
    .await?;
    if !redaction.redacted {
        return Ok(None);
    }
    Ok(Some(redaction.body_json.into_owned()))
}

#[cfg(test)]
mod tests {
    use std::net::SocketAddr;
    use std::sync::Arc;

    use aether_crypto::DEVELOPMENT_ENCRYPTION_KEY;
    use aether_data::repository::auth::{
        InMemoryAuthApiKeySnapshotRepository, StoredAuthApiKeyExportRecord,
    };
    use axum::http::{HeaderMap, Uri};
    use serde_json::{json, Value};

    use super::super::request::{
        build_planning_parts, normalize_followup_response_create, planned_response_create_event,
    };
    use super::super::state::ActiveResponsesWebSocketRequest;
    use super::super::turn::prepare_responses_websocket_turn_decision;
    use super::redact_responses_websocket_client_event;
    use crate::ai_serving::{AiExecutionDecision, ResponsesWebSocketBodyNormalization};
    use crate::control::{GatewayControlAuthContext, GatewayControlDecision};
    use crate::handlers::proxy::websocket::ingress::WebSocketRequestContext;
    use crate::AppState;

    const TEST_USER_ID: &str = "user-responses-ws-redaction";
    const TEST_API_KEY_ID: &str = "api-key-responses-ws-redaction";
    const TEST_EMAIL: &str = "ws.user@example.com";

    fn auth_export_record() -> StoredAuthApiKeyExportRecord {
        StoredAuthApiKeyExportRecord::new(
            TEST_USER_ID.to_string(),
            TEST_API_KEY_ID.to_string(),
            "hash-responses-ws-redaction".to_string(),
            None,
            Some("ws".to_string()),
            None,
            None,
            None,
            None,
            None,
            None,
            true,
            None,
            false,
            0,
            0,
            0.0,
            false,
        )
        .expect("auth api key export record should build")
        .with_feature_settings(Some(json!({
            "chat_pii_redaction": {"enabled": true}
        })))
    }

    /// 只装脱敏真正需要的东西：系统配置开关 + 规则、加密密钥、带 feature settings
    /// 的 API Key 导出记录。候选/上游都不需要，这条链路在 planner 之前。
    fn redaction_enabled_state() -> AppState {
        let auth_repository = Arc::new(
            InMemoryAuthApiKeySnapshotRepository::seed(vec![])
                .with_export_records(vec![auth_export_record()]),
        );
        let data_state =
            crate::data::GatewayDataState::with_auth_api_key_reader_for_tests(auth_repository)
                .with_encryption_key_for_tests(DEVELOPMENT_ENCRYPTION_KEY)
                .with_system_config_values_for_tests(vec![
                    ("module.chat_pii_redaction.enabled".to_string(), json!(true)),
                    (
                        "module.chat_pii_redaction.rules".to_string(),
                        json!([{
                            "id": "email",
                            "name": "邮箱",
                            "pattern": r"(?i)[A-Z0-9._%+-]{1,64}@[A-Z0-9.-]{1,253}\.[A-Z]{2,63}",
                            "enabled": true,
                            "features": {"validator": "email"},
                            "system": true
                        }]),
                    ),
                    (
                        "module.chat_pii_redaction.cache_ttl_seconds".to_string(),
                        json!(300),
                    ),
                ]);
        AppState::new()
            .expect("gateway state should build")
            .with_data_state_for_tests(data_state)
    }

    fn control_decision() -> GatewayControlDecision {
        let mut decision = GatewayControlDecision::synthetic(
            "/v1/responses".to_string(),
            Some("ai_public".to_string()),
            Some("openai".to_string()),
            Some("responses_websocket".to_string()),
            Some("openai:responses".to_string()),
        );
        decision.auth_context = Some(GatewayControlAuthContext {
            user_id: TEST_USER_ID.to_string(),
            api_key_id: TEST_API_KEY_ID.to_string(),
            username: Some("ws".to_string()),
            api_key_name: Some("ws".to_string()),
            balance_remaining: None,
            access_allowed: true,
            user_rate_limit: None,
            api_key_rate_limit: None,
            api_key_is_standalone: false,
            admin_bypass_limits: false,
            local_rejection: None,
            allowed_models: None,
            ip_rules: None,
        });
        decision
    }

    fn websocket_context(decision: GatewayControlDecision) -> WebSocketRequestContext {
        WebSocketRequestContext {
            trace_id: "trace-responses-ws-redaction".to_string(),
            headers: HeaderMap::new(),
            uri: Uri::from_static("/v1/responses"),
            remote_addr: "127.0.0.1:65000"
                .parse::<SocketAddr>()
                .expect("remote address should parse"),
            decision,
            rpm_bypassed: false,
            websocket_connection_permit: None,
        }
    }

    fn client_event() -> Value {
        json!({
            "type": "response.create",
            "model": "public-model",
            "previous_response_id": "resp-previous",
            "generate": false,
            "input": [{
                "role": "user",
                "content": [{"type": "input_text", "text": format!("mail {TEST_EMAIL}")}]
            }]
        })
    }

    #[tokio::test]
    async fn websocket_client_event_is_redacted_without_losing_protocol_fields() {
        let state = redaction_enabled_state();
        let context = websocket_context(control_decision());
        let parts = build_planning_parts(&context);
        let event = client_event();

        let redacted =
            redact_responses_websocket_client_event(&state, &parts, &context.decision, &event)
                .await
                .expect("redaction should resolve")
                .expect("an email in the request should be redacted");

        let serialized = serde_json::to_string(&redacted).expect("event should serialize");
        assert!(!serialized.contains(TEST_EMAIL), "{serialized}");
        assert!(serialized.contains("<AETHER:EMAIL:"), "{serialized}");
        // 协议字段必须原样保留，否则 continuation 链路会断。
        assert_eq!(redacted["type"], "response.create");
        assert_eq!(redacted["model"], "public-model");
        assert_eq!(redacted["previous_response_id"], "resp-previous");
        assert_eq!(redacted["generate"], false);
    }

    #[tokio::test]
    async fn redacting_an_already_redacted_event_is_a_no_op() {
        // re-plan 与配额重试路径会把已脱敏的事件再交给 planner，planner 内部会对
        // 同一个 body 再跑一遍 mask。占位符本身不该被任何规则命中，否则会被二次
        // 替换、破坏与上游已有 previous_response_id 链的一致性。
        let state = redaction_enabled_state();
        let context = websocket_context(control_decision());
        let parts = build_planning_parts(&context);
        let event = client_event();

        let redacted =
            redact_responses_websocket_client_event(&state, &parts, &context.decision, &event)
                .await
                .expect("redaction should resolve")
                .expect("an email in the request should be redacted");

        // 复用同一个 parts/slot，和 re-plan 在同一 turn 内二次脱敏的情形一致。
        let second_pass =
            redact_responses_websocket_client_event(&state, &parts, &context.decision, &redacted)
                .await
                .expect("second redaction pass should resolve");

        assert!(
            second_pass.is_none(),
            "already redacted event should stay byte-identical: {second_pass:?}"
        );
    }

    #[tokio::test]
    async fn redaction_is_skipped_without_a_local_auth_context() {
        let state = redaction_enabled_state();
        let mut decision = control_decision();
        decision.auth_context = None;
        let context = websocket_context(decision);
        let parts = build_planning_parts(&context);
        let event = client_event();

        let redacted =
            redact_responses_websocket_client_event(&state, &parts, &context.decision, &event)
                .await
                .expect("redaction should resolve");

        assert!(redacted.is_none());
    }

    /// 真跑一遍脱敏，拿到这一轮的「生效事件」。
    async fn redacted_client_event(state: &AppState, decision: &GatewayControlDecision) -> Value {
        let context = websocket_context(decision.clone());
        let parts = build_planning_parts(&context);
        let event = client_event();
        redact_responses_websocket_client_event(state, &parts, &context.decision, &event)
            .await
            .expect("redaction should resolve")
            .expect("an email in the request should be redacted")
    }

    /// 只有 `action` 没有 serde 默认值，其余字段都能省略。
    fn decision_template(
        provider_request_body: Value,
        report_context: Value,
    ) -> AiExecutionDecision {
        serde_json::from_value(json!({
            "action": "local",
            "candidate_id": "candidate-responses-ws",
            "provider_request_body": provider_request_body,
            "report_context": report_context,
        }))
        .expect("decision template should deserialize")
    }

    /// planner 在脱敏 body 上做模型映射后的 provider body。
    fn provider_body_from(effective_event: &Value) -> Value {
        let mut provider_body = effective_event.clone();
        provider_body["model"] = json!("provider-model");
        provider_body
    }

    /// 绑定那一轮留下的 report_context seed：故意带上原始 PII，用来证明这一轮
    /// 会用脱敏后的 body 覆盖它，而不是把原文带进审计。
    fn seed_report_context_with_raw_pii() -> Value {
        json!({
            "request_id": "connection",
            "candidate_id": "candidate-responses-ws",
            "original_request_body": {
                "type": "response.create",
                "model": "public-model",
                "input": format!("mail {TEST_EMAIL}")
            }
        })
    }

    fn assert_redacted_json(value: &Value, label: &str) {
        let serialized = serde_json::to_string(value).expect("value should serialize");
        assert!(
            !serialized.contains(TEST_EMAIL),
            "{label} must not carry raw PII: {serialized}"
        );
        assert!(
            serialized.contains("<AETHER:EMAIL:"),
            "{label} must carry the redaction sentinel: {serialized}"
        );
    }

    #[tokio::test]
    async fn first_turn_upstream_and_audit_bodies_are_redacted() {
        let state = redaction_enabled_state();
        let decision = control_decision();
        let effective_event = redacted_client_event(&state, &decision).await;
        let template = decision_template(
            provider_body_from(&effective_event),
            seed_report_context_with_raw_pii(),
        );
        // 首轮实际发上游的事件由 decision.provider_request_body 派生。
        let provider_event: Value = serde_json::from_str(
            &planned_response_create_event(&template, &effective_event)
                .expect("first provider event should serialize"),
        )
        .expect("first provider event should parse");

        let turn_decision = prepare_responses_websocket_turn_decision(
            &template,
            "turn-1".to_string(),
            true,
            &effective_event,
            &provider_event,
            "connection",
            1,
            "logical-turn-1",
            1,
        );

        assert_redacted_json(&provider_event, "first turn upstream event");
        assert_redacted_json(
            turn_decision
                .provider_request_body
                .as_ref()
                .expect("turn decision should carry a provider body"),
            "first turn provider request body",
        );
        let report_context = turn_decision
            .report_context
            .as_ref()
            .expect("turn decision should carry a report context");
        assert_redacted_json(
            &report_context["original_request_body"],
            "first turn audit body",
        );
        // 整个 report_context 都不该残留原文（seed 里的原始 body 必须被覆盖）。
        assert_redacted_json(report_context, "first turn report context");
        assert_eq!(provider_event["type"], "response.create");
        assert_eq!(provider_event["model"], "provider-model");
    }

    #[tokio::test]
    async fn continuation_upstream_and_audit_bodies_are_redacted() {
        let state = redaction_enabled_state();
        let decision = control_decision();
        let effective_event = redacted_client_event(&state, &decision).await;
        // continuation 复用已绑定的 upstream：不再规划，直接重放归一化器。
        let outbound = normalize_followup_response_create(
            &effective_event,
            "provider-model",
            &ResponsesWebSocketBodyNormalization::for_tests("provider-model"),
        )
        .expect("continuation should normalize");
        let provider_event: Value =
            serde_json::from_str(&outbound).expect("continuation event should parse");

        let template = decision_template(
            provider_body_from(&effective_event),
            seed_report_context_with_raw_pii(),
        );
        let turn_decision = prepare_responses_websocket_turn_decision(
            &template,
            "turn-2".to_string(),
            false,
            &effective_event,
            &provider_event,
            "connection",
            2,
            "logical-turn-2",
            1,
        );

        assert!(
            !outbound.contains(TEST_EMAIL),
            "continuation upstream frame must not carry raw PII: {outbound}"
        );
        assert!(
            outbound.contains("<AETHER:EMAIL:"),
            "continuation upstream frame must carry the sentinel: {outbound}"
        );
        assert_eq!(provider_event["previous_response_id"], "resp-previous");
        let report_context = turn_decision
            .report_context
            .as_ref()
            .expect("turn decision should carry a report context");
        assert_redacted_json(
            &report_context["original_request_body"],
            "continuation audit body",
        );
        assert_redacted_json(report_context, "continuation report context");
    }

    #[tokio::test]
    async fn quota_retry_replays_the_redacted_event() {
        let state = redaction_enabled_state();
        let decision = control_decision();
        let effective_event = redacted_client_event(&state, &decision).await;
        // 配额透明重试重放 active_response_create 里保存的事件，所以保存的必须
        // 已经是脱敏版，否则重试会把原文发给新的上游账号。
        let active = ActiveResponsesWebSocketRequest::new(
            effective_event.clone(),
            2,
            "logical-turn-2".to_string(),
        );
        assert_redacted_json(&active.client_event, "quota retry replay event");

        let template = decision_template(
            provider_body_from(&active.client_event),
            seed_report_context_with_raw_pii(),
        );
        let provider_event: Value = serde_json::from_str(
            &planned_response_create_event(&template, &active.client_event)
                .expect("retry provider event should serialize"),
        )
        .expect("retry provider event should parse");
        let turn_decision = prepare_responses_websocket_turn_decision(
            &template,
            "turn-2-retry".to_string(),
            true,
            &active.client_event,
            &provider_event,
            "connection",
            active.turn_index,
            "logical-turn-2",
            2,
        );

        assert_redacted_json(&provider_event, "quota retry upstream event");
        let report_context = turn_decision
            .report_context
            .as_ref()
            .expect("turn decision should carry a report context");
        assert_eq!(report_context["websocket_turn_attempt"], 2);
        assert_redacted_json(
            &report_context["original_request_body"],
            "quota retry audit body",
        );
        assert_redacted_json(report_context, "quota retry report context");
    }
}
