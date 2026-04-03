use std::collections::BTreeSet;

use aether_data::repository::candidate_selection::{
    StoredMinimalCandidateSelectionRow, StoredProviderModelMapping,
};
use aether_data::DataLayerError;
use regex::Regex;

use crate::gateway::gateway_data::{GatewayDataState, StoredGatewayAuthApiKeySnapshot};

use super::GatewayMinimalCandidateSelectionCandidate;

pub(super) fn auth_snapshot_allows_provider(
    auth_snapshot: Option<&StoredGatewayAuthApiKeySnapshot>,
    provider_id: &str,
    provider_name: &str,
) -> bool {
    let Some(allowed) =
        auth_snapshot.and_then(StoredGatewayAuthApiKeySnapshot::effective_allowed_providers)
    else {
        return true;
    };

    allowed.iter().any(|value| {
        value.trim().eq_ignore_ascii_case(provider_id.trim())
            || value.trim().eq_ignore_ascii_case(provider_name.trim())
    })
}

pub(super) fn auth_snapshot_allows_api_format(
    auth_snapshot: Option<&StoredGatewayAuthApiKeySnapshot>,
    api_format: &str,
) -> bool {
    let Some(allowed) =
        auth_snapshot.and_then(StoredGatewayAuthApiKeySnapshot::effective_allowed_api_formats)
    else {
        return true;
    };

    allowed
        .iter()
        .any(|value| normalize_api_format(value) == api_format)
}

pub(super) fn auth_snapshot_allows_model(
    auth_snapshot: Option<&StoredGatewayAuthApiKeySnapshot>,
    requested_model_name: &str,
    resolved_global_model_name: &str,
) -> bool {
    let Some(allowed) =
        auth_snapshot.and_then(StoredGatewayAuthApiKeySnapshot::effective_allowed_models)
    else {
        return true;
    };

    allowed
        .iter()
        .any(|value| value == requested_model_name || value == resolved_global_model_name)
}

pub(super) async fn read_requested_model_rows(
    state: &GatewayDataState,
    api_format: &str,
    requested_model_name: &str,
) -> Result<Option<(String, Vec<StoredMinimalCandidateSelectionRow>)>, DataLayerError> {
    let exact_rows = state
        .list_minimal_candidate_selection_rows(api_format, requested_model_name)
        .await?;
    if !exact_rows.is_empty() {
        return Ok(Some((requested_model_name.to_string(), exact_rows)));
    }

    let rows = state
        .list_minimal_candidate_selection_rows_for_api_format(api_format)
        .await?;
    let Some(resolved_global_model_name) =
        resolve_requested_global_model_name(&rows, requested_model_name, api_format)
    else {
        return Ok(None);
    };

    Ok(Some((
        resolved_global_model_name.clone(),
        rows.into_iter()
            .filter(|row| row.global_model_name == resolved_global_model_name)
            .collect(),
    )))
}

pub(super) fn resolve_requested_global_model_name(
    rows: &[StoredMinimalCandidateSelectionRow],
    requested_model_name: &str,
    api_format: &str,
) -> Option<String> {
    resolve_global_model_name_by(rows, |row| {
        row.model_provider_model_name == requested_model_name
    })
    .or_else(|| {
        resolve_global_model_name_by(rows, |row| {
            row.model_provider_model_mappings
                .as_ref()
                .is_some_and(|mappings| {
                    mappings.iter().any(|mapping| {
                        mapping_scope_matches(mapping, api_format)
                            && mapping.name == requested_model_name
                    })
                })
        })
    })
    .or_else(|| {
        resolve_global_model_name_by(rows, |row| {
            row.global_model_mappings.as_ref().is_some_and(|patterns| {
                patterns
                    .iter()
                    .any(|pattern| matches_model_mapping(pattern, requested_model_name))
            })
        })
    })
}

fn resolve_global_model_name_by<F>(
    rows: &[StoredMinimalCandidateSelectionRow],
    matches: F,
) -> Option<String>
where
    F: Fn(&StoredMinimalCandidateSelectionRow) -> bool,
{
    let mut matches = rows
        .iter()
        .filter(|row| matches(row))
        .map(|row| row.global_model_name.trim())
        .filter(|value| !value.is_empty())
        .map(ToOwned::to_owned)
        .collect::<BTreeSet<_>>()
        .into_iter();
    matches.next()
}

pub(super) fn resolve_provider_model_name(
    row: &StoredMinimalCandidateSelectionRow,
    requested_model_name: &str,
    api_format: &str,
) -> Option<(String, Option<String>)> {
    let selected_provider_model_name = select_provider_model_name(row, api_format);
    let Some(key_allowed_models) = row.key_allowed_models.as_ref() else {
        return Some((selected_provider_model_name, None));
    };
    if key_allowed_models.is_empty() {
        return None;
    }

    if key_allowed_models
        .iter()
        .any(|value| value == requested_model_name)
    {
        return Some((selected_provider_model_name, None));
    }

    let candidate_models = candidate_model_names(row, api_format);
    let mut sorted_allowed_models = key_allowed_models
        .iter()
        .map(|value| value.trim())
        .filter(|value| !value.is_empty())
        .map(ToOwned::to_owned)
        .collect::<Vec<_>>();
    sorted_allowed_models.sort();

    for allowed_model in &sorted_allowed_models {
        if candidate_models.contains(allowed_model.as_str()) {
            return Some((allowed_model.clone(), Some(allowed_model.clone())));
        }
    }

    let Some(global_model_mappings) = row.global_model_mappings.as_ref() else {
        return None;
    };
    for allowed_model in sorted_allowed_models {
        for pattern in global_model_mappings {
            if matches_model_mapping(pattern, &allowed_model) {
                return Some((allowed_model.clone(), Some(allowed_model)));
            }
        }
    }

    None
}

pub(super) fn select_provider_model_name(
    row: &StoredMinimalCandidateSelectionRow,
    api_format: &str,
) -> String {
    let Some(mappings) = row.model_provider_model_mappings.as_ref() else {
        return row.model_provider_model_name.clone();
    };

    let mut scoped = mappings
        .iter()
        .filter(|mapping| mapping_scope_matches(mapping, api_format))
        .collect::<Vec<_>>();
    if scoped.is_empty() {
        return row.model_provider_model_name.clone();
    }

    scoped.sort_by(|left, right| {
        left.priority
            .cmp(&right.priority)
            .then(left.name.cmp(&right.name))
    });
    let top_priority = scoped[0].priority;
    scoped
        .into_iter()
        .find(|mapping| mapping.priority == top_priority)
        .map(|mapping| mapping.name.clone())
        .unwrap_or_else(|| row.model_provider_model_name.clone())
}

pub(super) fn candidate_model_names(
    row: &StoredMinimalCandidateSelectionRow,
    api_format: &str,
) -> BTreeSet<String> {
    let mut names = BTreeSet::from([row.model_provider_model_name.clone()]);
    if let Some(mappings) = row.model_provider_model_mappings.as_ref() {
        for mapping in mappings {
            if mapping_scope_matches(mapping, api_format) {
                names.insert(mapping.name.clone());
            }
        }
    }
    names
}

fn mapping_scope_matches(mapping: &StoredProviderModelMapping, api_format: &str) -> bool {
    let Some(api_formats) = mapping.api_formats.as_ref() else {
        return true;
    };

    api_formats
        .iter()
        .any(|value| normalize_api_format(value) == api_format)
}

pub(super) fn row_supports_required_capability(
    row: &StoredMinimalCandidateSelectionRow,
    required_capability: &str,
) -> bool {
    capabilities_support_required_capability(row.key_capabilities.as_ref(), required_capability)
}

pub(super) fn candidate_supports_required_capability(
    candidate: &GatewayMinimalCandidateSelectionCandidate,
    required_capability: &str,
) -> bool {
    capabilities_support_required_capability(
        candidate.key_capabilities.as_ref(),
        required_capability,
    )
}

fn capabilities_support_required_capability(
    capabilities: Option<&serde_json::Value>,
    required_capability: &str,
) -> bool {
    let required_capability = required_capability.trim();
    if required_capability.is_empty() {
        return true;
    }
    let Some(capabilities) = capabilities else {
        return false;
    };

    if let Some(object) = capabilities.as_object() {
        return object.iter().any(|(key, value)| {
            key.eq_ignore_ascii_case(required_capability)
                && match value {
                    serde_json::Value::Bool(value) => *value,
                    serde_json::Value::String(value) => value.eq_ignore_ascii_case("true"),
                    serde_json::Value::Number(value) => {
                        value.as_i64().is_some_and(|value| value > 0)
                    }
                    _ => false,
                }
        });
    }

    if let Some(items) = capabilities.as_array() {
        return items.iter().any(|value| {
            value
                .as_str()
                .is_some_and(|value| value.eq_ignore_ascii_case(required_capability))
        });
    }

    false
}

pub(super) fn matches_model_mapping(pattern: &str, model_name: &str) -> bool {
    let Ok(compiled) = Regex::new(&format!("^(?:{pattern})$")) else {
        return false;
    };
    compiled.is_match(model_name)
}

pub(super) fn extract_global_priority_for_format(
    raw: Option<&serde_json::Value>,
    api_format: &str,
) -> Result<Option<i32>, DataLayerError> {
    let Some(raw) = raw else {
        return Ok(None);
    };
    let Some(object) = raw.as_object() else {
        return Err(DataLayerError::UnexpectedValue(
            "provider_api_keys.global_priority_by_format is not a JSON object".to_string(),
        ));
    };

    let Some(value) = object
        .iter()
        .find(|(key, _)| normalize_api_format(key) == api_format)
        .map(|(_, value)| value)
    else {
        return Ok(None);
    };

    if let Some(value) = value.as_i64() {
        return i32::try_from(value).map(Some).map_err(|_| {
            DataLayerError::UnexpectedValue(format!(
                "invalid provider_api_keys.global_priority_by_format value: {value}"
            ))
        });
    }

    if let Some(value) = value.as_str() {
        let value = value.trim().parse::<i32>().map_err(|_| {
            DataLayerError::UnexpectedValue(format!(
                "invalid provider_api_keys.global_priority_by_format value: {value}"
            ))
        })?;
        return Ok(Some(value));
    }

    Err(DataLayerError::UnexpectedValue(
        "provider_api_keys.global_priority_by_format contains a non-integer value".to_string(),
    ))
}

pub(super) fn normalize_api_format(value: &str) -> String {
    value.trim().to_ascii_lowercase()
}
