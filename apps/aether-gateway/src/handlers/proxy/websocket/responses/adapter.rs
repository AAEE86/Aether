//! Backend, public wire, and provider-observation boundaries for Responses WS.

use std::borrow::Cow;

use async_trait::async_trait;
use serde_json::{json, Value};

use super::adapters::CODEX_RESPONSES_PROVIDER_OBSERVER;
use super::frame::{
    responses_incomplete_reason, responses_incomplete_terminal_kind,
    ResponsesIncompleteTerminalKind,
};
use crate::ai_serving::AiExecutionDecision;
use crate::orchestration::ResponsesProviderObserverKind;
use crate::AppState;

#[derive(Debug, Clone, Copy)]
pub(super) struct ResponsesWebSocketDrainDirective {
    pub(super) error_code: &'static str,
    pub(super) retry_current_turn: bool,
    pub(super) retry_exclusion_until_unix_secs: Option<u64>,
}

#[derive(Debug, Clone)]
pub(super) struct ResponsesProviderObservation {
    pub(super) drain: Option<ResponsesWebSocketDrainDirective>,
    pub(super) quota_metadata: Option<Value>,
    pub(super) private_error: Option<ResponsesProviderPrivateError>,
}

#[derive(Debug, Clone, Copy)]
pub(super) struct ResponsesProviderPrivateError {
    pub(super) error_code: &'static str,
    pub(super) client_message: &'static str,
    pub(super) status_code: u16,
}

impl ResponsesProviderPrivateError {
    pub(super) fn public_event(self) -> Value {
        json!({
            "type": "error",
            "code": self.error_code,
            "message": self.client_message,
            "param": null,
        })
    }
}

#[derive(Debug, Clone, Default)]
pub(super) struct ResponsesProviderExclusionIdentity {
    pub(super) account_id: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum ResponsesWebSocketRebindSafety {
    Safe,
    Unsafe { reason: &'static str },
}

/// Provider-only side channel. Codex quota/account metadata lives here; it is
/// deliberately independent from the native transport and public codec.
#[async_trait]
pub(super) trait ResponsesProviderObserver: Send + Sync {
    fn kind(&self) -> ResponsesProviderObserverKind;

    fn decorate_turn_report_context(&self, report_context: &mut Option<Value>, event: &Value);

    fn observes_upstream_events(&self) -> bool;

    fn rebind_safety_for_upstream_event(&self, event: &Value) -> ResponsesWebSocketRebindSafety;

    fn observe_upstream_event(&self, event: &Value) -> Option<ResponsesProviderObservation>;

    /// Applies provider-specific fail-closed rules after the provider-neutral
    /// Responses projection and before the public FSM observes any event.
    fn sanitize_public_events(
        &self,
        _client_event: Option<&Value>,
        _public_events: &mut [Value],
    ) -> Result<(), ResponsesPublicWireError> {
        Ok(())
    }

    fn exhaustion_exclusion_identity(
        &self,
        _decision: &AiExecutionDecision,
    ) -> Option<ResponsesProviderExclusionIdentity> {
        None
    }

    async fn persist_upstream_observation(
        &self,
        state: &AppState,
        trace_id: &str,
        report_context: Option<&Value>,
        observation: ResponsesProviderObservation,
    );
}

struct StandardResponsesProviderObserver;

static STANDARD_RESPONSES_PROVIDER_OBSERVER: StandardResponsesProviderObserver =
    StandardResponsesProviderObserver;

#[async_trait]
impl ResponsesProviderObserver for StandardResponsesProviderObserver {
    fn kind(&self) -> ResponsesProviderObserverKind {
        ResponsesProviderObserverKind::Standard
    }

    fn decorate_turn_report_context(&self, _report_context: &mut Option<Value>, _event: &Value) {}

    fn observes_upstream_events(&self) -> bool {
        false
    }

    fn rebind_safety_for_upstream_event(&self, event: &Value) -> ResponsesWebSocketRebindSafety {
        let reason = if is_public_responses_event(event) {
            "standard_response_event"
        } else {
            "unrecognized_upstream_event"
        };
        ResponsesWebSocketRebindSafety::Unsafe { reason }
    }

    fn observe_upstream_event(&self, _event: &Value) -> Option<ResponsesProviderObservation> {
        None
    }

    async fn persist_upstream_observation(
        &self,
        _state: &AppState,
        _trace_id: &str,
        _report_context: Option<&Value>,
        _observation: ResponsesProviderObservation,
    ) {
    }
}

pub(super) fn resolve_responses_provider_observer(
    kind: ResponsesProviderObserverKind,
) -> &'static dyn ResponsesProviderObserver {
    match kind {
        ResponsesProviderObserverKind::Standard => &STANDARD_RESPONSES_PROVIDER_OBSERVER,
        ResponsesProviderObserverKind::Codex => &CODEX_RESPONSES_PROVIDER_OBSERVER,
    }
}

const MAX_PUBLIC_EVENTS_PER_PROVIDER_FRAME: usize = 256;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum ResponsesPublicWireError {
    TooManyEvents,
    UnknownEventType,
    InvalidEventShape,
    MultipleTerminalEvents,
    TerminalNotLast,
    EventBeforeCreated,
    DuplicateCreated,
    EventAfterTerminal,
    ResponseIdChanged,
}

impl ResponsesPublicWireError {
    pub(super) const fn as_str(self) -> &'static str {
        match self {
            Self::TooManyEvents => "too_many_public_events",
            Self::UnknownEventType => "unknown_public_event_type",
            Self::InvalidEventShape => "invalid_public_event_shape",
            Self::MultipleTerminalEvents => "multiple_terminal_events",
            Self::TerminalNotLast => "terminal_event_not_last",
            Self::EventBeforeCreated => "public_event_before_response_created",
            Self::DuplicateCreated => "duplicate_response_created",
            Self::EventAfterTerminal => "public_event_after_terminal",
            Self::ResponseIdChanged => "response_id_changed",
        }
    }
}

/// Cross-frame state for one public logical response. Provider frames are
/// projected first, then this state validates their public ordering before any
/// event is written to the client.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(super) enum ResponsesPublicEventState {
    #[default]
    AwaitingCreated,
    Active {
        response_id: String,
    },
    Terminal {
        response_id: Option<String>,
    },
}

impl ResponsesPublicEventState {
    pub(super) fn reset(&mut self) {
        *self = Self::AwaitingCreated;
    }

    pub(super) fn accept_events(
        &mut self,
        events: &[Value],
    ) -> Result<(), ResponsesPublicWireError> {
        // Validate a provider batch transactionally. An invalid later event
        // must not leave the connection state partially advanced.
        let mut next = self.clone();
        for event in events {
            next.accept_event(event)?;
        }
        *self = next;
        Ok(())
    }

    /// Records a gateway-generated request error that terminates the active
    /// public response. The caller closes the client socket immediately after
    /// writing the error, so no provider event may follow it on this turn.
    pub(super) fn accept_local_terminal_error(&mut self) -> Result<(), ResponsesPublicWireError> {
        self.accept_event(&json!({"type": "error"}))
    }

    fn accept_event(&mut self, event: &Value) -> Result<(), ResponsesPublicWireError> {
        let event_type = event
            .get("type")
            .and_then(Value::as_str)
            .ok_or(ResponsesPublicWireError::InvalidEventShape)?;
        if matches!(self, Self::Terminal { .. }) {
            return Err(ResponsesPublicWireError::EventAfterTerminal);
        }

        if event_type == "response.created" {
            if !matches!(self, Self::AwaitingCreated) {
                return Err(ResponsesPublicWireError::DuplicateCreated);
            }
            let response_id = response_event_id(event)?;
            *self = Self::Active { response_id };
            return Ok(());
        }

        if event_type == "error" {
            let response_id = match self {
                Self::AwaitingCreated => None,
                Self::Active { response_id } => Some(response_id.clone()),
                Self::Terminal { .. } => unreachable!("terminal was rejected above"),
            };
            *self = Self::Terminal { response_id };
            return Ok(());
        }

        let Self::Active { response_id } = self else {
            return Err(ResponsesPublicWireError::EventBeforeCreated);
        };
        if response_event_has_snapshot(event_type) && response_event_id(event)? != *response_id {
            return Err(ResponsesPublicWireError::ResponseIdChanged);
        }
        if is_public_terminal_event(event) {
            *self = Self::Terminal {
                response_id: Some(response_id.clone()),
            };
        }
        Ok(())
    }
}

fn response_event_has_snapshot(event_type: &str) -> bool {
    matches!(
        event_type,
        "response.created"
            | "response.in_progress"
            | "response.queued"
            | "response.completed"
            | "response.failed"
            | "response.incomplete"
    )
}

fn response_event_id(event: &Value) -> Result<String, ResponsesPublicWireError> {
    event
        .pointer("/response/id")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string)
        .ok_or(ResponsesPublicWireError::InvalidEventShape)
}

/// Public OpenAI Responses WebSocket encoder. A provider frame may contain a
/// private batch envelope, so encoding can yield zero, one, or many owned
/// public events while preserving their document order. Ownership is
/// intentional: provider-private fields must never survive this boundary by
/// accident.
pub(super) trait ResponsesPublicWireCodec: Send + Sync {
    fn public_events(&self, provider_event: &Value)
        -> Result<Vec<Value>, ResponsesPublicWireError>;
}

struct OpenAiResponsesPublicWireCodec;

static OPENAI_RESPONSES_PUBLIC_WIRE_CODEC: OpenAiResponsesPublicWireCodec =
    OpenAiResponsesPublicWireCodec;

impl ResponsesPublicWireCodec for OpenAiResponsesPublicWireCodec {
    fn public_events(
        &self,
        provider_event: &Value,
    ) -> Result<Vec<Value>, ResponsesPublicWireError> {
        let mut events = Vec::new();
        let root_has_type = provider_event.get("type").and_then(Value::as_str).is_some();
        let chunks = match provider_event.get("chunks") {
            Some(Value::Array(chunks)) if !chunks.is_empty() => Some(chunks),
            Some(_) => return Err(ResponsesPublicWireError::InvalidEventShape),
            None => None,
        };
        if root_has_type {
            collect_public_event(&mut events, provider_event)?;
        } else if chunks.is_none() {
            return Err(ResponsesPublicWireError::InvalidEventShape);
        }
        if let Some(chunks) = chunks {
            for event in chunks {
                collect_public_event(&mut events, event)?;
            }
        }

        let terminal_indexes = events
            .iter()
            .enumerate()
            .filter_map(|(index, event)| is_public_terminal_event(event).then_some(index))
            .collect::<Vec<_>>();
        if terminal_indexes.len() > 1 {
            return Err(ResponsesPublicWireError::MultipleTerminalEvents);
        }
        if terminal_indexes
            .first()
            .is_some_and(|index| *index + 1 != events.len())
        {
            return Err(ResponsesPublicWireError::TerminalNotLast);
        }
        Ok(events)
    }
}

fn collect_public_event(
    events: &mut Vec<Value>,
    provider_event: &Value,
) -> Result<(), ResponsesPublicWireError> {
    let Some(provider_event_type) = provider_event.get("type").and_then(Value::as_str) else {
        return Err(ResponsesPublicWireError::InvalidEventShape);
    };
    let provider_event = normalize_public_terminal_event(provider_event)?;
    let event_type = provider_event
        .get("type")
        .and_then(Value::as_str)
        .expect("terminal normalization preserves the event type");
    let Some(fields) = public_server_event_fields(event_type) else {
        // Provider namespaces are side-channel observations and are filtered.
        // A response-prefixed event claims to be part of the public protocol;
        // fail closed unless it is one of the documented server events.
        if provider_event_type.starts_with("response.") {
            return Err(ResponsesPublicWireError::UnknownEventType);
        }
        return Ok(());
    };
    if events.len() == MAX_PUBLIC_EVENTS_PER_PROVIDER_FRAME {
        return Err(ResponsesPublicWireError::TooManyEvents);
    }
    events.push(public_event(provider_event.as_ref(), fields)?);
    Ok(())
}

fn normalize_public_terminal_event(
    event: &Value,
) -> Result<Cow<'_, Value>, ResponsesPublicWireError> {
    let event_type = event
        .get("type")
        .and_then(Value::as_str)
        .ok_or(ResponsesPublicWireError::InvalidEventShape)?;
    if event_type == "response.cancelled" {
        let mut normalized = event.clone();
        let object = normalized
            .as_object_mut()
            .ok_or(ResponsesPublicWireError::InvalidEventShape)?;
        object.insert(
            "type".to_string(),
            Value::String("response.failed".to_string()),
        );
        let response = object
            .get_mut("response")
            .and_then(Value::as_object_mut)
            .ok_or(ResponsesPublicWireError::InvalidEventShape)?;
        response.insert("status".to_string(), Value::String("failed".to_string()));
        response.insert(
            "error".to_string(),
            json!({
                "code": "response_cancelled",
                "message": "Provider cancelled the response",
            }),
        );
        response.insert("incomplete_details".to_string(), Value::Null);
        return Ok(Cow::Owned(normalized));
    }
    if event_type != "response.incomplete" {
        return Ok(Cow::Borrowed(event));
    }

    if responses_incomplete_reason(event).is_none() {
        return Err(ResponsesPublicWireError::InvalidEventShape);
    }
    let kind = responses_incomplete_terminal_kind(event)
        .ok_or(ResponsesPublicWireError::InvalidEventShape)?;
    let (public_event_type, public_status, public_reason) = match kind {
        ResponsesIncompleteTerminalKind::MaxOutputTokens => (
            "response.incomplete",
            "incomplete",
            Some("max_output_tokens"),
        ),
        ResponsesIncompleteTerminalKind::ContentFilter => {
            ("response.incomplete", "incomplete", Some("content_filter"))
        }
        ResponsesIncompleteTerminalKind::ToolCall => ("response.completed", "completed", None),
    };

    let mut normalized = event.clone();
    let object = normalized
        .as_object_mut()
        .ok_or(ResponsesPublicWireError::InvalidEventShape)?;
    object.insert(
        "type".to_string(),
        Value::String(public_event_type.to_string()),
    );
    let response = object
        .get_mut("response")
        .and_then(Value::as_object_mut)
        .ok_or(ResponsesPublicWireError::InvalidEventShape)?;
    response.insert(
        "status".to_string(),
        Value::String(public_status.to_string()),
    );
    response.insert("error".to_string(), Value::Null);
    match public_reason {
        Some(reason) => {
            response.insert("incomplete_details".to_string(), json!({"reason": reason}));
        }
        None => {
            response.insert("error".to_string(), Value::Null);
            response.insert("incomplete_details".to_string(), Value::Null);
        }
    }
    Ok(Cow::Owned(normalized))
}

fn public_event(
    event: &Value,
    fields: &'static [&'static str],
) -> Result<Value, ResponsesPublicWireError> {
    let event_type = event
        .get("type")
        .and_then(Value::as_str)
        .ok_or(ResponsesPublicWireError::InvalidEventShape)?;
    validate_public_server_event(event, event_type)?;
    if event_type == "error" {
        return Ok(sanitized_public_error(event));
    }
    let mut public = serde_json::Map::new();
    for field in fields {
        if let Some(value) = event.get(*field) {
            let value = match *field {
                "response" => project_response(value),
                "item" => project_output_item(value),
                "part" => project_content_part(value),
                "annotation" => project_annotation(value),
                "logprobs"
                    if matches!(
                        event_type,
                        "response.output_text.delta" | "response.output_text.done"
                    ) =>
                {
                    project_response_logprobs(value)
                }
                _ => Ok(value.clone()),
            }?;
            public.insert((*field).to_string(), value);
        }
    }
    Ok(Value::Object(public))
}

fn validate_public_server_event(
    event: &Value,
    event_type: &str,
) -> Result<(), ResponsesPublicWireError> {
    // The provider's sequence is not authoritative because Aether stamps the
    // public sequence after projection. Native providers observed in the wild
    // may omit it, but an explicitly supplied value must still have the public
    // wire type so an object cannot hide under this allowlisted field name.
    validate_optional_indexes(event, &["sequence_number"])?;
    if event
        .as_object()
        .is_some_and(|object| object.contains_key("agent"))
    {
        return Err(ResponsesPublicWireError::InvalidEventShape);
    }

    match event_type {
        "error" => validate_public_error_event(event),
        "response.created"
        | "response.in_progress"
        | "response.queued"
        | "response.completed"
        | "response.failed"
        | "response.incomplete" => {
            validate_required_objects(event, &["response"])?;
            let response = event
                .get("response")
                .expect("the required response field was checked");
            validate_response_snapshot(response)
        }
        "response.audio.delta" | "response.audio.transcript.delta" => {
            validate_required_strings(event, &["delta"])
        }
        "response.audio.done" | "response.audio.transcript.done" => Ok(()),
        "response.code_interpreter_call.completed"
        | "response.code_interpreter_call.in_progress"
        | "response.code_interpreter_call.interpreting"
        | "response.file_search_call.completed"
        | "response.file_search_call.in_progress"
        | "response.file_search_call.searching"
        | "response.image_generation_call.completed"
        | "response.image_generation_call.generating"
        | "response.image_generation_call.in_progress"
        | "response.mcp_call.completed"
        | "response.mcp_call.failed"
        | "response.mcp_call.in_progress"
        | "response.mcp_list_tools.completed"
        | "response.mcp_list_tools.failed"
        | "response.mcp_list_tools.in_progress"
        | "response.web_search_call.completed"
        | "response.web_search_call.in_progress"
        | "response.web_search_call.searching" => {
            validate_required_strings(event, &["item_id"])?;
            validate_required_indexes(event, &["output_index"])
        }
        "response.code_interpreter_call_code.delta"
        | "response.custom_tool_call_input.delta"
        | "response.function_call_arguments.delta"
        | "response.mcp_call_arguments.delta" => {
            validate_required_strings(event, &["delta", "item_id"])?;
            validate_required_indexes(event, &["output_index"])
        }
        "response.code_interpreter_call_code.done" => {
            validate_required_strings(event, &["code", "item_id"])?;
            validate_required_indexes(event, &["output_index"])
        }
        "response.content_part.added" | "response.content_part.done" => {
            validate_required_strings(event, &["item_id"])?;
            validate_required_indexes(event, &["content_index", "output_index"])?;
            validate_required_objects(event, &["part"])
        }
        "response.custom_tool_call_input.done" => {
            validate_required_strings(event, &["input", "item_id"])?;
            validate_required_indexes(event, &["output_index"])
        }
        "response.function_call_arguments.done" => {
            validate_required_strings(event, &["arguments", "item_id", "name"])?;
            validate_required_indexes(event, &["output_index"])
        }
        "response.image_generation_call.partial_image" => {
            validate_required_strings(event, &["item_id", "partial_image_b64"])?;
            validate_required_indexes(event, &["output_index", "partial_image_index"])
        }
        "response.mcp_call_arguments.done" => {
            validate_required_strings(event, &["arguments", "item_id"])?;
            validate_required_indexes(event, &["output_index"])
        }
        "response.output_item.added" | "response.output_item.done" => {
            validate_required_objects(event, &["item"])?;
            validate_required_indexes(event, &["output_index"])
        }
        "response.output_text.annotation.added" => {
            validate_required_strings(event, &["item_id"])?;
            validate_required_indexes(
                event,
                &["annotation_index", "content_index", "output_index"],
            )?;
            validate_required_objects(event, &["annotation"])
        }
        "response.output_text.delta" => {
            validate_required_strings(event, &["delta", "item_id"])?;
            validate_required_indexes(event, &["content_index", "output_index"])?;
            validate_required_arrays(event, &["logprobs"])
        }
        "response.output_text.done" => {
            validate_required_strings(event, &["item_id", "text"])?;
            validate_required_indexes(event, &["content_index", "output_index"])?;
            validate_required_arrays(event, &["logprobs"])
        }
        "response.reasoning_summary_part.added" | "response.reasoning_summary_part.done" => {
            validate_required_strings(event, &["item_id"])?;
            validate_required_indexes(event, &["output_index", "summary_index"])?;
            validate_required_objects(event, &["part"])?;
            if event_type.ends_with(".done") {
                validate_optional_literal(event, "status", "incomplete")?;
            }
            Ok(())
        }
        "response.reasoning_summary_text.delta" => {
            validate_required_strings(event, &["delta", "item_id"])?;
            validate_required_indexes(event, &["output_index", "summary_index"])
        }
        "response.reasoning_summary_text.done" => {
            validate_required_strings(event, &["item_id", "text"])?;
            validate_required_indexes(event, &["output_index", "summary_index"])
        }
        "response.reasoning_text.delta" | "response.refusal.delta" => {
            validate_required_strings(event, &["delta", "item_id"])?;
            validate_required_indexes(event, &["content_index", "output_index"])
        }
        "response.reasoning_text.done" => {
            validate_required_strings(event, &["item_id", "text"])?;
            validate_required_indexes(event, &["content_index", "output_index"])
        }
        "response.refusal.done" => {
            validate_required_strings(event, &["item_id", "refusal"])?;
            validate_required_indexes(event, &["content_index", "output_index"])
        }
        _ => Err(ResponsesPublicWireError::UnknownEventType),
    }
}

fn validate_response_snapshot(response: &Value) -> Result<(), ResponsesPublicWireError> {
    require_fields(
        response,
        &[
            "id",
            "created_at",
            "error",
            "incomplete_details",
            "instructions",
            "metadata",
            "model",
            "object",
            "output",
            "parallel_tool_calls",
            "temperature",
            "tool_choice",
            "tools",
            "top_p",
        ],
    )?;
    validate_required_strings(response, &["id", "model"])?;
    validate_present_fields(response, &["created_at"], Value::is_number)?;
    validate_present_fields(response, &["error", "incomplete_details"], |value| {
        value.is_null() || value.is_object()
    })?;
    validate_present_fields(response, &["instructions"], |value| {
        value.is_null() || value.is_string() || value.is_array()
    })?;
    validate_present_fields(response, &["metadata"], |value| {
        value.is_null() || value.is_object()
    })?;
    validate_optional_literal(response, "object", "response")?;
    validate_required_arrays(response, &["output", "tools"])?;
    validate_present_fields(response, &["parallel_tool_calls"], Value::is_boolean)?;
    validate_present_fields(response, &["temperature", "top_p"], |value| {
        value.is_null() || value.is_number()
    })?;
    validate_response_tool_choice(response)?;

    if let Some(error) = response.get("error").filter(|value| !value.is_null()) {
        validate_required_strings(error, &["code", "message"])?;
    }
    validate_present_fields(response, &["background"], |value| {
        value.is_null() || value.is_boolean()
    })?;
    validate_present_fields(response, &["completed_at"], |value| {
        value.is_null() || value.is_number()
    })?;
    validate_present_fields(
        response,
        &["max_output_tokens", "max_tool_calls"],
        |value| value.is_null() || value.as_u64().is_some(),
    )?;
    validate_present_fields(
        response,
        &[
            "previous_response_id",
            "prompt_cache_key",
            "safety_identifier",
        ],
        |value| value.is_null() || value.is_string(),
    )?;
    validate_present_fields(response, &["store"], Value::is_boolean)?;
    validate_present_fields(response, &["user"], |value| {
        value.is_null() || value.is_string()
    })?;
    validate_optional_one_of(
        response,
        "prompt_cache_retention",
        &["in_memory", "24h"],
        true,
    )?;
    validate_optional_one_of(
        response,
        "service_tier",
        &["auto", "default", "flex", "scale", "priority", "fast"],
        true,
    )?;
    validate_optional_one_of(
        response,
        "status",
        &[
            "completed",
            "failed",
            "in_progress",
            "cancelled",
            "queued",
            "incomplete",
        ],
        false,
    )?;
    validate_optional_one_of(response, "truncation", &["auto", "disabled"], true)?;
    validate_present_fields(response, &["top_logprobs"], |value| {
        value.is_null() || value.as_u64().is_some_and(|value| value <= 20)
    })?;
    Ok(())
}

fn validate_response_tool_choice(response: &Value) -> Result<(), ResponsesPublicWireError> {
    let choice = response
        .get("tool_choice")
        .expect("the required tool_choice field was checked");
    match choice {
        Value::String(value) if matches!(value.as_str(), "none" | "auto" | "required") => Ok(()),
        Value::Object(_) => project_tool_choice(choice).map(|_| ()),
        _ => Err(ResponsesPublicWireError::InvalidEventShape),
    }
}

fn validate_public_error_event(event: &Value) -> Result<(), ResponsesPublicWireError> {
    let top_level_count = ["code", "message", "param"]
        .iter()
        .filter(|field| event.get(**field).is_some())
        .count();
    let source = match top_level_count {
        3 => event,
        0 => event
            .get("error")
            .filter(|error| error.is_object())
            .ok_or(ResponsesPublicWireError::InvalidEventShape)?,
        _ => return Err(ResponsesPublicWireError::InvalidEventShape),
    };
    require_fields(source, &["code", "message", "param"])?;
    validate_optional_string_or_null(source, &["code", "param"])?;
    validate_required_strings(source, &["message"])
}

fn validate_required_strings(
    value: &Value,
    fields: &[&str],
) -> Result<(), ResponsesPublicWireError> {
    require_fields(value, fields)?;
    validate_optional_strings(value, fields)
}

fn validate_optional_strings(
    value: &Value,
    fields: &[&str],
) -> Result<(), ResponsesPublicWireError> {
    validate_present_fields(value, fields, Value::is_string)
}

fn validate_optional_nullable_strings(
    value: &Value,
    fields: &[&str],
) -> Result<(), ResponsesPublicWireError> {
    validate_present_fields(value, fields, |value| value.is_null() || value.is_string())
}

fn validate_required_numbers(
    value: &Value,
    fields: &[&str],
) -> Result<(), ResponsesPublicWireError> {
    require_fields(value, fields)?;
    validate_present_fields(value, fields, Value::is_number)
}

fn validate_optional_numbers(
    value: &Value,
    fields: &[&str],
) -> Result<(), ResponsesPublicWireError> {
    validate_present_fields(value, fields, Value::is_number)
}

fn validate_required_booleans(
    value: &Value,
    fields: &[&str],
) -> Result<(), ResponsesPublicWireError> {
    require_fields(value, fields)?;
    validate_present_fields(value, fields, Value::is_boolean)
}

fn validate_optional_booleans(
    value: &Value,
    fields: &[&str],
) -> Result<(), ResponsesPublicWireError> {
    validate_present_fields(value, fields, Value::is_boolean)
}

fn validate_optional_nullable_booleans(
    value: &Value,
    fields: &[&str],
) -> Result<(), ResponsesPublicWireError> {
    validate_present_fields(value, fields, |value| value.is_null() || value.is_boolean())
}

fn validate_required_indexes(
    value: &Value,
    fields: &[&str],
) -> Result<(), ResponsesPublicWireError> {
    require_fields(value, fields)?;
    validate_optional_indexes(value, fields)
}

fn validate_optional_indexes(
    value: &Value,
    fields: &[&str],
) -> Result<(), ResponsesPublicWireError> {
    validate_present_fields(value, fields, |value| value.as_u64().is_some())
}

fn validate_required_objects(
    value: &Value,
    fields: &[&str],
) -> Result<(), ResponsesPublicWireError> {
    require_fields(value, fields)?;
    validate_present_fields(value, fields, Value::is_object)
}

fn validate_required_arrays(
    value: &Value,
    fields: &[&str],
) -> Result<(), ResponsesPublicWireError> {
    require_fields(value, fields)?;
    validate_optional_arrays(value, fields)
}

fn validate_optional_arrays(
    value: &Value,
    fields: &[&str],
) -> Result<(), ResponsesPublicWireError> {
    validate_present_fields(value, fields, Value::is_array)
}

fn validate_required_string_arrays(
    value: &Value,
    fields: &[&str],
) -> Result<(), ResponsesPublicWireError> {
    require_fields(value, fields)?;
    validate_optional_string_arrays(value, fields)
}

fn validate_optional_string_arrays(
    value: &Value,
    fields: &[&str],
) -> Result<(), ResponsesPublicWireError> {
    validate_present_fields(value, fields, |value| {
        value
            .as_array()
            .is_some_and(|values| values.iter().all(Value::is_string))
    })
}

fn validate_required_integer_arrays(
    value: &Value,
    fields: &[&str],
) -> Result<(), ResponsesPublicWireError> {
    require_fields(value, fields)?;
    validate_present_fields(value, fields, |value| {
        value
            .as_array()
            .is_some_and(|values| values.iter().all(|value| value.as_u64().is_some()))
    })
}

fn validate_optional_string_or_null(
    value: &Value,
    fields: &[&str],
) -> Result<(), ResponsesPublicWireError> {
    validate_present_fields(value, fields, |value| value.is_string() || value.is_null())
}

fn validate_optional_literal(
    value: &Value,
    field: &str,
    expected: &str,
) -> Result<(), ResponsesPublicWireError> {
    validate_present_fields(value, &[field], |value| value.as_str() == Some(expected))
}

fn validate_optional_one_of(
    value: &Value,
    field: &str,
    expected: &[&str],
    nullable: bool,
) -> Result<(), ResponsesPublicWireError> {
    validate_present_fields(value, &[field], |value| {
        (nullable && value.is_null())
            || value
                .as_str()
                .is_some_and(|value| expected.contains(&value))
    })
}

fn validate_present_fields(
    value: &Value,
    fields: &[&str],
    valid: impl Fn(&Value) -> bool,
) -> Result<(), ResponsesPublicWireError> {
    let object = value
        .as_object()
        .ok_or(ResponsesPublicWireError::InvalidEventShape)?;
    if fields
        .iter()
        .filter_map(|field| object.get(*field))
        .all(valid)
    {
        Ok(())
    } else {
        Err(ResponsesPublicWireError::InvalidEventShape)
    }
}

fn project_object(value: &Value, fields: &[&str]) -> Result<Value, ResponsesPublicWireError> {
    let source = value
        .as_object()
        .ok_or(ResponsesPublicWireError::InvalidEventShape)?;
    let mut projected = serde_json::Map::new();
    for field in fields {
        if let Some(value) = source.get(*field) {
            projected.insert((*field).to_string(), value.clone());
        }
    }
    Ok(Value::Object(projected))
}

fn project_array(
    value: &Value,
    project: fn(&Value) -> Result<Value, ResponsesPublicWireError>,
) -> Result<Value, ResponsesPublicWireError> {
    let values = value
        .as_array()
        .ok_or(ResponsesPublicWireError::InvalidEventShape)?;
    values
        .iter()
        .map(project)
        .collect::<Result<Vec<_>, _>>()
        .map(Value::Array)
}

fn require_fields(value: &Value, fields: &[&str]) -> Result<(), ResponsesPublicWireError> {
    let object = value
        .as_object()
        .ok_or(ResponsesPublicWireError::InvalidEventShape)?;
    if fields.iter().all(|field| object.contains_key(*field)) {
        Ok(())
    } else {
        Err(ResponsesPublicWireError::InvalidEventShape)
    }
}

fn project_nullable_object(
    value: &Value,
    fields: &[&str],
) -> Result<Value, ResponsesPublicWireError> {
    if value.is_null() {
        Ok(Value::Null)
    } else {
        project_object(value, fields)
    }
}

fn project_response(value: &Value) -> Result<Value, ResponsesPublicWireError> {
    const FIELDS: &[&str] = &[
        "background",
        "completed_at",
        "conversation",
        "created_at",
        "error",
        "id",
        "incomplete_details",
        "instructions",
        "max_output_tokens",
        "max_tool_calls",
        "metadata",
        "model",
        "moderation",
        "object",
        "output",
        "parallel_tool_calls",
        "previous_response_id",
        "prompt",
        "prompt_cache_key",
        "prompt_cache_options",
        "prompt_cache_retention",
        "reasoning",
        "safety_identifier",
        "service_tier",
        "status",
        "store",
        "temperature",
        "text",
        "tool_choice",
        "tools",
        "top_logprobs",
        "top_p",
        "truncation",
        "usage",
        "user",
    ];
    let mut projected = project_object(value, FIELDS)?;
    let object = projected
        .as_object_mut()
        .expect("project_object always returns an object");
    if let Some(output) = object.get_mut("output") {
        *output = project_array(output, project_output_item)?;
    }
    if let Some(metadata) = object.get_mut("metadata") {
        *metadata = project_metadata(metadata)?;
    }
    if let Some(conversation) = object.get_mut("conversation") {
        *conversation = project_response_conversation(conversation)?;
    }
    if let Some(error) = object.get_mut("error") {
        *error = project_response_error(error)?;
    }
    if let Some(details) = object.get_mut("incomplete_details") {
        *details = project_response_incomplete_details(details)?;
    }
    if let Some(instructions) = object.get_mut("instructions") {
        *instructions = project_response_instructions(instructions)?;
    }
    if let Some(moderation) = object.get_mut("moderation") {
        *moderation = project_moderation(moderation)?;
    }
    if let Some(prompt) = object.get_mut("prompt") {
        *prompt = project_response_prompt(prompt)?;
    }
    if let Some(options) = object.get_mut("prompt_cache_options") {
        *options = project_prompt_cache_options(options)?;
    }
    if let Some(reasoning) = object.get_mut("reasoning") {
        *reasoning = project_response_reasoning(reasoning)?;
    }
    if let Some(text) = object.get_mut("text") {
        *text = project_response_text(text)?;
    }
    if let Some(tool_choice) = object.get_mut("tool_choice") {
        *tool_choice = project_tool_choice(tool_choice)?;
    }
    if let Some(tools) = object.get_mut("tools") {
        *tools = project_array(tools, project_tool)?;
    }
    if let Some(usage) = object.get_mut("usage") {
        *usage = project_response_usage(usage)?;
    }
    Ok(projected)
}

fn project_response_conversation(value: &Value) -> Result<Value, ResponsesPublicWireError> {
    if value.is_null() {
        return Ok(Value::Null);
    }
    validate_required_strings(value, &["id"])?;
    project_object(value, &["id"])
}

fn project_response_error(value: &Value) -> Result<Value, ResponsesPublicWireError> {
    if value.is_null() {
        return Ok(Value::Null);
    }
    validate_required_strings(value, &["code", "message"])?;
    project_object(value, &["code", "message"])
}

fn project_response_incomplete_details(value: &Value) -> Result<Value, ResponsesPublicWireError> {
    if value.is_null() {
        return Ok(Value::Null);
    }
    require_fields(value, &["reason"])?;
    validate_optional_one_of(
        value,
        "reason",
        &["max_output_tokens", "content_filter"],
        false,
    )?;
    project_object(value, &["reason"])
}

fn project_prompt_cache_options(value: &Value) -> Result<Value, ResponsesPublicWireError> {
    validate_optional_strings(value, &["mode", "ttl"])?;
    project_object(value, &["mode", "ttl"])
}

fn project_response_reasoning(value: &Value) -> Result<Value, ResponsesPublicWireError> {
    if value.is_null() {
        return Ok(Value::Null);
    }
    validate_optional_nullable_strings(
        value,
        &["context", "effort", "generate_summary", "mode", "summary"],
    )?;
    project_object(
        value,
        &["context", "effort", "generate_summary", "mode", "summary"],
    )
}

pub(super) fn project_metadata(value: &Value) -> Result<Value, ResponsesPublicWireError> {
    if value.is_null() {
        return Ok(Value::Null);
    }
    let metadata = value
        .as_object()
        .ok_or(ResponsesPublicWireError::InvalidEventShape)?;
    Ok(Value::Object(
        metadata
            .iter()
            .filter(|(_, value)| value.is_string())
            .take(16)
            .map(|(key, value)| (key.clone(), value.clone()))
            .collect(),
    ))
}

fn project_response_instructions(value: &Value) -> Result<Value, ResponsesPublicWireError> {
    match value {
        Value::Null | Value::String(_) => Ok(value.clone()),
        Value::Array(_) => project_array(value, project_input_item),
        _ => Err(ResponsesPublicWireError::InvalidEventShape),
    }
}

fn project_response_prompt(value: &Value) -> Result<Value, ResponsesPublicWireError> {
    if value.is_null() {
        return Ok(Value::Null);
    }
    validate_required_strings(value, &["id"])?;
    validate_optional_strings(value, &["version"])?;
    validate_present_fields(value, &["variables"], |value| {
        value.is_null() || value.is_object()
    })?;
    let mut projected = project_object(value, &["id", "variables", "version"])?;
    if let Some(variables) = projected
        .as_object_mut()
        .and_then(|object| object.get_mut("variables"))
    {
        if !variables.is_null() {
            let values = variables
                .as_object()
                .ok_or(ResponsesPublicWireError::InvalidEventShape)?;
            *variables = Value::Object(
                values
                    .iter()
                    .map(|(key, value)| {
                        project_prompt_variable(value).map(|value| (key.clone(), value))
                    })
                    .collect::<Result<_, _>>()?,
            );
        }
    }
    Ok(projected)
}

fn project_prompt_variable(value: &Value) -> Result<Value, ResponsesPublicWireError> {
    match value {
        Value::String(_) => Ok(value.clone()),
        Value::Object(_) => project_input_content_part(value),
        _ => Err(ResponsesPublicWireError::InvalidEventShape),
    }
}

fn project_input_item(value: &Value) -> Result<Value, ResponsesPublicWireError> {
    match value.get("type").and_then(Value::as_str) {
        Some("message") => {
            project_input_message(value, &["content", "id", "phase", "role", "status", "type"])
        }
        Some("compaction_trigger") => {
            require_fields(value, &["type"])?;
            project_object(value, &["type"])
        }
        Some("item_reference") => {
            validate_required_strings(value, &["id"])?;
            project_object(value, &["id", "type"])
        }
        Some(_) => project_output_item_inner(value, false),
        // EasyInputMessage deliberately does not require its `type` field.
        None => {
            require_fields(value, &["role", "content"])?;
            project_input_message(value, &["content", "phase", "role", "type"])
        }
    }
}

fn project_input_message(
    value: &Value,
    fields: &[&str],
) -> Result<Value, ResponsesPublicWireError> {
    if value
        .as_object()
        .is_some_and(|object| object.contains_key("agent"))
    {
        return Err(ResponsesPublicWireError::InvalidEventShape);
    }
    require_fields(value, &["content", "role"])?;
    validate_required_strings(value, &["role"])?;
    validate_optional_nullable_strings(value, &["id"])?;
    validate_optional_one_of(
        value,
        "role",
        &["user", "assistant", "system", "developer"],
        false,
    )?;
    validate_optional_one_of(value, "phase", &["commentary", "final_answer"], true)?;
    validate_optional_one_of(
        value,
        "status",
        &["in_progress", "completed", "incomplete"],
        false,
    )?;
    let mut projected = project_object(value, fields)?;
    if let Some(content) = projected
        .as_object_mut()
        .and_then(|object| object.get_mut("content"))
    {
        *content = match content {
            Value::String(_) => content.clone(),
            Value::Array(_) => project_array(content, project_input_content_part)?,
            _ => return Err(ResponsesPublicWireError::InvalidEventShape),
        };
    }
    Ok(projected)
}

fn project_input_content_part(value: &Value) -> Result<Value, ResponsesPublicWireError> {
    let mut projected = match value.get("type").and_then(Value::as_str) {
        Some("input_text") => {
            validate_required_strings(value, &["text"])?;
            project_object(value, &["prompt_cache_breakpoint", "text", "type"])
        }
        Some("input_image") => {
            validate_optional_nullable_strings(value, &["detail", "file_id", "image_url"])?;
            project_object(
                value,
                &[
                    "detail",
                    "file_id",
                    "image_url",
                    "prompt_cache_breakpoint",
                    "type",
                ],
            )
        }
        Some("input_file") => {
            validate_optional_nullable_strings(
                value,
                &["detail", "file_data", "file_id", "file_url", "filename"],
            )?;
            project_object(
                value,
                &[
                    "detail",
                    "file_data",
                    "file_id",
                    "file_url",
                    "filename",
                    "prompt_cache_breakpoint",
                    "type",
                ],
            )
        }
        Some("output_text" | "refusal" | "reasoning_text" | "summary_text") => {
            project_content_part(value)
        }
        _ => Err(ResponsesPublicWireError::UnknownEventType),
    }?;
    if let Some(breakpoint) = projected
        .as_object_mut()
        .and_then(|object| object.get_mut("prompt_cache_breakpoint"))
    {
        *breakpoint = project_prompt_cache_breakpoint(breakpoint)?;
    }
    Ok(projected)
}

fn project_prompt_cache_breakpoint(value: &Value) -> Result<Value, ResponsesPublicWireError> {
    require_fields(value, &["mode"])?;
    if value.get("mode").and_then(Value::as_str) != Some("explicit") {
        return Err(ResponsesPublicWireError::InvalidEventShape);
    }
    project_object(value, &["mode"])
}

fn project_response_usage(value: &Value) -> Result<Value, ResponsesPublicWireError> {
    if value.is_null() {
        return Ok(Value::Null);
    }
    validate_required_indexes(value, &["input_tokens", "output_tokens", "total_tokens"])?;
    validate_required_objects(value, &["input_tokens_details", "output_tokens_details"])?;
    let mut projected = project_object(
        value,
        &[
            "input_tokens",
            "input_tokens_details",
            "output_tokens",
            "output_tokens_details",
            "total_tokens",
        ],
    )?;
    let object = projected
        .as_object_mut()
        .expect("project_object always returns an object");
    if let Some(details) = object.get_mut("input_tokens_details") {
        validate_optional_indexes(details, &["cache_write_tokens", "cached_tokens"])?;
        *details = project_object(details, &["cache_write_tokens", "cached_tokens"])?;
    }
    if let Some(details) = object.get_mut("output_tokens_details") {
        validate_optional_indexes(details, &["reasoning_tokens"])?;
        *details = project_object(details, &["reasoning_tokens"])?;
    }
    Ok(projected)
}

fn project_response_text(value: &Value) -> Result<Value, ResponsesPublicWireError> {
    validate_optional_nullable_strings(value, &["verbosity"])?;
    let mut projected = project_object(value, &["format", "verbosity"])?;
    if let Some(format) = projected
        .as_object_mut()
        .and_then(|object| object.get_mut("format"))
    {
        *format = match format.get("type").and_then(Value::as_str) {
            Some("text" | "json_object") => project_object(format, &["type"]),
            Some("json_schema") => {
                validate_required_strings(format, &["name"])?;
                validate_optional_strings(format, &["description"])?;
                validate_optional_nullable_booleans(format, &["strict"])?;
                validate_required_objects(format, &["schema"])?;
                project_object(format, &["description", "name", "schema", "strict", "type"])
            }
            _ => Err(ResponsesPublicWireError::UnknownEventType),
        }?;
    }
    Ok(projected)
}

fn project_tool_choice(value: &Value) -> Result<Value, ResponsesPublicWireError> {
    if let Some(choice) = value.as_str() {
        return matches!(choice, "none" | "auto" | "required")
            .then(|| value.clone())
            .ok_or(ResponsesPublicWireError::InvalidEventShape);
    }
    let tool_type = value
        .get("type")
        .and_then(Value::as_str)
        .ok_or(ResponsesPublicWireError::UnknownEventType)?;
    let mut projected = match tool_type {
        "allowed_tools" => {
            require_fields(value, &["mode", "tools", "type"])?;
            validate_optional_one_of(value, "mode", &["auto", "required"], false)?;
            validate_required_arrays(value, &["tools"])?;
            project_object(value, &["mode", "tools", "type"])
        }
        "function" | "custom" => {
            validate_required_strings(value, &["name"])?;
            project_object(value, &["name", "type"])
        }
        "mcp" => {
            validate_required_strings(value, &["server_label"])?;
            validate_optional_nullable_strings(value, &["name"])?;
            project_object(value, &["name", "server_label", "type"])
        }
        "apply_patch"
        | "code_interpreter"
        | "computer"
        | "computer_use"
        | "computer_use_preview"
        | "file_search"
        | "image_generation"
        | "programmatic_tool_calling"
        | "shell"
        | "web_search"
        | "web_search_preview"
        | "web_search_preview_2025_03_11" => project_object(value, &["type"]),
        _ => Err(ResponsesPublicWireError::UnknownEventType),
    }?;
    if let Some(tools) = projected
        .as_object_mut()
        .and_then(|object| object.get_mut("tools"))
    {
        *tools = project_array(tools, project_allowed_tool_choice)?;
    }
    Ok(projected)
}

fn project_tool(value: &Value) -> Result<Value, ResponsesPublicWireError> {
    project_tool_with_context(value, false)
}

fn project_namespace_tool(value: &Value) -> Result<Value, ResponsesPublicWireError> {
    match value.get("type").and_then(Value::as_str) {
        Some("function" | "custom") => project_tool_with_context(value, true),
        _ => Err(ResponsesPublicWireError::UnknownEventType),
    }
}

fn project_allowed_tool_choice(value: &Value) -> Result<Value, ResponsesPublicWireError> {
    if !value.is_object() || value.get("type").and_then(Value::as_str) == Some("allowed_tools") {
        return Err(ResponsesPublicWireError::InvalidEventShape);
    }
    project_tool_choice(value)
}

fn project_tool_with_context(
    value: &Value,
    namespace_member: bool,
) -> Result<Value, ResponsesPublicWireError> {
    let tool_type = value
        .get("type")
        .and_then(Value::as_str)
        .ok_or(ResponsesPublicWireError::UnknownEventType)?;
    let mut projected = match tool_type {
        "function" => project_object(
            value,
            &[
                "allowed_callers",
                "defer_loading",
                "description",
                "name",
                "output_schema",
                "parameters",
                "strict",
                "type",
            ],
        ),
        "file_search" => project_object(
            value,
            &[
                "filters",
                "max_num_results",
                "ranking_options",
                "type",
                "vector_store_ids",
            ],
        ),
        "computer" | "local_shell" | "programmatic_tool_calling" => {
            project_object(value, &["type"])
        }
        "computer_use_preview" => project_object(
            value,
            &["display_height", "display_width", "environment", "type"],
        ),
        "web_search" | "web_search_2025_08_26" => project_object(
            value,
            &["filters", "search_context_size", "type", "user_location"],
        ),
        "mcp" => project_object(
            value,
            &[
                "allowed_callers",
                "allowed_tools",
                "connector_id",
                "defer_loading",
                "require_approval",
                "server_description",
                "server_label",
                "server_url",
                "tunnel_id",
                "type",
            ],
        ),
        "code_interpreter" => project_object(value, &["allowed_callers", "container", "type"]),
        "image_generation" => project_object(
            value,
            &[
                "action",
                "background",
                "input_fidelity",
                "input_image_mask",
                "model",
                "moderation",
                "output_compression",
                "output_format",
                "partial_images",
                "quality",
                "size",
                "type",
            ],
        ),
        "shell" => project_object(value, &["allowed_callers", "environment", "type"]),
        "custom" => project_object(
            value,
            &[
                "allowed_callers",
                "defer_loading",
                "description",
                "format",
                "name",
                "type",
            ],
        ),
        "namespace" => project_object(value, &["description", "name", "tools", "type"]),
        "tool_search" => project_object(value, &["description", "execution", "parameters", "type"]),
        "web_search_preview" | "web_search_preview_2025_03_11" => project_object(
            value,
            &[
                "search_content_types",
                "search_context_size",
                "type",
                "user_location",
            ],
        ),
        "apply_patch" => project_object(value, &["allowed_callers", "type"]),
        _ => Err(ResponsesPublicWireError::UnknownEventType),
    }?;

    let object = projected
        .as_object_mut()
        .expect("project_object always returns an object");
    if let Some(callers) = object.get_mut("allowed_callers") {
        *callers = project_array(callers, project_tool_caller)?;
    }
    if let Some(options) = object.get_mut("ranking_options") {
        validate_optional_strings(options, &["ranker"])?;
        validate_optional_numbers(options, &["score_threshold"])?;
        *options = project_object(options, &["hybrid_search", "ranker", "score_threshold"])?;
        if let Some(hybrid_search) = options
            .as_object_mut()
            .and_then(|options| options.get_mut("hybrid_search"))
        {
            *hybrid_search = project_hybrid_search_options(hybrid_search)?;
        }
    }
    if let Some(filters) = object.get_mut("filters") {
        *filters = match tool_type {
            "file_search" => project_file_search_filter(filters),
            "web_search" | "web_search_2025_08_26" => project_web_search_filters(filters),
            _ => Err(ResponsesPublicWireError::InvalidEventShape),
        }?;
    }
    if let Some(location) = object.get_mut("user_location") {
        *location = project_web_search_location(
            location,
            matches!(
                tool_type,
                "web_search_preview" | "web_search_preview_2025_03_11"
            ),
        )?;
    }
    if let Some(mask) = object.get_mut("input_image_mask") {
        *mask = project_image_mask(mask)?;
    }
    if let Some(container) = object.get_mut("container") {
        *container = project_code_interpreter_container(container)?;
    }
    if let Some(environment) = object.get_mut("environment") {
        *environment = project_nullable_environment(environment)?;
    }
    if let Some(format) = object.get_mut("format") {
        *format = project_custom_tool_format(format)?;
    }
    if let Some(tools) = object.get_mut("tools") {
        *tools = project_array(tools, project_namespace_tool)?;
    }
    if let Some(allowed_tools) = object.get_mut("allowed_tools") {
        *allowed_tools = project_mcp_allowed_tools(allowed_tools)?;
    }
    if let Some(require_approval) = object.get_mut("require_approval") {
        *require_approval = project_mcp_require_approval(require_approval)?;
    }
    validate_tool_shape(&projected, tool_type, namespace_member)?;
    Ok(projected)
}

fn validate_tool_shape(
    tool: &Value,
    tool_type: &str,
    namespace_member: bool,
) -> Result<(), ResponsesPublicWireError> {
    match tool_type {
        "function" => {
            validate_required_strings(tool, &["name"])?;
            validate_optional_nullable_strings(tool, &["description"])?;
            validate_optional_booleans(tool, &["defer_loading"])?;
            validate_present_fields(tool, &["parameters"], |value| {
                value.is_null() || value.is_object()
            })?;
            validate_present_fields(tool, &["strict"], |value| {
                value.is_null() || value.is_boolean()
            })?;
            if !namespace_member {
                require_fields(tool, &["parameters", "strict"])?;
            }
        }
        "file_search" => {
            validate_required_string_arrays(tool, &["vector_store_ids"])?;
            validate_present_fields(tool, &["max_num_results"], |value| {
                value
                    .as_u64()
                    .is_some_and(|value| (1..=50).contains(&value))
            })?;
        }
        "computer" | "local_shell" | "programmatic_tool_calling" | "apply_patch" => {}
        "computer_use_preview" => {
            validate_required_indexes(tool, &["display_height", "display_width"])?;
            validate_required_strings(tool, &["environment"])?;
        }
        "web_search" | "web_search_2025_08_26" => {
            validate_optional_one_of(
                tool,
                "search_context_size",
                &["low", "medium", "high"],
                false,
            )?;
        }
        "mcp" => {
            validate_required_strings(tool, &["server_label"])?;
            validate_optional_strings(tool, &["server_description", "server_url", "tunnel_id"])?;
            validate_optional_booleans(tool, &["defer_loading"])?;
            validate_optional_one_of(
                tool,
                "connector_id",
                &[
                    "connector_dropbox",
                    "connector_gmail",
                    "connector_googlecalendar",
                    "connector_googledrive",
                    "connector_microsoftteams",
                    "connector_outlookcalendar",
                    "connector_outlookemail",
                    "connector_sharepoint",
                ],
                false,
            )?;
        }
        "code_interpreter" => require_fields(tool, &["container"])?,
        "image_generation" => {
            validate_optional_one_of(tool, "action", &["auto", "generate", "edit"], false)?;
            validate_optional_one_of(
                tool,
                "background",
                &["transparent", "opaque", "auto"],
                false,
            )?;
            validate_optional_one_of(tool, "input_fidelity", &["low", "high"], true)?;
            validate_optional_strings(tool, &["model", "size"])?;
            validate_optional_one_of(tool, "moderation", &["auto", "low"], false)?;
            validate_present_fields(tool, &["output_compression"], |value| {
                value.as_u64().is_some_and(|value| value <= 100)
            })?;
            validate_optional_one_of(tool, "output_format", &["png", "webp", "jpeg"], false)?;
            validate_present_fields(tool, &["partial_images"], |value| {
                value.as_u64().is_some_and(|value| value <= 3)
            })?;
            validate_optional_one_of(tool, "quality", &["low", "medium", "high", "auto"], false)?;
        }
        "shell" => {}
        "custom" => {
            validate_required_strings(tool, &["name"])?;
            validate_optional_strings(tool, &["description"])?;
            validate_optional_booleans(tool, &["defer_loading"])?;
        }
        "namespace" => {
            validate_required_strings(tool, &["description", "name"])?;
            validate_required_arrays(tool, &["tools"])?;
        }
        "tool_search" => {
            validate_optional_nullable_strings(tool, &["description"])?;
            validate_optional_one_of(tool, "execution", &["client", "server"], false)?;
            validate_present_fields(tool, &["parameters"], |value| {
                value.is_null() || value.is_object()
            })?;
        }
        "web_search_preview" | "web_search_preview_2025_03_11" => {
            validate_optional_string_arrays(tool, &["search_content_types"])?;
            validate_optional_one_of(
                tool,
                "search_context_size",
                &["low", "medium", "high"],
                false,
            )?;
        }
        _ => return Err(ResponsesPublicWireError::UnknownEventType),
    }
    Ok(())
}

fn project_hybrid_search_options(value: &Value) -> Result<Value, ResponsesPublicWireError> {
    require_fields(value, &["embedding_weight", "text_weight"])?;
    if !value.get("embedding_weight").is_some_and(Value::is_number)
        || !value.get("text_weight").is_some_and(Value::is_number)
    {
        return Err(ResponsesPublicWireError::InvalidEventShape);
    }
    project_object(value, &["embedding_weight", "text_weight"])
}

fn project_file_search_filter(value: &Value) -> Result<Value, ResponsesPublicWireError> {
    if value.is_null() {
        return Ok(Value::Null);
    }
    let filter_type = value
        .get("type")
        .and_then(Value::as_str)
        .ok_or(ResponsesPublicWireError::UnknownEventType)?;
    match filter_type {
        "and" | "or" => {
            require_fields(value, &["type", "filters"])?;
            let mut projected = project_object(value, &["filters", "type"])?;
            let filters = projected
                .as_object_mut()
                .and_then(|object| object.get_mut("filters"))
                .expect("required filters field survives projection");
            *filters = project_array(filters, project_file_search_filter)?;
            Ok(projected)
        }
        "eq" | "ne" | "gt" | "gte" | "lt" | "lte" | "in" | "nin" => {
            require_fields(value, &["type", "key", "value"])?;
            validate_required_strings(value, &["key"])?;
            let filter_value = value
                .get("value")
                .expect("required filter value was checked");
            let valid_value = matches!(
                filter_value,
                Value::String(_) | Value::Number(_) | Value::Bool(_)
            ) || filter_value.as_array().is_some_and(|values| {
                values
                    .iter()
                    .all(|value| value.is_string() || value.is_number())
            });
            if !valid_value {
                return Err(ResponsesPublicWireError::InvalidEventShape);
            }
            project_object(value, &["key", "type", "value"])
        }
        _ => Err(ResponsesPublicWireError::UnknownEventType),
    }
}

fn project_web_search_filters(value: &Value) -> Result<Value, ResponsesPublicWireError> {
    if value.is_null() {
        return Ok(Value::Null);
    }
    let projected = project_object(value, &["allowed_domains"])?;
    if let Some(domains) = projected.get("allowed_domains") {
        let valid = domains.is_null()
            || domains
                .as_array()
                .is_some_and(|values| values.iter().all(Value::is_string));
        if !valid {
            return Err(ResponsesPublicWireError::InvalidEventShape);
        }
    }
    Ok(projected)
}

fn project_web_search_location(
    value: &Value,
    preview: bool,
) -> Result<Value, ResponsesPublicWireError> {
    if value.is_null() {
        return Ok(Value::Null);
    }
    if preview {
        require_fields(value, &["type"])?;
        validate_optional_literal(value, "type", "approximate")?;
        validate_optional_nullable_strings(value, &["city", "country", "region", "timezone"])?;
    } else {
        validate_optional_strings(value, &["city", "country", "region", "timezone"])?;
        if value.get("type").is_some() {
            validate_optional_literal(value, "type", "approximate")?;
        }
    }
    project_object(value, &["city", "country", "region", "timezone", "type"])
}

fn project_image_mask(value: &Value) -> Result<Value, ResponsesPublicWireError> {
    validate_optional_strings(value, &["file_id", "image_url"])?;
    project_object(value, &["file_id", "image_url"])
}

fn project_tool_caller(value: &Value) -> Result<Value, ResponsesPublicWireError> {
    if value
        .as_str()
        .is_some_and(|caller| matches!(caller, "direct" | "programmatic"))
    {
        return Ok(value.clone());
    }
    project_caller(value)
}

fn project_caller(value: &Value) -> Result<Value, ResponsesPublicWireError> {
    if value.is_null() {
        return Ok(Value::Null);
    }
    match value.get("type").and_then(Value::as_str) {
        Some("direct") => project_object(value, &["type"]),
        Some("program") => {
            validate_required_strings(value, &["caller_id"])?;
            project_object(value, &["caller_id", "type"])
        }
        _ => Err(ResponsesPublicWireError::UnknownEventType),
    }
}

fn project_nullable_environment(value: &Value) -> Result<Value, ResponsesPublicWireError> {
    if value.is_null() {
        return Ok(value.clone());
    }
    if let Some(environment) = value.as_str() {
        return matches!(environment, "local" | "container_auto")
            .then(|| value.clone())
            .ok_or(ResponsesPublicWireError::InvalidEventShape);
    }
    match value.get("type").and_then(Value::as_str) {
        Some("local") => project_object(value, &["type"]),
        Some("container_reference") => {
            validate_required_strings(value, &["container_id"])?;
            project_object(value, &["container_id", "type"])
        }
        Some("container_auto") => project_shell_container(value),
        _ => Err(ResponsesPublicWireError::UnknownEventType),
    }
}

fn project_shell_container(value: &Value) -> Result<Value, ResponsesPublicWireError> {
    let mut projected = project_object(
        value,
        &[
            "file_ids",
            "memory_limit",
            "network_policy",
            "skills",
            "type",
        ],
    )?;
    validate_optional_string_arrays(&projected, &["file_ids"])?;
    validate_container_memory_limit(&projected)?;
    let object = projected
        .as_object_mut()
        .expect("project_object always returns an object");
    project_field(object, "network_policy", project_container_network_policy)?;
    project_field_array(object, "skills", project_container_skill)?;
    Ok(projected)
}

fn project_container_skill(value: &Value) -> Result<Value, ResponsesPublicWireError> {
    match value.get("type").and_then(Value::as_str) {
        Some("skill_reference") => {
            validate_required_strings(value, &["skill_id"])?;
            validate_optional_strings(value, &["version"])?;
            project_object(value, &["skill_id", "type", "version"])
        }
        Some("inline") => {
            validate_required_strings(value, &["description", "name"])?;
            validate_required_objects(value, &["source"])?;
            let mut projected = project_object(value, &["description", "name", "source", "type"])?;
            let source = projected
                .as_object_mut()
                .and_then(|object| object.get_mut("source"))
                .expect("required source survives projection");
            validate_required_strings(source, &["data"])?;
            require_fields(source, &["media_type", "type"])?;
            validate_optional_literal(source, "media_type", "application/zip")?;
            validate_optional_literal(source, "type", "base64")?;
            *source = project_object(source, &["data", "media_type", "type"])?;
            Ok(projected)
        }
        _ => Err(ResponsesPublicWireError::UnknownEventType),
    }
}

fn project_custom_tool_format(value: &Value) -> Result<Value, ResponsesPublicWireError> {
    match value.get("type").and_then(Value::as_str) {
        Some("text") => project_object(value, &["type"]),
        Some("grammar") => {
            validate_required_strings(value, &["definition"])?;
            require_fields(value, &["syntax"])?;
            validate_optional_one_of(value, "syntax", &["lark", "regex"], false)?;
            project_object(value, &["definition", "syntax", "type"])
        }
        _ => Err(ResponsesPublicWireError::UnknownEventType),
    }
}

fn project_mcp_allowed_tools(value: &Value) -> Result<Value, ResponsesPublicWireError> {
    match value {
        Value::Null => Ok(Value::Null),
        Value::Array(values) if values.iter().all(Value::is_string) => Ok(value.clone()),
        Value::Object(_) => project_mcp_tool_filter(value),
        _ => Err(ResponsesPublicWireError::InvalidEventShape),
    }
}

fn project_mcp_require_approval(value: &Value) -> Result<Value, ResponsesPublicWireError> {
    match value {
        Value::Null => Ok(Value::Null),
        Value::String(mode) if matches!(mode.as_str(), "always" | "never") => Ok(value.clone()),
        Value::Object(_) => {
            let mut projected = project_object(value, &["always", "never"])?;
            let object = projected
                .as_object_mut()
                .expect("project_object always returns an object");
            for field in ["always", "never"] {
                project_field(object, field, project_mcp_tool_filter)?;
            }
            Ok(projected)
        }
        _ => Err(ResponsesPublicWireError::InvalidEventShape),
    }
}

fn project_mcp_tool_filter(value: &Value) -> Result<Value, ResponsesPublicWireError> {
    let projected = project_object(value, &["read_only", "tool_names"])?;
    if projected
        .get("read_only")
        .is_some_and(|read_only| !read_only.is_boolean())
    {
        return Err(ResponsesPublicWireError::InvalidEventShape);
    }
    if projected.get("tool_names").is_some_and(|tool_names| {
        !tool_names
            .as_array()
            .is_some_and(|names| names.iter().all(Value::is_string))
    }) {
        return Err(ResponsesPublicWireError::InvalidEventShape);
    }
    Ok(projected)
}

fn project_code_interpreter_container(value: &Value) -> Result<Value, ResponsesPublicWireError> {
    if value.is_string() {
        return Ok(value.clone());
    }
    require_fields(value, &["type"])?;
    if value.get("type").and_then(Value::as_str) != Some("auto") {
        return Err(ResponsesPublicWireError::InvalidEventShape);
    }
    let mut projected = project_object(
        value,
        &["file_ids", "memory_limit", "network_policy", "type"],
    )?;
    validate_container_memory_limit(&projected)?;
    let object = projected
        .as_object_mut()
        .expect("project_object always returns an object");
    if object.get("file_ids").is_some_and(|file_ids| {
        !file_ids
            .as_array()
            .is_some_and(|ids| ids.iter().all(Value::is_string))
    }) {
        return Err(ResponsesPublicWireError::InvalidEventShape);
    }
    project_field(object, "network_policy", project_container_network_policy)?;
    Ok(projected)
}

fn validate_container_memory_limit(value: &Value) -> Result<(), ResponsesPublicWireError> {
    validate_present_fields(value, &["memory_limit"], |memory_limit| {
        memory_limit.is_null()
            || memory_limit
                .as_str()
                .is_some_and(|limit| matches!(limit, "1g" | "4g" | "16g" | "64g"))
    })
}

fn project_container_network_policy(value: &Value) -> Result<Value, ResponsesPublicWireError> {
    match value.get("type").and_then(Value::as_str) {
        Some("disabled") => {
            require_fields(value, &["type"])?;
            project_object(value, &["type"])
        }
        Some("allowlist") => {
            require_fields(value, &["allowed_domains", "type"])?;
            let mut projected =
                project_object(value, &["allowed_domains", "domain_secrets", "type"])?;
            let object = projected
                .as_object_mut()
                .expect("project_object always returns an object");
            if !object
                .get("allowed_domains")
                .and_then(Value::as_array)
                .is_some_and(|domains| domains.iter().all(Value::is_string))
            {
                return Err(ResponsesPublicWireError::InvalidEventShape);
            }
            project_field_array(
                object,
                "domain_secrets",
                project_container_network_policy_domain_secret,
            )?;
            Ok(projected)
        }
        _ => Err(ResponsesPublicWireError::UnknownEventType),
    }
}

fn project_container_network_policy_domain_secret(
    value: &Value,
) -> Result<Value, ResponsesPublicWireError> {
    require_fields(value, &["domain", "name"])?;
    if ["domain", "name"]
        .iter()
        .any(|field| !value.get(*field).is_some_and(Value::is_string))
    {
        return Err(ResponsesPublicWireError::InvalidEventShape);
    }
    project_object(value, &["domain", "name"])
}

fn project_moderation(value: &Value) -> Result<Value, ResponsesPublicWireError> {
    if value.is_null() {
        return Ok(Value::Null);
    }
    let mut projected = project_object(value, &["input", "output"])?;
    let object = projected
        .as_object_mut()
        .expect("project_object always returns an object");
    for field in ["input", "output"] {
        if let Some(result) = object.get_mut(field) {
            *result = project_moderation_result(result)?;
        }
    }
    Ok(projected)
}

fn project_moderation_result(value: &Value) -> Result<Value, ResponsesPublicWireError> {
    let mut projected = match value.get("type").and_then(Value::as_str) {
        Some("moderation_result") => {
            validate_optional_booleans(value, &["flagged"])?;
            validate_optional_strings(value, &["model"])?;
            project_object(
                value,
                &[
                    "categories",
                    "category_applied_input_types",
                    "category_scores",
                    "flagged",
                    "model",
                    "type",
                ],
            )
        }
        Some("error") => {
            validate_optional_strings(value, &["code", "message"])?;
            project_object(value, &["code", "message", "type"])
        }
        _ => Err(ResponsesPublicWireError::UnknownEventType),
    }?;
    if let Some(object) = projected.as_object_mut() {
        project_field(object, "categories", project_moderation_categories)?;
        project_field(
            object,
            "category_applied_input_types",
            project_moderation_category_input_types,
        )?;
        project_field(
            object,
            "category_scores",
            project_moderation_category_scores,
        )?;
    }
    Ok(projected)
}

fn project_moderation_categories(value: &Value) -> Result<Value, ResponsesPublicWireError> {
    project_moderation_category_map(value, Value::is_boolean)
}

fn project_moderation_category_scores(value: &Value) -> Result<Value, ResponsesPublicWireError> {
    project_moderation_category_map(value, Value::is_number)
}

fn project_moderation_category_input_types(
    value: &Value,
) -> Result<Value, ResponsesPublicWireError> {
    project_moderation_category_map(value, |value| {
        value
            .as_array()
            .is_some_and(|values| values.iter().all(Value::is_string))
    })
}

fn project_moderation_category_map(
    value: &Value,
    valid: fn(&Value) -> bool,
) -> Result<Value, ResponsesPublicWireError> {
    const FIELDS: &[&str] = &[
        "harassment",
        "harassment/threatening",
        "hate",
        "hate/threatening",
        "illicit",
        "illicit/violent",
        "self-harm",
        "self-harm/instructions",
        "self-harm/intent",
        "sexual",
        "sexual/minors",
        "violence",
        "violence/graphic",
    ];
    validate_present_fields(value, FIELDS, valid)?;
    project_object(value, FIELDS)
}

fn project_output_item(value: &Value) -> Result<Value, ResponsesPublicWireError> {
    project_output_item_inner(value, true)
}

const OUTPUT_ITEM_STATUS: &[&str] = &["in_progress", "completed", "incomplete"];

fn is_json_integer(value: &Value) -> bool {
    value.as_i64().is_some() || value.as_u64().is_some()
}

fn validate_optional_integers(
    value: &Value,
    fields: &[&str],
) -> Result<(), ResponsesPublicWireError> {
    validate_present_fields(value, fields, is_json_integer)
}

fn validate_optional_integer_or_null(
    value: &Value,
    fields: &[&str],
) -> Result<(), ResponsesPublicWireError> {
    validate_present_fields(value, fields, |value| {
        value.is_null() || is_json_integer(value)
    })
}

fn validate_optional_nullable_string_arrays(
    value: &Value,
    fields: &[&str],
) -> Result<(), ResponsesPublicWireError> {
    validate_present_fields(value, fields, |value| {
        value.is_null()
            || value
                .as_array()
                .is_some_and(|values| values.iter().all(Value::is_string))
    })
}

fn validate_optional_string_map(
    value: &Value,
    field: &str,
) -> Result<(), ResponsesPublicWireError> {
    validate_present_fields(value, &[field], |value| {
        value
            .as_object()
            .is_some_and(|entries| entries.values().all(Value::is_string))
    })
}

fn validate_optional_public_scalar_map(
    value: &Value,
    field: &str,
) -> Result<(), ResponsesPublicWireError> {
    validate_present_fields(value, &[field], |value| {
        value.is_null()
            || value.as_object().is_some_and(|entries| {
                entries
                    .values()
                    .all(|value| value.is_string() || value.is_number() || value.is_boolean())
            })
    })
}

fn validate_output_item_basic_shape(
    value: &Value,
    event_type: &str,
    enforce_output_requirements: bool,
) -> Result<(), ResponsesPublicWireError> {
    if value
        .as_object()
        .is_some_and(|object| object.contains_key("agent"))
    {
        return Err(ResponsesPublicWireError::InvalidEventShape);
    }
    match event_type {
        "message" => {
            validate_optional_strings(value, &["id"])?;
            validate_optional_one_of(value, "role", &["assistant"], false)?;
            validate_optional_one_of(value, "status", OUTPUT_ITEM_STATUS, false)?;
            validate_optional_one_of(value, "phase", &["commentary", "final_answer"], true)?;
            validate_optional_arrays(value, &["content"])
        }
        "file_search_call" => {
            validate_optional_strings(value, &["id"])?;
            validate_optional_string_arrays(value, &["queries"])?;
            validate_optional_one_of(
                value,
                "status",
                &[
                    "in_progress",
                    "searching",
                    "completed",
                    "incomplete",
                    "failed",
                ],
                false,
            )?;
            validate_present_fields(value, &["results"], |value| {
                value.is_null() || value.is_array()
            })
        }
        "function_call" => {
            validate_optional_strings(value, &["arguments", "call_id", "id", "name", "namespace"])?;
            validate_optional_one_of(value, "status", OUTPUT_ITEM_STATUS, false)
        }
        "function_call_output" | "custom_tool_call_output" => {
            validate_optional_strings(value, &["call_id", "created_by", "name", "namespace"])?;
            validate_present_fields(value, &["id"], |value| {
                value.is_string() || (!enforce_output_requirements && value.is_null())
            })?;
            validate_optional_one_of(
                value,
                "status",
                OUTPUT_ITEM_STATUS,
                !enforce_output_requirements,
            )
        }
        "web_search_call" => {
            validate_optional_strings(value, &["id"])?;
            validate_optional_one_of(
                value,
                "status",
                &["in_progress", "searching", "completed", "failed"],
                false,
            )?;
            validate_present_fields(value, &["action"], Value::is_object)
        }
        "computer_call" => {
            validate_optional_strings(value, &["call_id", "id"])?;
            validate_optional_one_of(value, "status", OUTPUT_ITEM_STATUS, false)?;
            validate_optional_arrays(value, &["actions", "pending_safety_checks"])?;
            validate_present_fields(value, &["action"], Value::is_object)
        }
        "computer_call_output" => {
            validate_optional_strings(value, &["call_id", "created_by"])?;
            validate_present_fields(value, &["id"], |value| {
                value.is_string() || (!enforce_output_requirements && value.is_null())
            })?;
            validate_optional_one_of(
                value,
                "status",
                &["completed", "incomplete", "failed"],
                !enforce_output_requirements,
            )?;
            validate_optional_arrays(value, &["acknowledged_safety_checks"])?;
            validate_present_fields(value, &["output"], Value::is_object)
        }
        "reasoning" => {
            validate_optional_strings(value, &["id"])?;
            validate_optional_string_or_null(value, &["encrypted_content"])?;
            validate_optional_one_of(value, "status", OUTPUT_ITEM_STATUS, false)?;
            validate_optional_arrays(value, &["content", "summary"])
        }
        "program" => validate_optional_strings(value, &["call_id", "code", "fingerprint", "id"]),
        "program_output" => {
            validate_optional_strings(value, &["call_id", "id", "result"])?;
            validate_optional_one_of(value, "status", &["completed", "incomplete"], false)
        }
        "tool_search_call" => {
            validate_optional_strings(value, &["created_by"])?;
            validate_present_fields(value, &["id"], |value| {
                value.is_string() || (!enforce_output_requirements && value.is_null())
            })?;
            validate_optional_string_or_null(value, &["call_id"])?;
            validate_optional_one_of(
                value,
                "execution",
                &["server", "client"],
                !enforce_output_requirements,
            )?;
            validate_optional_one_of(
                value,
                "status",
                OUTPUT_ITEM_STATUS,
                !enforce_output_requirements,
            )
        }
        "tool_search_output" => {
            validate_optional_strings(value, &["created_by"])?;
            validate_present_fields(value, &["id"], |value| {
                value.is_string() || (!enforce_output_requirements && value.is_null())
            })?;
            validate_optional_string_or_null(value, &["call_id"])?;
            validate_optional_one_of(
                value,
                "execution",
                &["server", "client"],
                !enforce_output_requirements,
            )?;
            validate_optional_one_of(
                value,
                "status",
                OUTPUT_ITEM_STATUS,
                !enforce_output_requirements,
            )?;
            validate_optional_arrays(value, &["tools"])
        }
        "additional_tools" => {
            validate_present_fields(value, &["id"], |value| {
                value.is_string() || (!enforce_output_requirements && value.is_null())
            })?;
            validate_optional_one_of(
                value,
                "role",
                &[
                    "unknown",
                    "user",
                    "assistant",
                    "system",
                    "critic",
                    "discriminator",
                    "developer",
                    "tool",
                ],
                false,
            )?;
            validate_optional_arrays(value, &["tools"])
        }
        "compaction" => {
            validate_optional_strings(value, &["created_by", "encrypted_content"])?;
            validate_present_fields(value, &["id"], |value| {
                value.is_string() || (!enforce_output_requirements && value.is_null())
            })
        }
        "image_generation_call" => {
            validate_optional_strings(value, &["id"])?;
            validate_optional_string_or_null(value, &["result"])?;
            validate_optional_one_of(
                value,
                "status",
                &["in_progress", "completed", "generating", "failed"],
                false,
            )
        }
        "code_interpreter_call" => {
            validate_optional_strings(value, &["container_id", "id"])?;
            validate_optional_string_or_null(value, &["code"])?;
            validate_optional_one_of(
                value,
                "status",
                &[
                    "in_progress",
                    "completed",
                    "incomplete",
                    "interpreting",
                    "failed",
                ],
                false,
            )?;
            validate_present_fields(value, &["outputs"], |value| {
                value.is_null() || value.is_array()
            })
        }
        "local_shell_call" => {
            validate_optional_strings(value, &["call_id", "id"])?;
            validate_optional_one_of(value, "status", OUTPUT_ITEM_STATUS, false)?;
            validate_present_fields(value, &["action"], Value::is_object)
        }
        "local_shell_call_output" => {
            validate_optional_strings(value, &["id", "output"])?;
            validate_optional_one_of(value, "status", OUTPUT_ITEM_STATUS, true)
        }
        "shell_call" => {
            validate_optional_strings(value, &["call_id", "created_by"])?;
            validate_present_fields(value, &["id"], |value| {
                value.is_string() || (!enforce_output_requirements && value.is_null())
            })?;
            validate_optional_one_of(
                value,
                "status",
                OUTPUT_ITEM_STATUS,
                !enforce_output_requirements,
            )?;
            validate_present_fields(value, &["action"], Value::is_object)?;
            validate_present_fields(value, &["environment"], |value| {
                value.is_null() || value.is_object()
            })
        }
        "shell_call_output" => {
            validate_optional_strings(value, &["call_id", "created_by"])?;
            validate_present_fields(value, &["id"], |value| {
                value.is_string() || (!enforce_output_requirements && value.is_null())
            })?;
            validate_optional_integer_or_null(value, &["max_output_length"])?;
            validate_optional_one_of(
                value,
                "status",
                OUTPUT_ITEM_STATUS,
                !enforce_output_requirements,
            )?;
            validate_optional_arrays(value, &["output"])
        }
        "apply_patch_call" => {
            validate_optional_strings(value, &["call_id", "created_by"])?;
            validate_present_fields(value, &["id"], |value| {
                value.is_string() || (!enforce_output_requirements && value.is_null())
            })?;
            validate_optional_one_of(value, "status", &["in_progress", "completed"], false)?;
            validate_present_fields(value, &["operation"], Value::is_object)
        }
        "apply_patch_call_output" => {
            validate_optional_strings(value, &["call_id", "created_by"])?;
            validate_present_fields(value, &["id"], |value| {
                value.is_string() || (!enforce_output_requirements && value.is_null())
            })?;
            validate_optional_string_or_null(value, &["output"])?;
            validate_optional_one_of(value, "status", &["completed", "failed"], false)
        }
        "mcp_call" => {
            validate_optional_strings(value, &["arguments", "id", "name", "server_label"])?;
            validate_optional_string_or_null(value, &["approval_request_id", "error", "output"])?;
            validate_optional_one_of(
                value,
                "status",
                &[
                    "in_progress",
                    "completed",
                    "incomplete",
                    "calling",
                    "failed",
                ],
                false,
            )
        }
        "mcp_list_tools" => {
            validate_optional_strings(value, &["id", "server_label"])?;
            validate_optional_string_or_null(value, &["error"])?;
            validate_optional_arrays(value, &["tools"])
        }
        "mcp_approval_request" => {
            validate_optional_strings(value, &["arguments", "id", "name", "server_label"])
        }
        "mcp_approval_response" => {
            validate_optional_strings(value, &["approval_request_id"])?;
            validate_present_fields(value, &["id"], |value| {
                value.is_string() || (!enforce_output_requirements && value.is_null())
            })?;
            validate_optional_booleans(value, &["approve"])?;
            validate_optional_string_or_null(value, &["reason"])
        }
        "custom_tool_call" => {
            validate_optional_strings(value, &["call_id", "id", "input", "name", "namespace"])
        }
        _ => Err(ResponsesPublicWireError::UnknownEventType),
    }
}

fn project_output_item_inner(
    value: &Value,
    enforce_output_requirements: bool,
) -> Result<Value, ResponsesPublicWireError> {
    let event_type = value
        .get("type")
        .and_then(Value::as_str)
        .ok_or(ResponsesPublicWireError::UnknownEventType)?;
    let (fields, required): (&[&str], &[&str]) = match event_type {
        "message" => (
            &["id", "content", "role", "status", "type", "phase"],
            &["id", "content", "role", "status", "type"],
        ),
        "file_search_call" => (
            &["id", "queries", "status", "type", "results"],
            &["id", "queries", "status", "type"],
        ),
        "function_call" => (
            &[
                "arguments",
                "call_id",
                "caller",
                "name",
                "type",
                "id",
                "namespace",
                "status",
            ],
            &["arguments", "call_id", "name", "type"],
        ),
        "function_call_output" => (
            &[
                "id",
                "call_id",
                "caller",
                "created_by",
                "name",
                "namespace",
                "output",
                "status",
                "type",
            ],
            &["id", "call_id", "output", "status", "type"],
        ),
        "web_search_call" => (
            &["id", "action", "status", "type"],
            &["id", "action", "status", "type"],
        ),
        "computer_call" => (
            &[
                "id",
                "call_id",
                "pending_safety_checks",
                "status",
                "type",
                "action",
                "actions",
            ],
            &["id", "call_id", "pending_safety_checks", "status", "type"],
        ),
        "computer_call_output" => (
            &[
                "id",
                "call_id",
                "output",
                "status",
                "type",
                "acknowledged_safety_checks",
                "created_by",
            ],
            &["id", "call_id", "output", "status", "type"],
        ),
        "reasoning" => (
            &[
                "id",
                "summary",
                "type",
                "content",
                "encrypted_content",
                "status",
            ],
            &["id", "summary", "type"],
        ),
        "program" => (
            &["id", "call_id", "code", "fingerprint", "type"],
            &["id", "call_id", "code", "fingerprint", "type"],
        ),
        "program_output" => (
            &["id", "call_id", "result", "status", "type"],
            &["id", "call_id", "result", "status", "type"],
        ),
        "tool_search_call" => (
            &[
                "id",
                "arguments",
                "call_id",
                "execution",
                "status",
                "type",
                "created_by",
            ],
            &["id", "arguments", "call_id", "execution", "status", "type"],
        ),
        "tool_search_output" => (
            &[
                "id",
                "call_id",
                "execution",
                "status",
                "tools",
                "type",
                "created_by",
            ],
            &["id", "call_id", "execution", "status", "tools", "type"],
        ),
        "additional_tools" => (
            &["id", "role", "tools", "type"],
            &["id", "role", "tools", "type"],
        ),
        "compaction" => (
            &["id", "encrypted_content", "type", "created_by"],
            &["id", "encrypted_content", "type"],
        ),
        "image_generation_call" => (
            &["id", "result", "status", "type"],
            &["id", "result", "status", "type"],
        ),
        "code_interpreter_call" => (
            &["id", "code", "container_id", "outputs", "status", "type"],
            &["id", "code", "container_id", "outputs", "status", "type"],
        ),
        "local_shell_call" => (
            &["id", "action", "call_id", "status", "type"],
            &["id", "action", "call_id", "status", "type"],
        ),
        "local_shell_call_output" => (
            &["id", "output", "type", "status"],
            &["id", "output", "type"],
        ),
        "shell_call" => (
            &[
                "id",
                "action",
                "call_id",
                "environment",
                "status",
                "type",
                "caller",
                "created_by",
            ],
            &["id", "action", "call_id", "environment", "status", "type"],
        ),
        "shell_call_output" => (
            &[
                "id",
                "call_id",
                "max_output_length",
                "output",
                "status",
                "type",
                "caller",
                "created_by",
            ],
            &[
                "id",
                "call_id",
                "max_output_length",
                "output",
                "status",
                "type",
            ],
        ),
        "apply_patch_call" => (
            &[
                "id",
                "call_id",
                "operation",
                "status",
                "type",
                "caller",
                "created_by",
            ],
            &["id", "call_id", "operation", "status", "type"],
        ),
        "apply_patch_call_output" => (
            &[
                "id",
                "call_id",
                "status",
                "type",
                "caller",
                "created_by",
                "output",
            ],
            &["id", "call_id", "status", "type"],
        ),
        "mcp_call" => (
            &[
                "id",
                "arguments",
                "name",
                "server_label",
                "type",
                "approval_request_id",
                "error",
                "output",
                "status",
            ],
            &["id", "arguments", "name", "server_label", "type"],
        ),
        "mcp_list_tools" => (
            &["id", "server_label", "tools", "type", "error"],
            &["id", "server_label", "tools", "type"],
        ),
        "mcp_approval_request" => (
            &["id", "arguments", "name", "server_label", "type"],
            &["id", "arguments", "name", "server_label", "type"],
        ),
        "mcp_approval_response" => (
            &["id", "approval_request_id", "approve", "type", "reason"],
            &["id", "approval_request_id", "approve", "type"],
        ),
        "custom_tool_call" => (
            &[
                "call_id",
                "caller",
                "input",
                "name",
                "type",
                "id",
                "namespace",
            ],
            &["call_id", "input", "name", "type"],
        ),
        "custom_tool_call_output" => (
            &[
                "id",
                "call_id",
                "caller",
                "created_by",
                "output",
                "type",
                "status",
            ],
            &["id", "call_id", "output", "type", "status"],
        ),
        _ => return Err(ResponsesPublicWireError::UnknownEventType),
    };
    if enforce_output_requirements {
        require_fields(value, required)?;
    }
    validate_output_item_basic_shape(value, event_type, enforce_output_requirements)?;
    let mut projected = project_object(value, fields)?;
    let object = projected
        .as_object_mut()
        .expect("project_object always returns an object");
    match event_type {
        "message" => project_field_array(object, "content", project_content_part)?,
        "file_search_call" => {
            project_nullable_field_array(object, "results", project_file_search_result)?
        }
        "function_call" | "custom_tool_call" => {
            project_field(object, "caller", project_output_item_caller)?
        }
        "function_call_output" | "custom_tool_call_output" => {
            project_field(object, "output", project_function_output)?;
            project_field(object, "caller", project_output_item_caller)?;
        }
        "web_search_call" => project_field(object, "action", project_web_search_action)?,
        "computer_call" => {
            project_field(object, "action", project_computer_action)?;
            project_field_array(object, "actions", project_computer_action)?;
            project_field_array(object, "pending_safety_checks", project_safety_check)?;
        }
        "computer_call_output" => {
            project_field_array(object, "acknowledged_safety_checks", project_safety_check)?;
            project_field(object, "output", project_computer_screenshot)?;
        }
        "reasoning" => {
            project_field_array(object, "summary", project_summary_text)?;
            project_field_array(object, "content", project_reasoning_text)?;
        }
        "tool_search_output" | "additional_tools" => {
            project_field_array(object, "tools", project_tool)?;
        }
        "code_interpreter_call" => {
            project_nullable_field_array(object, "outputs", project_code_interpreter_output)?;
        }
        "local_shell_call" => project_field(object, "action", project_local_shell_action)?,
        "shell_call" => {
            project_field(object, "action", project_shell_action)?;
            project_field(object, "environment", project_output_item_environment)?;
            project_field(object, "caller", project_output_item_caller)?;
        }
        "shell_call_output" => {
            project_field_array(object, "output", project_shell_output_content)?;
            project_field(object, "caller", project_output_item_caller)?;
        }
        "apply_patch_call" => {
            project_field(object, "operation", project_apply_patch_operation)?;
            project_field(object, "caller", project_output_item_caller)?;
        }
        "apply_patch_call_output" => project_field(object, "caller", project_output_item_caller)?,
        "mcp_list_tools" => project_field_array(object, "tools", project_mcp_list_tool)?,
        _ => {}
    }
    Ok(projected)
}

fn project_field(
    object: &mut serde_json::Map<String, Value>,
    field: &str,
    project: fn(&Value) -> Result<Value, ResponsesPublicWireError>,
) -> Result<(), ResponsesPublicWireError> {
    if let Some(value) = object.get_mut(field) {
        *value = project(value)?;
    }
    Ok(())
}

fn project_field_array(
    object: &mut serde_json::Map<String, Value>,
    field: &str,
    project: fn(&Value) -> Result<Value, ResponsesPublicWireError>,
) -> Result<(), ResponsesPublicWireError> {
    if let Some(value) = object.get_mut(field) {
        *value = project_array(value, project)?;
    }
    Ok(())
}

fn project_nullable_field_array(
    object: &mut serde_json::Map<String, Value>,
    field: &str,
    project: fn(&Value) -> Result<Value, ResponsesPublicWireError>,
) -> Result<(), ResponsesPublicWireError> {
    if let Some(value) = object.get_mut(field) {
        if !value.is_null() {
            *value = project_array(value, project)?;
        }
    }
    Ok(())
}

fn project_output_item_caller(value: &Value) -> Result<Value, ResponsesPublicWireError> {
    if value.is_null() {
        return Ok(Value::Null);
    }
    match value.get("type").and_then(Value::as_str) {
        Some("direct") => {
            require_fields(value, &["type"])?;
        }
        Some("program") => {
            validate_required_strings(value, &["caller_id", "type"])?;
        }
        _ => return Err(ResponsesPublicWireError::UnknownEventType),
    }
    project_caller(value)
}

fn project_output_item_environment(value: &Value) -> Result<Value, ResponsesPublicWireError> {
    if value.is_null() {
        return Ok(Value::Null);
    }
    match value.get("type").and_then(Value::as_str) {
        Some("local") => {
            require_fields(value, &["type"])?;
        }
        Some("container_reference") => {
            validate_required_strings(value, &["container_id", "type"])?;
        }
        _ => return Err(ResponsesPublicWireError::UnknownEventType),
    }
    project_nullable_environment(value)
}

fn project_file_search_result(value: &Value) -> Result<Value, ResponsesPublicWireError> {
    validate_optional_strings(value, &["file_id", "filename", "text"])?;
    validate_optional_numbers(value, &["score"])?;
    validate_optional_public_scalar_map(value, "attributes")?;
    project_object(
        value,
        &["attributes", "file_id", "filename", "score", "text"],
    )
}

fn project_function_output(value: &Value) -> Result<Value, ResponsesPublicWireError> {
    match value {
        Value::String(_) => Ok(value.clone()),
        Value::Array(_) => project_array(value, project_input_content_part),
        _ => Err(ResponsesPublicWireError::InvalidEventShape),
    }
}

fn project_web_search_action(value: &Value) -> Result<Value, ResponsesPublicWireError> {
    let mut projected = match value.get("type").and_then(Value::as_str) {
        Some("search") => {
            require_fields(value, &["type"])?;
            validate_optional_string_arrays(value, &["queries"])?;
            validate_optional_strings(value, &["query"])?;
            validate_optional_arrays(value, &["sources"])?;
            project_object(value, &["queries", "query", "sources", "type"])
        }
        Some("open_page") => {
            require_fields(value, &["type"])?;
            validate_optional_string_or_null(value, &["url"])?;
            project_object(value, &["type", "url"])
        }
        Some("find_in_page") => {
            validate_required_strings(value, &["pattern", "type", "url"])?;
            project_object(value, &["pattern", "type", "url"])
        }
        _ => Err(ResponsesPublicWireError::UnknownEventType),
    }?;
    if let Some(sources) = projected
        .as_object_mut()
        .and_then(|object| object.get_mut("sources"))
    {
        *sources = project_array(sources, |source| {
            validate_required_strings(source, &["type", "url"])?;
            if source.get("type").and_then(Value::as_str) != Some("url") {
                return Err(ResponsesPublicWireError::InvalidEventShape);
            }
            project_object(source, &["type", "url"])
        })?;
    }
    Ok(projected)
}

fn project_computer_action(value: &Value) -> Result<Value, ResponsesPublicWireError> {
    let (fields, required_strings, required_integers): (&[&str], &[&str], &[&str]) =
        match value.get("type").and_then(Value::as_str) {
            Some("click") => (
                &["button", "keys", "type", "x", "y"],
                &["button", "type"],
                &["x", "y"],
            ),
            Some("double_click") => (&["keys", "type", "x", "y"], &["type"], &["x", "y"]),
            Some("drag") => (&["keys", "path", "type"], &["type"], &[]),
            Some("keypress") => (&["keys", "type"], &["type"], &[]),
            Some("move") => (&["keys", "type", "x", "y"], &["type"], &["x", "y"]),
            Some("screenshot") => (&["type"], &["type"], &[]),
            Some("scroll") => (
                &["keys", "scroll_x", "scroll_y", "type", "x", "y"],
                &["type"],
                &["scroll_x", "scroll_y", "x", "y"],
            ),
            Some("type") => (&["text", "type"], &["text", "type"], &[]),
            Some("wait") => (&["type"], &["type"], &[]),
            _ => return Err(ResponsesPublicWireError::UnknownEventType),
        };
    let required: &[&str] = match value.get("type").and_then(Value::as_str) {
        Some("click") => &["button", "type", "x", "y"],
        Some("double_click") => &["keys", "type", "x", "y"],
        Some("drag") => &["path", "type"],
        Some("keypress") => &["keys", "type"],
        Some("move") => &["type", "x", "y"],
        Some("screenshot") | Some("wait") => &["type"],
        Some("scroll") => &["scroll_x", "scroll_y", "type", "x", "y"],
        Some("type") => &["text", "type"],
        _ => unreachable!("the action type was matched above"),
    };
    require_fields(value, required)?;
    validate_optional_strings(value, required_strings)?;
    validate_optional_integers(value, required_integers)?;
    validate_optional_nullable_string_arrays(value, &["keys"])?;
    if value.get("type").and_then(Value::as_str) == Some("click") {
        validate_optional_one_of(
            value,
            "button",
            &["left", "right", "wheel", "back", "forward"],
            false,
        )?;
    }
    if value.get("type").and_then(Value::as_str) == Some("drag") {
        validate_optional_arrays(value, &["path"])?;
    }
    let mut projected = project_object(value, fields)?;
    if let Some(path) = projected
        .as_object_mut()
        .and_then(|object| object.get_mut("path"))
    {
        *path = project_array(path, |point| {
            require_fields(point, &["x", "y"])?;
            validate_optional_integers(point, &["x", "y"])?;
            project_object(point, &["x", "y"])
        })?;
    }
    Ok(projected)
}

fn project_safety_check(value: &Value) -> Result<Value, ResponsesPublicWireError> {
    validate_required_strings(value, &["id"])?;
    validate_optional_string_or_null(value, &["code", "message"])?;
    project_object(value, &["code", "id", "message"])
}

fn project_computer_screenshot(value: &Value) -> Result<Value, ResponsesPublicWireError> {
    match value.get("type").and_then(Value::as_str) {
        Some("computer_screenshot") => {
            require_fields(value, &["type"])?;
            validate_optional_strings(value, &["file_id", "image_url"])?;
            project_object(value, &["file_id", "image_url", "type"])
        }
        _ => Err(ResponsesPublicWireError::UnknownEventType),
    }
}

fn project_summary_text(value: &Value) -> Result<Value, ResponsesPublicWireError> {
    match value.get("type").and_then(Value::as_str) {
        Some("summary_text") => {
            validate_required_strings(value, &["text", "type"])?;
            project_object(value, &["text", "type"])
        }
        _ => Err(ResponsesPublicWireError::UnknownEventType),
    }
}

fn project_reasoning_text(value: &Value) -> Result<Value, ResponsesPublicWireError> {
    match value.get("type").and_then(Value::as_str) {
        Some("reasoning_text") => {
            validate_required_strings(value, &["text", "type"])?;
            project_object(value, &["text", "type"])
        }
        _ => Err(ResponsesPublicWireError::UnknownEventType),
    }
}

fn project_code_interpreter_output(value: &Value) -> Result<Value, ResponsesPublicWireError> {
    match value.get("type").and_then(Value::as_str) {
        Some("logs") => {
            validate_required_strings(value, &["logs", "type"])?;
            project_object(value, &["logs", "type"])
        }
        Some("image") => {
            validate_required_strings(value, &["type", "url"])?;
            project_object(value, &["type", "url"])
        }
        _ => Err(ResponsesPublicWireError::UnknownEventType),
    }
}

fn project_local_shell_action(value: &Value) -> Result<Value, ResponsesPublicWireError> {
    match value.get("type").and_then(Value::as_str) {
        Some("exec") => {
            require_fields(value, &["command", "env", "type"])?;
            validate_optional_string_arrays(value, &["command"])?;
            validate_optional_string_map(value, "env")?;
            validate_optional_integer_or_null(value, &["timeout_ms"])?;
            validate_optional_string_or_null(value, &["user", "working_directory"])?;
            project_object(
                value,
                &[
                    "command",
                    "env",
                    "timeout_ms",
                    "type",
                    "user",
                    "working_directory",
                ],
            )
        }
        _ => Err(ResponsesPublicWireError::UnknownEventType),
    }
}

fn project_shell_action(value: &Value) -> Result<Value, ResponsesPublicWireError> {
    require_fields(value, &["commands", "max_output_length", "timeout_ms"])?;
    validate_optional_string_arrays(value, &["commands"])?;
    validate_optional_integer_or_null(value, &["max_output_length", "timeout_ms"])?;
    project_object(value, &["commands", "max_output_length", "timeout_ms"])
}

fn project_shell_output_content(value: &Value) -> Result<Value, ResponsesPublicWireError> {
    require_fields(value, &["outcome", "stderr", "stdout"])?;
    validate_optional_strings(value, &["created_by", "stderr", "stdout"])?;
    validate_present_fields(value, &["outcome"], Value::is_object)?;
    let mut projected = project_object(value, &["created_by", "outcome", "stderr", "stdout"])?;
    if let Some(outcome) = projected
        .as_object_mut()
        .and_then(|object| object.get_mut("outcome"))
    {
        *outcome = match outcome.get("type").and_then(Value::as_str) {
            Some("exit") => {
                require_fields(outcome, &["exit_code", "type"])?;
                validate_optional_integers(outcome, &["exit_code"])?;
                project_object(outcome, &["exit_code", "type"])
            }
            Some("timeout") => {
                require_fields(outcome, &["type"])?;
                project_object(outcome, &["type"])
            }
            _ => Err(ResponsesPublicWireError::UnknownEventType),
        }?;
    }
    Ok(projected)
}

fn project_apply_patch_operation(value: &Value) -> Result<Value, ResponsesPublicWireError> {
    match value.get("type").and_then(Value::as_str) {
        Some("create_file" | "update_file") => {
            validate_required_strings(value, &["diff", "path", "type"])?;
            project_object(value, &["diff", "path", "type"])
        }
        Some("delete_file") => {
            validate_required_strings(value, &["path", "type"])?;
            project_object(value, &["path", "type"])
        }
        _ => Err(ResponsesPublicWireError::UnknownEventType),
    }
}

fn project_mcp_list_tool(value: &Value) -> Result<Value, ResponsesPublicWireError> {
    validate_required_strings(value, &["name"])?;
    validate_required_objects(value, &["input_schema"])?;
    validate_optional_string_or_null(value, &["description"])?;
    validate_present_fields(value, &["annotations"], |value| {
        value.is_null() || value.is_object()
    })?;
    project_object(
        value,
        &["annotations", "description", "input_schema", "name"],
    )
}

fn project_content_part(value: &Value) -> Result<Value, ResponsesPublicWireError> {
    let fields: &[&str] = match value.get("type").and_then(Value::as_str) {
        Some("output_text") => {
            validate_required_strings(value, &["text"])?;
            validate_required_arrays(value, &["annotations", "logprobs"])?;
            &["annotations", "text", "type", "logprobs"]
        }
        Some("refusal") => {
            validate_required_strings(value, &["refusal"])?;
            &["refusal", "type"]
        }
        Some("reasoning_text" | "summary_text") => {
            validate_required_strings(value, &["text"])?;
            &["text", "type"]
        }
        _ => return Err(ResponsesPublicWireError::UnknownEventType),
    };
    let mut projected = project_object(value, fields)?;
    if let Some(annotations) = projected
        .as_object_mut()
        .and_then(|object| object.get_mut("annotations"))
    {
        *annotations = project_array(annotations, project_annotation)?;
    }
    if let Some(logprobs) = projected
        .as_object_mut()
        .and_then(|object| object.get_mut("logprobs"))
    {
        *logprobs = project_output_logprobs(logprobs)?;
    }
    Ok(projected)
}

fn project_annotation(value: &Value) -> Result<Value, ResponsesPublicWireError> {
    let fields: &[&str] = match value.get("type").and_then(Value::as_str) {
        Some("file_citation") => {
            validate_required_strings(value, &["file_id", "filename"])?;
            validate_required_indexes(value, &["index"])?;
            &["file_id", "filename", "index", "type"]
        }
        Some("url_citation") => {
            validate_required_strings(value, &["title", "url"])?;
            validate_required_indexes(value, &["end_index", "start_index"])?;
            &["end_index", "start_index", "title", "type", "url"]
        }
        Some("container_file_citation") => {
            validate_required_strings(value, &["container_id", "file_id", "filename"])?;
            validate_required_indexes(value, &["end_index", "start_index"])?;
            &[
                "container_id",
                "end_index",
                "file_id",
                "filename",
                "start_index",
                "type",
            ]
        }
        Some("file_path") => {
            validate_required_strings(value, &["file_id"])?;
            validate_required_indexes(value, &["index"])?;
            &["file_id", "index", "type"]
        }
        _ => return Err(ResponsesPublicWireError::UnknownEventType),
    };
    project_object(value, fields)
}

fn project_response_logprobs(value: &Value) -> Result<Value, ResponsesPublicWireError> {
    project_array(value, project_response_logprob)
}

fn project_response_logprob(value: &Value) -> Result<Value, ResponsesPublicWireError> {
    validate_required_strings(value, &["token"])?;
    validate_required_numbers(value, &["logprob"])?;
    validate_optional_arrays(value, &["top_logprobs"])?;
    let mut projected = project_object(value, &["logprob", "token", "top_logprobs"])?;
    if let Some(top_logprobs) = projected
        .as_object_mut()
        .and_then(|object| object.get_mut("top_logprobs"))
    {
        *top_logprobs = project_array(top_logprobs, project_response_top_logprob)?;
    }
    Ok(projected)
}

fn project_response_top_logprob(value: &Value) -> Result<Value, ResponsesPublicWireError> {
    validate_optional_strings(value, &["token"])?;
    validate_optional_numbers(value, &["logprob"])?;
    project_object(value, &["logprob", "token"])
}

fn project_output_logprobs(value: &Value) -> Result<Value, ResponsesPublicWireError> {
    project_array(value, project_output_logprob)
}

fn project_output_logprob(value: &Value) -> Result<Value, ResponsesPublicWireError> {
    validate_required_integer_arrays(value, &["bytes"])?;
    validate_required_numbers(value, &["logprob"])?;
    validate_required_strings(value, &["token"])?;
    validate_required_arrays(value, &["top_logprobs"])?;
    let mut projected = project_object(value, &["bytes", "logprob", "token", "top_logprobs"])?;
    if let Some(top_logprobs) = projected
        .as_object_mut()
        .and_then(|object| object.get_mut("top_logprobs"))
    {
        *top_logprobs = project_array(top_logprobs, project_output_top_logprob)?;
    }
    Ok(projected)
}

fn project_output_top_logprob(value: &Value) -> Result<Value, ResponsesPublicWireError> {
    validate_required_integer_arrays(value, &["bytes"])?;
    validate_required_numbers(value, &["logprob"])?;
    validate_required_strings(value, &["token"])?;
    project_object(value, &["bytes", "logprob", "token"])
}

fn sanitized_public_error(event: &Value) -> Value {
    // A complete reference top-level shape always wins. The guide's nested
    // envelope is accepted only when all three top-level fields are absent;
    // validation rejects mixed shapes before this projection is reached.
    let top_level_complete = ["code", "message", "param"]
        .iter()
        .all(|field| event.get(*field).is_some());
    let source = if top_level_complete {
        event
    } else {
        event
            .get("error")
            .expect("validated nested error envelope must be present")
    };
    let string_or_null = |field: &str| {
        source
            .get(field)
            .filter(|value| value.is_string() || value.is_null())
            .cloned()
            .unwrap_or(Value::Null)
    };
    let message = source
        .get("message")
        .and_then(Value::as_str)
        .filter(|value| !value.trim().is_empty())
        .unwrap_or("Provider returned an error");
    let sequence_number = event
        .get("sequence_number")
        .and_then(Value::as_u64)
        .unwrap_or_default();

    // Provider errors are ordinary Responses server events, whose reference
    // schema keeps these fields at the top level. Gateway-generated WebSocket
    // mode errors use the separate response-style envelope documented by the
    // WebSocket guide.
    json!({
        "type": "error",
        "code": string_or_null("code"),
        "message": message,
        "param": string_or_null("param"),
        "sequence_number": sequence_number,
    })
}

fn is_public_terminal_event(event: &Value) -> bool {
    matches!(
        event
            .get("type")
            .and_then(Value::as_str)
            .unwrap_or_default(),
        "error" | "response.completed" | "response.incomplete" | "response.failed"
    )
}

pub(super) fn responses_public_wire_codec() -> &'static dyn ResponsesPublicWireCodec {
    &OPENAI_RESPONSES_PUBLIC_WIRE_CODEC
}

pub(super) fn is_public_responses_event(event: &Value) -> bool {
    event
        .get("type")
        .and_then(Value::as_str)
        .is_some_and(|event_type| public_server_event_fields(event_type).is_some())
}

fn public_server_event_fields(event_type: &str) -> Option<&'static [&'static str]> {
    const RESPONSE: &[&str] = &["type", "response", "sequence_number"];
    const SIMPLE: &[&str] = &["type", "item_id", "output_index", "sequence_number"];
    const DELTA: &[&str] = &[
        "type",
        "delta",
        "item_id",
        "output_index",
        "sequence_number",
    ];
    const CONTENT_DELTA: &[&str] = &[
        "type",
        "content_index",
        "delta",
        "item_id",
        "output_index",
        "sequence_number",
    ];
    const ERROR: &[&str] = &["type", "code", "message", "param", "sequence_number"];

    match event_type {
        "error" => Some(ERROR),
        "response.created"
        | "response.in_progress"
        | "response.queued"
        | "response.completed"
        | "response.failed"
        | "response.incomplete" => Some(RESPONSE),
        "response.audio.delta" => Some(&["type", "delta", "sequence_number"]),
        "response.audio.done" | "response.audio.transcript.done" => {
            Some(&["type", "sequence_number"])
        }
        "response.audio.transcript.delta" => Some(&["type", "delta", "sequence_number"]),
        "response.code_interpreter_call.completed"
        | "response.code_interpreter_call.in_progress"
        | "response.code_interpreter_call.interpreting"
        | "response.file_search_call.completed"
        | "response.file_search_call.in_progress"
        | "response.file_search_call.searching"
        | "response.image_generation_call.completed"
        | "response.image_generation_call.generating"
        | "response.image_generation_call.in_progress"
        | "response.mcp_call.completed"
        | "response.mcp_call.failed"
        | "response.mcp_call.in_progress"
        | "response.mcp_list_tools.completed"
        | "response.mcp_list_tools.failed"
        | "response.mcp_list_tools.in_progress"
        | "response.web_search_call.completed"
        | "response.web_search_call.in_progress"
        | "response.web_search_call.searching" => Some(SIMPLE),
        "response.code_interpreter_call_code.delta"
        | "response.custom_tool_call_input.delta"
        | "response.function_call_arguments.delta"
        | "response.mcp_call_arguments.delta" => Some(DELTA),
        "response.code_interpreter_call_code.done" => {
            Some(&["type", "code", "item_id", "output_index", "sequence_number"])
        }
        "response.content_part.added" | "response.content_part.done" => Some(&[
            "type",
            "content_index",
            "item_id",
            "output_index",
            "part",
            "sequence_number",
        ]),
        "response.custom_tool_call_input.done" => Some(&[
            "type",
            "input",
            "item_id",
            "output_index",
            "sequence_number",
        ]),
        "response.function_call_arguments.done" => Some(&[
            "type",
            "arguments",
            "item_id",
            "name",
            "output_index",
            "sequence_number",
        ]),
        "response.image_generation_call.partial_image" => Some(&[
            "type",
            "item_id",
            "output_index",
            "partial_image_b64",
            "partial_image_index",
            "sequence_number",
        ]),
        "response.mcp_call_arguments.done" => Some(&[
            "type",
            "arguments",
            "item_id",
            "output_index",
            "sequence_number",
        ]),
        "response.output_item.added" | "response.output_item.done" => {
            Some(&["type", "item", "output_index", "sequence_number"])
        }
        "response.output_text.annotation.added" => Some(&[
            "type",
            "annotation",
            "annotation_index",
            "content_index",
            "item_id",
            "output_index",
            "sequence_number",
        ]),
        "response.output_text.delta" => Some(&[
            "type",
            "content_index",
            "delta",
            "item_id",
            "logprobs",
            "output_index",
            "sequence_number",
        ]),
        "response.output_text.done" => Some(&[
            "type",
            "content_index",
            "item_id",
            "logprobs",
            "output_index",
            "sequence_number",
            "text",
        ]),
        "response.reasoning_summary_part.added" => Some(&[
            "type",
            "item_id",
            "output_index",
            "part",
            "sequence_number",
            "summary_index",
        ]),
        "response.reasoning_summary_part.done" => Some(&[
            "type",
            "item_id",
            "output_index",
            "part",
            "sequence_number",
            "status",
            "summary_index",
        ]),
        "response.reasoning_summary_text.delta" => Some(&[
            "type",
            "delta",
            "item_id",
            "output_index",
            "sequence_number",
            "summary_index",
        ]),
        "response.reasoning_summary_text.done" => Some(&[
            "type",
            "item_id",
            "output_index",
            "sequence_number",
            "summary_index",
            "text",
        ]),
        "response.reasoning_text.delta" | "response.refusal.delta" => Some(CONTENT_DELTA),
        "response.reasoning_text.done" => Some(&[
            "type",
            "content_index",
            "item_id",
            "output_index",
            "sequence_number",
            "text",
        ]),
        "response.refusal.done" => Some(&[
            "type",
            "content_index",
            "item_id",
            "output_index",
            "refusal",
            "sequence_number",
        ]),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use serde_json::{json, Value};

    use super::super::backend::{
        resolve_native_responses_websocket_backend, NativeResponsesWebSocketBackend,
    };
    use super::{
        resolve_responses_provider_observer, responses_public_wire_codec,
        ResponsesProviderObserver, ResponsesPublicEventState, ResponsesPublicWireError,
    };
    use crate::orchestration::{ResponsesProviderObserverKind, ResponsesWebSocketBackendKind};

    fn response_snapshot(id: &str, overrides: Value) -> Value {
        let mut response = json!({
            "id": id,
            "created_at": 1.0,
            "error": null,
            "incomplete_details": null,
            "instructions": null,
            "metadata": {},
            "model": "gpt-test",
            "object": "response",
            "output": [],
            "parallel_tool_calls": true,
            "temperature": null,
            "tool_choice": "auto",
            "tools": [],
            "top_p": null
        });
        response.as_object_mut().expect("response snapshot").extend(
            overrides
                .as_object()
                .expect("response snapshot overrides")
                .clone(),
        );
        response
    }

    fn response_event(event_type: &str, id: &str, overrides: Value) -> Value {
        json!({
            "type": event_type,
            "response": response_snapshot(id, overrides)
        })
    }

    fn output_text_delta(delta: &str) -> Value {
        json!({
            "type": "response.output_text.delta",
            "delta": delta,
            "item_id": "msg_1",
            "content_index": 0,
            "output_index": 0,
            "logprobs": []
        })
    }

    #[test]
    fn backend_and_provider_observer_are_independent_axes() {
        let backend = resolve_native_responses_websocket_backend(
            ResponsesWebSocketBackendKind::NativeResponsesWebSocket,
        );
        let observer = resolve_responses_provider_observer(ResponsesProviderObserverKind::Codex);

        assert_eq!(
            backend.kind(),
            ResponsesWebSocketBackendKind::NativeResponsesWebSocket
        );
        assert_eq!(observer.kind(), ResponsesProviderObserverKind::Codex);
        assert_eq!(
            backend.upstream_errors().handshake_failed,
            "responses_websocket_handshake_failed"
        );
    }

    #[test]
    fn public_codec_filters_private_codex_events_and_unwraps_batches() {
        let event = json!({
            "type": "codex.response.metadata",
            "chunks": [
                {"type": "codex.rate_limits", "rate_limits": {"allowed": true}},
                response_event("response.created", "resp_1", json!({})),
                output_text_delta("hi"),
                response_event("response.completed", "resp_1", json!({}))
            ]
        });

        let public = responses_public_wire_codec()
            .public_events(&event)
            .expect("valid public batch");
        assert_eq!(public.len(), 3);
        assert_eq!(public[0]["type"], "response.created");
        assert_eq!(public[1]["type"], "response.output_text.delta");
        assert_eq!(public[2]["type"], "response.completed");
    }

    #[test]
    fn public_codec_never_exposes_a_batch_envelope_on_a_public_top_level_event() {
        let event = json!({
            "type": "response.created",
            "response": response_snapshot("resp_top", json!({})),
            "headers": {"x-provider-account": "private"},
            "provider_debug": {"account_id": "acct_private"},
            "chunks": [
                {"type": "codex.rate_limits", "rate_limits": {"allowed": true}},
                output_text_delta("hi")
            ]
        });

        let public = responses_public_wire_codec()
            .public_events(&event)
            .expect("valid public event with provider batch");

        assert_eq!(public.len(), 2);
        assert_eq!(public[0]["type"], "response.created");
        assert!(public[0].get("chunks").is_none());
        assert!(public[0].get("headers").is_none());
        assert!(public[0].get("provider_debug").is_none());
        assert_eq!(public[1]["type"], "response.output_text.delta");
    }

    #[test]
    fn public_codec_rejects_unknown_response_prefixed_events() {
        let event = json!({
            "type": "response.codex_internal",
            "account_id": "acct_private"
        });

        assert_eq!(
            responses_public_wire_codec().public_events(&event),
            Err(super::ResponsesPublicWireError::UnknownEventType)
        );
    }

    #[test]
    fn public_codec_projects_nested_response_items_and_annotations() {
        let event = json!({
            "type": "response.completed",
            "sequence_number": 4,
            "response": response_snapshot("resp_1", json!({
                "status": "completed",
                "provider_debug": {"account_id": "acct_private"},
                "metadata": {
                    "public": "value",
                    "provider_object": {"account_id": "acct_private"}
                },
                "output": [{
                    "type": "message",
                    "id": "msg_1",
                    "role": "assistant",
                    "status": "completed",
                    "account_id": "acct_private",
                    "content": [{
                        "type": "output_text",
                        "text": "hello",
                        "logprobs": [],
                        "provider_debug": true,
                        "annotations": [{
                            "type": "url_citation",
                            "start_index": 0,
                            "end_index": 5,
                            "title": "source",
                            "url": "https://example.test",
                            "account_id": "acct_private"
                        }]
                    }]
                }]
            }))
        });

        let public = responses_public_wire_codec()
            .public_events(&event)
            .expect("valid completed event");
        let response = &public[0]["response"];

        assert!(response.get("provider_debug").is_none());
        assert_eq!(response["metadata"], json!({"public": "value"}));
        assert!(response["output"][0].get("account_id").is_none());
        assert!(response["output"][0]["content"][0]
            .get("provider_debug")
            .is_none());
        assert!(response["output"][0]["content"][0]["annotations"][0]
            .get("account_id")
            .is_none());
    }

    #[test]
    fn public_codec_rejects_multi_agent_beta_events_and_items() {
        let event_with_agent = json!({
            "type": "response.output_text.delta",
            "item_id": "msg_123",
            "output_index": 0,
            "content_index": 0,
            "delta": "hello",
            "agent": null,
        });
        assert_eq!(
            responses_public_wire_codec().public_events(&event_with_agent),
            Err(ResponsesPublicWireError::InvalidEventShape)
        );

        let item_with_agent = json!({
            "type": "response.output_item.done",
            "output_index": 0,
            "item": {
                "type": "message",
                "id": "msg_123",
                "role": "assistant",
                "status": "completed",
                "content": [],
                "agent": null,
            }
        });
        assert_eq!(
            responses_public_wire_codec().public_events(&item_with_agent),
            Err(ResponsesPublicWireError::InvalidEventShape)
        );

        for item_type in [
            "agent_message",
            "multi_agent_call",
            "multi_agent_call_output",
        ] {
            let snapshot = response_event(
                "response.completed",
                "resp_multi",
                json!({"output": [{"type": item_type}]}),
            );
            assert_eq!(
                responses_public_wire_codec().public_events(&snapshot),
                Err(ResponsesPublicWireError::UnknownEventType),
                "beta item must not enter the stable public protocol: {item_type}"
            );
        }
    }

    #[test]
    fn public_codec_projects_response_error_usage_and_current_response_fields() {
        let event = json!({
            "type": "response.failed",
            "response": response_snapshot("resp_failed", json!({
                "error": {
                    "code": "server_error",
                    "message": "failed",
                    "details": {"provider_account": "private"}
                },
                "usage": {
                    "input_tokens": 2,
                    "input_tokens_details": {
                        "cache_write_tokens": 1,
                        "cached_tokens": 1,
                        "provider_cached_tokens": 9
                    },
                    "output_tokens": 1,
                    "output_tokens_details": {
                        "reasoning_tokens": 1,
                        "provider_reasoning_tokens": 9
                    },
                    "total_tokens": 3,
                    "provider_cost": 42
                },
                "reasoning": {
                    "context": "all_turns",
                    "effort": "high",
                    "mode": "pro",
                    "provider_trace": "private"
                },
                "prompt_cache_options": {
                    "mode": "explicit",
                    "ttl": "30m",
                    "provider_cache_id": "private"
                },
                "store": false,
                "user": null,
                "output_text": "SDK-only convenience value"
            }))
        });

        let public = responses_public_wire_codec()
            .public_events(&event)
            .expect("valid failed response");
        let response = &public[0]["response"];

        assert_eq!(
            response["error"],
            json!({"code": "server_error", "message": "failed"})
        );
        assert_eq!(
            response["usage"],
            json!({
                "input_tokens": 2,
                "input_tokens_details": {"cache_write_tokens": 1, "cached_tokens": 1},
                "output_tokens": 1,
                "output_tokens_details": {"reasoning_tokens": 1},
                "total_tokens": 3
            })
        );
        assert_eq!(
            response["reasoning"],
            json!({"context": "all_turns", "effort": "high", "mode": "pro"})
        );
        assert_eq!(
            response["prompt_cache_options"],
            json!({"mode": "explicit", "ttl": "30m"})
        );
        assert_eq!(response["store"], false);
        assert!(response["user"].is_null());
        assert!(response.get("output_text").is_none());
    }

    #[test]
    fn public_codec_accepts_type_optional_instructions_and_compaction_triggers() {
        let event = json!({
            "type": "response.completed",
            "response": response_snapshot("resp_instructions", json!({
                "instructions": [
                    {
                        "role": "developer",
                        "content": [{
                            "type": "input_text",
                            "text": "Use the repository rules",
                            "prompt_cache_breakpoint": {
                                "mode": "explicit",
                                "provider_debug": true
                            },
                            "provider_debug": true
                        }],
                        "provider_debug": true
                    },
                    {
                        "type": "compaction_trigger",
                        "provider_debug": true
                    }
                ]
            }))
        });

        let public = responses_public_wire_codec()
            .public_events(&event)
            .expect("valid current response instructions");

        assert_eq!(
            public[0]["response"]["instructions"],
            json!([
                {
                    "role": "developer",
                    "content": [{
                        "type": "input_text",
                        "text": "Use the repository rules",
                        "prompt_cache_breakpoint": {"mode": "explicit"}
                    }]
                },
                {"type": "compaction_trigger"}
            ])
        );
    }

    #[test]
    fn public_codec_rejects_agent_attributed_response_instructions() {
        for event_type in ["response.created", "response.completed"] {
            let event = response_event(
                event_type,
                "resp_multi_agent_instructions",
                json!({
                    "instructions": [{
                        "type": "message",
                        "role": "developer",
                        "content": "private beta instruction",
                        "agent": {"agent_name": "/root"}
                    }]
                }),
            );

            assert_eq!(
                responses_public_wire_codec().public_events(&event),
                Err(ResponsesPublicWireError::InvalidEventShape),
                "agent-attributed instructions must not enter the stable protocol: {event_type}"
            );
        }
    }

    #[test]
    fn public_codec_accepts_local_shell_output_without_nonstandard_call_id() {
        let event = json!({
            "type": "response.output_item.done",
            "output_index": 0,
            "item": {
                "type": "local_shell_call_output",
                "id": "shell_output_1",
                "call_id": "shell_call_1",
                "output": "{}",
                "status": "completed",
                "provider_debug": true
            }
        });
        let public = responses_public_wire_codec()
            .public_events(&event)
            .expect("valid local shell output");
        assert!(public[0]["item"].get("call_id").is_none());
        assert!(public[0]["item"].get("provider_debug").is_none());
        assert_eq!(public[0]["item"]["id"], "shell_output_1");
        assert_eq!(public[0]["item"]["output"], "{}");
        assert_eq!(public[0]["item"]["status"], "completed");
    }

    #[test]
    fn public_codec_recursively_projects_mcp_approval_filters() {
        let event = json!({
            "type": "response.completed",
            "response": response_snapshot("resp_mcp", json!({
                "tools": [{
                    "type": "mcp",
                    "server_label": "docs",
                    "allowed_tools": {
                        "read_only": true,
                        "tool_names": ["search"],
                        "provider_account": "private"
                    },
                    "require_approval": {
                        "always": {
                            "read_only": false,
                            "tool_names": ["write"],
                            "codex_account_id": "acct-private"
                        },
                        "never": {
                            "tool_names": ["search"],
                            "provider_debug": true
                        },
                        "provider_policy": "private"
                    }
                }]
            }))
        });

        let public = responses_public_wire_codec()
            .public_events(&event)
            .expect("valid MCP approval filters");
        let tool = &public[0]["response"]["tools"][0];

        assert_eq!(
            tool["allowed_tools"],
            json!({"read_only": true, "tool_names": ["search"]})
        );
        assert_eq!(
            tool["require_approval"],
            json!({
                "always": {"read_only": false, "tool_names": ["write"]},
                "never": {"tool_names": ["search"]}
            })
        );
    }

    #[test]
    fn public_codec_removes_request_credentials_from_mcp_response_tools() {
        let event = json!({
            "type": "response.completed",
            "response": response_snapshot("resp_mcp_credentials", json!({
                "tools": [{
                    "type": "mcp",
                    "server_label": "docs",
                    "server_description": "Search the documentation",
                    "server_url": "https://mcp.example.test",
                    "authorization": "Bearer mcp-secret",
                    "headers": {
                        "authorization": "Bearer header-secret",
                        "x-api-key": "header-api-key"
                    },
                    "require_approval": "never"
                }]
            }))
        });

        let public = responses_public_wire_codec()
            .public_events(&event)
            .expect("valid MCP response tool");
        let tool = &public[0]["response"]["tools"][0];

        assert_eq!(
            tool,
            &json!({
                "type": "mcp",
                "server_label": "docs",
                "server_description": "Search the documentation",
                "server_url": "https://mcp.example.test",
                "require_approval": "never"
            })
        );
        let public_text = serde_json::to_string(&public).expect("serialize public events");
        for secret in ["mcp-secret", "header-secret", "header-api-key"] {
            assert!(!public_text.contains(secret));
        }
    }

    #[test]
    fn public_codec_recursively_removes_credentials_from_namespace_tools() {
        let event = json!({
            "type": "response.completed",
            "response": response_snapshot("resp_namespace_credentials", json!({
                "tools": [{
                    "type": "namespace",
                    "name": "crm",
                    "description": "CRM tools",
                    "tools": [
                        {
                            "type": "function",
                            "name": "lookup",
                            "description": "Look up a customer",
                            "parameters": {"type": "object"},
                            "authorization": "Bearer nested-function-secret",
                            "headers": {"x-api-key": "nested-function-header"},
                            "container": {
                                "network_policy": {
                                    "domain_secrets": [{"value": "nested-function-domain-secret"}]
                                }
                            }
                        },
                        {
                            "type": "custom",
                            "name": "render",
                            "description": "Render a customer record",
                            "format": {"type": "text"},
                            "authorization": "Bearer nested-custom-secret",
                            "headers": {"x-api-key": "nested-custom-header"}
                        }
                    ]
                }]
            }))
        });

        let public = responses_public_wire_codec()
            .public_events(&event)
            .expect("valid namespace response tool");
        let namespace = &public[0]["response"]["tools"][0];

        assert_eq!(
            namespace,
            &json!({
                "type": "namespace",
                "name": "crm",
                "description": "CRM tools",
                "tools": [
                    {
                        "type": "function",
                        "name": "lookup",
                        "description": "Look up a customer",
                        "parameters": {"type": "object"}
                    },
                    {
                        "type": "custom",
                        "name": "render",
                        "description": "Render a customer record",
                        "format": {"type": "text"}
                    }
                ]
            })
        );
        let public_text = serde_json::to_string(&public).expect("serialize public events");
        for secret in [
            "nested-function-secret",
            "nested-function-header",
            "nested-function-domain-secret",
            "nested-custom-secret",
            "nested-custom-header",
        ] {
            assert!(!public_text.contains(secret));
        }
    }

    #[test]
    fn public_codec_rejects_malformed_mcp_approval_filters() {
        for require_approval in [
            json!("sometimes"),
            json!({"always": "not-a-filter"}),
            json!({"never": {"tool_names": "not-an-array"}}),
            json!({"always": {"read_only": "not-a-boolean"}}),
        ] {
            let event = json!({
                "type": "response.completed",
                "response": response_snapshot("resp_mcp_invalid", json!({
                    "tools": [{
                        "type": "mcp",
                        "server_label": "docs",
                        "require_approval": require_approval
                    }]
                }))
            });

            assert_eq!(
                responses_public_wire_codec().public_events(&event),
                Err(super::ResponsesPublicWireError::InvalidEventShape)
            );
        }
    }

    #[test]
    fn public_codec_recursively_projects_code_interpreter_network_policy() {
        let event = json!({
            "type": "response.completed",
            "response": response_snapshot("resp_network_policy", json!({
                "tools": [
                    {
                        "type": "code_interpreter",
                        "container": {
                            "type": "auto",
                            "file_ids": ["file_1"],
                            "memory_limit": "4g",
                            "network_policy": {
                                "type": "allowlist",
                                "allowed_domains": ["example.com"],
                                "domain_secrets": [{
                                    "domain": "example.com",
                                    "name": "API_KEY",
                                    "value": "secret-value",
                                    "provider_debug": true
                                }],
                                "provider_policy": "private"
                            },
                            "provider_debug": true
                        }
                    },
                    {
                        "type": "code_interpreter",
                        "container": {
                            "type": "auto",
                            "network_policy": {
                                "type": "disabled",
                                "provider_debug": true
                            }
                        }
                    }
                ]
            }))
        });

        let public = responses_public_wire_codec()
            .public_events(&event)
            .expect("valid code interpreter network policy");
        let tools = public[0]["response"]["tools"]
            .as_array()
            .expect("projected tools");

        assert_eq!(
            tools[0]["container"],
            json!({
                "type": "auto",
                "file_ids": ["file_1"],
                "memory_limit": "4g",
                "network_policy": {
                    "type": "allowlist",
                    "allowed_domains": ["example.com"],
                    "domain_secrets": [{
                        "domain": "example.com",
                        "name": "API_KEY"
                    }]
                }
            })
        );
        assert!(!serde_json::to_string(&public)
            .expect("serialize public events")
            .contains("secret-value"));
        assert_eq!(
            tools[1]["container"]["network_policy"],
            json!({"type": "disabled"})
        );
    }

    #[test]
    fn public_codec_recursively_projects_file_search_filter_and_ranking_options() {
        let event = json!({
            "type": "response.completed",
            "response": response_snapshot("resp_file_search", json!({
                "tools": [{
                    "type": "file_search",
                    "vector_store_ids": ["vs_1"],
                    "filters": {
                        "type": "and",
                        "filters": [
                            {
                                "type": "eq",
                                "key": "region",
                                "value": "apac",
                                "provider_debug": true
                            },
                            {
                                "type": "or",
                                "filters": [{
                                    "type": "in",
                                    "key": "year",
                                    "value": [2025, 2026],
                                    "provider_debug": true
                                }],
                                "provider_debug": true
                            }
                        ],
                        "provider_debug": true
                    },
                    "ranking_options": {
                        "ranker": "auto",
                        "score_threshold": 0.5,
                        "hybrid_search": {
                            "embedding_weight": 0.7,
                            "text_weight": 0.3,
                            "provider_debug": true
                        },
                        "provider_debug": true
                    }
                }]
            }))
        });

        let public = responses_public_wire_codec()
            .public_events(&event)
            .expect("valid recursive file search schema");
        let tool = &public[0]["response"]["tools"][0];

        assert_eq!(
            tool["filters"],
            json!({
                "type": "and",
                "filters": [
                    {"type": "eq", "key": "region", "value": "apac"},
                    {
                        "type": "or",
                        "filters": [{"type": "in", "key": "year", "value": [2025, 2026]}]
                    }
                ]
            })
        );
        assert_eq!(
            tool["ranking_options"],
            json!({
                "ranker": "auto",
                "score_threshold": 0.5,
                "hybrid_search": {"embedding_weight": 0.7, "text_weight": 0.3}
            })
        );
    }

    #[test]
    fn public_codec_fails_closed_for_unknown_filters_and_invalid_breakpoints() {
        let unknown_filter = json!({
            "type": "response.completed",
            "response": response_snapshot("resp_unknown_filter", json!({
                "tools": [{
                    "type": "file_search",
                    "filters": {"type": "provider_filter", "secret": true}
                }]
            }))
        });
        let invalid_breakpoint = json!({
            "type": "response.completed",
            "response": response_snapshot("resp_invalid_breakpoint", json!({
                "instructions": [{
                    "role": "developer",
                    "content": [{
                        "type": "input_text",
                        "text": "hello",
                        "prompt_cache_breakpoint": {"mode": "provider_mode"}
                    }]
                }]
            }))
        });

        assert_eq!(
            responses_public_wire_codec().public_events(&unknown_filter),
            Err(super::ResponsesPublicWireError::UnknownEventType)
        );
        assert_eq!(
            responses_public_wire_codec().public_events(&invalid_breakpoint),
            Err(super::ResponsesPublicWireError::InvalidEventShape)
        );
    }

    #[test]
    fn public_codec_supports_current_program_and_caller_output_items() {
        let event = json!({
            "chunks": [
                {
                    "type": "response.output_item.added",
                    "output_index": 0,
                    "item": {
                        "type": "program",
                        "id": "prog_1",
                        "call_id": "call_1",
                        "code": "print('ok')",
                        "fingerprint": "fp_1",
                        "provider_debug": true
                    }
                },
                {
                    "type": "response.output_item.done",
                    "output_index": 1,
                    "item": {
                        "type": "function_call",
                        "arguments": "{}",
                        "call_id": "call_2",
                        "caller": {
                            "type": "program",
                            "caller_id": "call_1",
                            "provider_debug": true
                        },
                        "name": "lookup",
                        "provider_debug": true
                    }
                }
            ]
        });

        let public = responses_public_wire_codec()
            .public_events(&event)
            .expect("current public output items");

        assert_eq!(
            public[0]["item"],
            json!({
                "type": "program",
                "id": "prog_1",
                "call_id": "call_1",
                "code": "print('ok')",
                "fingerprint": "fp_1"
            })
        );
        assert_eq!(
            public[1]["item"]["caller"],
            json!({"type": "program", "caller_id": "call_1"})
        );
        assert!(public[1]["item"].get("provider_debug").is_none());
    }

    #[test]
    fn public_codec_rejects_objects_hidden_under_output_item_scalar_fields() {
        for item in [
            json!({
                "type": "program",
                "id": {"account_id": "acct_private"},
                "call_id": "call_1",
                "code": "print('ok')",
                "fingerprint": "fp_1"
            }),
            json!({
                "type": "function_call",
                "arguments": "{}",
                "call_id": "call_2",
                "name": {"provider_debug": true}
            }),
            json!({
                "type": "computer_call",
                "id": "computer_1",
                "call_id": "call_3",
                "pending_safety_checks": [{
                    "id": {"account_id": "acct_private"}
                }],
                "status": "completed"
            }),
        ] {
            let event = json!({
                "type": "response.output_item.added",
                "output_index": 0,
                "item": item
            });
            assert_eq!(
                responses_public_wire_codec().public_events(&event),
                Err(super::ResponsesPublicWireError::InvalidEventShape)
            );
        }
    }

    #[test]
    fn public_codec_projects_tools_actions_operations_and_logprobs() {
        let event = json!({
            "type": "response.completed",
            "response": response_snapshot("resp_nested", json!({
                "metadata": null,
                "tools": [{
                    "type": "function",
                    "name": "lookup",
                    "parameters": {"type": "object", "x-openai-valid-schema": true},
                    "strict": true,
                    "provider_debug": true
                }],
                "output": [
                    {
                        "type": "web_search_call",
                        "id": "ws_1",
                        "status": "completed",
                        "action": {
                            "type": "search",
                            "query": "test",
                            "sources": [{
                                "type": "url",
                                "url": "https://example.test",
                                "provider_debug": true
                            }],
                            "provider_debug": true
                        }
                    },
                    {
                        "type": "apply_patch_call",
                        "id": "patch_1",
                        "call_id": "call_patch",
                        "operation": {
                            "type": "create_file",
                            "path": "a.txt",
                            "diff": "+hello",
                            "provider_debug": true
                        },
                        "status": "completed"
                    },
                    {
                        "type": "message",
                        "id": "msg_1",
                        "role": "assistant",
                        "status": "completed",
                        "content": [{
                            "type": "output_text",
                            "text": "x",
                            "annotations": [],
                            "logprobs": [{
                                "bytes": [120],
                                "logprob": -0.1,
                                "token": "x",
                                "top_logprobs": [{
                                    "bytes": [120],
                                    "logprob": -0.1,
                                    "token": "x",
                                    "provider_debug": true
                                }],
                                "provider_debug": true
                            }]
                        }]
                    }
                ]
            }))
        });

        let public = responses_public_wire_codec()
            .public_events(&event)
            .expect("nested public schema");
        let response = &public[0]["response"];

        assert!(response["metadata"].is_null());
        assert!(response["tools"][0].get("provider_debug").is_none());
        assert_eq!(
            response["tools"][0]["parameters"]["x-openai-valid-schema"],
            true
        );
        assert!(response["output"][0]["action"]
            .get("provider_debug")
            .is_none());
        assert!(response["output"][0]["action"]["sources"][0]
            .get("provider_debug")
            .is_none());
        assert!(response["output"][1]["operation"]
            .get("provider_debug")
            .is_none());
        assert!(response["output"][2]["content"][0]["logprobs"][0]
            .get("provider_debug")
            .is_none());
        assert!(
            response["output"][2]["content"][0]["logprobs"][0]["top_logprobs"][0]
                .get("provider_debug")
                .is_none()
        );
    }

    #[test]
    fn public_codec_accepts_optional_stream_top_logprob_fields() {
        let event = json!({
            "type": "response.output_text.delta",
            "item_id": "msg_1",
            "output_index": 0,
            "content_index": 0,
            "delta": "x",
            "logprobs": [{
                "token": "x",
                "logprob": -0.1,
                "top_logprobs": [{"provider_debug": "private"}]
            }]
        });

        let public = responses_public_wire_codec()
            .public_events(&event)
            .expect("stream top_logprobs fields are optional");

        assert_eq!(public[0]["logprobs"][0]["top_logprobs"][0], json!({}));
    }

    #[test]
    fn public_codec_accepts_nullable_json_schema_strict() {
        let event = response_event(
            "response.completed",
            "resp_json_schema",
            json!({
                "text": {
                    "format": {
                        "type": "json_schema",
                        "name": "result",
                        "schema": {"type": "object"},
                        "strict": null
                    }
                }
            }),
        );

        let public = responses_public_wire_codec()
            .public_events(&event)
            .expect("json_schema strict may be null");

        assert!(public[0]["response"]["text"]["format"]["strict"].is_null());
    }

    #[test]
    fn public_codec_rejects_unknown_or_malformed_nested_output_items() {
        let unknown = json!({
            "type": "response.output_item.added",
            "output_index": 0,
            "item": {"type": "future_provider_item", "secret": "private"}
        });
        let malformed = json!({
            "type": "response.completed",
            "response": {"id": "resp_malformed", "output": "not-an-array"}
        });
        let missing_required = json!({
            "type": "response.output_item.added",
            "output_index": 0,
            "item": {"type": "additional_tools", "id": "tools_1", "tools": []}
        });

        assert_eq!(
            responses_public_wire_codec().public_events(&unknown),
            Err(super::ResponsesPublicWireError::UnknownEventType)
        );
        assert_eq!(
            responses_public_wire_codec().public_events(&malformed),
            Err(super::ResponsesPublicWireError::InvalidEventShape)
        );
        assert_eq!(
            responses_public_wire_codec().public_events(&missing_required),
            Err(super::ResponsesPublicWireError::InvalidEventShape)
        );
    }

    #[test]
    fn public_codec_preserves_reasoning_summary_done_status() {
        let event = json!({
            "type": "response.reasoning_summary_part.done",
            "item_id": "rs_1",
            "output_index": 0,
            "part": {"type": "summary_text", "text": "summary"},
            "sequence_number": 4,
            "status": "incomplete",
            "summary_index": 0
        });

        let public = responses_public_wire_codec()
            .public_events(&event)
            .expect("valid reasoning summary done event");

        assert_eq!(public[0]["status"], "incomplete");
        assert_eq!(
            public[0]["part"],
            json!({"type": "summary_text", "text": "summary"})
        );
    }

    #[test]
    fn public_codec_emits_nothing_for_provider_only_quota_frames() {
        let event = json!({
            "type": "codex.rate_limits",
            "rate_limits": {"allowed": true}
        });

        assert!(responses_public_wire_codec()
            .public_events(&event)
            .expect("provider-only frame")
            .is_empty());
    }

    #[test]
    fn public_codec_rejects_untyped_provider_documents_but_accepts_typed_chunks_envelopes() {
        assert_eq!(
            responses_public_wire_codec().public_events(&json!({"unexpected": true})),
            Err(ResponsesPublicWireError::InvalidEventShape)
        );
        assert_eq!(
            responses_public_wire_codec().public_events(&json!({
                "chunks": [{"unexpected": true}]
            })),
            Err(ResponsesPublicWireError::InvalidEventShape)
        );
        assert_eq!(
            responses_public_wire_codec().public_events(&json!({"chunks": []})),
            Err(ResponsesPublicWireError::InvalidEventShape)
        );

        let events = responses_public_wire_codec()
            .public_events(&json!({
                "chunks": [response_event("response.created", "resp_1", json!({}))]
            }))
            .expect("a chunks-only envelope with typed public events is valid");
        assert_eq!(events.len(), 1);
        assert_eq!(events[0]["type"], "response.created");
    }

    #[test]
    fn public_codec_preserves_standard_terminal_order_inside_private_envelope() {
        let event = json!({
            "type": "codex.response.metadata",
            "chunks": [
                output_text_delta("a"),
                {"type": "codex.rate_limits", "rate_limits": {"allowed": true}},
                response_event(
                    "response.completed",
                    "resp_1",
                    json!({
                        "usage": {
                            "input_tokens": 2,
                            "input_tokens_details": {"cached_tokens": 0},
                            "output_tokens": 1,
                            "output_tokens_details": {"reasoning_tokens": 0},
                            "total_tokens": 3
                        }
                    })
                )
            ]
        });

        let public = responses_public_wire_codec()
            .public_events(&event)
            .expect("valid public batch");
        assert_eq!(public.len(), 2);
        assert_eq!(public[0]["type"], "response.output_text.delta");
        assert_eq!(public[1]["type"], "response.completed");
    }

    #[test]
    fn public_codec_maps_provider_cancelled_to_a_neutral_standard_failure() {
        let event = response_event(
            "response.cancelled",
            "resp_cancelled",
            json!({
                "status": "cancelled",
                "error": {
                    "code": "codex_account_cancelled",
                    "message": "account acct_private was disabled"
                },
                "incomplete_details": {"reason": "provider_private_reason"},
                "output": [{
                    "type": "message",
                    "id": "msg_partial",
                    "role": "assistant",
                    "status": "incomplete",
                    "content": [{
                        "type": "output_text",
                        "text": "partial",
                        "annotations": [],
                        "logprobs": []
                    }]
                }],
                "usage": {
                    "input_tokens": 1,
                    "input_tokens_details": {"cached_tokens": 0},
                    "output_tokens": 2,
                    "output_tokens_details": {"reasoning_tokens": 0},
                    "total_tokens": 3
                }
            }),
        );

        let public = responses_public_wire_codec()
            .public_events(&event)
            .expect("provider cancellation should map to a public terminal");

        assert_eq!(public.len(), 1);
        assert_eq!(public[0]["type"], "response.failed");
        assert_eq!(public[0]["response"]["id"], "resp_cancelled");
        assert_eq!(public[0]["response"]["status"], "failed");
        assert_eq!(
            public[0]["response"]["error"],
            json!({
                "code": "response_cancelled",
                "message": "Provider cancelled the response"
            })
        );
        assert!(public[0]["response"]["incomplete_details"].is_null());
        assert_eq!(public[0]["response"]["output"][0]["id"], "msg_partial");
        assert_eq!(public[0]["response"]["usage"]["total_tokens"], 3);
        let wire = public[0].to_string();
        assert!(!wire.contains("codex"));
        assert!(!wire.contains("acct_private"));
        assert!(!wire.contains("provider_private_reason"));
    }

    #[test]
    fn public_codec_normalizes_non_public_incomplete_reason_variants() {
        for reason in ["max_tokens", "MAX_OUTPUT_TOKENS"] {
            let event = response_event(
                "response.incomplete",
                "resp_length",
                json!({
                    "status": "incomplete",
                    "error": {
                        "code": "provider_private",
                        "message": "account acct_private reached a private limit"
                    },
                    "incomplete_details": {"reason": reason}
                }),
            );
            let public = responses_public_wire_codec()
                .public_events(&event)
                .expect("length aliases should normalize");
            assert_eq!(public[0]["type"], "response.incomplete");
            assert_eq!(
                public[0]["response"]["incomplete_details"]["reason"],
                "max_output_tokens"
            );
            assert!(public[0]["response"]["error"].is_null());
            assert!(!public[0].to_string().contains("acct_private"));
        }

        for reason in ["tool_calls", "function_call"] {
            let event = response_event(
                "response.incomplete",
                "resp_tool",
                json!({
                    "status": "incomplete",
                    "error": {"code": "provider_private", "message": "acct_private"},
                    "incomplete_details": {"reason": reason}
                }),
            );
            let public = responses_public_wire_codec()
                .public_events(&event)
                .expect("tool-call terminals should normalize");
            assert_eq!(public[0]["type"], "response.completed");
            assert_eq!(public[0]["response"]["status"], "completed");
            assert!(public[0]["response"]["error"].is_null());
            assert!(public[0]["response"]["incomplete_details"].is_null());
            assert!(!public[0].to_string().contains("acct_private"));
        }

        let standard = response_event(
            "response.incomplete",
            "resp_filter",
            json!({
                "status": "incomplete",
                "incomplete_details": {"reason": "content_filter"}
            }),
        );
        let public = responses_public_wire_codec()
            .public_events(&standard)
            .expect("standard incomplete reason should pass");
        assert_eq!(public[0]["type"], "response.incomplete");
        assert_eq!(
            public[0]["response"]["incomplete_details"]["reason"],
            "content_filter"
        );

        let unknown = response_event(
            "response.incomplete",
            "resp_unknown",
            json!({
                "status": "incomplete",
                "incomplete_details": {"reason": "provider_private_reason"}
            }),
        );
        assert_eq!(
            responses_public_wire_codec().public_events(&unknown),
            Err(ResponsesPublicWireError::InvalidEventShape)
        );
    }

    #[test]
    fn public_codec_keeps_standard_error_but_hides_unknown_provider_events() {
        let error = json!({
            "type": "error",
            "status_code": 429,
            "headers": {"x-codex-primary-reset-at": "secret"},
            "plan_type": "free",
            "error": {
                "type": "usage_limit_reached",
                "code": "rate_limit_exceeded",
                "message": "quota exhausted",
                "param": null,
                "resets_in_seconds": 123,
            }
        });
        let private = json!({"type": "codex.unknown"});

        let public_error = responses_public_wire_codec()
            .public_events(&error)
            .expect("standard error");
        assert_eq!(public_error.len(), 1);
        assert_eq!(public_error[0]["code"], "rate_limit_exceeded");
        assert_eq!(public_error[0]["message"], "quota exhausted");
        assert!(public_error[0]["param"].is_null());
        assert_eq!(public_error[0]["sequence_number"], 0);
        assert!(public_error[0].get("status").is_none());
        assert!(public_error[0].get("error").is_none());
        assert!(public_error[0].get("headers").is_none());
        assert!(public_error[0].get("plan_type").is_none());
        assert!(responses_public_wire_codec()
            .public_events(&private)
            .expect("private frame")
            .is_empty());
    }

    #[test]
    fn public_codec_accepts_the_reference_top_level_error_shape() {
        let error = json!({
            "type": "error",
            "code": "invalid_request",
            "message": "The request was invalid",
            "param": "input",
            "sequence_number": 7,
            "provider_debug": {"account": "private"}
        });

        let public = responses_public_wire_codec()
            .public_events(&error)
            .expect("reference error shape");

        assert_eq!(public.len(), 1);
        assert_eq!(public[0]["code"], "invalid_request");
        assert_eq!(public[0]["message"], "The request was invalid");
        assert_eq!(public[0]["param"], "input");
        assert_eq!(public[0]["sequence_number"], 7);
        assert!(public[0].get("status").is_none());
        assert!(public[0].get("error").is_none());
        assert!(public[0].get("provider_debug").is_none());
    }

    #[test]
    fn public_codec_rejects_objects_hidden_under_allowlisted_leaf_fields() {
        for event in [
            json!({
                "type": "response.output_text.delta",
                "item_id": "msg_1",
                "output_index": 0,
                "content_index": 0,
                "delta": {"account_id": "acct_private"}
            }),
            json!({
                "type": "response.output_text.delta",
                "item_id": {"account_id": "acct_private"},
                "output_index": 0,
                "content_index": 0,
                "delta": "hello"
            }),
            json!({
                "type": "response.output_text.delta",
                "item_id": "msg_1",
                "output_index": {"provider_debug": true},
                "content_index": 0,
                "delta": "hello"
            }),
            json!({
                "type": "response.output_text.delta",
                "item_id": "msg_1",
                "output_index": 0,
                "content_index": 0,
                "delta": "hello",
                "logprobs": {"provider_debug": true}
            }),
            json!({
                "type": "response.output_text.delta",
                "item_id": "msg_1",
                "output_index": 0,
                "content_index": 0,
                "delta": "hello",
                "logprobs": [{
                    "token": {"account_id": "acct_private"},
                    "logprob": {"provider_debug": true}
                }]
            }),
            response_event(
                "response.incomplete",
                "resp_private_reason",
                json!({
                    "incomplete_details": {
                        "reason": {"account_id": "acct_private"}
                    }
                }),
            ),
            response_event(
                "response.created",
                "resp_private_tool_choice",
                json!({
                    "tool_choice": {
                        "type": "function",
                        "name": {"account_id": "acct_private"}
                    }
                }),
            ),
            response_event(
                "response.completed",
                "resp_private_moderation",
                json!({
                    "moderation": {
                        "output": {
                            "type": "moderation_result",
                            "flagged": false,
                            "model": "omni-moderation-latest",
                            "categories": {
                                "violence": {"account_id": "acct_private"}
                            },
                            "category_scores": {"violence": 0.1},
                            "category_applied_input_types": {"violence": ["text"]}
                        }
                    }
                }),
            ),
        ] {
            assert_eq!(
                responses_public_wire_codec().public_events(&event),
                Err(super::ResponsesPublicWireError::InvalidEventShape)
            );
        }
    }

    #[test]
    fn public_codec_rejects_missing_core_server_event_fields() {
        for event in [
            json!({"type": "response.created", "response": {}}),
            json!({
                "type": "response.output_text.delta",
                "item_id": "msg_1",
                "content_index": 0,
                "delta": "hello"
            }),
            json!({
                "type": "response.output_item.added",
                "output_index": 0
            }),
            json!({
                "type": "response.content_part.added",
                "item_id": "msg_1",
                "output_index": 0,
                "content_index": 0
            }),
        ] {
            assert_eq!(
                responses_public_wire_codec().public_events(&event),
                Err(super::ResponsesPublicWireError::InvalidEventShape)
            );
        }
    }

    #[test]
    fn public_codec_allows_missing_provider_sequence_but_rejects_wrong_sequence_type() {
        let without_sequence = json!({
            "type": "response.output_text.delta",
            "item_id": "msg_1",
            "output_index": 0,
            "content_index": 0,
            "delta": "hello",
            "logprobs": []
        });
        assert!(responses_public_wire_codec()
            .public_events(&without_sequence)
            .is_ok());

        let malformed_sequence = json!({
            "type": "response.output_text.delta",
            "item_id": "msg_1",
            "output_index": 0,
            "content_index": 0,
            "delta": "hello",
            "logprobs": [],
            "sequence_number": {"account_id": "acct_private"}
        });
        assert_eq!(
            responses_public_wire_codec().public_events(&malformed_sequence),
            Err(super::ResponsesPublicWireError::InvalidEventShape)
        );
    }

    #[test]
    fn public_codec_rejects_incomplete_response_snapshots() {
        let mut snapshot = json!({
            "id": "resp_1",
            "created_at": 1.0,
            "error": null,
            "incomplete_details": null,
            "instructions": null,
            "metadata": {},
            "model": "gpt-test",
            "object": "response",
            "output": [],
            "parallel_tool_calls": true,
            "temperature": null,
            "tool_choice": "auto",
            "tools": [],
            "top_p": null
        });
        snapshot
            .as_object_mut()
            .expect("snapshot object")
            .remove("parallel_tool_calls");

        assert_eq!(
            responses_public_wire_codec().public_events(&json!({
                "type": "response.created",
                "response": snapshot
            })),
            Err(super::ResponsesPublicWireError::InvalidEventShape)
        );
    }

    #[test]
    fn public_codec_requires_output_text_logprobs() {
        let event = json!({
            "type": "response.output_text.delta",
            "item_id": "msg_1",
            "output_index": 0,
            "content_index": 0,
            "delta": "hello"
        });
        assert_eq!(
            responses_public_wire_codec().public_events(&event),
            Err(super::ResponsesPublicWireError::InvalidEventShape)
        );
    }

    #[test]
    fn public_codec_rejects_hybrid_errors_instead_of_leaking_nested_message() {
        let hybrid = json!({
            "type": "error",
            "code": "public_code",
            "message": "public message",
            "error": {
                "code": "private_code",
                "message": "private account detail",
                "param": "private_param"
            }
        });
        assert_eq!(
            responses_public_wire_codec().public_events(&hybrid),
            Err(super::ResponsesPublicWireError::InvalidEventShape)
        );

        let top_level = json!({
            "type": "error",
            "code": "public_code",
            "message": "public message",
            "param": null,
            "error": {
                "code": "private_code",
                "message": "private account detail",
                "param": "private_param"
            }
        });
        let public = responses_public_wire_codec()
            .public_events(&top_level)
            .expect("complete top-level error wins");
        assert_eq!(public[0]["message"], "public message");
        assert_eq!(public[0]["code"], "public_code");
    }

    #[test]
    fn public_codec_rejects_terminal_followed_by_another_event() {
        let event = json!({
            "chunks": [
                response_event("response.completed", "resp_1", json!({})),
                output_text_delta("too late")
            ]
        });

        assert_eq!(
            responses_public_wire_codec().public_events(&event),
            Err(super::ResponsesPublicWireError::TerminalNotLast)
        );
    }

    #[test]
    fn public_codec_rejects_multiple_terminals() {
        let event = json!({
            "chunks": [
                response_event("response.failed", "resp_1", json!({})),
                {"type": "error", "code": "server_error", "message": "failed", "param": null}
            ]
        });

        assert_eq!(
            responses_public_wire_codec().public_events(&event),
            Err(super::ResponsesPublicWireError::MultipleTerminalEvents)
        );
    }

    #[test]
    fn public_codec_bounds_events_in_one_provider_frame() {
        let chunks = (0..=super::MAX_PUBLIC_EVENTS_PER_PROVIDER_FRAME)
            .map(|index| output_text_delta(&index.to_string()))
            .collect::<Vec<_>>();

        assert_eq!(
            responses_public_wire_codec().public_events(&json!({"chunks": chunks})),
            Err(super::ResponsesPublicWireError::TooManyEvents)
        );
    }

    #[test]
    fn public_event_state_requires_created_before_response_events() {
        let mut state = ResponsesPublicEventState::default();
        assert_eq!(
            state.accept_events(&[output_text_delta("too early")]),
            Err(ResponsesPublicWireError::EventBeforeCreated)
        );
        assert_eq!(state, ResponsesPublicEventState::AwaitingCreated);
    }

    #[test]
    fn public_event_state_rejects_duplicate_created_across_frames() {
        let mut state = ResponsesPublicEventState::default();
        state
            .accept_events(&[response_event("response.created", "resp_1", json!({}))])
            .expect("first created event should be accepted");
        assert_eq!(
            state.accept_events(&[response_event("response.created", "resp_1", json!({}))]),
            Err(ResponsesPublicWireError::DuplicateCreated)
        );
    }

    #[test]
    fn public_event_state_rejects_response_id_changes() {
        let mut state = ResponsesPublicEventState::default();
        state
            .accept_events(&[response_event("response.created", "resp_1", json!({}))])
            .expect("created event should be accepted");
        assert_eq!(
            state.accept_events(&[response_event("response.in_progress", "resp_2", json!({}))]),
            Err(ResponsesPublicWireError::ResponseIdChanged)
        );
    }

    #[test]
    fn public_event_state_rejects_events_after_terminal_across_frames() {
        let mut state = ResponsesPublicEventState::default();
        state
            .accept_events(&[
                response_event("response.created", "resp_1", json!({})),
                response_event("response.completed", "resp_1", json!({})),
            ])
            .expect("complete response sequence should be accepted");
        assert_eq!(
            state.accept_events(&[output_text_delta("too late")]),
            Err(ResponsesPublicWireError::EventAfterTerminal)
        );
    }

    #[test]
    fn public_event_state_validates_batches_transactionally() {
        let mut state = ResponsesPublicEventState::default();
        assert_eq!(
            state.accept_events(&[
                response_event("response.created", "resp_1", json!({})),
                response_event("response.in_progress", "resp_2", json!({})),
            ]),
            Err(ResponsesPublicWireError::ResponseIdChanged)
        );
        assert_eq!(state, ResponsesPublicEventState::AwaitingCreated);
    }

    #[test]
    fn public_event_state_allows_request_error_before_created() {
        let mut state = ResponsesPublicEventState::default();
        state
            .accept_events(&[json!({
                "type": "error",
                "code": "invalid_request_error",
                "message": "invalid request",
                "param": null
            })])
            .expect("a request-level error may precede response creation");
        assert!(matches!(
            state,
            ResponsesPublicEventState::Terminal { response_id: None }
        ));
    }

    #[test]
    fn local_terminal_error_blocks_later_provider_events() {
        let mut state = ResponsesPublicEventState::default();
        state
            .accept_events(&[response_event("response.created", "resp_1", json!({}))])
            .expect("created event should be accepted");
        state
            .accept_local_terminal_error()
            .expect("local request error should terminate the public response");
        assert_eq!(
            state.accept_events(&[output_text_delta("too late")]),
            Err(ResponsesPublicWireError::EventAfterTerminal)
        );
    }
}
