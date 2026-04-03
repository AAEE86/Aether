use crate::gateway::{AppState, GatewayError};

impl AppState {
    pub(crate) async fn read_request_candidates_by_request_id(
        &self,
        request_id: &str,
    ) -> Result<Vec<aether_data::repository::candidates::StoredRequestCandidate>, GatewayError>
    {
        self.data
            .list_request_candidates_by_request_id(request_id)
            .await
            .map_err(|err| GatewayError::Internal(err.to_string()))
    }

    pub(crate) async fn read_request_candidates_by_provider_id(
        &self,
        provider_id: &str,
        limit: usize,
    ) -> Result<Vec<aether_data::repository::candidates::StoredRequestCandidate>, GatewayError>
    {
        self.data
            .list_request_candidates_by_provider_id(provider_id, limit)
            .await
            .map_err(|err| GatewayError::Internal(err.to_string()))
    }

    pub(crate) async fn summarize_provider_usage_since(
        &self,
        provider_id: &str,
        since_unix_secs: u64,
    ) -> Result<aether_data::repository::usage::StoredProviderUsageSummary, GatewayError> {
        self.data
            .summarize_provider_usage_since(provider_id, since_unix_secs)
            .await
            .map_err(|err| GatewayError::Internal(err.to_string()))
    }

    pub(crate) async fn list_usage_audits(
        &self,
        query: &aether_data::repository::usage::UsageAuditListQuery,
    ) -> Result<Vec<aether_data::repository::usage::StoredRequestUsageAudit>, GatewayError> {
        self.data
            .list_usage_audits(query)
            .await
            .map_err(|err| GatewayError::Internal(err.to_string()))
    }

    pub(crate) async fn list_recent_usage_audits(
        &self,
        user_id: Option<&str>,
        limit: usize,
    ) -> Result<Vec<aether_data::repository::usage::StoredRequestUsageAudit>, GatewayError> {
        self.data
            .list_recent_usage_audits(user_id, limit)
            .await
            .map_err(|err| GatewayError::Internal(err.to_string()))
    }

    pub(crate) async fn summarize_usage_total_tokens_by_api_key_ids(
        &self,
        api_key_ids: &[String],
    ) -> Result<std::collections::BTreeMap<String, u64>, GatewayError> {
        self.data
            .summarize_usage_total_tokens_by_api_key_ids(api_key_ids)
            .await
            .map_err(|err| GatewayError::Internal(err.to_string()))
    }

    pub(crate) async fn list_users_by_ids(
        &self,
        user_ids: &[String],
    ) -> Result<Vec<aether_data::repository::users::StoredUserSummary>, GatewayError> {
        self.data
            .list_users_by_ids(user_ids)
            .await
            .map_err(|err| GatewayError::Internal(err.to_string()))
    }
}
