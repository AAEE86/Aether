use super::{AppState, GatewayError};

use super::super::async_task;
use super::super::async_task::video as video_tasks;

impl AppState {
    pub(crate) async fn read_data_backed_video_task_response(
        &self,
        route_family: Option<&str>,
        request_path: &str,
    ) -> Result<Option<video_tasks::LocalVideoTaskReadResponse>, GatewayError> {
        self.data
            .read_video_task_response(route_family, request_path)
            .await
            .map_err(|err| GatewayError::Internal(err.to_string()))
    }

    pub(crate) async fn find_video_task_by_id(
        &self,
        task_id: &str,
    ) -> Result<Option<aether_data::repository::video_tasks::StoredVideoTask>, GatewayError> {
        self.data
            .find_video_task(aether_data::repository::video_tasks::VideoTaskLookupKey::Id(task_id))
            .await
            .map_err(|err| GatewayError::Internal(err.to_string()))
    }

    pub(crate) async fn find_video_task_by_short_id(
        &self,
        short_id: &str,
    ) -> Result<Option<aether_data::repository::video_tasks::StoredVideoTask>, GatewayError> {
        self.data
            .find_video_task(
                aether_data::repository::video_tasks::VideoTaskLookupKey::ShortId(short_id),
            )
            .await
            .map_err(|err| GatewayError::Internal(err.to_string()))
    }

    pub(crate) async fn upsert_video_task_snapshot(
        &self,
        snapshot: &video_tasks::LocalVideoTaskSnapshot,
    ) -> Result<Option<aether_data::repository::video_tasks::StoredVideoTask>, GatewayError> {
        self.data
            .upsert_video_task(snapshot.to_upsert_record())
            .await
            .map_err(|err| GatewayError::Internal(err.to_string()))
    }

    pub(crate) async fn hydrate_video_task_for_route(
        &self,
        route_family: Option<&str>,
        request_path: &str,
    ) -> Result<bool, GatewayError> {
        let lookup = match route_family {
            Some("openai") => video_tasks::extract_openai_task_id_from_path(request_path)
                .or_else(|| video_tasks::extract_openai_task_id_from_cancel_path(request_path))
                .or_else(|| video_tasks::extract_openai_task_id_from_remix_path(request_path))
                .or_else(|| video_tasks::extract_openai_task_id_from_content_path(request_path))
                .map(aether_data::repository::video_tasks::VideoTaskLookupKey::Id),
            Some("gemini") => video_tasks::extract_gemini_short_id_from_path(request_path)
                .or_else(|| video_tasks::extract_gemini_short_id_from_cancel_path(request_path))
                .map(aether_data::repository::video_tasks::VideoTaskLookupKey::ShortId),
            _ => None,
        };
        let Some(lookup) = lookup else {
            return Ok(false);
        };
        let Some(task) = self
            .data
            .find_video_task(lookup)
            .await
            .map_err(|err| GatewayError::Internal(err.to_string()))?
        else {
            return Ok(false);
        };
        if self.video_tasks.hydrate_from_stored_task(&task) {
            return Ok(true);
        }

        let Some(snapshot) = self.reconstruct_video_task_snapshot(&task).await? else {
            return Ok(false);
        };
        self.video_tasks.record_snapshot(snapshot);
        Ok(true)
    }

    pub(in crate::gateway) async fn reconstruct_video_task_snapshot(
        &self,
        task: &aether_data::repository::video_tasks::StoredVideoTask,
    ) -> Result<Option<video_tasks::LocalVideoTaskSnapshot>, GatewayError> {
        let provider_api_format = task
            .provider_api_format
            .as_deref()
            .unwrap_or_default()
            .trim();
        if !matches!(provider_api_format, "openai:video" | "gemini:video") {
            return Ok(None);
        }

        let Some(provider_id) = task.provider_id.as_deref() else {
            return Ok(None);
        };
        let Some(endpoint_id) = task.endpoint_id.as_deref() else {
            return Ok(None);
        };
        let Some(key_id) = task.key_id.as_deref() else {
            return Ok(None);
        };

        let Some(transport) = self
            .data
            .read_provider_transport_snapshot(provider_id, endpoint_id, key_id)
            .await
            .map_err(|err| GatewayError::Internal(err.to_string()))?
        else {
            return Ok(None);
        };

        Ok(video_tasks::LocalVideoTaskSnapshot::from_stored_task_with_transport(task, &transport))
    }

    pub(crate) async fn claim_due_video_tasks(
        &self,
        now_unix_secs: u64,
        claim_until_unix_secs: u64,
        limit: usize,
    ) -> Result<Vec<aether_data::repository::video_tasks::StoredVideoTask>, GatewayError> {
        self.data
            .claim_due_video_tasks(now_unix_secs, claim_until_unix_secs, limit)
            .await
            .map_err(|err| GatewayError::Internal(err.to_string()))
    }

    pub(crate) async fn update_active_video_task(
        &self,
        task: aether_data::repository::video_tasks::UpsertVideoTask,
    ) -> Result<Option<aether_data::repository::video_tasks::StoredVideoTask>, GatewayError> {
        self.data
            .update_active_video_task(task)
            .await
            .map_err(|err| GatewayError::Internal(err.to_string()))
    }

    pub(crate) async fn list_video_task_page(
        &self,
        filter: &aether_data::repository::video_tasks::VideoTaskQueryFilter,
        offset: usize,
        limit: usize,
    ) -> Result<Vec<aether_data::repository::video_tasks::StoredVideoTask>, GatewayError> {
        self.data
            .list_video_task_page(filter, offset, limit)
            .await
            .map_err(|err| GatewayError::Internal(err.to_string()))
    }

    pub(crate) async fn count_video_tasks(
        &self,
        filter: &aether_data::repository::video_tasks::VideoTaskQueryFilter,
    ) -> Result<u64, GatewayError> {
        self.data
            .count_video_tasks(filter)
            .await
            .map_err(|err| GatewayError::Internal(err.to_string()))
    }

    pub(crate) async fn count_video_tasks_by_status(
        &self,
        filter: &aether_data::repository::video_tasks::VideoTaskQueryFilter,
    ) -> Result<Vec<aether_data::repository::video_tasks::VideoTaskStatusCount>, GatewayError> {
        self.data
            .count_video_tasks_by_status(filter)
            .await
            .map_err(|err| GatewayError::Internal(err.to_string()))
    }

    pub(crate) async fn count_distinct_video_task_users(
        &self,
        filter: &aether_data::repository::video_tasks::VideoTaskQueryFilter,
    ) -> Result<u64, GatewayError> {
        self.data
            .count_distinct_video_task_users(filter)
            .await
            .map_err(|err| GatewayError::Internal(err.to_string()))
    }

    pub(crate) async fn top_video_task_models(
        &self,
        filter: &aether_data::repository::video_tasks::VideoTaskQueryFilter,
        limit: usize,
    ) -> Result<Vec<aether_data::repository::video_tasks::VideoTaskModelCount>, GatewayError> {
        self.data
            .top_video_task_models(filter, limit)
            .await
            .map_err(|err| GatewayError::Internal(err.to_string()))
    }

    pub(crate) async fn count_video_tasks_created_since(
        &self,
        filter: &aether_data::repository::video_tasks::VideoTaskQueryFilter,
        created_since_unix_secs: u64,
    ) -> Result<u64, GatewayError> {
        self.data
            .count_video_tasks_created_since(filter, created_since_unix_secs)
            .await
            .map_err(|err| GatewayError::Internal(err.to_string()))
    }

    pub(crate) async fn execute_video_task_refresh_plan(
        &self,
        refresh_plan: &video_tasks::LocalVideoTaskReadRefreshPlan,
    ) -> Result<bool, GatewayError> {
        async_task::execute_video_task_refresh_plan(self, refresh_plan).await
    }
}
