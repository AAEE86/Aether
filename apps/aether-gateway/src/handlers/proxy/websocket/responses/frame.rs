//! Parsed OpenAI Responses WebSocket text frames.
//!
//! A relay frame is parsed once and then shared by the protocol adapter, turn
//! accounting, retry safety, and connection lifecycle code.  Keeping the raw
//! text as a borrow avoids copying the websocket payload while the relay is
//! processing it.

use serde_json::Value;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct ResponsesWebSocketFrameTerminal {
    pub(super) status_code: u16,
    pub(super) cancelled: bool,
}

#[derive(Debug)]
pub(super) struct ParsedResponsesWebSocketFrame<'a> {
    raw_text: &'a str,
    event: Value,
    event_type: Option<String>,
    status: Option<u16>,
    started: bool,
    terminal: Option<ResponsesWebSocketFrameTerminal>,
    terminal_event: Option<Value>,
    chunked: bool,
}

impl<'a> ParsedResponsesWebSocketFrame<'a> {
    pub(super) fn parse(raw_text: &'a str) -> serde_json::Result<Self> {
        let event = serde_json::from_str::<Value>(raw_text)?;
        let events = protocol_events_of(&event);
        let started = events.iter().copied().any(event_is_started);
        // A batch carries at most one terminal in practice.  Taking the first
        // in document order keeps the outcome deterministic if that ever
        // stops being true.
        let terminal_entry = events
            .iter()
            .copied()
            .find_map(|candidate| terminal_for_event(candidate).map(|term| (candidate, term)));
        let terminal = terminal_entry.map(|(_, terminal)| terminal);
        // The terminal event describes the turn's outcome, so it is the one
        // worth naming in logs and recording as the terminal error body.
        let event_type = terminal_entry
            .map(|(candidate, _)| candidate)
            .or_else(|| events.last().copied())
            .and_then(event_type_of)
            .map(str::to_string);
        let terminal_event = terminal_entry.map(|(candidate, _)| candidate.clone());
        let chunked = event.get("chunks").and_then(Value::as_array).is_some();
        let status = terminal.map(|terminal| terminal.status_code);

        Ok(Self {
            raw_text,
            event,
            event_type,
            status,
            started,
            terminal,
            terminal_event,
            chunked,
        })
    }

    /// The protocol events this frame carries.
    ///
    /// Codex batches standard `response.*` events into a `{"chunks":[...]}`
    /// envelope, so one frame can carry several events — and the terminal one
    /// may be buried inside the batch.  Every consumer that interprets event
    /// semantics must walk this rather than the envelope, or a batched
    /// `response.completed` goes unnoticed and wedges the turn.
    pub(super) fn protocol_events(&self) -> Vec<&Value> {
        protocol_events_of(&self.event)
    }

    /// The individual event that ended the turn, unwrapped from its batch.
    pub(super) fn terminal_event(&self) -> Option<&Value> {
        self.terminal_event.as_ref()
    }

    pub(super) fn is_chunked(&self) -> bool {
        self.chunked
    }

    pub(super) fn raw_text(&self) -> &'a str {
        self.raw_text
    }

    pub(super) fn event(&self) -> &Value {
        &self.event
    }

    pub(super) fn event_type(&self) -> Option<&str> {
        self.event_type.as_deref()
    }

    pub(super) fn status(&self) -> Option<u16> {
        self.status
    }

    pub(super) fn is_started(&self) -> bool {
        self.started
    }

    pub(super) fn is_terminal(&self) -> bool {
        self.terminal.is_some()
    }

    pub(super) fn terminal(&self) -> Option<ResponsesWebSocketFrameTerminal> {
        self.terminal
    }

    /// Return a bounded label suitable for structured logs.  Event payloads
    /// are never inserted directly into a log field.
    pub(super) fn event_type_for_log(&self) -> String {
        self.event_type
            .as_deref()
            .map(safe_websocket_event_label)
            .unwrap_or_else(|| "invalid_json".to_string())
    }
}

/// Flattens a frame into the events it carries.  An envelope may name its own
/// `type` *and* batch further events under `chunks`; both are protocol events.
fn protocol_events_of(event: &Value) -> Vec<&Value> {
    let mut events = Vec::new();
    if event_type_of(event).is_some() {
        events.push(event);
    }
    if let Some(chunks) = event.get("chunks").and_then(Value::as_array) {
        events.extend(chunks.iter().filter(|chunk| event_type_of(chunk).is_some()));
    }
    // An unrecognized shape is still relayed and still accounted for, so it
    // must not vanish from the observer's view of the stream.
    if events.is_empty() {
        events.push(event);
    }
    events
}

fn event_type_of(event: &Value) -> Option<&str> {
    event.get("type").and_then(Value::as_str)
}

fn event_is_started(event: &Value) -> bool {
    matches!(
        event_type_of(event).unwrap_or_default(),
        "response.created" | "response.in_progress" | "response.queued"
    )
}

fn terminal_for_event(event: &Value) -> Option<ResponsesWebSocketFrameTerminal> {
    match event_type_of(event).unwrap_or_default() {
        "response.completed" => Some(ResponsesWebSocketFrameTerminal {
            status_code: websocket_event_status_code(event, 200),
            cancelled: false,
        }),
        "response.incomplete" => Some(ResponsesWebSocketFrameTerminal {
            status_code: websocket_event_status_code(event, 502),
            cancelled: false,
        }),
        "response.cancelled" => Some(ResponsesWebSocketFrameTerminal {
            status_code: 499,
            cancelled: true,
        }),
        "response.failed" => Some(ResponsesWebSocketFrameTerminal {
            status_code: websocket_event_status_code(event, 502),
            cancelled: false,
        }),
        "error" => Some(ResponsesWebSocketFrameTerminal {
            status_code: websocket_event_status_code(event, 502),
            cancelled: false,
        }),
        _ => None,
    }
}

fn websocket_event_status_code(event: &Value, default: u16) -> u16 {
    if let Some(status_code) = event
        .get("status_code")
        .or_else(|| event.get("status"))
        .or_else(|| {
            event
                .get("response")
                .and_then(|response| response.get("status_code"))
        })
        .and_then(Value::as_u64)
        .and_then(|value| u16::try_from(value).ok())
        .filter(|value| *value > 0)
    {
        return status_code;
    }

    let error_code = [
        event.pointer("/error/type"),
        event.pointer("/error/code"),
        event.pointer("/response/error/type"),
        event.pointer("/response/error/code"),
    ]
    .into_iter()
    .flatten()
    .filter_map(Value::as_str)
    .map(str::to_ascii_lowercase)
    .find(|value| !value.trim().is_empty());
    match error_code.as_deref() {
        Some(
            "usage_limit_reached" | "insufficient_quota" | "rate_limit_exceeded" | "quota_exceeded",
        ) => 429,
        Some("invalid_api_key" | "authentication_error") => 401,
        Some("invalid_request_error" | "invalid_request" | "model_not_found") => 400,
        Some("overloaded" | "server_error" | "service_unavailable") => 503,
        _ => default,
    }
}

fn safe_websocket_event_label(value: &str) -> String {
    let value = value.trim();
    if value.is_empty()
        || value.len() > 80
        || !value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'_' | b'-'))
    {
        return "unknown".to_string();
    }
    value.to_string()
}

#[cfg(test)]
mod tests {
    use super::ParsedResponsesWebSocketFrame;

    #[test]
    fn parses_started_frame_once_with_raw_text_and_event_metadata() {
        let raw = r#"{"type":"response.in_progress","response":{"status":200}}"#;
        let frame = ParsedResponsesWebSocketFrame::parse(raw).expect("valid frame");

        assert_eq!(frame.raw_text(), raw);
        assert_eq!(frame.event_type(), Some("response.in_progress"));
        assert_eq!(frame.status(), None);
        assert!(frame.is_started());
        assert!(!frame.is_terminal());
        assert_eq!(frame.event()["response"]["status"], 200);
        assert_eq!(frame.event_type_for_log(), "response.in_progress");
    }

    #[test]
    fn classifies_terminal_status_and_cancellation() {
        let completed = ParsedResponsesWebSocketFrame::parse(
            r#"{"type":"response.completed","status_code":201}"#,
        )
        .expect("valid frame");
        assert_eq!(completed.status(), Some(201));
        assert_eq!(
            completed
                .terminal()
                .map(|terminal| (terminal.status_code, terminal.cancelled)),
            Some((201, false))
        );

        let cancelled = ParsedResponsesWebSocketFrame::parse(r#"{"type":"response.cancelled"}"#)
            .expect("valid frame");
        assert_eq!(cancelled.status(), Some(499));
        assert_eq!(
            cancelled
                .terminal()
                .map(|terminal| (terminal.status_code, terminal.cancelled)),
            Some((499, true))
        );

        let error = ParsedResponsesWebSocketFrame::parse(
            r#"{"type":"error","status_code":429,"error":{"type":"usage_limit_reached"}}"#,
        )
        .expect("valid frame");
        assert_eq!(error.status(), Some(429));
        assert!(error.is_terminal());

        let failed = ParsedResponsesWebSocketFrame::parse(
            r#"{"type":"response.failed","response":{"error":{"code":"rate_limit_exceeded"}}}"#,
        )
        .expect("valid frame");
        assert_eq!(failed.status(), Some(429));
    }

    #[test]
    fn detects_a_terminal_batched_inside_a_chunks_envelope() {
        let frame = ParsedResponsesWebSocketFrame::parse(
            r#"{"chunks":[{"type":"response.output_text.delta","delta":"hi"},{"type":"response.completed","response":{"usage":{"total_tokens":8}}}]}"#,
        )
        .expect("valid frame");

        assert!(frame.is_chunked());
        assert!(frame.is_terminal());
        assert_eq!(frame.status(), Some(200));
        // The label and the recorded error body must name the event that ended
        // the turn, not the envelope.
        assert_eq!(frame.event_type(), Some("response.completed"));
        assert_eq!(
            frame.terminal_event().and_then(|event| event
                .pointer("/response/usage/total_tokens")
                .and_then(serde_json::Value::as_u64)),
            Some(8)
        );
        assert_eq!(frame.protocol_events().len(), 2);
    }

    #[test]
    fn detects_a_start_event_batched_inside_a_chunks_envelope() {
        let frame = ParsedResponsesWebSocketFrame::parse(
            r#"{"chunks":[{"type":"codex.rate_limits"},{"type":"response.created"}]}"#,
        )
        .expect("valid frame");

        assert!(frame.is_started());
        assert!(!frame.is_terminal());
        assert_eq!(frame.protocol_events().len(), 2);
    }

    #[test]
    fn an_envelope_may_carry_its_own_type_alongside_batched_events() {
        let frame = ParsedResponsesWebSocketFrame::parse(
            r#"{"type":"codex.response.metadata","chunks":[{"type":"response.failed","response":{"error":{"code":"rate_limit_exceeded"}}}]}"#,
        )
        .expect("valid frame");

        assert_eq!(frame.protocol_events().len(), 2);
        assert!(frame.is_terminal());
        assert_eq!(frame.status(), Some(429));
        assert_eq!(frame.event_type(), Some("response.failed"));
    }

    #[test]
    fn a_batch_without_a_terminal_does_not_end_the_turn() {
        let frame = ParsedResponsesWebSocketFrame::parse(
            r#"{"chunks":[{"type":"response.output_text.delta","delta":"a"},{"type":"response.output_text.delta","delta":"b"}]}"#,
        )
        .expect("valid frame");

        assert!(!frame.is_terminal());
        assert!(!frame.is_started());
        assert!(frame.terminal_event().is_none());
    }

    #[test]
    fn an_unrecognized_shape_is_still_surfaced_as_one_event() {
        let frame =
            ParsedResponsesWebSocketFrame::parse(r#"{"unexpected":true}"#).expect("valid frame");

        assert_eq!(frame.protocol_events().len(), 1);
        assert!(!frame.is_chunked());
        assert!(!frame.is_terminal());
        assert_eq!(frame.event_type(), None);
        assert_eq!(frame.event_type_for_log(), "invalid_json");
    }

    #[test]
    fn preserves_safe_log_label_boundaries() {
        let unsafe_label =
            ParsedResponsesWebSocketFrame::parse(r#"{"type":"not safe / contains spaces"}"#)
                .expect("valid frame");
        assert_eq!(unsafe_label.event_type_for_log(), "unknown");

        let missing_label =
            ParsedResponsesWebSocketFrame::parse(r#"{"message":"ok"}"#).expect("valid frame");
        assert_eq!(missing_label.event_type_for_log(), "invalid_json");
    }

    #[test]
    fn rejects_invalid_json() {
        assert!(ParsedResponsesWebSocketFrame::parse("not-json").is_err());
    }
}
