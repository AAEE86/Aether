//! Server-owned identity for one logical execution request.

use uuid::Uuid;

/// Internal request identity used by planning, candidate persistence, usage,
/// and runtime reservations. It is deliberately independent from the
/// client-controlled trace header.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct ExecutionRequestId(String);

impl ExecutionRequestId {
    pub(crate) fn generate() -> Self {
        Self(Uuid::new_v4().to_string())
    }

    pub(crate) fn server_owned(value: impl Into<String>) -> Self {
        let value = value.into();
        debug_assert!(!value.trim().is_empty());
        Self(value)
    }

    pub(crate) fn as_str(&self) -> &str {
        self.0.as_str()
    }

    pub(crate) fn into_string(self) -> String {
        self.0
    }
}

/// Reads the server-owned identity attached at the ingress boundary. The
/// fallback preserves compatibility for trusted internal callers and focused
/// unit tests that construct request parts directly.
pub(crate) fn execution_request_id_from_parts<'a>(
    parts: &'a http::request::Parts,
    fallback: &'a str,
) -> &'a str {
    parts
        .extensions
        .get::<ExecutionRequestId>()
        .map(ExecutionRequestId::as_str)
        .unwrap_or(fallback)
}

#[cfg(test)]
mod tests {
    use super::{execution_request_id_from_parts, ExecutionRequestId};

    #[test]
    fn identical_client_traces_do_not_control_execution_identity() {
        let trace_id = "shared-client-trace";
        let mut first = http::Request::new(()).into_parts().0;
        let mut second = http::Request::new(()).into_parts().0;
        first.extensions.insert(ExecutionRequestId::generate());
        second.extensions.insert(ExecutionRequestId::generate());

        let first_id = execution_request_id_from_parts(&first, trace_id);
        let second_id = execution_request_id_from_parts(&second, trace_id);

        assert_ne!(first_id, trace_id);
        assert_ne!(second_id, trace_id);
        assert_ne!(first_id, second_id);
    }
}
