use crate::gateway::handlers::query_param_value;

pub(super) fn admin_monitoring_escape_like_pattern(value: &str) -> String {
    value
        .chars()
        .map(|ch| match ch {
            '%' | '_' => format!("\\{}", ch),
            _ => ch.to_string(),
        })
        .collect::<Vec<_>>()
        .join("")
}

pub(super) fn parse_admin_monitoring_offset(query: Option<&str>) -> Result<usize, String> {
    match query_param_value(query, "offset") {
        Some(value) => value
            .parse::<usize>()
            .map_err(|_| "offset must be a non-negative integer".to_string()),
        None => Ok(0),
    }
}

pub(super) fn parse_admin_monitoring_days(query: Option<&str>) -> Result<i64, String> {
    match query_param_value(query, "days") {
        Some(value) => {
            let parsed = value
                .parse::<i64>()
                .map_err(|_| "days must be an integer between 0 and 365".to_string())?;
            if (0..=365).contains(&parsed) {
                Ok(parsed)
            } else {
                Err("days must be an integer between 0 and 365".to_string())
            }
        }
        None => Ok(30),
    }
}

pub(super) fn parse_admin_monitoring_hours(query: Option<&str>) -> Result<i64, String> {
    match query_param_value(query, "hours") {
        Some(value) => {
            let parsed = value
                .parse::<i64>()
                .map_err(|_| "hours must be an integer between 1 and 720".to_string())?;
            if (1..=720).contains(&parsed) {
                Ok(parsed)
            } else {
                Err("hours must be an integer between 1 and 720".to_string())
            }
        }
        None => Ok(24),
    }
}

pub(super) fn parse_admin_monitoring_username_filter(query: Option<&str>) -> Option<String> {
    query_param_value(query, "username")
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string)
}

pub(super) fn parse_admin_monitoring_event_type_filter(query: Option<&str>) -> Option<String> {
    query_param_value(query, "event_type")
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string)
}

pub(super) fn parse_admin_monitoring_limit(query: Option<&str>) -> Result<usize, String> {
    match query_param_value(query, "limit") {
        Some(value) => {
            let parsed = value
                .parse::<usize>()
                .map_err(|_| "limit must be an integer between 1 and 1000".to_string())?;
            if (1..=1000).contains(&parsed) {
                Ok(parsed)
            } else {
                Err("limit must be an integer between 1 and 1000".to_string())
            }
        }
        None => Ok(100),
    }
}
