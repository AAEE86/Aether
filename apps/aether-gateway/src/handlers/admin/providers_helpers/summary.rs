use crate::gateway::handlers::public::{
    provider_key_api_formats, request_candidate_event_unix_secs, request_candidate_status_label,
};
use crate::gateway::handlers::unix_secs_to_rfc3339;
use crate::gateway::AppState;
use aether_data::repository::candidates::{RequestCandidateStatus, StoredRequestCandidate};
use aether_data::repository::provider_catalog::{
    StoredProviderCatalogEndpoint, StoredProviderCatalogKey, StoredProviderCatalogProvider,
};
use serde_json::json;
use std::collections::{BTreeMap, BTreeSet};
use std::time::{SystemTime, UNIX_EPOCH};

use super::super::{endpoint_timestamp_or_now, json_truthy};

pub(crate) async fn build_admin_providers_payload(
    state: &AppState,
    skip: usize,
    limit: usize,
    is_active: Option<bool>,
) -> Option<serde_json::Value> {
    if !state.has_provider_catalog_data_reader() {
        return None;
    }

    let active_only = is_active.unwrap_or(false);
    let mut providers = state
        .list_provider_catalog_providers(active_only)
        .await
        .ok()
        .unwrap_or_default();
    if matches!(is_active, Some(false)) {
        providers.retain(|provider| !provider.is_active);
    }
    providers.sort_by(|left, right| {
        left.provider_priority
            .cmp(&right.provider_priority)
            .then_with(|| left.name.cmp(&right.name))
    });

    let providers = providers
        .into_iter()
        .skip(skip)
        .take(limit)
        .collect::<Vec<_>>();
    let provider_ids = providers
        .iter()
        .map(|provider| provider.id.clone())
        .collect::<Vec<_>>();
    let endpoints = if provider_ids.is_empty() {
        Vec::new()
    } else {
        state
            .list_provider_catalog_endpoints_by_provider_ids(&provider_ids)
            .await
            .ok()
            .unwrap_or_default()
    };
    let keys = if provider_ids.is_empty() {
        Vec::new()
    } else {
        state
            .list_provider_catalog_keys_by_provider_ids(&provider_ids)
            .await
            .ok()
            .unwrap_or_default()
    };
    let first_endpoint_by_provider = endpoints
        .into_iter()
        .filter(|endpoint| endpoint.is_active)
        .fold(
            BTreeMap::<String, StoredProviderCatalogEndpoint>::new(),
            |mut acc, endpoint| {
                acc.entry(endpoint.provider_id.clone()).or_insert(endpoint);
                acc
            },
        );
    let has_any_key_by_provider =
        keys.into_iter()
            .fold(BTreeSet::<String>::new(), |mut acc, key| {
                acc.insert(key.provider_id);
                acc
            });

    Some(serde_json::Value::Array(
        providers
            .into_iter()
            .map(|provider| {
                let provider_id = provider.id.clone();
                let endpoint = first_endpoint_by_provider.get(&provider_id);
                json!({
                    "id": provider_id.clone(),
                    "name": provider.name,
                    "api_format": endpoint.map(|item| item.api_format.clone()),
                    "base_url": endpoint.map(|item| item.base_url.clone()),
                    "api_key": has_any_key_by_provider.contains(&provider_id).then_some("***"),
                    "priority": provider.provider_priority,
                    "is_active": provider.is_active,
                    "created_at": provider.created_at_unix_secs.and_then(unix_secs_to_rfc3339),
                    "updated_at": provider.updated_at_unix_secs.and_then(unix_secs_to_rfc3339),
                })
            })
            .collect(),
    ))
}

pub(crate) fn build_admin_provider_summary_value(
    provider: &StoredProviderCatalogProvider,
    endpoints: &[StoredProviderCatalogEndpoint],
    keys: &[StoredProviderCatalogKey],
    quota_snapshot: Option<&aether_data::repository::quota::StoredProviderQuotaSnapshot>,
    model_stats: Option<&aether_data::repository::global_models::StoredProviderModelStats>,
    active_global_model_ids: Vec<String>,
    now_unix_secs: u64,
) -> serde_json::Value {
    let total_endpoints = endpoints.len();
    let active_endpoints = endpoints
        .iter()
        .filter(|endpoint| endpoint.is_active)
        .count();
    let total_keys = keys.len();
    let active_keys = keys.iter().filter(|key| key.is_active).count();
    let total_models = model_stats
        .map(|stats| stats.total_models as usize)
        .unwrap_or(0);
    let active_models = model_stats
        .map(|stats| stats.active_models as usize)
        .unwrap_or(0);
    let api_formats = endpoints
        .iter()
        .map(|endpoint| endpoint.api_format.clone())
        .collect::<Vec<_>>();

    let format_to_endpoint_id = endpoints
        .iter()
        .map(|endpoint| (endpoint.api_format.clone(), endpoint.id.clone()))
        .collect::<BTreeMap<_, _>>();
    let mut keys_by_endpoint = BTreeMap::<String, Vec<&StoredProviderCatalogKey>>::new();
    for endpoint in endpoints {
        keys_by_endpoint.entry(endpoint.id.clone()).or_default();
    }
    for key in keys {
        for api_format in provider_key_api_formats(key) {
            if let Some(endpoint_id) = format_to_endpoint_id.get(&api_format) {
                keys_by_endpoint
                    .entry(endpoint_id.clone())
                    .or_default()
                    .push(key);
            }
        }
    }

    let mut endpoint_health_scores = Vec::with_capacity(endpoints.len());
    let endpoint_health_details = endpoints
        .iter()
        .map(|endpoint| {
            let endpoint_keys = keys_by_endpoint
                .get(&endpoint.id)
                .cloned()
                .unwrap_or_default();
            let health_score = if endpoint_keys.is_empty() {
                1.0
            } else {
                let mut scores = Vec::new();
                for key in &endpoint_keys {
                    let score = key
                        .health_by_format
                        .as_ref()
                        .and_then(|value| value.get(&endpoint.api_format))
                        .and_then(|value| value.get("health_score"))
                        .and_then(serde_json::Value::as_f64)
                        .unwrap_or(1.0);
                    scores.push(score);
                }
                scores.iter().sum::<f64>() / scores.len() as f64
            };
            endpoint_health_scores.push(health_score);
            json!({
                "api_format": endpoint.api_format,
                "health_score": health_score,
                "is_active": endpoint.is_active,
                "total_keys": endpoint_keys.len(),
                "active_keys": endpoint_keys.iter().filter(|key| key.is_active).count(),
            })
        })
        .collect::<Vec<_>>();
    let avg_health_score = if endpoint_health_scores.is_empty() {
        1.0
    } else {
        endpoint_health_scores.iter().sum::<f64>() / endpoint_health_scores.len() as f64
    };
    let unhealthy_endpoints = endpoint_health_scores
        .iter()
        .filter(|score| **score < 0.5)
        .count();

    let provider_config = provider.config.clone();
    let config = provider_config
        .as_ref()
        .and_then(serde_json::Value::as_object);
    let provider_ops_config = config.and_then(|cfg| cfg.get("provider_ops"));
    let ops_configured = provider_ops_config.is_some_and(json_truthy);
    let ops_architecture_id = provider_ops_config
        .and_then(serde_json::Value::as_object)
        .and_then(|cfg| cfg.get("architecture_id"))
        .and_then(serde_json::Value::as_str)
        .map(ToOwned::to_owned);

    json!({
        "id": provider.id.clone(),
        "name": provider.name.clone(),
        "provider_type": provider.provider_type.clone(),
        "description": provider.description.clone(),
        "website": provider.website.clone(),
        "provider_priority": provider.provider_priority,
        "keep_priority_on_conversion": provider.keep_priority_on_conversion,
        "enable_format_conversion": provider.enable_format_conversion,
        "is_active": provider.is_active,
        "billing_type": quota_snapshot.map(|quota| quota.billing_type.clone()),
        "monthly_quota_usd": quota_snapshot.and_then(|quota| quota.monthly_quota_usd),
        "monthly_used_usd": quota_snapshot.map(|quota| quota.monthly_used_usd),
        "quota_reset_day": quota_snapshot.and_then(|quota| quota.quota_reset_day),
        "quota_last_reset_at": quota_snapshot
            .and_then(|quota| quota.quota_last_reset_at_unix_secs)
            .and_then(unix_secs_to_rfc3339),
        "quota_expires_at": quota_snapshot
            .and_then(|quota| quota.quota_expires_at_unix_secs)
            .and_then(unix_secs_to_rfc3339),
        "max_retries": provider.max_retries,
        "proxy": provider.proxy.clone(),
        "stream_first_byte_timeout": provider.stream_first_byte_timeout_secs,
        "request_timeout": provider.request_timeout_secs,
        "claude_code_advanced": config.and_then(|cfg| cfg.get("claude_code_advanced")).cloned(),
        "pool_advanced": config.and_then(|cfg| cfg.get("pool_advanced")).cloned(),
        "failover_rules": config.and_then(|cfg| cfg.get("failover_rules")).cloned(),
        "total_endpoints": total_endpoints,
        "active_endpoints": active_endpoints,
        "total_keys": total_keys,
        "active_keys": active_keys,
        "total_models": total_models,
        "active_models": active_models,
        "global_model_ids": active_global_model_ids,
        "avg_health_score": avg_health_score,
        "unhealthy_endpoints": unhealthy_endpoints,
        "api_formats": api_formats,
        "endpoint_health_details": endpoint_health_details,
        "ops_configured": ops_configured,
        "ops_architecture_id": ops_architecture_id,
        "created_at": endpoint_timestamp_or_now(provider.created_at_unix_secs, now_unix_secs),
        "updated_at": endpoint_timestamp_or_now(provider.updated_at_unix_secs, now_unix_secs),
    })
}

pub(crate) async fn build_admin_provider_summary_payload(
    state: &AppState,
    provider_id: &str,
) -> Option<serde_json::Value> {
    if !state.has_provider_catalog_data_reader() {
        return None;
    }

    let provider_ids = vec![provider_id.to_string()];
    let provider = state
        .read_provider_catalog_providers_by_ids(&provider_ids)
        .await
        .ok()?
        .into_iter()
        .next()?;
    let (
        endpoints_result,
        keys_result,
        quota_snapshot_result,
        model_stats_result,
        active_global_model_ids_result,
    ) = tokio::join!(
        state.list_provider_catalog_endpoints_by_provider_ids(&provider_ids),
        state.list_provider_catalog_keys_by_provider_ids(&provider_ids),
        state.read_provider_quota_snapshot(provider_id),
        state.list_provider_model_stats(&provider_ids),
        state.list_active_global_model_ids_by_provider_ids(&provider_ids),
    );
    let endpoints = endpoints_result.ok().unwrap_or_default();
    let keys = keys_result.ok().unwrap_or_default();
    let quota_snapshot = quota_snapshot_result.ok().flatten();
    let model_stats = model_stats_result
        .ok()
        .unwrap_or_default()
        .into_iter()
        .find(|stats| stats.provider_id == provider_id);
    let active_global_model_ids = active_global_model_ids_result
        .ok()
        .unwrap_or_default()
        .into_iter()
        .filter(|row| row.provider_id == provider_id)
        .map(|row| row.global_model_id)
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect::<Vec<_>>();
    let now_unix_secs = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();
    Some(build_admin_provider_summary_value(
        &provider,
        &endpoints,
        &keys,
        quota_snapshot.as_ref(),
        model_stats.as_ref(),
        active_global_model_ids,
        now_unix_secs,
    ))
}

pub(crate) async fn build_admin_providers_summary_payload(
    state: &AppState,
    page: usize,
    page_size: usize,
    search: &str,
    status: &str,
    api_format: &str,
    model_id: &str,
) -> Option<serde_json::Value> {
    if !state.has_provider_catalog_data_reader() {
        return None;
    }

    let normalized_search = search.trim().to_ascii_lowercase();
    let search_keywords = normalized_search
        .split_whitespace()
        .filter(|value| !value.is_empty())
        .collect::<Vec<_>>();
    let normalized_status = status.trim().to_ascii_lowercase();
    let normalized_api_format = api_format.trim();
    let normalized_model_id = model_id.trim();

    let mut providers = state
        .list_provider_catalog_providers(false)
        .await
        .ok()
        .unwrap_or_default();
    let all_provider_ids = providers
        .iter()
        .map(|provider| provider.id.clone())
        .collect::<Vec<_>>();
    let all_endpoints = if all_provider_ids.is_empty() {
        Vec::new()
    } else {
        state
            .list_provider_catalog_endpoints_by_provider_ids(&all_provider_ids)
            .await
            .ok()
            .unwrap_or_default()
    };
    let active_global_model_refs = if all_provider_ids.is_empty() {
        Vec::new()
    } else {
        state
            .list_active_global_model_ids_by_provider_ids(&all_provider_ids)
            .await
            .ok()
            .unwrap_or_default()
    };

    let mut api_formats_by_provider = BTreeMap::<String, BTreeSet<String>>::new();
    for endpoint in &all_endpoints {
        api_formats_by_provider
            .entry(endpoint.provider_id.clone())
            .or_default()
            .insert(endpoint.api_format.clone());
    }
    let mut active_global_model_ids_by_provider = BTreeMap::<String, BTreeSet<String>>::new();
    for row in active_global_model_refs {
        active_global_model_ids_by_provider
            .entry(row.provider_id)
            .or_default()
            .insert(row.global_model_id);
    }

    providers.retain(|provider| {
        if !search_keywords.is_empty() {
            let provider_name = provider.name.to_ascii_lowercase();
            if !search_keywords
                .iter()
                .all(|keyword| provider_name.contains(keyword))
            {
                return false;
            }
        }

        match normalized_status.as_str() {
            "active" if !provider.is_active => return false,
            "inactive" if provider.is_active => return false,
            _ => {}
        }

        if normalized_api_format != "all"
            && !normalized_api_format.is_empty()
            && !api_formats_by_provider
                .get(&provider.id)
                .is_some_and(|items| items.contains(normalized_api_format))
        {
            return false;
        }

        if normalized_model_id != "all"
            && !normalized_model_id.is_empty()
            && !active_global_model_ids_by_provider
                .get(&provider.id)
                .is_some_and(|items| items.contains(normalized_model_id))
        {
            return false;
        }

        true
    });

    providers.sort_by(|left, right| {
        right
            .is_active
            .cmp(&left.is_active)
            .then_with(|| left.provider_priority.cmp(&right.provider_priority))
            .then_with(|| left.created_at_unix_secs.cmp(&right.created_at_unix_secs))
    });

    let total = providers.len();
    let offset = page.saturating_sub(1).saturating_mul(page_size);
    let providers = providers
        .into_iter()
        .skip(offset)
        .take(page_size)
        .collect::<Vec<_>>();
    let provider_ids = providers
        .iter()
        .map(|provider| provider.id.clone())
        .collect::<Vec<_>>();
    let endpoints = if provider_ids.is_empty() {
        Vec::new()
    } else {
        state
            .list_provider_catalog_endpoints_by_provider_ids(&provider_ids)
            .await
            .ok()
            .unwrap_or_default()
    };
    let keys = if provider_ids.is_empty() {
        Vec::new()
    } else {
        state
            .list_provider_catalog_keys_by_provider_ids(&provider_ids)
            .await
            .ok()
            .unwrap_or_default()
    };
    let model_stats = if provider_ids.is_empty() {
        Vec::new()
    } else {
        state
            .list_provider_model_stats(&provider_ids)
            .await
            .ok()
            .unwrap_or_default()
    };
    let mut endpoints_by_provider = BTreeMap::<String, Vec<StoredProviderCatalogEndpoint>>::new();
    for endpoint in endpoints {
        endpoints_by_provider
            .entry(endpoint.provider_id.clone())
            .or_default()
            .push(endpoint);
    }
    let mut keys_by_provider = BTreeMap::<String, Vec<StoredProviderCatalogKey>>::new();
    for key in keys {
        keys_by_provider
            .entry(key.provider_id.clone())
            .or_default()
            .push(key);
    }
    let model_stats_by_provider = model_stats
        .into_iter()
        .map(|stats| (stats.provider_id.clone(), stats))
        .collect::<BTreeMap<_, _>>();
    let now_unix_secs = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();
    let mut items = Vec::with_capacity(providers.len());
    for provider in providers {
        let quota_snapshot = state
            .read_provider_quota_snapshot(&provider.id)
            .await
            .ok()
            .flatten();
        let active_global_model_ids = active_global_model_ids_by_provider
            .get(&provider.id)
            .cloned()
            .unwrap_or_default()
            .into_iter()
            .collect::<Vec<_>>();
        items.push(build_admin_provider_summary_value(
            &provider,
            endpoints_by_provider
                .get(&provider.id)
                .map(Vec::as_slice)
                .unwrap_or(&[]),
            keys_by_provider
                .get(&provider.id)
                .map(Vec::as_slice)
                .unwrap_or(&[]),
            quota_snapshot.as_ref(),
            model_stats_by_provider.get(&provider.id),
            active_global_model_ids,
            now_unix_secs,
        ));
    }

    Some(json!({
        "total": total,
        "page": page,
        "page_size": page_size,
        "items": items,
    }))
}

pub(crate) async fn build_admin_provider_health_monitor_payload(
    state: &AppState,
    provider_id: &str,
    lookback_hours: u64,
    per_endpoint_limit: usize,
) -> Option<serde_json::Value> {
    if !state.has_provider_catalog_data_reader() || !state.has_request_candidate_data_reader() {
        return None;
    }

    let provider = state
        .read_provider_catalog_providers_by_ids(&[provider_id.to_string()])
        .await
        .ok()
        .and_then(|mut providers| providers.drain(..).next())?;
    let now_unix_secs = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .ok()
        .map(|duration| duration.as_secs())
        .unwrap_or(0);
    let since_unix_secs = now_unix_secs.saturating_sub(lookback_hours * 3600);

    let mut endpoints = state
        .list_provider_catalog_endpoints_by_provider_ids(std::slice::from_ref(&provider.id))
        .await
        .ok()
        .unwrap_or_default();
    endpoints.sort_by(|left, right| {
        left.api_format
            .cmp(&right.api_format)
            .then_with(|| left.id.cmp(&right.id))
    });

    if endpoints.is_empty() {
        return Some(json!({
            "provider_id": provider.id,
            "provider_name": provider.name,
            "generated_at": unix_secs_to_rfc3339(now_unix_secs),
            "endpoints": [],
        }));
    }

    let endpoint_ids = endpoints
        .iter()
        .map(|endpoint| endpoint.id.clone())
        .collect::<Vec<_>>();
    let fetch_limit = per_endpoint_limit
        .saturating_mul(endpoint_ids.len())
        .max(per_endpoint_limit);
    let attempts = state
        .list_finalized_request_candidates_by_endpoint_ids_since(
            &endpoint_ids,
            since_unix_secs,
            fetch_limit,
        )
        .await
        .ok()
        .unwrap_or_default();

    let mut attempts_by_endpoint = BTreeMap::<String, Vec<StoredRequestCandidate>>::new();
    for candidate in attempts {
        let Some(endpoint_id) = candidate.endpoint_id.clone() else {
            continue;
        };
        attempts_by_endpoint
            .entry(endpoint_id)
            .or_default()
            .push(candidate);
    }

    for candidates in attempts_by_endpoint.values_mut() {
        candidates.sort_by(|left, right| {
            right
                .created_at_unix_secs
                .cmp(&left.created_at_unix_secs)
                .then_with(|| right.id.cmp(&left.id))
        });
        candidates.truncate(per_endpoint_limit);
        candidates.sort_by(|left, right| {
            request_candidate_event_unix_secs(left)
                .cmp(&request_candidate_event_unix_secs(right))
                .then_with(|| left.id.cmp(&right.id))
        });
    }

    let endpoints = endpoints
        .into_iter()
        .map(|endpoint| {
            let candidates = attempts_by_endpoint.remove(&endpoint.id).unwrap_or_default();
            let success_count = candidates
                .iter()
                .filter(|candidate| candidate.status == RequestCandidateStatus::Success)
                .count();
            let failed_count = candidates
                .iter()
                .filter(|candidate| candidate.status == RequestCandidateStatus::Failed)
                .count();
            let skipped_count = candidates
                .iter()
                .filter(|candidate| candidate.status == RequestCandidateStatus::Skipped)
                .count();
            let total_attempts = candidates.len();
            let success_rate = if total_attempts > 0 {
                success_count as f64 / total_attempts as f64
            } else {
                1.0
            };
            let last_event_at = candidates
                .last()
                .and_then(|candidate| unix_secs_to_rfc3339(request_candidate_event_unix_secs(candidate)));
            let events = candidates
                .into_iter()
                .filter_map(|candidate| {
                    Some(json!({
                        "timestamp": unix_secs_to_rfc3339(request_candidate_event_unix_secs(&candidate))?,
                        "status": request_candidate_status_label(candidate.status),
                        "status_code": candidate.status_code,
                        "latency_ms": candidate.latency_ms,
                        "error_type": candidate.error_type,
                        "error_message": candidate.error_message,
                    }))
                })
                .collect::<Vec<_>>();

            json!({
                "endpoint_id": endpoint.id,
                "api_format": endpoint.api_format,
                "is_active": endpoint.is_active,
                "total_attempts": total_attempts,
                "success_count": success_count,
                "failed_count": failed_count,
                "skipped_count": skipped_count,
                "success_rate": success_rate,
                "last_event_at": last_event_at,
                "events": events,
            })
        })
        .collect::<Vec<_>>();

    Some(json!({
        "provider_id": provider.id,
        "provider_name": provider.name,
        "generated_at": unix_secs_to_rfc3339(now_unix_secs),
        "endpoints": endpoints,
    }))
}
