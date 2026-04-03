use super::{
    AdminStatsLeaderboardItem, AdminStatsLeaderboardMetric, AdminStatsSortOrder,
    AdminStatsUserMetadata,
};
use crate::gateway::{AppState, GatewayError};

pub(super) fn build_model_leaderboard_items(
    items: &[aether_data::repository::usage::StoredRequestUsageAudit],
) -> Vec<AdminStatsLeaderboardItem> {
    let mut grouped: std::collections::BTreeMap<String, AdminStatsLeaderboardItem> =
        std::collections::BTreeMap::new();
    for item in items {
        if matches!(item.status.as_str(), "pending" | "streaming")
            || matches!(item.provider_name.as_str(), "unknown" | "pending")
        {
            continue;
        }
        let entry =
            grouped
                .entry(item.model.clone())
                .or_insert_with(|| AdminStatsLeaderboardItem {
                    id: item.model.clone(),
                    name: item.model.clone(),
                    requests: 0,
                    tokens: 0,
                    cost: 0.0,
                });
        entry.requests = entry.requests.saturating_add(1);
        entry.tokens = entry.tokens.saturating_add(
            item.input_tokens
                .saturating_add(item.output_tokens)
                .saturating_add(item.cache_creation_input_tokens)
                .saturating_add(item.cache_read_input_tokens),
        );
        entry.cost += item.total_cost_usd;
    }
    grouped.into_values().collect()
}

pub(super) fn build_api_key_leaderboard_items(
    items: &[aether_data::repository::usage::StoredRequestUsageAudit],
    snapshots: Option<&[aether_data::repository::auth::StoredAuthApiKeySnapshot]>,
    include_inactive: bool,
    exclude_admin: bool,
) -> Vec<AdminStatsLeaderboardItem> {
    let snapshot_by_api_key_id: std::collections::BTreeMap<_, _> = snapshots
        .unwrap_or(&[])
        .iter()
        .map(|snapshot| (snapshot.api_key_id.as_str(), snapshot))
        .collect();
    let mut grouped: std::collections::BTreeMap<String, AdminStatsLeaderboardItem> =
        std::collections::BTreeMap::new();
    let snapshots_available = snapshots.is_some();

    for item in items {
        if matches!(item.status.as_str(), "pending" | "streaming")
            || matches!(item.provider_name.as_str(), "unknown" | "pending")
        {
            continue;
        }
        let Some(api_key_id) = item.api_key_id.as_deref() else {
            continue;
        };

        let entry_name = if let Some(snapshot) = snapshot_by_api_key_id.get(api_key_id) {
            if snapshot.user_is_deleted {
                continue;
            }
            if !include_inactive && !snapshot.api_key_is_active {
                continue;
            }
            if exclude_admin && snapshot.user_role.eq_ignore_ascii_case("admin") {
                continue;
            }
            snapshot
                .api_key_name
                .clone()
                .or_else(|| item.api_key_name.clone())
                .unwrap_or_else(|| api_key_id.to_string())
        } else {
            if snapshots_available {
                continue;
            }
            item.api_key_name
                .clone()
                .unwrap_or_else(|| api_key_id.to_string())
        };

        let entry =
            grouped
                .entry(api_key_id.to_string())
                .or_insert_with(|| AdminStatsLeaderboardItem {
                    id: api_key_id.to_string(),
                    name: entry_name,
                    requests: 0,
                    tokens: 0,
                    cost: 0.0,
                });
        entry.requests = entry.requests.saturating_add(1);
        entry.tokens = entry.tokens.saturating_add(
            item.input_tokens
                .saturating_add(item.output_tokens)
                .saturating_add(item.cache_creation_input_tokens)
                .saturating_add(item.cache_read_input_tokens),
        );
        entry.cost += item.total_cost_usd;
    }

    grouped.into_values().collect()
}

pub(super) fn build_user_leaderboard_items(
    items: &[aether_data::repository::usage::StoredRequestUsageAudit],
    users: &std::collections::BTreeMap<String, AdminStatsUserMetadata>,
    include_inactive: bool,
    exclude_admin: bool,
) -> Vec<AdminStatsLeaderboardItem> {
    let mut grouped: std::collections::BTreeMap<String, AdminStatsLeaderboardItem> =
        std::collections::BTreeMap::new();

    for item in items {
        if matches!(item.status.as_str(), "pending" | "streaming")
            || matches!(item.provider_name.as_str(), "unknown" | "pending")
        {
            continue;
        }
        let Some(user_id) = item.user_id.as_deref() else {
            continue;
        };
        let entry_name = if let Some(user) = users.get(user_id) {
            if user.is_deleted {
                continue;
            }
            if !include_inactive && !user.is_active {
                continue;
            }
            if exclude_admin && user.role.eq_ignore_ascii_case("admin") {
                continue;
            }
            user.name.clone()
        } else {
            if exclude_admin {
                continue;
            }
            item.username
                .as_deref()
                .map(str::trim)
                .filter(|value| !value.is_empty())
                .map(ToOwned::to_owned)
                .unwrap_or_else(|| user_id.to_string())
        };

        let entry =
            grouped
                .entry(user_id.to_string())
                .or_insert_with(|| AdminStatsLeaderboardItem {
                    id: user_id.to_string(),
                    name: entry_name,
                    requests: 0,
                    tokens: 0,
                    cost: 0.0,
                });
        entry.requests = entry.requests.saturating_add(1);
        entry.tokens = entry.tokens.saturating_add(
            item.input_tokens
                .saturating_add(item.output_tokens)
                .saturating_add(item.cache_creation_input_tokens)
                .saturating_add(item.cache_read_input_tokens),
        );
        entry.cost += item.total_cost_usd;
    }

    grouped.into_values().collect()
}

pub(super) async fn load_user_leaderboard_metadata(
    state: &AppState,
    user_ids: &[String],
) -> Result<std::collections::BTreeMap<String, AdminStatsUserMetadata>, GatewayError> {
    let mut metadata = std::collections::BTreeMap::new();

    if state.has_user_data_reader() {
        for user in state.list_users_by_ids(user_ids).await? {
            let name = if !user.username.trim().is_empty() {
                user.username
            } else {
                user.email.unwrap_or(user.id.clone())
            };
            metadata.insert(
                user.id,
                AdminStatsUserMetadata {
                    name,
                    role: user.role,
                    is_active: user.is_active,
                    is_deleted: user.is_deleted,
                },
            );
        }
        return Ok(metadata);
    }

    for user_id in user_ids {
        let Some(user) = state.find_user_auth_by_id(user_id).await? else {
            continue;
        };
        let name = if !user.username.trim().is_empty() {
            user.username.clone()
        } else {
            user.email.clone().unwrap_or_else(|| user.id.clone())
        };
        metadata.insert(
            user.id.clone(),
            AdminStatsUserMetadata {
                name,
                role: user.role.clone(),
                is_active: user.is_active,
                is_deleted: user.is_deleted,
            },
        );
    }

    Ok(metadata)
}

pub(super) fn compare_leaderboard_items(
    metric: AdminStatsLeaderboardMetric,
    order: AdminStatsSortOrder,
    left: &AdminStatsLeaderboardItem,
    right: &AdminStatsLeaderboardItem,
) -> std::cmp::Ordering {
    let metric_order = match metric {
        AdminStatsLeaderboardMetric::Requests => left.requests.cmp(&right.requests),
        AdminStatsLeaderboardMetric::Tokens => left.tokens.cmp(&right.tokens),
        AdminStatsLeaderboardMetric::Cost => left
            .cost
            .partial_cmp(&right.cost)
            .unwrap_or(std::cmp::Ordering::Equal),
    };
    let metric_order = match order {
        AdminStatsSortOrder::Asc => metric_order,
        AdminStatsSortOrder::Desc => metric_order.reverse(),
    };
    if metric_order == std::cmp::Ordering::Equal {
        left.id.cmp(&right.id)
    } else {
        metric_order
    }
}

fn leaderboard_metric_equal(
    metric: AdminStatsLeaderboardMetric,
    left: &AdminStatsLeaderboardItem,
    right: &AdminStatsLeaderboardItem,
) -> bool {
    match metric {
        AdminStatsLeaderboardMetric::Requests => left.requests == right.requests,
        AdminStatsLeaderboardMetric::Tokens => left.tokens == right.tokens,
        AdminStatsLeaderboardMetric::Cost => (left.cost - right.cost).abs() < 1e-9,
    }
}

pub(super) fn compute_dense_rank(
    metric: AdminStatsLeaderboardMetric,
    items: &[AdminStatsLeaderboardItem],
    index: usize,
) -> usize {
    if index == 0 {
        return 1;
    }
    let mut rank = 1usize;
    for current in 1..=index {
        if !leaderboard_metric_equal(metric, &items[current - 1], &items[current]) {
            rank = rank.saturating_add(1);
        }
    }
    rank
}
