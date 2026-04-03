use axum::routing::any;
use axum::Router;

use super::{
    api, middleware, prometheus_response, proxy_request, AppState, ConcurrencyError,
    DistributedConcurrencyError,
};

pub fn build_router() -> Result<Router, reqwest::Error> {
    Ok(build_router_with_state(AppState::new()?))
}

pub fn build_router_with_state(state: AppState) -> Router {
    let cors_state = state.clone();
    let mut router = Router::<AppState>::new();
    router = api::mount_core_routes(router);
    router = api::mount_operational_routes(router);
    router = api::mount_ai_routes(router);
    router = api::mount_public_support_routes(router);
    router = api::mount_oauth_routes(router);
    router = api::mount_internal_routes(router);
    router = api::mount_admin_routes(router);
    let mut router = router
        .route("/{*path}", any(proxy_request))
        .with_state(state);
    if cors_state.frontdoor_cors().is_some() {
        router = router.layer(axum::middleware::from_fn_with_state(
            cors_state,
            middleware::frontdoor_cors_middleware,
        ));
    }
    router
}

pub(crate) async fn metrics(
    axum::extract::State(state): axum::extract::State<AppState>,
) -> impl axum::response::IntoResponse {
    prometheus_response(&state.metric_samples().await)
}

#[derive(Debug)]
pub(crate) enum RequestAdmissionError {
    Local(ConcurrencyError),
    Distributed(DistributedConcurrencyError),
}

pub async fn serve_tcp(bind: &str) -> Result<(), Box<dyn std::error::Error>> {
    let listener = tokio::net::TcpListener::bind(bind).await?;
    let router = build_router()?;
    axum::serve(
        listener,
        router.into_make_service_with_connect_info::<std::net::SocketAddr>(),
    )
    .await?;
    Ok(())
}
