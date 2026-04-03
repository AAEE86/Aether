use crate::gateway::{AppState, GatewayError, GatewayPublicRequestContext};
use axum::{body::Body, response::Response};

#[path = "video_tasks/builders.rs"]
mod video_tasks_builders;
#[path = "video_tasks/routes.rs"]
mod video_tasks_routes;

pub(crate) async fn maybe_build_local_admin_video_tasks_response(
    state: &AppState,
    request_context: &GatewayPublicRequestContext,
) -> Result<Option<Response<Body>>, GatewayError> {
    video_tasks_routes::maybe_build_local_admin_video_tasks_response(state, request_context).await
}
