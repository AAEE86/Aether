use chrono::{DateTime, Utc};

use crate::gateway::gateway_data::StoredUserSessionRecord;

pub(super) fn format_users_me_optional_datetime_iso8601(
    value: Option<DateTime<Utc>>,
) -> Option<String> {
    value.map(|value| value.to_rfc3339())
}

pub(super) fn format_users_me_optional_unix_secs_iso8601(value: Option<u64>) -> Option<String> {
    let secs = value?;
    let secs = i64::try_from(secs).ok()?;
    DateTime::<Utc>::from_timestamp(secs, 0).map(|value| value.to_rfc3339())
}

pub(super) fn format_users_me_required_session_datetime_iso8601(
    session: &StoredUserSessionRecord,
) -> Option<String> {
    session
        .created_at
        .or(session.updated_at)
        .or(session.last_seen_at)
        .map(|value| value.to_rfc3339())
}
