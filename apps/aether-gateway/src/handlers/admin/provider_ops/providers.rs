use crate::gateway::handlers::{
    admin_provider_id_for_provider_ops_balance, admin_provider_id_for_provider_ops_checkin,
    admin_provider_id_for_provider_ops_config, admin_provider_id_for_provider_ops_connect,
    admin_provider_id_for_provider_ops_disconnect, admin_provider_id_for_provider_ops_status,
    admin_provider_id_for_provider_ops_verify, admin_provider_ops_action_route_parts,
    decrypt_catalog_secret_with_fallbacks, encrypt_catalog_secret_with_fallbacks,
};
use crate::gateway::{AppState, GatewayError, GatewayPublicRequestContext};
use aether_data::repository::provider_catalog::{
    StoredProviderCatalogEndpoint, StoredProviderCatalogProvider,
};
use axum::{
    body::{Body, Bytes},
    http,
    response::{IntoResponse, Response},
    Json,
};
use serde::Deserialize;
use std::collections::BTreeMap;
use std::time::{SystemTime, UNIX_EPOCH};

#[path = "providers/actions.rs"]
mod actions;
#[path = "providers/config.rs"]
mod provider_config;
#[path = "providers/routes.rs"]
mod provider_routes;
#[path = "providers/verify.rs"]
mod provider_verify;
use self::actions::admin_provider_ops_is_valid_action_type;
pub(crate) use self::actions::admin_provider_ops_local_action_response;
use self::provider_config::{
    admin_provider_ops_config_object, admin_provider_ops_connector_object,
    admin_provider_ops_decrypted_credentials, admin_provider_ops_merge_credentials,
    admin_provider_ops_uses_python_verify_fallback, build_admin_provider_ops_config_payload,
    build_admin_provider_ops_saved_config_value, build_admin_provider_ops_status_payload,
    resolve_admin_provider_ops_base_url,
};
pub(super) use self::provider_routes::maybe_build_local_admin_provider_ops_providers_response;
use self::provider_verify::{
    admin_provider_ops_local_verify_response, admin_provider_ops_normalized_verify_architecture_id,
    admin_provider_ops_value_as_f64, admin_provider_ops_verify_failure,
    admin_provider_ops_verify_headers,
};

const ADMIN_PROVIDER_OPS_SENSITIVE_FIELDS: &[&str] = &[
    "api_key",
    "password",
    "refresh_token",
    "session_token",
    "session_cookie",
    "token_cookie",
    "auth_cookie",
    "cookie_string",
    "cookie",
];
const ADMIN_PROVIDER_OPS_CONNECT_RUST_ONLY_MESSAGE: &str =
    "Provider 连接仅支持 Rust execution runtime";
const ADMIN_PROVIDER_OPS_ACTION_RUST_ONLY_MESSAGE: &str =
    "Provider 操作仅支持 Rust execution runtime";
const ADMIN_PROVIDER_OPS_VERIFY_RUST_ONLY_MESSAGE: &str = "认证验证仅支持 Rust execution runtime";

#[derive(Debug, Deserialize)]
struct AdminProviderOpsSaveConfigRequest {
    #[serde(default = "default_admin_provider_ops_architecture_id")]
    architecture_id: String,
    #[serde(default)]
    base_url: Option<String>,
    connector: AdminProviderOpsConnectorConfigRequest,
    #[serde(default)]
    actions: BTreeMap<String, AdminProviderOpsActionConfigRequest>,
    #[serde(default)]
    schedule: BTreeMap<String, String>,
}

#[derive(Debug, Deserialize)]
struct AdminProviderOpsConnectorConfigRequest {
    auth_type: String,
    #[serde(default)]
    config: serde_json::Map<String, serde_json::Value>,
    #[serde(default)]
    credentials: serde_json::Map<String, serde_json::Value>,
}

#[derive(Debug, Deserialize)]
struct AdminProviderOpsActionConfigRequest {
    #[serde(default = "default_admin_provider_ops_action_enabled")]
    enabled: bool,
    #[serde(default)]
    config: serde_json::Map<String, serde_json::Value>,
}

#[derive(Debug, Deserialize)]
struct AdminProviderOpsConnectRequest {
    #[serde(default)]
    credentials: Option<serde_json::Map<String, serde_json::Value>>,
}

#[derive(Debug, Deserialize)]
struct AdminProviderOpsExecuteActionRequest {
    #[serde(default)]
    config: Option<serde_json::Map<String, serde_json::Value>>,
}

#[derive(Debug, Clone)]
struct AdminProviderOpsCheckinOutcome {
    success: Option<bool>,
    message: String,
    cookie_expired: bool,
}

fn default_admin_provider_ops_architecture_id() -> String {
    "generic_api".to_string()
}

fn default_admin_provider_ops_action_enabled() -> bool {
    true
}
