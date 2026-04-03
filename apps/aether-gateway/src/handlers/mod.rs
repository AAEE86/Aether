use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use aether_contracts::{ExecutionPlan, ExecutionResult, ExecutionTimeouts, RequestBody};
#[cfg(test)]
use aether_crypto::DEVELOPMENT_ENCRYPTION_KEY;
use aether_crypto::{decrypt_python_fernet_ciphertext, encrypt_python_fernet_plaintext};
use aether_data::redis::{RedisKeyspace, RedisKvRunner};
use aether_data::repository::candidate_selection::StoredMinimalCandidateSelectionRow;
use aether_data::repository::candidates::{
    PublicHealthTimelineBucket, RequestCandidateStatus, StoredRequestCandidate,
};
use aether_data::repository::global_models::{
    AdminGlobalModelListQuery, AdminProviderModelListQuery, CreateAdminGlobalModelRecord,
    PublicGlobalModelQuery, StoredAdminGlobalModel, StoredAdminProviderModel,
    StoredPublicGlobalModel, UpdateAdminGlobalModelRecord, UpsertAdminProviderModelRecord,
};
use aether_data::repository::management_tokens::{
    CreateManagementTokenRecord, ManagementTokenListQuery, RegenerateManagementTokenSecret,
    StoredManagementToken, StoredManagementTokenUserSummary, UpdateManagementTokenRecord,
};
use aether_data::repository::oauth_providers::{
    EncryptedSecretUpdate, UpsertOAuthProviderConfigRecord,
};
use aether_data::repository::provider_catalog::{
    StoredProviderCatalogEndpoint, StoredProviderCatalogKey, StoredProviderCatalogProvider,
};
use aether_data::repository::proxy_nodes::{
    ProxyNodeHeartbeatMutation, ProxyNodeTunnelStatusMutation, StoredProxyNode,
    StoredProxyNodeEvent,
};
use aether_runtime::{maybe_hold_axum_response_permit, AdmissionPermit};
use axum::body::{to_bytes, Body, Bytes};
use axum::extract::{ConnectInfo, Request, State};
use axum::http::header::{HeaderName, HeaderValue};
use axum::http::Response;
use axum::response::IntoResponse;
use axum::Json;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use base64::Engine as _;
use chrono::{Datelike, SecondsFormat, Utc};
use futures_util::TryStreamExt;
use regex::Regex;
pub(crate) use serde::Deserialize;
pub(crate) use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use tracing::info;
pub(crate) use tracing::warn;
use url::form_urlencoded;
use url::Url;
use uuid::Uuid;

use crate::gateway::api::ai::{
    admin_default_body_rules_for_signature, admin_endpoint_signature_parts,
    public_api_format_local_path,
};
use crate::gateway::api::response::{
    build_client_response, build_local_auth_rejection_response, build_local_http_error_response,
    build_local_overloaded_response, build_local_user_rpm_limited_response,
};
use crate::gateway::constants::*;
use crate::gateway::headers::{
    extract_or_generate_trace_id, header_value_str, should_skip_request_header,
};
use crate::gateway::provider_transport::{
    fixed_provider_template, provider_type_enables_format_conversion_by_default,
    provider_type_is_fixed,
};
use crate::gateway::scheduler::{
    count_recent_rpm_requests_for_provider_key_since, is_provider_key_circuit_open,
    provider_key_health_score,
};
use crate::gateway::{
    allows_control_execute_emergency, execute_execution_runtime_stream,
    execute_execution_runtime_sync, maybe_build_stream_decision_payload,
    maybe_build_stream_plan_payload, maybe_build_sync_decision_payload,
    maybe_build_sync_finalize_outcome, maybe_build_sync_plan_payload, maybe_execute_via_control,
    maybe_execute_via_execution_runtime_stream, maybe_execute_via_execution_runtime_sync,
    record_shadow_result_non_blocking, request_model_local_rejection,
    resolve_public_request_context, should_buffer_request_for_local_auth,
    trusted_auth_local_rejection, AppState, FrontdoorUserRpmOutcome, GatewayControlDecision,
    GatewayError, GatewayFallbackMetricKind, GatewayFallbackReason, GatewayPublicRequestContext,
    LocalProviderDeleteTaskState,
};

const ADMIN_PROVIDER_MAPPING_PREVIEW_MAX_KEYS: usize = 200;
const ADMIN_PROVIDER_MAPPING_PREVIEW_MAX_MODELS: usize = 500;
const ADMIN_PROVIDER_MAPPING_PREVIEW_FETCH_LIMIT: usize = 10_000;
const ADMIN_PROVIDER_POOL_SCAN_BATCH: u64 = 200;
const ADMIN_EXTERNAL_MODELS_CACHE_KEY: &str = "aether:external:models_dev";
const ADMIN_EXTERNAL_MODELS_CACHE_TTL_SECS: u64 = 15 * 60;
const ADMIN_PROVIDER_OAUTH_DATA_UNAVAILABLE_DETAIL: &str = "Admin provider OAuth data unavailable";

pub(crate) mod admin;
pub(crate) mod internal;
pub(crate) mod proxy;
pub(crate) mod public;
pub(crate) mod shared;

pub(crate) use proxy::proxy_request;
pub(crate) use shared::*;

pub(crate) const OFFICIAL_EXTERNAL_MODEL_PROVIDERS: &[&str] = &[
    "anthropic",
    "openai",
    "google",
    "google-vertex",
    "azure",
    "amazon-bedrock",
    "xai",
    "meta",
    "deepseek",
    "mistral",
    "cohere",
    "zhipuai",
    "alibaba",
    "minimax",
    "moonshot",
    "baichuan",
    "ai21",
];

#[derive(Debug, Clone, Copy)]
pub(crate) struct AdminProviderPoolConfig {
    pub(crate) lru_enabled: bool,
    pub(crate) cost_window_seconds: u64,
    pub(crate) cost_limit_per_key_tokens: Option<u64>,
}

#[derive(Debug, Default)]
pub(crate) struct AdminProviderPoolRuntimeState {
    pub(crate) total_sticky_sessions: usize,
    pub(crate) sticky_sessions_by_key: BTreeMap<String, usize>,
    pub(crate) cooldown_reason_by_key: BTreeMap<String, String>,
    pub(crate) cooldown_ttl_by_key: BTreeMap<String, u64>,
    pub(crate) cost_window_usage_by_key: BTreeMap<String, u64>,
    pub(crate) lru_score_by_key: BTreeMap<String, f64>,
}
