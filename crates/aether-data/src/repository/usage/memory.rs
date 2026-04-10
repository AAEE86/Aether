use std::collections::BTreeMap;
use std::sync::RwLock;

use async_trait::async_trait;

use super::{
    StoredProviderApiKeyUsageSummary, StoredProviderUsageSummary, StoredProviderUsageWindow,
    StoredRequestUsageAudit, UpsertUsageRecord, UsageAuditListQuery, UsageReadRepository,
    UsageWriteRepository,
};
use crate::DataLayerError;

#[derive(Debug, Default)]
pub struct InMemoryUsageReadRepository {
    by_request_id: RwLock<BTreeMap<String, StoredRequestUsageAudit>>,
    provider_usage_windows: RwLock<Vec<StoredProviderUsageWindow>>,
}

impl InMemoryUsageReadRepository {
    pub fn seed<I>(items: I) -> Self
    where
        I: IntoIterator<Item = StoredRequestUsageAudit>,
    {
        let mut by_request_id = BTreeMap::new();
        for item in items {
            by_request_id.insert(item.request_id.clone(), item);
        }
        Self {
            by_request_id: RwLock::new(by_request_id),
            provider_usage_windows: RwLock::new(Vec::new()),
        }
    }

    pub fn with_provider_usage_windows<I>(self, items: I) -> Self
    where
        I: IntoIterator<Item = StoredProviderUsageWindow>,
    {
        Self {
            by_request_id: self.by_request_id,
            provider_usage_windows: RwLock::new(items.into_iter().collect()),
        }
    }
}

#[async_trait]
impl UsageReadRepository for InMemoryUsageReadRepository {
    async fn find_by_id(
        &self,
        id: &str,
    ) -> Result<Option<StoredRequestUsageAudit>, DataLayerError> {
        Ok(self
            .by_request_id
            .read()
            .expect("usage repository lock")
            .values()
            .find(|item| item.id == id)
            .cloned())
    }

    async fn find_by_request_id(
        &self,
        request_id: &str,
    ) -> Result<Option<StoredRequestUsageAudit>, DataLayerError> {
        Ok(self
            .by_request_id
            .read()
            .expect("usage repository lock")
            .get(request_id)
            .cloned())
    }

    async fn list_usage_audits(
        &self,
        query: &UsageAuditListQuery,
    ) -> Result<Vec<StoredRequestUsageAudit>, DataLayerError> {
        let mut items: Vec<_> = self
            .by_request_id
            .read()
            .expect("usage repository lock")
            .values()
            .filter(|item| {
                // The field is historically named `created_at_unix_ms`, but usage audit rows
                // across gateway handlers, SQL repositories and tests are stored as epoch seconds.
                if let Some(created_from_unix_secs) = query.created_from_unix_secs {
                    if item.created_at_unix_ms < created_from_unix_secs {
                        return false;
                    }
                }
                if let Some(created_until_unix_secs) = query.created_until_unix_secs {
                    if item.created_at_unix_ms >= created_until_unix_secs {
                        return false;
                    }
                }
                if let Some(user_id) = query.user_id.as_deref() {
                    if item.user_id.as_deref() != Some(user_id) {
                        return false;
                    }
                }
                if let Some(provider_name) = query.provider_name.as_deref() {
                    if item.provider_name != provider_name {
                        return false;
                    }
                }
                if let Some(model) = query.model.as_deref() {
                    if item.model != model {
                        return false;
                    }
                }
                true
            })
            .cloned()
            .collect();
        items.sort_by(|left, right| {
            left.created_at_unix_ms
                .cmp(&right.created_at_unix_ms)
                .then_with(|| left.request_id.cmp(&right.request_id))
        });
        Ok(items)
    }

    async fn list_recent_usage_audits(
        &self,
        user_id: Option<&str>,
        limit: usize,
    ) -> Result<Vec<StoredRequestUsageAudit>, DataLayerError> {
        let mut items: Vec<_> = self
            .by_request_id
            .read()
            .expect("usage repository lock")
            .values()
            .filter(|item| match user_id {
                Some(user_id) => item.user_id.as_deref() == Some(user_id),
                None => true,
            })
            .cloned()
            .collect();
        items.sort_by(|left, right| {
            right
                .created_at_unix_ms
                .cmp(&left.created_at_unix_ms)
                .then_with(|| left.id.cmp(&right.id))
        });
        items.truncate(limit);
        Ok(items)
    }

    async fn summarize_total_tokens_by_api_key_ids(
        &self,
        api_key_ids: &[String],
    ) -> Result<BTreeMap<String, u64>, DataLayerError> {
        let api_key_id_set = api_key_ids.iter().map(String::as_str).collect::<Vec<_>>();
        let mut totals = BTreeMap::<String, u64>::new();
        for item in self
            .by_request_id
            .read()
            .expect("usage repository lock")
            .values()
        {
            let Some(api_key_id) = item.api_key_id.as_deref() else {
                continue;
            };
            if !api_key_id_set.contains(&api_key_id) {
                continue;
            }
            let entry = totals.entry(api_key_id.to_string()).or_insert(0);
            *entry = (*entry).saturating_add(item.total_tokens);
        }
        Ok(totals)
    }

    async fn summarize_usage_by_provider_api_key_ids(
        &self,
        provider_api_key_ids: &[String],
    ) -> Result<BTreeMap<String, StoredProviderApiKeyUsageSummary>, DataLayerError> {
        let provider_api_key_id_set = provider_api_key_ids
            .iter()
            .map(String::as_str)
            .collect::<Vec<_>>();
        let mut summaries = BTreeMap::<String, StoredProviderApiKeyUsageSummary>::new();
        for item in self
            .by_request_id
            .read()
            .expect("usage repository lock")
            .values()
        {
            let Some(provider_api_key_id) = item.provider_api_key_id.as_deref() else {
                continue;
            };
            if !provider_api_key_id_set.contains(&provider_api_key_id) {
                continue;
            }
            let entry = summaries
                .entry(provider_api_key_id.to_string())
                .or_insert_with(|| StoredProviderApiKeyUsageSummary {
                    provider_api_key_id: provider_api_key_id.to_string(),
                    ..StoredProviderApiKeyUsageSummary::default()
                });
            entry.request_count = entry.request_count.saturating_add(1);
            entry.total_tokens = entry.total_tokens.saturating_add(item.total_tokens);
            entry.total_cost_usd += item.total_cost_usd;
            entry.last_used_at_unix_secs = Some(
                entry
                    .last_used_at_unix_secs
                    .unwrap_or(0)
                    .max(item.created_at_unix_ms),
            );
        }
        Ok(summaries)
    }

    async fn summarize_provider_usage_since(
        &self,
        provider_id: &str,
        since_unix_secs: u64,
    ) -> Result<StoredProviderUsageSummary, DataLayerError> {
        let windows = self
            .provider_usage_windows
            .read()
            .expect("provider usage repository lock");

        let mut summary = StoredProviderUsageSummary::default();
        let mut response_time_samples = 0u64;
        for window in windows.iter().filter(|window| {
            window.provider_id == provider_id && window.window_start_unix_secs >= since_unix_secs
        }) {
            summary.total_requests = summary.total_requests.saturating_add(window.total_requests);
            summary.successful_requests = summary
                .successful_requests
                .saturating_add(window.successful_requests);
            summary.failed_requests = summary
                .failed_requests
                .saturating_add(window.failed_requests);
            summary.total_cost_usd += window.total_cost_usd;
            summary.avg_response_time_ms += window.avg_response_time_ms;
            response_time_samples = response_time_samples.saturating_add(1);
        }

        if response_time_samples > 0 {
            summary.avg_response_time_ms /= response_time_samples as f64;
        }

        Ok(summary)
    }
}

#[async_trait]
impl UsageWriteRepository for InMemoryUsageReadRepository {
    async fn upsert(
        &self,
        usage: UpsertUsageRecord,
    ) -> Result<StoredRequestUsageAudit, DataLayerError> {
        usage.validate()?;
        let mut by_request_id = self.by_request_id.write().expect("usage repository lock");

        let created_at_unix_ms = by_request_id
            .get(&usage.request_id)
            .map(|existing| existing.created_at_unix_ms)
            .or(usage.created_at_unix_ms)
            .unwrap_or(usage.updated_at_unix_secs);

        let total_tokens = usage
            .total_tokens
            .or_else(|| {
                Some(
                    usage.input_tokens.unwrap_or_default()
                        + usage.output_tokens.unwrap_or_default(),
                )
            })
            .unwrap_or_default();
        let existing = by_request_id.get(&usage.request_id);

        let stored = StoredRequestUsageAudit {
            id: existing
                .map(|existing| existing.id.clone())
                .unwrap_or_else(|| format!("usage-{}", usage.request_id)),
            request_id: usage.request_id.clone(),
            user_id: usage.user_id,
            api_key_id: usage.api_key_id,
            username: usage.username,
            api_key_name: usage.api_key_name,
            provider_name: usage.provider_name,
            model: usage.model,
            target_model: usage.target_model,
            provider_id: usage.provider_id,
            provider_endpoint_id: usage.provider_endpoint_id,
            provider_api_key_id: usage.provider_api_key_id,
            request_type: usage.request_type,
            api_format: usage.api_format,
            api_family: usage.api_family,
            endpoint_kind: usage.endpoint_kind,
            endpoint_api_format: usage.endpoint_api_format,
            provider_api_family: usage.provider_api_family,
            provider_endpoint_kind: usage.provider_endpoint_kind,
            has_format_conversion: usage.has_format_conversion.unwrap_or(false),
            is_stream: usage.is_stream.unwrap_or(false),
            input_tokens: usage.input_tokens.unwrap_or_default(),
            output_tokens: usage.output_tokens.unwrap_or_default(),
            total_tokens,
            cache_creation_input_tokens: usage.cache_creation_input_tokens.unwrap_or_else(|| {
                existing
                    .map(|existing| existing.cache_creation_input_tokens)
                    .unwrap_or_default()
            }),
            cache_read_input_tokens: usage.cache_read_input_tokens.unwrap_or_else(|| {
                existing
                    .map(|existing| existing.cache_read_input_tokens)
                    .unwrap_or_default()
            }),
            cache_creation_cost_usd: usage.cache_creation_cost_usd.unwrap_or_else(|| {
                existing
                    .map(|existing| existing.cache_creation_cost_usd)
                    .unwrap_or_default()
            }),
            cache_read_cost_usd: usage.cache_read_cost_usd.unwrap_or_else(|| {
                existing
                    .map(|existing| existing.cache_read_cost_usd)
                    .unwrap_or_default()
            }),
            output_price_per_1m: usage
                .output_price_per_1m
                .or_else(|| existing.and_then(|existing| existing.output_price_per_1m)),
            total_cost_usd: usage.total_cost_usd.unwrap_or_else(|| {
                existing
                    .map(|existing| existing.total_cost_usd)
                    .unwrap_or_default()
            }),
            actual_total_cost_usd: usage.actual_total_cost_usd.unwrap_or_else(|| {
                existing
                    .map(|existing| existing.actual_total_cost_usd)
                    .unwrap_or_default()
            }),
            status_code: usage.status_code,
            error_message: usage.error_message,
            error_category: usage.error_category,
            response_time_ms: usage.response_time_ms,
            first_byte_time_ms: usage.first_byte_time_ms,
            status: usage.status,
            billing_status: usage.billing_status,
            request_headers: usage
                .request_headers
                .or_else(|| existing.and_then(|existing| existing.request_headers.clone())),
            request_body: usage
                .request_body
                .or_else(|| existing.and_then(|existing| existing.request_body.clone())),
            provider_request_headers: usage.provider_request_headers.or_else(|| {
                existing.and_then(|existing| existing.provider_request_headers.clone())
            }),
            provider_request_body: usage
                .provider_request_body
                .or_else(|| existing.and_then(|existing| existing.provider_request_body.clone())),
            response_headers: usage
                .response_headers
                .or_else(|| existing.and_then(|existing| existing.response_headers.clone())),
            response_body: usage
                .response_body
                .or_else(|| existing.and_then(|existing| existing.response_body.clone())),
            client_response_headers: usage
                .client_response_headers
                .or_else(|| existing.and_then(|existing| existing.client_response_headers.clone())),
            client_response_body: usage
                .client_response_body
                .or_else(|| existing.and_then(|existing| existing.client_response_body.clone())),
            request_metadata: usage
                .request_metadata
                .or_else(|| existing.and_then(|existing| existing.request_metadata.clone())),
            created_at_unix_ms,
            updated_at_unix_secs: usage.updated_at_unix_secs,
            finalized_at_unix_secs: usage.finalized_at_unix_secs,
        };

        by_request_id.insert(stored.request_id.clone(), stored.clone());
        Ok(stored)
    }
}

#[cfg(test)]
mod tests {
    use super::InMemoryUsageReadRepository;
    use crate::repository::usage::{
        StoredProviderUsageWindow, StoredRequestUsageAudit, UpsertUsageRecord, UsageReadRepository,
        UsageWriteRepository,
    };
    use serde_json::json;

    fn sample_usage(request_id: &str, created_at_unix_ms: i64) -> StoredRequestUsageAudit {
        StoredRequestUsageAudit::new(
            "usage-1".to_string(),
            request_id.to_string(),
            Some("user-1".to_string()),
            Some("api-key-1".to_string()),
            Some("alice".to_string()),
            Some("default".to_string()),
            "OpenAI".to_string(),
            "gpt-4.1".to_string(),
            Some("gpt-4.1-mini".to_string()),
            Some("provider-1".to_string()),
            Some("endpoint-1".to_string()),
            Some("provider-key-1".to_string()),
            Some("chat".to_string()),
            Some("openai:chat".to_string()),
            Some("openai".to_string()),
            Some("chat".to_string()),
            Some("openai:chat".to_string()),
            Some("openai".to_string()),
            Some("chat".to_string()),
            true,
            false,
            100,
            50,
            150,
            0.12,
            0.18,
            Some(200),
            None,
            None,
            Some(420),
            Some(120),
            "completed".to_string(),
            "settled".to_string(),
            created_at_unix_ms,
            created_at_unix_ms + 1,
            Some(created_at_unix_ms + 2),
        )
        .expect("usage should build")
    }

    #[tokio::test]
    async fn finds_usage_by_request_id() {
        let repository = InMemoryUsageReadRepository::seed(vec![
            sample_usage("req-1", 100),
            sample_usage("req-2", 200),
        ]);

        let usage = repository
            .find_by_request_id("req-2")
            .await
            .expect("find should succeed")
            .expect("usage should exist");

        assert_eq!(usage.request_id, "req-2");
        assert_eq!(usage.total_tokens, 150);
    }

    #[tokio::test]
    async fn upsert_writes_usage_record() {
        let repository = InMemoryUsageReadRepository::default();
        let stored = repository
            .upsert(UpsertUsageRecord {
                request_id: "req-upsert-1".to_string(),
                user_id: Some("user-1".to_string()),
                api_key_id: Some("key-1".to_string()),
                username: None,
                api_key_name: None,
                provider_name: "OpenAI".to_string(),
                model: "gpt-5".to_string(),
                target_model: Some("gpt-5-mini".to_string()),
                provider_id: Some("provider-1".to_string()),
                provider_endpoint_id: Some("endpoint-1".to_string()),
                provider_api_key_id: Some("provider-key-1".to_string()),
                request_type: Some("chat".to_string()),
                api_format: Some("openai:chat".to_string()),
                api_family: Some("openai".to_string()),
                endpoint_kind: Some("chat".to_string()),
                endpoint_api_format: Some("openai:chat".to_string()),
                provider_api_family: Some("openai".to_string()),
                provider_endpoint_kind: Some("chat".to_string()),
                has_format_conversion: Some(false),
                is_stream: Some(true),
                input_tokens: Some(10),
                output_tokens: Some(20),
                total_tokens: None,
                cache_creation_input_tokens: None,
                cache_read_input_tokens: None,
                cache_creation_cost_usd: None,
                cache_read_cost_usd: None,
                output_price_per_1m: None,
                total_cost_usd: Some(0.25),
                actual_total_cost_usd: Some(0.15),
                status_code: Some(200),
                error_message: None,
                error_category: None,
                response_time_ms: Some(300),
                first_byte_time_ms: Some(120),
                status: "completed".to_string(),
                billing_status: "pending".to_string(),
                request_headers: Some(json!({"authorization": "Bearer test"})),
                request_body: Some(json!({"model": "gpt-5"})),
                provider_request_headers: None,
                provider_request_body: None,
                response_headers: None,
                response_body: None,
                client_response_headers: None,
                client_response_body: None,
                request_metadata: None,
                finalized_at_unix_secs: None,
                created_at_unix_ms: Some(100),
                updated_at_unix_secs: 101,
            })
            .await
            .expect("upsert should succeed");

        assert_eq!(stored.request_id, "req-upsert-1");
        assert_eq!(stored.total_tokens, 30);
        assert_eq!(stored.total_cost_usd, 0.25);
        assert_eq!(stored.actual_total_cost_usd, 0.15);
        assert_eq!(
            repository
                .find_by_request_id("req-upsert-1")
                .await
                .expect("find should succeed")
                .expect("usage should exist")
                .model,
            "gpt-5"
        );
    }

    #[tokio::test]
    async fn upsert_defaults_created_at_to_second_timestamp() {
        let repository = InMemoryUsageReadRepository::default();
        let stored = repository
            .upsert(UpsertUsageRecord {
                request_id: "req-upsert-ms-default".to_string(),
                user_id: None,
                api_key_id: None,
                username: None,
                api_key_name: None,
                provider_name: "OpenAI".to_string(),
                model: "gpt-5".to_string(),
                target_model: None,
                provider_id: None,
                provider_endpoint_id: None,
                provider_api_key_id: None,
                request_type: None,
                api_format: None,
                api_family: None,
                endpoint_kind: None,
                endpoint_api_format: None,
                provider_api_family: None,
                provider_endpoint_kind: None,
                has_format_conversion: None,
                is_stream: None,
                input_tokens: None,
                output_tokens: None,
                total_tokens: None,
                cache_creation_input_tokens: None,
                cache_read_input_tokens: None,
                cache_creation_cost_usd: None,
                cache_read_cost_usd: None,
                output_price_per_1m: None,
                total_cost_usd: None,
                actual_total_cost_usd: None,
                status_code: None,
                error_message: None,
                error_category: None,
                response_time_ms: None,
                first_byte_time_ms: None,
                status: "completed".to_string(),
                billing_status: "pending".to_string(),
                request_headers: None,
                request_body: None,
                provider_request_headers: None,
                provider_request_body: None,
                response_headers: None,
                response_body: None,
                client_response_headers: None,
                client_response_body: None,
                request_metadata: None,
                finalized_at_unix_secs: None,
                created_at_unix_ms: None,
                updated_at_unix_secs: 101,
            })
            .await
            .expect("upsert should succeed");

        assert_eq!(stored.created_at_unix_ms, 101);
    }

    #[tokio::test]
    async fn summarizes_provider_usage_windows_since_timestamp() {
        let repository = InMemoryUsageReadRepository::default().with_provider_usage_windows(vec![
            StoredProviderUsageWindow::new(
                "provider-1".to_string(),
                1_700_000_000,
                10,
                9,
                1,
                120.0,
                1.25,
            )
            .expect("window should build"),
            StoredProviderUsageWindow::new(
                "provider-1".to_string(),
                1_700_003_600,
                6,
                5,
                1,
                180.0,
                0.75,
            )
            .expect("window should build"),
            StoredProviderUsageWindow::new(
                "provider-2".to_string(),
                1_700_003_600,
                99,
                99,
                0,
                50.0,
                5.0,
            )
            .expect("window should build"),
        ]);

        let summary = repository
            .summarize_provider_usage_since("provider-1", 1_700_000_100)
            .await
            .expect("summary should succeed");

        assert_eq!(summary.total_requests, 6);
        assert_eq!(summary.successful_requests, 5);
        assert_eq!(summary.failed_requests, 1);
        assert_eq!(summary.avg_response_time_ms, 180.0);
        assert_eq!(summary.total_cost_usd, 0.75);
    }

    #[tokio::test]
    async fn list_usage_audits_applies_second_based_time_filters() {
        let repository = InMemoryUsageReadRepository::seed(vec![
            sample_usage("req-1", 1),
            sample_usage("req-2", 2),
            sample_usage("req-3", 3),
        ]);

        let items = repository
            .list_usage_audits(&crate::repository::usage::UsageAuditListQuery {
                created_from_unix_secs: Some(2),
                created_until_unix_secs: Some(3),
                ..Default::default()
            })
            .await
            .expect("list should succeed");

        assert_eq!(items.len(), 1);
        assert_eq!(items[0].request_id, "req-2");
    }

    #[tokio::test]
    async fn summarizes_provider_api_key_last_used_at_in_seconds() {
        let repository = InMemoryUsageReadRepository::seed(vec![
            sample_usage("req-1", 1_999),
            sample_usage("req-2", 2_500),
        ]);

        let summary = repository
            .summarize_usage_by_provider_api_key_ids(&["provider-key-1".to_string()])
            .await
            .expect("summary should succeed");

        let usage = summary
            .get("provider-key-1")
            .expect("provider key summary should exist");
        assert_eq!(usage.request_count, 2);
        assert_eq!(usage.last_used_at_unix_secs, Some(2_500));
    }
}
