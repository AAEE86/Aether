//! Codex-specific extensions for the standard Responses WebSocket session.

use async_trait::async_trait;
use serde_json::{Map, Value};

use super::super::adapter::{
    is_public_responses_event, project_metadata, ResponsesProviderExclusionIdentity,
    ResponsesProviderObservation, ResponsesProviderObserver, ResponsesProviderPrivateError,
    ResponsesPublicWireError, ResponsesWebSocketDrainDirective, ResponsesWebSocketRebindSafety,
};
use crate::ai_serving::AiExecutionDecision;
use crate::clock::current_unix_secs;
use crate::orchestration::{
    codex_account_id_from_headers, codex_quota_exhaustion_reset_at,
    sync_codex_websocket_quota_metadata, ResponsesProviderObserverKind,
};
use crate::AppState;

const CODEX_WEBSOCKET_LOG_TARGET: &str = "aether_gateway::handlers::proxy::codex_ws";
const CODEX_WEBSOCKET_RATE_LIMITS_REPORT_CONTEXT_FIELD: &str = "codex_websocket_rate_limits";

pub(crate) static CODEX_RESPONSES_PROVIDER_OBSERVER: CodexResponsesProviderObserver =
    CodexResponsesProviderObserver;

pub(crate) struct CodexResponsesProviderObserver;

#[async_trait]
impl ResponsesProviderObserver for CodexResponsesProviderObserver {
    fn kind(&self) -> ResponsesProviderObserverKind {
        ResponsesProviderObserverKind::Codex
    }

    fn decorate_turn_report_context(&self, report_context: &mut Option<Value>, event: &Value) {
        let Some(rate_limits) = parse_codex_rate_limits(event) else {
            return;
        };
        let context = report_context.get_or_insert_with(|| Value::Object(Map::new()));
        let Some(context) = context.as_object_mut() else {
            return;
        };
        context.insert(
            CODEX_WEBSOCKET_RATE_LIMITS_REPORT_CONTEXT_FIELD.to_string(),
            rate_limits,
        );
    }

    fn observes_upstream_events(&self) -> bool {
        true
    }

    fn rebind_safety_for_upstream_event(&self, event: &Value) -> ResponsesWebSocketRebindSafety {
        let mut events = Vec::new();
        if event.get("type").and_then(Value::as_str).is_some() {
            events.push(event);
        }
        if let Some(chunks) = event.get("chunks").and_then(Value::as_array) {
            events.extend(
                chunks
                    .iter()
                    .filter(|chunk| chunk.get("type").and_then(Value::as_str).is_some()),
            );
        }
        if events.is_empty() {
            return ResponsesWebSocketRebindSafety::Unsafe {
                reason: "unrecognized_upstream_event",
            };
        }
        events
            .into_iter()
            .map(codex_direct_rebind_safety)
            .find(|safety| matches!(safety, ResponsesWebSocketRebindSafety::Unsafe { .. }))
            .unwrap_or(ResponsesWebSocketRebindSafety::Safe)
    }

    fn observe_upstream_event(&self, event: &Value) -> Option<ResponsesProviderObservation> {
        let rate_limits = parse_codex_rate_limits(event);
        let exhausted = rate_limits.as_ref().is_some_and(|rate_limits| {
            aether_admin::provider::quota::codex_rate_limit_metadata_exhausted(rate_limits)
        });
        let private_error = codex_private_error(event, exhausted);
        if rate_limits.is_none() && private_error.is_none() {
            return None;
        }
        let retry_exclusion_until_unix_secs = rate_limits.as_ref().and_then(|rate_limits| {
            codex_quota_exhaustion_reset_at(rate_limits, current_unix_secs())
        });
        Some(ResponsesProviderObservation {
            drain: exhausted.then_some(ResponsesWebSocketDrainDirective {
                error_code: "provider_quota_exhausted",
                retry_current_turn: true,
                retry_exclusion_until_unix_secs,
            }),
            quota_metadata: rate_limits,
            private_error,
        })
    }

    fn sanitize_public_events(
        &self,
        client_event: Option<&Value>,
        public_events: &mut [Value],
    ) -> Result<(), ResponsesPublicWireError> {
        let client_metadata = match client_event.and_then(|event| event.get("metadata")) {
            Some(metadata) => project_metadata(metadata)?,
            None => Value::Object(Map::new()),
        };
        for event in public_events {
            let Some(response) = event.get_mut("response").and_then(Value::as_object_mut) else {
                continue;
            };
            // Codex removes public request metadata before sending upstream.
            // Any metadata in its response is therefore provider-owned and may
            // identify the selected account/profile. Rebuild it exclusively
            // from the current public request instead of trusting the provider.
            response.insert("metadata".to_string(), client_metadata.clone());
        }
        Ok(())
    }

    fn exhaustion_exclusion_identity(
        &self,
        decision: &AiExecutionDecision,
    ) -> Option<ResponsesProviderExclusionIdentity> {
        Some(ResponsesProviderExclusionIdentity {
            account_id: codex_account_id_from_headers(&decision.provider_request_headers)
                .map(str::to_string),
        })
    }

    async fn persist_upstream_observation(
        &self,
        state: &AppState,
        trace_id: &str,
        report_context: Option<&Value>,
        observation: ResponsesProviderObservation,
    ) {
        let Some(rate_limits) = observation.quota_metadata else {
            return;
        };
        if let Err(error) =
            sync_codex_websocket_quota_metadata(state, report_context, rate_limits).await
        {
            tracing::warn!(
                target: CODEX_WEBSOCKET_LOG_TARGET,
                event_name = "codex_websocket_quota_sync_failed",
                log_type = "ops",
                transport = "websocket",
                websocket = true,
                trace_id = %trace_id,
                error = ?error,
                "gateway failed to persist Codex WebSocket quota metadata"
            );
        }
    }
}

fn codex_direct_rebind_safety(event: &Value) -> ResponsesWebSocketRebindSafety {
    let event_type = event
        .get("type")
        .and_then(Value::as_str)
        .unwrap_or_default();
    if matches!(event_type, "codex.rate_limits" | "codex.response.metadata") {
        // Codex emits these as pre-response advisory metadata. They do
        // not create a public `response.*` object, so a replacement
        // upstream can safely emit its own current snapshot.
        return ResponsesWebSocketRebindSafety::Safe;
    }
    if event_type == "error" && parse_codex_rate_limits(event).is_some() {
        // The quota error is withheld from the client when the shared
        // session successfully rebinds, therefore it remains replay-safe.
        return ResponsesWebSocketRebindSafety::Safe;
    }
    let reason = if is_public_responses_event(event) {
        "standard_response_event"
    } else {
        "unrecognized_upstream_event"
    };
    ResponsesWebSocketRebindSafety::Unsafe { reason }
}

fn codex_private_error(
    event: &Value,
    quota_exhausted: bool,
) -> Option<ResponsesProviderPrivateError> {
    let mut private_error = codex_direct_private_error(event).or_else(|| {
        event
            .get("chunks")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .find_map(codex_direct_private_error)
    })?;
    if quota_exhausted {
        private_error.status_code = 429;
    }
    Some(private_error)
}

fn codex_direct_private_error(event: &Value) -> Option<ResponsesProviderPrivateError> {
    let event_type = event.get("type").and_then(Value::as_str);
    let response_snapshot_error = event_type.is_some_and(|event_type| {
        event_type.starts_with("response.")
            && event
                .pointer("/response/error")
                .is_some_and(|error| !error.is_null())
    });
    if !matches!(event_type, Some("error" | "response.failed")) && !response_snapshot_error {
        return None;
    }

    if codex_error_is_explicitly_safe_for_public_wire(event) {
        return None;
    }

    // Codex errors are provider-owned documents. Their code, message, and
    // nested fields may carry account/profile identifiers even when the
    // taxonomy is unfamiliar, so unknown errors must never reach the public
    // Responses socket by default.
    Some(ResponsesProviderPrivateError {
        error_code: "responses_provider_error",
        client_message: "Provider failed the Responses request",
        status_code: 502,
    })
}

fn codex_error_is_explicitly_safe_for_public_wire(event: &Value) -> bool {
    // A `response.failed` is a provider-owned response snapshot. Even when its
    // error code and message look like a public invalid-request error, sibling
    // response fields may contain account-scoped metadata. Only the minimal
    // top-level Responses `error` event is eligible for public passthrough.
    if event.get("type").and_then(Value::as_str) != Some("error") {
        return false;
    }
    let Some((code, message, param)) = codex_public_error_fields(event) else {
        return false;
    };
    if !matches!(
        code.trim().to_ascii_lowercase().as_str(),
        "invalid_request" | "invalid_request_error"
    ) {
        return false;
    }
    if codex_error_status(event).is_some_and(|status| status != 400) {
        return false;
    }
    if !codex_public_invalid_request_message(message) {
        return false;
    }
    param.is_none_or(codex_public_invalid_request_param)
}

fn codex_public_error_fields(event: &Value) -> Option<(&str, &str, Option<&str>)> {
    if event.get("type").and_then(Value::as_str)? != "error" {
        return None;
    }
    let param = match event.get("param")? {
        Value::Null => None,
        Value::String(param) => Some(param.as_str()),
        _ => return None,
    };
    Some((
        event.get("code")?.as_str()?,
        event.get("message")?.as_str()?,
        param,
    ))
}

fn codex_error_status(event: &Value) -> Option<u64> {
    [
        event.get("status_code"),
        event.get("status"),
        event.pointer("/error/status_code"),
        event.pointer("/error/status"),
        event.pointer("/response/status_code"),
        event.pointer("/response/error/status_code"),
        event.pointer("/response/error/status"),
    ]
    .into_iter()
    .flatten()
    .find_map(Value::as_u64)
}

fn codex_public_invalid_request_message(message: &str) -> bool {
    matches!(
        message.trim().to_ascii_lowercase().as_str(),
        "invalid request"
            | "the request was invalid"
            | "request body is invalid"
            | "input was invalid"
    )
}

fn codex_public_invalid_request_param(param: &str) -> bool {
    matches!(
        param.trim(),
        "background"
            | "context_management"
            | "generate"
            | "include"
            | "input"
            | "instructions"
            | "max_output_tokens"
            | "metadata"
            | "model"
            | "parallel_tool_calls"
            | "previous_response_id"
            | "prompt"
            | "prompt_cache_key"
            | "reasoning"
            | "service_tier"
            | "store"
            | "stream"
            | "temperature"
            | "text"
            | "tool_choice"
            | "tools"
            | "top_logprobs"
            | "top_p"
            | "truncation"
            | "user"
    )
}

fn parse_codex_rate_limits(event: &Value) -> Option<Value> {
    aether_admin::provider::quota::parse_codex_websocket_rate_limits_response(
        event,
        current_unix_secs(),
    )
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::{
        CodexResponsesProviderObserver, ResponsesProviderObserver, ResponsesWebSocketRebindSafety,
    };
    use crate::handlers::proxy::websocket::responses::adapter::responses_public_wire_codec;

    #[test]
    fn codex_rate_limit_chunk_is_kept_for_the_terminal_report() {
        let observer = CodexResponsesProviderObserver;
        assert!(observer.observes_upstream_events());
        let mut context = Some(json!({"key_id": "codex-key"}));
        observer.decorate_turn_report_context(
            &mut context,
            &json!({
                "chunks": [{
                    "type": "codex.rate_limits",
                    "plan_type": "free",
                    "rate_limits": {
                        "allowed": true,
                        "limit_reached": false,
                        "primary": {
                            "used_percent": 91,
                            "window_minutes": 43200,
                            "reset_after_seconds": 2590791
                        }
                    }
                }]
            }),
        );

        assert_eq!(
            context.as_ref().and_then(
                |context| context.pointer("/codex_websocket_rate_limits/primary_used_percent")
            ),
            Some(&json!(91.0))
        );
    }

    #[test]
    fn usage_limit_error_is_kept_for_the_terminal_report() {
        let observer = CodexResponsesProviderObserver;
        let mut context = Some(json!({"key_id": "codex-key"}));
        observer.decorate_turn_report_context(
            &mut context,
            &json!({
                "type": "error",
                "error": {
                    "type": "usage_limit_reached",
                    "plan_type": "free",
                    "resets_at": 1_787_274_385u64,
                },
                "status_code": 429,
                "headers": {
                    "X-Codex-Primary-Used-Percent": "100",
                    "X-Codex-Primary-Reset-At": "1787274385",
                },
            }),
        );

        assert_eq!(
            context
                .as_ref()
                .and_then(|context| context.pointer("/codex_websocket_rate_limits/allowed")),
            Some(&json!(false))
        );
        assert_eq!(
            context.as_ref().and_then(|context| {
                context.pointer("/codex_websocket_rate_limits/primary_used_percent")
            }),
            Some(&json!(100.0))
        );
    }

    #[test]
    fn codex_provider_errors_are_private_and_get_a_stable_public_mapping() {
        let observer = CodexResponsesProviderObserver;
        for event in [
            json!({
                "type": "error",
                "status_code": 401,
                "code": "invalid_token_for_acct_private",
                "message": "account acct_private access token expired",
                "param": null
            }),
            json!({
                "type": "error",
                "status_code": 403,
                "error": {
                    "type": "authentication_error",
                    "message": "account acct_private has been deactivated"
                }
            }),
            json!({
                "type": "response.failed",
                "response": {
                    "id": "resp_private",
                    "status": "failed",
                    "error": {
                        "code": "oauth_token_invalid",
                        "message": "refresh token for account acct_private was revoked"
                    }
                }
            }),
            json!({
                "type": "codex.response.metadata",
                "chunks": [{
                    "type": "response.failed",
                    "response": {
                        "id": "resp_private_chunk",
                        "status": "failed",
                        "error": {
                            "code": "server_error",
                            "message": "personal access token owner is inactive: acct_private"
                        }
                    }
                }]
            }),
        ] {
            let observation = observer
                .observe_upstream_event(&event)
                .expect("Codex account/auth errors must be provider observations");
            assert!(observation.drain.is_none());
            assert!(observation.quota_metadata.is_none());
            let public = observation
                .private_error
                .expect("Codex provider errors need a stable public replacement");
            assert_eq!(public.status_code, 502);
            let public = public.public_event();
            assert_eq!(public["type"], "error");
            assert_eq!(public["code"], "responses_provider_error");
            assert!(!public.to_string().contains("acct_private"));
            assert!(!public.to_string().contains("deactivated"));
        }
    }

    #[test]
    fn unknown_codex_errors_cannot_leak_account_or_profile_details() {
        let observer = CodexResponsesProviderObserver;
        for event in [
            json!({
                "type": "error",
                "status_code": 418,
                "code": "future_codex_failure_acct_42",
                "message": "profile_id=profile_secret belongs to acct_private",
                "param": "profile_secret"
            }),
            json!({
                "type": "codex.response.metadata",
                "chunks": [{
                    "type": "response.failed",
                    "response": {
                        "id": "resp_private",
                        "status": "failed",
                        "error": {
                            "code": "unknown_provider_failure",
                            "message": "acct_private profile profile_secret cannot run"
                        }
                    }
                }]
            }),
            json!({
                "type": "error",
                "status_code": 429,
                "code": "future_quota_error",
                "message": "acct_private profile profile_secret exhausted",
                "param": null,
                "error": {
                    "type": "usage_limit_reached",
                    "plan_type": "free",
                    "resets_at": 1_787_274_385u64
                },
                "headers": {
                    "X-Codex-Primary-Used-Percent": "100",
                    "X-Codex-Primary-Reset-At": "1787274385"
                }
            }),
        ] {
            let observation = observer
                .observe_upstream_event(&event)
                .expect("unknown Codex errors must be observed");
            let public = observation
                .private_error
                .expect("unknown Codex errors must be replaced")
                .public_event();
            assert_eq!(public["code"], "responses_provider_error");
            assert_eq!(public["message"], "Provider failed the Responses request");
            let wire = public.to_string();
            assert!(!wire.contains("acct_private"));
            assert!(!wire.contains("profile_secret"));
            assert!(!wire.contains("future_codex_failure"));
            assert!(!wire.contains("unknown_provider_failure"));

            let projected = responses_public_wire_codec()
                .public_events(&public)
                .expect("stable replacement must be a valid public Responses event");
            assert_eq!(projected.len(), 1);
            assert_eq!(projected[0]["type"], "error");
            assert_eq!(projected[0]["code"], "responses_provider_error");
            assert_eq!(
                projected[0]["message"],
                "Provider failed the Responses request"
            );
            assert!(!projected[0].to_string().contains("acct_private"));
            assert!(!projected[0].to_string().contains("profile_secret"));
            if event
                .pointer("/error/type")
                .and_then(serde_json::Value::as_str)
                == Some("usage_limit_reached")
            {
                assert!(observation.drain.is_some());
                assert!(observation.quota_metadata.is_some());
            }
        }
    }

    #[test]
    fn quota_exhaustion_uses_a_provider_neutral_public_error_code() {
        let observer = CodexResponsesProviderObserver;
        let observation = observer
            .observe_upstream_event(&json!({
                "type": "error",
                "error": {
                    "type": "usage_limit_reached",
                    "plan_type": "free",
                    "resets_at": 1_787_274_385u64
                },
                "status_code": 429
            }))
            .expect("Codex quota exhaustion must be observed");

        assert_eq!(
            observation.drain.map(|drain| drain.error_code),
            Some("provider_quota_exhausted")
        );
        assert_eq!(
            observation
                .private_error
                .expect("quota error should be privately mapped")
                .status_code,
            429
        );
    }

    #[test]
    fn only_allowlisted_invalid_request_errors_remain_public() {
        let observer = CodexResponsesProviderObserver;
        assert!(observer
            .observe_upstream_event(&json!({
                "type": "error",
                "status_code": 400,
                "code": "invalid_request",
                "message": "input was invalid",
                "param": "input"
            }))
            .is_none());

        for event in [
            json!({
                "type": "error",
                "status_code": 400,
                "code": "invalid_request",
                "message": "invalid request for profile profile_secret",
                "param": "input"
            }),
            json!({
                "type": "error",
                "status_code": 400,
                "code": "server_error",
                "message": "input was invalid",
                "param": "input"
            }),
            json!({
                "type": "error",
                "status_code": 400,
                "code": "invalid_request",
                "message": "input was invalid",
                "param": "profile_id"
            }),
        ] {
            let public = observer
                .observe_upstream_event(&event)
                .expect("non-allowlisted Codex errors must be observed")
                .private_error
                .expect("non-allowlisted Codex errors must be replaced")
                .public_event();
            assert_eq!(public["code"], "responses_provider_error");
            assert!(!public.to_string().contains("profile_secret"));
        }
    }

    #[test]
    fn response_failed_is_private_even_when_its_error_looks_public() {
        let observer = CodexResponsesProviderObserver;
        let event = json!({
            "type": "response.failed",
            "response": {
                "id": "resp_public_looking",
                "created_at": 1.0,
                "error": {
                    "code": "invalid_request",
                    "message": "input was invalid"
                },
                "incomplete_details": null,
                "instructions": null,
                "metadata": {"account_id": "acct_private"},
                "model": "gpt-test",
                "object": "response",
                "output": [],
                "parallel_tool_calls": true,
                "prompt_cache_key": "cache_acct_private",
                "status": "failed",
                "temperature": null,
                "tool_choice": "auto",
                "tools": [],
                "top_p": null,
                "user": "user_acct_private"
            }
        });

        let public = observer
            .observe_upstream_event(&event)
            .expect("all Codex response.failed events must be observed")
            .private_error
            .expect("Codex response.failed must be replaced")
            .public_event();
        let projected = responses_public_wire_codec()
            .public_events(&public)
            .expect("stable replacement must be a public Responses error");

        assert_eq!(projected.len(), 1);
        assert_eq!(projected[0]["type"], "error");
        assert_eq!(projected[0]["code"], "responses_provider_error");
        let wire = projected[0].to_string();
        assert!(!wire.contains("acct_private"));
        assert!(!wire.contains("cache_acct_private"));
        assert!(!wire.contains("user_acct_private"));
        assert!(projected[0].get("response").is_none());
    }

    #[test]
    fn non_failed_codex_snapshots_with_errors_are_also_private() {
        let observer = CodexResponsesProviderObserver;
        for event_type in [
            "response.created",
            "response.in_progress",
            "response.queued",
            "response.completed",
        ] {
            let event = json!({
                "type": event_type,
                "response": {
                    "error": {
                        "code": "provider_profile_error",
                        "message": "account acct_private profile profile_private failed"
                    }
                }
            });
            let public = observer
                .observe_upstream_event(&event)
                .expect("Codex snapshots with provider errors must be observed")
                .private_error
                .expect("Codex snapshot errors must be replaced")
                .public_event();

            assert_eq!(public["code"], "responses_provider_error");
            assert!(!public.to_string().contains("acct_private"));
            assert!(!public.to_string().contains("profile_private"));
        }
    }

    #[test]
    fn allowlisted_top_level_invalid_request_projects_only_public_error_fields() {
        let observer = CodexResponsesProviderObserver;
        let event = json!({
            "type": "error",
            "status_code": 400,
            "code": "invalid_request_error",
            "message": "request body is invalid",
            "param": "input",
            "account_id": "acct_private",
            "headers": {"x-codex-account-id": "acct_private"},
            "error": {
                "code": "provider_private",
                "message": "profile acct_private is unavailable",
                "param": "account_id"
            }
        });

        assert!(
            observer.observe_upstream_event(&event).is_none(),
            "the exact top-level invalid-request tuple is public"
        );
        let projected = responses_public_wire_codec()
            .public_events(&event)
            .expect("allowlisted top-level error must use the public codec");

        assert_eq!(projected.len(), 1);
        assert_eq!(projected[0]["type"], "error");
        assert_eq!(projected[0]["code"], "invalid_request_error");
        assert_eq!(projected[0]["message"], "request body is invalid");
        assert_eq!(projected[0]["param"], "input");
        assert!(projected[0].get("account_id").is_none());
        assert!(projected[0].get("headers").is_none());
        assert!(projected[0].get("error").is_none());
        assert!(!projected[0].to_string().contains("acct_private"));
    }

    #[test]
    fn successful_codex_snapshots_rebuild_metadata_from_the_public_request() {
        let observer = CodexResponsesProviderObserver;
        let client_event = json!({
            "type": "response.create",
            "metadata": {"client": "public", "nested": {"drop": true}}
        });
        let mut public = vec![json!({
            "type": "response.completed",
            "response": {
                "metadata": {
                    "account_id": "acct_private",
                    "profile": "profile_private"
                }
            }
        })];

        observer
            .sanitize_public_events(Some(&client_event), &mut public)
            .expect("public request metadata should be valid");

        assert_eq!(
            public[0]["response"]["metadata"],
            json!({"client": "public"})
        );
        assert!(!public[0].to_string().contains("acct_private"));
        assert!(!public[0].to_string().contains("profile_private"));
    }

    #[test]
    fn only_known_codex_pre_response_metadata_is_safe_to_rebind() {
        let observer = CodexResponsesProviderObserver;

        assert_eq!(
            observer.rebind_safety_for_upstream_event(&json!({
                "type": "codex.rate_limits",
                "rate_limits": {"allowed": true}
            })),
            ResponsesWebSocketRebindSafety::Safe
        );
        assert_eq!(
            observer.rebind_safety_for_upstream_event(&json!({
                "type": "codex.response.metadata"
            })),
            ResponsesWebSocketRebindSafety::Safe
        );
        assert_eq!(
            observer.rebind_safety_for_upstream_event(&json!({
                "chunks": [
                    {"type": "codex.rate_limits", "rate_limits": {"allowed": true}},
                    {"type": "codex.response.metadata"}
                ]
            })),
            ResponsesWebSocketRebindSafety::Safe
        );
        assert_eq!(
            observer.rebind_safety_for_upstream_event(&json!({
                "type": "response.created"
            })),
            ResponsesWebSocketRebindSafety::Unsafe {
                reason: "standard_response_event"
            }
        );
        assert_eq!(
            observer.rebind_safety_for_upstream_event(&json!({
                "type": "codex.unknown"
            })),
            ResponsesWebSocketRebindSafety::Unsafe {
                reason: "unrecognized_upstream_event"
            }
        );

        assert_eq!(
            observer.rebind_safety_for_upstream_event(&json!({
                "type": "response.created",
                "chunks": [
                    {"type": "codex.rate_limits", "rate_limits": {"allowed": true}}
                ]
            })),
            ResponsesWebSocketRebindSafety::Unsafe {
                reason: "standard_response_event"
            }
        );
    }
}
