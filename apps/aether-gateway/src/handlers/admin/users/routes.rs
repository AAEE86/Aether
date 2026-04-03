use super::{
    build_admin_create_user_api_key_response, build_admin_create_user_response,
    build_admin_delete_user_api_key_response, build_admin_delete_user_response,
    build_admin_delete_user_session_response, build_admin_delete_user_sessions_response,
    build_admin_get_user_response, build_admin_list_user_api_keys_response,
    build_admin_list_user_sessions_response, build_admin_list_users_response,
    build_admin_reveal_user_api_key_response, build_admin_toggle_user_api_key_lock_response,
    build_admin_update_user_api_key_response, build_admin_update_user_response,
    build_admin_users_data_unavailable_response,
};
use crate::gateway::{AppState, GatewayError, GatewayPublicRequestContext};
use axum::{body::Body, http, response::Response};

fn is_admin_users_route(request_context: &GatewayPublicRequestContext) -> bool {
    let path = request_context.request_path.as_str();
    (request_context.request_method == http::Method::GET
        && matches!(path, "/api/admin/users" | "/api/admin/users/"))
        || (request_context.request_method == http::Method::POST
            && matches!(path, "/api/admin/users" | "/api/admin/users/"))
        || ((request_context.request_method == http::Method::GET
            || request_context.request_method == http::Method::PUT
            || request_context.request_method == http::Method::DELETE)
            && path.starts_with("/api/admin/users/")
            && !path.ends_with("/sessions")
            && !path.contains("/sessions/")
            && !path.ends_with("/api-keys")
            && !path.contains("/api-keys/")
            && path.matches('/').count() == 4)
        || (request_context.request_method == http::Method::GET
            && path.starts_with("/api/admin/users/")
            && path.ends_with("/sessions")
            && path.matches('/').count() == 5)
        || (request_context.request_method == http::Method::DELETE
            && path.starts_with("/api/admin/users/")
            && path.ends_with("/sessions")
            && path.matches('/').count() == 5)
        || (request_context.request_method == http::Method::DELETE
            && path.starts_with("/api/admin/users/")
            && path.contains("/sessions/")
            && path.matches('/').count() == 6)
        || ((request_context.request_method == http::Method::GET
            || request_context.request_method == http::Method::POST)
            && path.starts_with("/api/admin/users/")
            && path.ends_with("/api-keys")
            && path.matches('/').count() == 5)
        || ((request_context.request_method == http::Method::DELETE
            || request_context.request_method == http::Method::PUT)
            && path.starts_with("/api/admin/users/")
            && path.contains("/api-keys/")
            && !path.ends_with("/lock")
            && !path.ends_with("/full-key")
            && path.matches('/').count() == 6)
        || (request_context.request_method == http::Method::PATCH
            && path.starts_with("/api/admin/users/")
            && path.ends_with("/lock")
            && path.matches('/').count() == 7)
        || (request_context.request_method == http::Method::GET
            && path.starts_with("/api/admin/users/")
            && path.ends_with("/full-key")
            && path.matches('/').count() == 7)
}

pub(super) async fn maybe_build_local_admin_users_routes_response(
    state: &AppState,
    request_context: &GatewayPublicRequestContext,
    request_body: Option<&axum::body::Bytes>,
) -> Result<Option<Response<Body>>, GatewayError> {
    let Some(decision) = request_context.control_decision.as_ref() else {
        return Ok(None);
    };

    if decision.route_family.as_deref() != Some("users_manage") {
        return Ok(None);
    }

    match decision.route_kind.as_deref() {
        Some("create_user") => Ok(Some(
            build_admin_create_user_response(state, request_context, request_body).await?,
        )),
        Some("list_users") => Ok(Some(
            build_admin_list_users_response(state, request_context).await?,
        )),
        Some("get_user") => Ok(Some(
            build_admin_get_user_response(state, request_context).await?,
        )),
        Some("update_user") => Ok(Some(
            build_admin_update_user_response(state, request_context, request_body).await?,
        )),
        Some("delete_user") => Ok(Some(
            build_admin_delete_user_response(state, request_context).await?,
        )),
        Some("list_user_sessions") => Ok(Some(
            build_admin_list_user_sessions_response(state, request_context).await?,
        )),
        Some("list_user_api_keys") => Ok(Some(
            build_admin_list_user_api_keys_response(state, request_context).await?,
        )),
        Some("create_user_api_key") => Ok(Some(
            build_admin_create_user_api_key_response(state, request_context, request_body).await?,
        )),
        Some("update_user_api_key") => Ok(Some(
            build_admin_update_user_api_key_response(state, request_context, request_body).await?,
        )),
        Some("delete_user_api_key") => Ok(Some(
            build_admin_delete_user_api_key_response(state, request_context).await?,
        )),
        Some("lock_user_api_key") => Ok(Some(
            build_admin_toggle_user_api_key_lock_response(state, request_context, request_body)
                .await?,
        )),
        Some("delete_user_session") => Ok(Some(
            build_admin_delete_user_session_response(state, request_context).await?,
        )),
        Some("delete_user_sessions") => Ok(Some(
            build_admin_delete_user_sessions_response(state, request_context).await?,
        )),
        Some("reveal_user_api_key") => Ok(Some(
            build_admin_reveal_user_api_key_response(state, request_context).await?,
        )),
        _ => {
            if !is_admin_users_route(request_context) {
                return Ok(None);
            }
            Ok(Some(build_admin_users_data_unavailable_response()))
        }
    }
}
