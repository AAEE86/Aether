use crate::gateway::{AppState, GatewayError, GatewayPublicRequestContext};
use axum::{body::Body, response::Response};

const ADMIN_USERS_DATA_UNAVAILABLE_DETAIL: &str = "Admin user management data unavailable";

#[path = "users/api_keys.rs"]
mod users_api_keys;
#[path = "users/lifecycle.rs"]
mod users_lifecycle;
#[path = "users/routes.rs"]
mod users_routes;
#[path = "users/sessions.rs"]
mod users_sessions;
#[path = "users/shared.rs"]
mod users_shared;

use self::users_api_keys::{
    build_admin_create_user_api_key_response, build_admin_delete_user_api_key_response,
    build_admin_list_user_api_keys_response, build_admin_reveal_user_api_key_response,
    build_admin_toggle_user_api_key_lock_response, build_admin_update_user_api_key_response,
};
pub(crate) use self::users_api_keys::{
    default_admin_user_api_key_name, format_optional_unix_secs_iso8601,
    generate_admin_user_api_key_plaintext, hash_admin_user_api_key, masked_user_api_key_display,
    normalize_admin_optional_api_key_name,
};
use self::users_lifecycle::{
    build_admin_create_user_response, build_admin_delete_user_response,
    build_admin_get_user_response, build_admin_list_users_response,
    build_admin_update_user_response,
};
use self::users_sessions::{
    build_admin_delete_user_session_response, build_admin_delete_user_sessions_response,
    build_admin_list_user_sessions_response,
};
use self::users_shared::{
    admin_default_user_initial_gift, build_admin_users_bad_request_response,
    build_admin_users_data_unavailable_response, build_admin_users_read_only_response,
    format_optional_datetime_iso8601, normalize_admin_optional_user_email,
    normalize_admin_user_role, normalize_admin_username, validate_admin_user_password,
    AdminCreateUserApiKeyRequest, AdminCreateUserRequest, AdminToggleUserApiKeyLockRequest,
    AdminUpdateUserApiKeyRequest, AdminUpdateUserFieldPresence, AdminUpdateUserRequest,
};
pub(crate) use self::users_shared::{
    normalize_admin_user_api_formats, normalize_admin_user_string_list,
};

pub(crate) async fn maybe_build_local_admin_users_response(
    state: &AppState,
    request_context: &GatewayPublicRequestContext,
    request_body: Option<&axum::body::Bytes>,
) -> Result<Option<Response<Body>>, GatewayError> {
    users_routes::maybe_build_local_admin_users_routes_response(
        state,
        request_context,
        request_body,
    )
    .await
}
