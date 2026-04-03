use serde_json::{Map, Value};

pub(crate) fn context_text(context: &Map<String, Value>, key: &str) -> Option<String> {
    context
        .get(key)
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string)
}

pub(crate) fn context_u64(context: &Map<String, Value>, key: &str) -> Option<u64> {
    let value = context.get(key)?;
    match value {
        Value::Number(number) => number.as_u64(),
        Value::String(text) => text.trim().parse().ok(),
        _ => None,
    }
}

pub(crate) fn request_body_text(context: &Map<String, Value>, key: &str) -> Option<String> {
    context
        .get("original_request_body")
        .and_then(Value::as_object)
        .and_then(|body| body.get(key))
        .and_then(|value| match value {
            Value::String(text) => Some(text.trim().to_string()),
            Value::Number(number) => Some(number.to_string()),
            _ => None,
        })
        .filter(|value| !value.is_empty())
}

pub(crate) fn request_body_string(body: &Value, key: &str) -> Option<String> {
    body.as_object()
        .and_then(|map| map.get(key))
        .and_then(|value| match value {
            Value::String(text) => Some(text.trim().to_string()),
            Value::Number(number) => Some(number.to_string()),
            _ => None,
        })
        .filter(|value| !value.is_empty())
}

pub(crate) fn request_body_u32(body: &Value, key: &str) -> Option<u32> {
    body.as_object()
        .and_then(|map| map.get(key))
        .and_then(|value| match value {
            Value::Number(number) => number.as_u64().and_then(|value| u32::try_from(value).ok()),
            Value::String(text) => text.trim().parse().ok(),
            _ => None,
        })
}
