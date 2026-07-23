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
}

impl<'a> ParsedResponsesWebSocketFrame<'a> {
    pub(super) fn parse(raw_text: &'a str) -> serde_json::Result<Self> {
        let event = serde_json::from_str::<Value>(raw_text)?;
        let event_type = event
            .get("type")
            .and_then(Value::as_str)
            .map(str::to_string);
        let event_type_value = event_type.as_deref().unwrap_or_default();
        let started = matches!(
            event_type_value,
            "response.created" | "response.in_progress" | "response.queued"
        );
        let terminal = match event_type_value {
            "response.completed" => Some(ResponsesWebSocketFrameTerminal {
                status_code: websocket_event_status_code(&event, 200),
                cancelled: false,
            }),
            "response.incomplete" => Some(ResponsesWebSocketFrameTerminal {
                status_code: websocket_event_status_code(&event, 502),
                cancelled: false,
            }),
            "response.cancelled" => Some(ResponsesWebSocketFrameTerminal {
                status_code: 499,
                cancelled: true,
            }),
            "response.failed" => Some(ResponsesWebSocketFrameTerminal {
                status_code: websocket_event_status_code(&event, 502),
                cancelled: false,
            }),
            "error" => Some(ResponsesWebSocketFrameTerminal {
                status_code: websocket_event_status_code(&event, 502),
                cancelled: false,
            }),
            _ => None,
        };
        let status = terminal.map(|terminal| terminal.status_code);

        Ok(Self {
            raw_text,
            event,
            event_type,
            status,
            started,
            terminal,
        })
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
