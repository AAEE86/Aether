use crate::gateway::handlers::admin::{
    default_admin_user_api_key_name, format_optional_unix_secs_iso8601,
    generate_admin_user_api_key_plaintext, hash_admin_user_api_key, masked_user_api_key_display,
    normalize_admin_optional_api_key_name, normalize_admin_user_api_formats,
    normalize_admin_user_string_list,
};
use crate::gateway::handlers::public::serialize_admin_system_users_export_wallet;
use crate::gateway::handlers::{
    decrypt_catalog_secret_with_fallbacks, encrypt_catalog_secret_with_fallbacks, query_param_bool,
    query_param_optional_bool, query_param_value,
};
use crate::gateway::{AppState, GatewayError, GatewayPublicRequestContext};
use axum::{
    body::Body,
    http,
    response::{IntoResponse, Response},
    Json,
};
use serde_json::json;

const ADMIN_API_KEYS_DATA_UNAVAILABLE_DETAIL: &str = "Admin standalone API key data unavailable";

#[path = "api_keys/mutation_routes.rs"]
mod admin_api_keys_mutation_routes;
#[path = "api_keys/read_routes.rs"]
mod admin_api_keys_read_routes;
#[path = "api_keys/routes.rs"]
mod admin_api_keys_routes;
#[path = "api_keys/shared.rs"]
mod admin_api_keys_shared;

use self::admin_api_keys_mutation_routes::{
    build_admin_create_api_key_response, build_admin_delete_api_key_response,
    build_admin_toggle_api_key_response, build_admin_update_api_key_response,
};
use self::admin_api_keys_read_routes::{
    build_admin_api_key_detail_response, build_admin_list_api_keys_response,
};
use self::admin_api_keys_shared::{
    admin_api_key_total_tokens_by_ids, admin_api_keys_id_from_path, admin_api_keys_operator_id,
    admin_api_keys_parse_limit, admin_api_keys_parse_skip, build_admin_api_key_detail_payload,
    build_admin_api_key_list_item_payload, build_admin_api_keys_bad_request_response,
    build_admin_api_keys_data_unavailable_response, build_admin_api_keys_not_found_response,
    AdminStandaloneApiKeyCreateRequest, AdminStandaloneApiKeyFieldPresence,
    AdminStandaloneApiKeyToggleRequest, AdminStandaloneApiKeyUpdateRequest,
};

pub(crate) async fn maybe_build_local_admin_api_keys_response(
    state: &AppState,
    request_context: &GatewayPublicRequestContext,
    request_body: Option<&axum::body::Bytes>,
) -> Result<Option<Response<Body>>, GatewayError> {
    admin_api_keys_routes::maybe_build_local_admin_api_keys_routes_response(
        state,
        request_context,
        request_body,
    )
    .await
}
