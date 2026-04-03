use crate::gateway::{AppState, GatewayError};

impl AppState {
    pub(crate) async fn list_minimal_candidate_selection_rows_for_api_format(
        &self,
        api_format: &str,
    ) -> Result<
        Vec<aether_data::repository::candidate_selection::StoredMinimalCandidateSelectionRow>,
        GatewayError,
    > {
        self.data
            .list_minimal_candidate_selection_rows_for_api_format(api_format)
            .await
            .map_err(|err| GatewayError::Internal(err.to_string()))
    }

    pub(crate) async fn list_minimal_candidate_selection_rows_for_api_format_and_global_model(
        &self,
        api_format: &str,
        global_model_name: &str,
    ) -> Result<
        Vec<aether_data::repository::candidate_selection::StoredMinimalCandidateSelectionRow>,
        GatewayError,
    > {
        self.data
            .list_minimal_candidate_selection_rows(api_format, global_model_name)
            .await
            .map_err(|err| GatewayError::Internal(err.to_string()))
    }

    pub(crate) async fn read_provider_quota_snapshot(
        &self,
        provider_id: &str,
    ) -> Result<Option<aether_data::repository::quota::StoredProviderQuotaSnapshot>, GatewayError>
    {
        self.data
            .find_provider_quota_by_provider_id(provider_id)
            .await
            .map_err(|err| GatewayError::Internal(err.to_string()))
    }

    pub(crate) async fn read_recent_request_candidates(
        &self,
        limit: usize,
    ) -> Result<Vec<aether_data::repository::candidates::StoredRequestCandidate>, GatewayError>
    {
        self.data
            .list_recent_request_candidates(limit)
            .await
            .map_err(|err| GatewayError::Internal(err.to_string()))
    }

    pub(crate) async fn upsert_request_candidate(
        &self,
        candidate: aether_data::repository::candidates::UpsertRequestCandidateRecord,
    ) -> Result<Option<aether_data::repository::candidates::StoredRequestCandidate>, GatewayError>
    {
        self.data
            .upsert_request_candidate(candidate)
            .await
            .map_err(|err| GatewayError::Internal(err.to_string()))
    }
}
