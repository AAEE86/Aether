use super::{
    merge_shadow_result_sample, read_decision_trace, read_minimal_candidate_selection,
    read_provider_transport_snapshot, read_request_audit_bundle, read_request_candidate_trace,
    read_request_usage_audit, read_video_task_response, AnnouncementListQuery,
    CreateAnnouncementRecord, DataLayerError, DecisionTrace, GatewayDataState,
    GatewayMinimalCandidateSelectionCandidate, GatewayProviderTransportSnapshot,
    LocalVideoTaskReadResponse, RecordShadowResultSample, RedisStreamRunner, RequestAuditBundle,
    RequestCandidateTrace, RequestUsageAudit, ShadowResultLookupKey, StoredAnnouncement,
    StoredAnnouncementPage, StoredBillingModelContext, StoredGatewayAuthApiKeySnapshot,
    StoredProviderQuotaSnapshot, StoredProviderUsageSummary, StoredRequestUsageAudit,
    StoredShadowResult, StoredUsageSettlement, StoredUserAuthRecord, StoredUserExportRow,
    StoredUserSummary, StoredVideoTask, StoredWalletSnapshot, UpdateAnnouncementRecord,
    UpsertUsageRecord, UpsertVideoTask, UsageSettlementInput, VideoTaskLookupKey,
    VideoTaskModelCount, VideoTaskQueryFilter, VideoTaskStatusCount, WalletLookupKey,
};

fn is_missing_shadow_results_relation_error(error: &DataLayerError) -> bool {
    error
        .to_string()
        .contains("relation \"gateway_shadow_results\" does not exist")
}

impl GatewayDataState {
    pub(crate) async fn list_announcements(
        &self,
        query: &AnnouncementListQuery,
    ) -> Result<StoredAnnouncementPage, DataLayerError> {
        match &self.announcement_reader {
            Some(repository) => repository.list_announcements(query).await,
            None => Ok(StoredAnnouncementPage::default()),
        }
    }

    pub(crate) async fn find_announcement_by_id(
        &self,
        announcement_id: &str,
    ) -> Result<Option<StoredAnnouncement>, DataLayerError> {
        match &self.announcement_reader {
            Some(repository) => repository.find_by_id(announcement_id).await,
            None => Ok(None),
        }
    }

    pub(crate) async fn count_unread_active_announcements(
        &self,
        user_id: &str,
        now_unix_secs: u64,
    ) -> Result<u64, DataLayerError> {
        match &self.announcement_reader {
            Some(repository) => {
                repository
                    .count_unread_active_announcements(user_id, now_unix_secs)
                    .await
            }
            None => Ok(0),
        }
    }

    pub(crate) async fn create_announcement(
        &self,
        record: CreateAnnouncementRecord,
    ) -> Result<Option<StoredAnnouncement>, DataLayerError> {
        match &self.announcement_writer {
            Some(repository) => repository.create_announcement(record).await.map(Some),
            None => Ok(None),
        }
    }

    pub(crate) async fn update_announcement(
        &self,
        record: UpdateAnnouncementRecord,
    ) -> Result<Option<StoredAnnouncement>, DataLayerError> {
        match &self.announcement_writer {
            Some(repository) => repository.update_announcement(record).await,
            None => Ok(None),
        }
    }

    pub(crate) async fn delete_announcement(
        &self,
        announcement_id: &str,
    ) -> Result<bool, DataLayerError> {
        match &self.announcement_writer {
            Some(repository) => repository.delete_announcement(announcement_id).await,
            None => Ok(false),
        }
    }

    pub(crate) async fn mark_announcement_as_read(
        &self,
        user_id: &str,
        announcement_id: &str,
        read_at_unix_secs: u64,
    ) -> Result<bool, DataLayerError> {
        match &self.announcement_writer {
            Some(repository) => {
                repository
                    .mark_announcement_as_read(user_id, announcement_id, read_at_unix_secs)
                    .await
            }
            None => Ok(false),
        }
    }

    pub(crate) async fn find_video_task(
        &self,
        key: VideoTaskLookupKey<'_>,
    ) -> Result<Option<StoredVideoTask>, DataLayerError> {
        match &self.video_task_reader {
            Some(repository) => repository.find(key).await,
            None => Ok(None),
        }
    }

    pub(crate) async fn list_video_task_page(
        &self,
        filter: &VideoTaskQueryFilter,
        offset: usize,
        limit: usize,
    ) -> Result<Vec<StoredVideoTask>, DataLayerError> {
        match &self.video_task_reader {
            Some(repository) => repository.list_page(filter, offset, limit).await,
            None => Ok(Vec::new()),
        }
    }

    pub(crate) async fn count_video_tasks(
        &self,
        filter: &VideoTaskQueryFilter,
    ) -> Result<u64, DataLayerError> {
        match &self.video_task_reader {
            Some(repository) => repository.count(filter).await,
            None => Ok(0),
        }
    }

    pub(crate) async fn count_video_tasks_by_status(
        &self,
        filter: &VideoTaskQueryFilter,
    ) -> Result<Vec<VideoTaskStatusCount>, DataLayerError> {
        match &self.video_task_reader {
            Some(repository) => repository.count_by_status(filter).await,
            None => Ok(Vec::new()),
        }
    }

    pub(crate) async fn count_distinct_video_task_users(
        &self,
        filter: &VideoTaskQueryFilter,
    ) -> Result<u64, DataLayerError> {
        match &self.video_task_reader {
            Some(repository) => repository.count_distinct_users(filter).await,
            None => Ok(0),
        }
    }

    pub(crate) async fn top_video_task_models(
        &self,
        filter: &VideoTaskQueryFilter,
        limit: usize,
    ) -> Result<Vec<VideoTaskModelCount>, DataLayerError> {
        match &self.video_task_reader {
            Some(repository) => repository.top_models(filter, limit).await,
            None => Ok(Vec::new()),
        }
    }

    pub(crate) async fn count_video_tasks_created_since(
        &self,
        filter: &VideoTaskQueryFilter,
        created_since_unix_secs: u64,
    ) -> Result<u64, DataLayerError> {
        match &self.video_task_reader {
            Some(repository) => {
                repository
                    .count_created_since(filter, created_since_unix_secs)
                    .await
            }
            None => Ok(0),
        }
    }

    pub(crate) async fn upsert_video_task(
        &self,
        task: UpsertVideoTask,
    ) -> Result<Option<StoredVideoTask>, DataLayerError> {
        match &self.video_task_writer {
            Some(repository) => repository.upsert(task).await.map(Some),
            None => Ok(None),
        }
    }

    pub(crate) async fn update_active_video_task(
        &self,
        task: UpsertVideoTask,
    ) -> Result<Option<StoredVideoTask>, DataLayerError> {
        match &self.video_task_writer {
            Some(repository) => repository.update_if_active(task).await,
            None => Ok(None),
        }
    }

    pub(crate) async fn claim_due_video_tasks(
        &self,
        now_unix_secs: u64,
        claim_until_unix_secs: u64,
        limit: usize,
    ) -> Result<Vec<StoredVideoTask>, DataLayerError> {
        match &self.video_task_writer {
            Some(repository) => {
                repository
                    .claim_due(now_unix_secs, claim_until_unix_secs, limit)
                    .await
            }
            None => Ok(Vec::new()),
        }
    }

    pub(crate) async fn find_wallet(
        &self,
        key: WalletLookupKey<'_>,
    ) -> Result<Option<StoredWalletSnapshot>, DataLayerError> {
        match &self.wallet_reader {
            Some(repository) => repository.find(key).await,
            None => Ok(None),
        }
    }

    pub(crate) async fn list_wallets_by_api_key_ids(
        &self,
        api_key_ids: &[String],
    ) -> Result<Vec<StoredWalletSnapshot>, DataLayerError> {
        match &self.wallet_reader {
            Some(repository) => repository.list_wallets_by_api_key_ids(api_key_ids).await,
            None => Ok(Vec::new()),
        }
    }

    pub(crate) async fn list_wallets_by_user_ids(
        &self,
        user_ids: &[String],
    ) -> Result<Vec<StoredWalletSnapshot>, DataLayerError> {
        match &self.wallet_reader {
            Some(repository) => repository.list_wallets_by_user_ids(user_ids).await,
            None => Ok(Vec::new()),
        }
    }

    pub(crate) async fn settle_usage(
        &self,
        input: UsageSettlementInput,
    ) -> Result<Option<StoredUsageSettlement>, DataLayerError> {
        match &self.wallet_writer {
            Some(repository) => repository.settle_usage(input).await,
            None => Ok(None),
        }
    }

    pub(crate) async fn reset_due_provider_quotas(
        &self,
        now_unix_secs: u64,
    ) -> Result<usize, DataLayerError> {
        match &self.provider_quota_writer {
            Some(repository) => repository.reset_due(now_unix_secs).await,
            None => Ok(0),
        }
    }

    pub(crate) async fn find_provider_quota_by_provider_id(
        &self,
        provider_id: &str,
    ) -> Result<Option<StoredProviderQuotaSnapshot>, DataLayerError> {
        match &self.provider_quota_reader {
            Some(repository) => repository.find_by_provider_id(provider_id).await,
            None => Ok(None),
        }
    }

    #[allow(dead_code)]

    pub(crate) async fn upsert_usage(
        &self,
        usage: UpsertUsageRecord,
    ) -> Result<Option<StoredRequestUsageAudit>, DataLayerError> {
        match &self.usage_writer {
            Some(repository) => repository.upsert(usage).await.map(Some),
            None => Ok(None),
        }
    }

    pub(in crate::gateway) async fn find_request_usage_by_request_id(
        &self,
        request_id: &str,
    ) -> Result<Option<StoredRequestUsageAudit>, DataLayerError> {
        match &self.usage_reader {
            Some(repository) => repository.find_by_request_id(request_id).await,
            None => Ok(None),
        }
    }

    pub(in crate::gateway) async fn find_request_usage_by_id(
        &self,
        usage_id: &str,
    ) -> Result<Option<StoredRequestUsageAudit>, DataLayerError> {
        match &self.usage_reader {
            Some(repository) => repository.find_by_id(usage_id).await,
            None => Ok(None),
        }
    }

    pub(crate) async fn list_usage_audits(
        &self,
        query: &aether_data::repository::usage::UsageAuditListQuery,
    ) -> Result<Vec<StoredRequestUsageAudit>, DataLayerError> {
        match &self.usage_reader {
            Some(repository) => repository.list_usage_audits(query).await,
            None => Ok(Vec::new()),
        }
    }

    pub(crate) async fn list_recent_usage_audits(
        &self,
        user_id: Option<&str>,
        limit: usize,
    ) -> Result<Vec<StoredRequestUsageAudit>, DataLayerError> {
        match &self.usage_reader {
            Some(repository) => repository.list_recent_usage_audits(user_id, limit).await,
            None => Ok(Vec::new()),
        }
    }

    pub(crate) async fn summarize_usage_total_tokens_by_api_key_ids(
        &self,
        api_key_ids: &[String],
    ) -> Result<std::collections::BTreeMap<String, u64>, DataLayerError> {
        match &self.usage_reader {
            Some(repository) => {
                repository
                    .summarize_total_tokens_by_api_key_ids(api_key_ids)
                    .await
            }
            None => Ok(std::collections::BTreeMap::new()),
        }
    }

    pub(crate) async fn list_users_by_ids(
        &self,
        user_ids: &[String],
    ) -> Result<Vec<StoredUserSummary>, DataLayerError> {
        match &self.user_reader {
            Some(repository) => repository.list_users_by_ids(user_ids).await,
            None => Ok(Vec::new()),
        }
    }

    pub(crate) async fn list_export_users(
        &self,
    ) -> Result<Vec<StoredUserExportRow>, DataLayerError> {
        match &self.user_reader {
            Some(repository) => repository.list_export_users().await,
            None => Ok(Vec::new()),
        }
    }

    pub(crate) async fn list_export_users_page(
        &self,
        query: &aether_data::repository::users::UserExportListQuery,
    ) -> Result<Vec<StoredUserExportRow>, DataLayerError> {
        match &self.user_reader {
            Some(repository) => repository.list_export_users_page(query).await,
            None => Ok(Vec::new()),
        }
    }

    pub(crate) async fn summarize_export_users(
        &self,
    ) -> Result<aether_data::repository::users::UserExportSummary, DataLayerError> {
        match &self.user_reader {
            Some(repository) => repository.summarize_export_users().await,
            None => Ok(aether_data::repository::users::UserExportSummary::default()),
        }
    }

    pub(crate) async fn find_export_user_by_id(
        &self,
        user_id: &str,
    ) -> Result<Option<StoredUserExportRow>, DataLayerError> {
        match &self.user_reader {
            Some(repository) => repository.find_export_user_by_id(user_id).await,
            None => Ok(None),
        }
    }

    pub(crate) async fn list_non_admin_export_users(
        &self,
    ) -> Result<Vec<StoredUserExportRow>, DataLayerError> {
        match &self.user_reader {
            Some(repository) => repository.list_non_admin_export_users().await,
            None => Ok(Vec::new()),
        }
    }

    pub(crate) async fn list_user_auth_by_ids(
        &self,
        user_ids: &[String],
    ) -> Result<Vec<StoredUserAuthRecord>, DataLayerError> {
        match &self.user_reader {
            Some(repository) => repository.list_user_auth_by_ids(user_ids).await,
            None => Ok(Vec::new()),
        }
    }

    pub(crate) async fn summarize_provider_usage_since(
        &self,
        provider_id: &str,
        since_unix_secs: u64,
    ) -> Result<StoredProviderUsageSummary, DataLayerError> {
        match &self.usage_reader {
            Some(repository) => {
                repository
                    .summarize_provider_usage_since(provider_id, since_unix_secs)
                    .await
            }
            None => Ok(StoredProviderUsageSummary::default()),
        }
    }

    pub(crate) fn usage_worker_runner(&self) -> Option<RedisStreamRunner> {
        self.usage_worker_runner.clone()
    }

    pub(in crate::gateway) async fn find_billing_model_context(
        &self,
        provider_id: &str,
        provider_api_key_id: Option<&str>,
        global_model_name: &str,
    ) -> Result<Option<StoredBillingModelContext>, DataLayerError> {
        match &self.billing_reader {
            Some(repository) => {
                repository
                    .find_model_context(provider_id, provider_api_key_id, global_model_name)
                    .await
            }
            None => Ok(None),
        }
    }

    pub(crate) async fn read_request_candidate_trace(
        &self,
        request_id: &str,
        attempted_only: bool,
    ) -> Result<Option<RequestCandidateTrace>, DataLayerError> {
        read_request_candidate_trace(self, request_id, attempted_only).await
    }

    pub(crate) async fn read_decision_trace(
        &self,
        request_id: &str,
        attempted_only: bool,
    ) -> Result<Option<DecisionTrace>, DataLayerError> {
        read_decision_trace(self, request_id, attempted_only).await
    }

    pub(crate) async fn read_request_usage_audit(
        &self,
        request_id: &str,
    ) -> Result<Option<RequestUsageAudit>, DataLayerError> {
        read_request_usage_audit(self, request_id).await
    }

    pub(crate) async fn read_request_audit_bundle(
        &self,
        request_id: &str,
        attempted_only: bool,
        now_unix_secs: u64,
    ) -> Result<Option<RequestAuditBundle>, DataLayerError> {
        read_request_audit_bundle(self, request_id, attempted_only, now_unix_secs).await
    }

    #[allow(dead_code)]
    pub(crate) async fn read_provider_transport_snapshot(
        &self,
        provider_id: &str,
        endpoint_id: &str,
        key_id: &str,
    ) -> Result<Option<GatewayProviderTransportSnapshot>, DataLayerError> {
        read_provider_transport_snapshot(self, provider_id, endpoint_id, key_id).await
    }

    pub(crate) async fn read_minimal_candidate_selection(
        &self,
        api_format: &str,
        global_model_name: &str,
        require_streaming: bool,
        auth_snapshot: Option<&StoredGatewayAuthApiKeySnapshot>,
    ) -> Result<Vec<GatewayMinimalCandidateSelectionCandidate>, DataLayerError> {
        read_minimal_candidate_selection(
            self,
            api_format,
            global_model_name,
            require_streaming,
            auth_snapshot,
        )
        .await
    }

    pub(crate) async fn read_video_task_response(
        &self,
        route_family: Option<&str>,
        request_path: &str,
    ) -> Result<Option<LocalVideoTaskReadResponse>, DataLayerError> {
        read_video_task_response(self, route_family, request_path).await
    }

    #[cfg(test)]
    pub(crate) async fn write_shadow_result(
        &self,
        result: aether_data::repository::shadow_results::UpsertShadowResult,
    ) -> Result<Option<StoredShadowResult>, DataLayerError> {
        match &self.shadow_result_writer {
            Some(repository) => match repository.upsert(result).await {
                Ok(stored) => Ok(Some(stored)),
                Err(err) if is_missing_shadow_results_relation_error(&err) => Ok(None),
                Err(err) => Err(err),
            },
            None => Ok(None),
        }
    }

    pub(crate) async fn record_shadow_result_sample(
        &self,
        sample: RecordShadowResultSample,
    ) -> Result<Option<StoredShadowResult>, DataLayerError> {
        let Some(writer) = &self.shadow_result_writer else {
            return Ok(None);
        };

        let existing = match &self.shadow_result_reader {
            Some(reader) => {
                match reader
                    .find(ShadowResultLookupKey::TraceFingerprint {
                        trace_id: &sample.trace_id,
                        request_fingerprint: &sample.request_fingerprint,
                    })
                    .await
                {
                    Ok(existing) => existing,
                    Err(err) if is_missing_shadow_results_relation_error(&err) => return Ok(None),
                    Err(err) => return Err(err),
                }
            }
            None => None,
        };
        let merged = merge_shadow_result_sample(existing.as_ref(), sample);

        match writer.upsert(merged).await {
            Ok(stored) => Ok(Some(stored)),
            Err(err) if is_missing_shadow_results_relation_error(&err) => Ok(None),
            Err(err) => Err(err),
        }
    }

    pub(crate) async fn list_recent_shadow_results(
        &self,
        limit: usize,
    ) -> Result<Vec<StoredShadowResult>, DataLayerError> {
        match &self.shadow_result_reader {
            Some(repository) => match repository.list_recent(limit).await {
                Ok(results) => Ok(results),
                Err(err) if is_missing_shadow_results_relation_error(&err) => Ok(Vec::new()),
                Err(err) => Err(err),
            },
            None => Ok(Vec::new()),
        }
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use aether_data::repository::users::{InMemoryUserReadRepository, StoredUserExportRow};
    use aether_data::DataLayerError;

    use super::{is_missing_shadow_results_relation_error, GatewayDataState};

    #[tokio::test]
    async fn lists_non_admin_export_users_from_user_reader() {
        let repository = Arc::new(InMemoryUserReadRepository::seed_export_users(vec![
            StoredUserExportRow::new(
                "user-1".to_string(),
                Some("alice@example.com".to_string()),
                true,
                "alice".to_string(),
                Some("hash".to_string()),
                "user".to_string(),
                "local".to_string(),
                Some(serde_json::json!(["openai"])),
                Some(serde_json::json!(["openai:chat"])),
                Some(serde_json::json!(["gpt-4.1"])),
                Some(60),
                Some(serde_json::json!({"gpt-4.1": {"cache_1h": true}})),
                true,
            )
            .expect("user export row should build"),
        ]));
        let state = GatewayDataState::with_user_reader_for_tests(repository);

        let rows = state
            .list_non_admin_export_users()
            .await
            .expect("export users should succeed");

        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].username, "alice");
        assert!(rows[0].email_verified);
        assert_eq!(rows[0].password_hash.as_deref(), Some("hash"));
        assert_eq!(rows[0].allowed_models, Some(vec!["gpt-4.1".to_string()]));
        assert_eq!(
            rows[0].model_capability_settings,
            Some(serde_json::json!({"gpt-4.1": {"cache_1h": true}}))
        );
    }

    #[test]
    fn detects_missing_shadow_results_relation_error_text() {
        let missing_relation = DataLayerError::UnexpectedValue(
            "postgres error: error returned from database: relation \"gateway_shadow_results\" does not exist"
                .to_string(),
        );
        let other = DataLayerError::UnexpectedValue(
            "postgres error: error returned from database: relation \"usage\" does not exist"
                .to_string(),
        );

        assert!(is_missing_shadow_results_relation_error(&missing_relation));
        assert!(!is_missing_shadow_results_relation_error(&other));
    }
}
