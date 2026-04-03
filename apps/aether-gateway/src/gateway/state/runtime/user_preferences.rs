use crate::gateway::{AppState, GatewayError};

impl AppState {
    pub(crate) async fn read_user_preferences(
        &self,
        user_id: &str,
    ) -> Result<Option<crate::gateway::gateway_data::StoredUserPreferenceRecord>, GatewayError>
    {
        self.data
            .read_user_preferences(user_id)
            .await
            .map_err(|err| GatewayError::Internal(err.to_string()))
    }

    pub(crate) async fn write_user_preferences(
        &self,
        preferences: &crate::gateway::gateway_data::StoredUserPreferenceRecord,
    ) -> Result<Option<crate::gateway::gateway_data::StoredUserPreferenceRecord>, GatewayError>
    {
        self.data
            .write_user_preferences(preferences)
            .await
            .map_err(|err| GatewayError::Internal(err.to_string()))
    }
}
