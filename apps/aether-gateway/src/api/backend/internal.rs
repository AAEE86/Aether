use axum::routing::{any, get, post};
use axum::Router;

use crate::gateway::{
    proxy_request, proxy_tunnel, relay_request, AppState, PROXY_TUNNEL_PATH, TUNNEL_HEARTBEAT_PATH,
    TUNNEL_NODE_STATUS_PATH, TUNNEL_RELAY_PATH_PATTERN,
};

pub(crate) fn mount_internal_routes(router: Router<AppState>) -> Router<AppState> {
    router
        .route(
            "/api/internal/gateway/{*internal_gateway_path}",
            any(proxy_request),
        )
        .route(PROXY_TUNNEL_PATH, get(proxy_tunnel))
        .route(TUNNEL_HEARTBEAT_PATH, post(proxy_request))
        .route(TUNNEL_NODE_STATUS_PATH, post(proxy_request))
        .route(TUNNEL_RELAY_PATH_PATTERN, post(relay_request))
}
