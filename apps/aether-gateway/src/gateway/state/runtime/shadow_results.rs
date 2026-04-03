use crate::gateway::{AppState, GatewayError};

impl AppState {
    pub(crate) async fn record_shadow_result_sample(
        &self,
        sample: aether_data::repository::shadow_results::RecordShadowResultSample,
    ) -> Result<Option<aether_data::repository::shadow_results::StoredShadowResult>, GatewayError>
    {
        self.data
            .record_shadow_result_sample(sample)
            .await
            .map_err(|err| GatewayError::Internal(err.to_string()))
    }

    pub(crate) async fn list_recent_shadow_results(
        &self,
        limit: usize,
    ) -> Result<Vec<aether_data::repository::shadow_results::StoredShadowResult>, GatewayError>
    {
        self.data
            .list_recent_shadow_results(limit)
            .await
            .map_err(|err| GatewayError::Internal(err.to_string()))
    }
}
