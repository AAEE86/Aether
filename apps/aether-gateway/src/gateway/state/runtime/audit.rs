use crate::gateway::{scheduler, usage, AppState, GatewayError};

use super::super::{AUTH_API_KEY_LAST_USED_MAX_ENTRIES, AUTH_API_KEY_LAST_USED_TTL};

impl AppState {
    pub(crate) async fn read_request_candidate_trace(
        &self,
        request_id: &str,
        attempted_only: bool,
    ) -> Result<Option<crate::gateway::gateway_data::RequestCandidateTrace>, GatewayError> {
        self.data
            .read_request_candidate_trace(request_id, attempted_only)
            .await
            .map_err(|err| GatewayError::Internal(err.to_string()))
    }

    pub(crate) async fn read_decision_trace(
        &self,
        request_id: &str,
        attempted_only: bool,
    ) -> Result<Option<crate::gateway::gateway_data::DecisionTrace>, GatewayError> {
        self.data
            .read_decision_trace(request_id, attempted_only)
            .await
            .map_err(|err| GatewayError::Internal(err.to_string()))
    }

    pub(crate) async fn read_request_usage_audit(
        &self,
        request_id: &str,
    ) -> Result<Option<usage::RequestUsageAudit>, GatewayError> {
        self.data
            .read_request_usage_audit(request_id)
            .await
            .map_err(|err| GatewayError::Internal(err.to_string()))
    }

    pub(crate) async fn find_request_usage_by_id(
        &self,
        usage_id: &str,
    ) -> Result<Option<aether_data::repository::usage::StoredRequestUsageAudit>, GatewayError> {
        self.data
            .find_request_usage_by_id(usage_id)
            .await
            .map_err(|err| GatewayError::Internal(err.to_string()))
    }

    pub(crate) async fn read_request_audit_bundle(
        &self,
        request_id: &str,
        attempted_only: bool,
        now_unix_secs: u64,
    ) -> Result<Option<usage::RequestAuditBundle>, GatewayError> {
        self.data
            .read_request_audit_bundle(request_id, attempted_only, now_unix_secs)
            .await
            .map_err(|err| GatewayError::Internal(err.to_string()))
    }

    pub(crate) async fn read_auth_api_key_snapshot(
        &self,
        user_id: &str,
        api_key_id: &str,
        now_unix_secs: u64,
    ) -> Result<Option<crate::gateway::gateway_data::StoredGatewayAuthApiKeySnapshot>, GatewayError>
    {
        self.data
            .read_auth_api_key_snapshot(user_id, api_key_id, now_unix_secs)
            .await
            .map_err(|err| GatewayError::Internal(err.to_string()))
    }

    pub(crate) async fn read_auth_api_key_snapshot_by_key_hash(
        &self,
        key_hash: &str,
        now_unix_secs: u64,
    ) -> Result<Option<crate::gateway::gateway_data::StoredGatewayAuthApiKeySnapshot>, GatewayError>
    {
        self.data
            .read_auth_api_key_snapshot_by_key_hash(key_hash, now_unix_secs)
            .await
            .map_err(|err| GatewayError::Internal(err.to_string()))
    }

    pub(crate) async fn read_auth_api_key_snapshots_by_ids(
        &self,
        api_key_ids: &[String],
    ) -> Result<Vec<aether_data::repository::auth::StoredAuthApiKeySnapshot>, GatewayError> {
        self.data
            .list_auth_api_key_snapshots_by_ids(api_key_ids)
            .await
            .map_err(|err| GatewayError::Internal(err.to_string()))
    }

    pub(crate) fn has_auth_api_key_writer(&self) -> bool {
        self.data.has_auth_api_key_writer()
    }

    pub(crate) async fn touch_auth_api_key_last_used_best_effort(&self, api_key_id: &str) {
        let api_key_id = api_key_id.trim();
        if api_key_id.is_empty() || !self.has_auth_api_key_writer() {
            return;
        }
        if !self.auth_api_key_last_used_cache.should_touch(
            api_key_id,
            AUTH_API_KEY_LAST_USED_TTL,
            AUTH_API_KEY_LAST_USED_MAX_ENTRIES,
        ) {
            return;
        }
        if let Err(err) = self.data.touch_auth_api_key_last_used(api_key_id).await {
            tracing::warn!(
                api_key_id = %api_key_id,
                error = ?err,
                "gateway auth api key last_used_at touch failed"
            );
        }
    }

    pub(crate) async fn read_minimal_candidate_selection(
        &self,
        api_format: &str,
        global_model_name: &str,
        require_streaming: bool,
        auth_snapshot: Option<&crate::gateway::gateway_data::StoredGatewayAuthApiKeySnapshot>,
    ) -> Result<Vec<scheduler::GatewayMinimalCandidateSelectionCandidate>, GatewayError> {
        self.data
            .read_minimal_candidate_selection(
                api_format,
                global_model_name,
                require_streaming,
                auth_snapshot,
            )
            .await
            .map_err(|err| GatewayError::Internal(err.to_string()))
    }
}
