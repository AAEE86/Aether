//! Standalone low-memory Responses WebSocket end-to-end test.
//!
//! This starts a protocol-aware mock upstream, seeds a temporary SQLite data
//! store, mounts the real gateway router, and verifies that a continuation is
//! relayed over the same physical upstream WebSocket connection.

use std::path::PathBuf;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::Duration;

use aether_crypto::{encrypt_python_fernet_plaintext, DEVELOPMENT_ENCRYPTION_KEY};
use aether_data::repository::auth::CreateStandaloneApiKeyRecord;
use aether_data::repository::wallet::WalletLookupKey;
use aether_data::{
    DataBackends, DataLayerConfig, DatabaseDriver, SqlDatabaseConfig, SqlPoolConfig,
};
use aether_data_contracts::repository::global_models::{
    CreateAdminGlobalModelRecord, UpsertAdminProviderModelRecord,
};
use aether_data_contracts::repository::provider_catalog::{
    StoredProviderCatalogEndpoint, StoredProviderCatalogKey, StoredProviderCatalogProvider,
};
use aether_gateway::{build_router_with_state, AppState, GatewayDataConfig};
use aether_testkit::SpawnedServer;
use axum::extract::ws::{Message as AxumWsMessage, WebSocket, WebSocketUpgrade};
use axum::extract::State;
use axum::http::HeaderMap;
use axum::response::Response;
use axum::routing::get;
use axum::Router;
use futures_util::{SinkExt, StreamExt};
use serde_json::{json, Value};
use sha2::Digest;
use tokio::sync::Mutex;
use tokio_tungstenite::tungstenite::client::IntoClientRequest;
use tokio_tungstenite::tungstenite::Message;

const CLIENT_API_KEY: &str = "sk-aether-responses-ws-e2e";
const PROVIDER_API_KEY: &str = "sk-upstream-responses-ws-e2e";
const PROVIDER_ID: &str = "provider-responses-ws-e2e";
const ENDPOINT_ID: &str = "endpoint-responses-ws-e2e";
const PROVIDER_KEY_ID: &str = "provider-key-responses-ws-e2e";
const GLOBAL_MODEL_ID: &str = "global-model-responses-ws-e2e";
const PROVIDER_MODEL_ID: &str = "provider-model-responses-ws-e2e";
const API_KEY_ID: &str = "api-key-responses-ws-e2e";
const PUBLIC_MODEL: &str = "gpt-responses-ws-e2e";
const UPSTREAM_MODEL: &str = "gpt-responses-ws-upstream";

#[derive(Debug, Default)]
struct MockUpstreamState {
    connections: AtomicUsize,
    events: Mutex<Vec<Value>>,
    authorization_headers: Mutex<Vec<Option<String>>>,
}

struct TemporarySqlite {
    directory: PathBuf,
    config: SqlDatabaseConfig,
}

impl TemporarySqlite {
    fn new() -> Self {
        let directory = std::env::temp_dir().join(format!(
            "aether-responses-ws-e2e-{}-{}",
            std::process::id(),
            uuid::Uuid::new_v4()
        ));
        let database_path = directory.join("aether.db");
        Self {
            directory,
            config: SqlDatabaseConfig {
                driver: DatabaseDriver::Sqlite,
                url: format!("sqlite://{}", database_path.display()),
                pool: SqlPoolConfig {
                    min_connections: 1,
                    max_connections: 4,
                    acquire_timeout_ms: 5_000,
                    idle_timeout_ms: 30_000,
                    max_lifetime_ms: 300_000,
                    statement_cache_capacity: 64,
                    require_ssl: false,
                },
            },
        }
    }
}

impl Drop for TemporarySqlite {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.directory);
    }
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mock_state = Arc::new(MockUpstreamState::default());
    let mock_server = SpawnedServer::start(mock_upstream_router(Arc::clone(&mock_state))).await?;

    let database = TemporarySqlite::new();
    prepare_and_seed_database(&database.config, mock_server.base_url()).await?;

    let gateway_data_config = GatewayDataConfig::from_database_config(database.config.clone())
        .with_encryption_key(DEVELOPMENT_ENCRYPTION_KEY);
    let gateway_state =
        AppState::new()?.with_data_config_and_background_isolation(gateway_data_config, false)?;
    let gateway = SpawnedServer::start(build_router_with_state(gateway_state)).await?;

    let ws_url = format!(
        "{}/v1/responses",
        gateway.base_url().replacen("http://", "ws://", 1)
    );
    let mut request = ws_url.into_client_request()?;
    request.headers_mut().insert(
        "authorization",
        http::HeaderValue::from_static("Bearer sk-aether-responses-ws-e2e"),
    );
    let (mut client, response) = tokio::time::timeout(
        Duration::from_secs(10),
        tokio_tungstenite::connect_async(request),
    )
    .await
    .map_err(|_| "timed out connecting to the gateway WebSocket")??;
    if response.status() != http::StatusCode::SWITCHING_PROTOCOLS {
        return Err(format!("unexpected gateway handshake status: {}", response.status()).into());
    }

    client
        .send(Message::Text(
            json!({
                "type": "response.create",
                "model": PUBLIC_MODEL,
                "input": "first turn"
            })
            .to_string()
            .into(),
        ))
        .await?;
    let first_completed = receive_until_event(&mut client, "response.completed").await?;
    if first_completed
        .pointer("/response/id")
        .and_then(Value::as_str)
        != Some("resp-e2e-1")
    {
        return Err("first turn returned an unexpected response id".into());
    }

    client
        .send(Message::Text(
            json!({
                "type": "response.create",
                "model": PUBLIC_MODEL,
                "previous_response_id": "resp-e2e-1",
                "input": "second turn"
            })
            .to_string()
            .into(),
        ))
        .await?;
    let second_completed = receive_until_event(&mut client, "response.completed").await?;
    if second_completed
        .pointer("/response/id")
        .and_then(Value::as_str)
        != Some("resp-e2e-2")
    {
        return Err("continuation returned an unexpected response id".into());
    }

    let _ = client.close(None).await;
    tokio::time::timeout(Duration::from_secs(2), async {
        loop {
            if mock_state.events.lock().await.len() >= 2 {
                break;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .map_err(|_| "mock upstream did not observe both response.create events")?;

    let observed_events = mock_state.events.lock().await.clone();
    if mock_state.connections.load(Ordering::Acquire) != 1 {
        return Err(format!(
            "expected one reused upstream connection, observed {}",
            mock_state.connections.load(Ordering::Acquire)
        )
        .into());
    }
    if observed_events.len() != 2 {
        return Err(format!(
            "expected two upstream events, observed {}",
            observed_events.len()
        )
        .into());
    }
    if observed_events[0].get("model").and_then(Value::as_str) != Some(UPSTREAM_MODEL)
        || observed_events[1].get("model").and_then(Value::as_str) != Some(UPSTREAM_MODEL)
    {
        return Err("gateway did not consistently rewrite the public model".into());
    }
    if observed_events[1]
        .get("previous_response_id")
        .and_then(Value::as_str)
        != Some("resp-e2e-1")
    {
        return Err("gateway did not preserve the continuation response id".into());
    }
    let authorization_headers = mock_state.authorization_headers.lock().await;
    if authorization_headers.as_slice() != [Some(format!("Bearer {PROVIDER_API_KEY}"))] {
        return Err("gateway did not send the configured upstream authorization header".into());
    }

    println!(
        "{}",
        json!({
            "status": "passed",
            "gateway_handshake_status": 101,
            "turns": observed_events.len(),
            "upstream_connections": mock_state.connections.load(Ordering::Acquire),
            "continuation_confirmed": true
        })
    );
    Ok(())
}

async fn receive_until_event<S>(
    socket: &mut tokio_tungstenite::WebSocketStream<S>,
    expected_type: &str,
) -> Result<Value, Box<dyn std::error::Error>>
where
    S: tokio::io::AsyncRead + tokio::io::AsyncWrite + Unpin,
{
    tokio::time::timeout(Duration::from_secs(10), async {
        loop {
            let message = socket
                .next()
                .await
                .ok_or("gateway WebSocket closed before the expected event")??;
            match message {
                Message::Text(text) => {
                    let event: Value = serde_json::from_str(text.as_ref())?;
                    if event.get("type").and_then(Value::as_str) == Some("error") {
                        return Err(format!("gateway returned an error event: {event}").into());
                    }
                    if event.get("type").and_then(Value::as_str) == Some(expected_type) {
                        return Ok(event);
                    }
                }
                Message::Ping(payload) => socket.send(Message::Pong(payload)).await?,
                Message::Close(frame) => {
                    return Err(format!("gateway closed before {expected_type}: {frame:?}").into())
                }
                _ => {}
            }
        }
    })
    .await
    .map_err(|_| format!("timed out waiting for {expected_type}"))?
}

fn mock_upstream_router(state: Arc<MockUpstreamState>) -> Router {
    Router::new()
        .route("/v1/responses", get(mock_responses_websocket))
        .with_state(state)
}

async fn mock_responses_websocket(
    State(state): State<Arc<MockUpstreamState>>,
    headers: HeaderMap,
    ws: WebSocketUpgrade,
) -> Response {
    let authorization = headers
        .get("authorization")
        .and_then(|value| value.to_str().ok())
        .map(str::to_string);
    ws.on_upgrade(move |socket| run_mock_upstream(socket, state, authorization))
}

async fn run_mock_upstream(
    mut socket: WebSocket,
    state: Arc<MockUpstreamState>,
    authorization: Option<String>,
) {
    state.connections.fetch_add(1, Ordering::AcqRel);
    state.authorization_headers.lock().await.push(authorization);
    while let Some(message) = socket.recv().await {
        let Ok(message) = message else {
            break;
        };
        match message {
            AxumWsMessage::Text(text) => {
                let Ok(event) = serde_json::from_str::<Value>(text.as_str()) else {
                    break;
                };
                if event.get("type").and_then(Value::as_str) != Some("response.create") {
                    continue;
                }
                let turn = {
                    let mut events = state.events.lock().await;
                    events.push(event);
                    events.len()
                };
                if send_mock_turn(&mut socket, turn).await.is_err() {
                    break;
                }
            }
            AxumWsMessage::Ping(payload) => {
                if socket.send(AxumWsMessage::Pong(payload)).await.is_err() {
                    break;
                }
            }
            AxumWsMessage::Close(_) => break,
            _ => {}
        }
    }
}

async fn send_mock_turn(socket: &mut WebSocket, turn: usize) -> Result<(), axum::Error> {
    let response_id = format!("resp-e2e-{turn}");
    for event in [
        json!({
            "type": "response.created",
            "response": {
                "id": response_id,
                "status": "in_progress",
                "model": UPSTREAM_MODEL
            }
        }),
        json!({
            "type": "response.output_text.delta",
            "response_id": response_id,
            "delta": format!("turn-{turn}")
        }),
        json!({
            "type": "response.completed",
            "response": {
                "id": response_id,
                "status": "completed",
                "model": UPSTREAM_MODEL,
                "output": [],
                "usage": {
                    "input_tokens": 4,
                    "output_tokens": 2,
                    "total_tokens": 6
                }
            }
        }),
    ] {
        socket
            .send(AxumWsMessage::Text(event.to_string().into()))
            .await?;
    }
    Ok(())
}

async fn prepare_and_seed_database(
    database: &SqlDatabaseConfig,
    upstream_base_url: &str,
) -> Result<(), Box<dyn std::error::Error>> {
    let backends = DataBackends::from_config(DataLayerConfig::from_database(database.clone()))?;
    let pending = backends
        .prepare_database_for_startup()
        .await?
        .unwrap_or_default();
    if !pending.is_empty() {
        backends.run_database_migrations().await?;
    }

    seed_provider_catalog(&backends, upstream_base_url).await?;
    seed_models(&backends).await?;
    let user_id = seed_user(&backends).await?;
    seed_client_api_key(&backends, &user_id).await?;

    let candidates = backends
        .read()
        .minimal_candidate_selection()
        .ok_or("candidate selection reader unavailable")?
        .list_for_exact_api_format_and_requested_model("openai:responses", PUBLIC_MODEL)
        .await?;
    if !candidates.iter().any(|candidate| {
        candidate.provider_id == PROVIDER_ID
            && candidate.endpoint_id == ENDPOINT_ID
            && candidate.key_id == PROVIDER_KEY_ID
    }) {
        return Err("seeded Responses WebSocket candidate is not visible".into());
    }
    drop(backends);
    Ok(())
}

async fn seed_provider_catalog(
    backends: &DataBackends,
    upstream_base_url: &str,
) -> Result<(), Box<dyn std::error::Error>> {
    let writer = backends
        .write()
        .provider_catalog()
        .ok_or("provider catalog writer unavailable")?;
    writer
        .create_provider(
            &StoredProviderCatalogProvider::new(
                PROVIDER_ID.to_string(),
                "Responses WebSocket E2E".to_string(),
                None,
                "openai".to_string(),
            )?
            .with_transport_fields(
                true,
                false,
                false,
                None,
                Some(0),
                None,
                Some(30.0),
                Some(10.0),
                Some(json!({"responses_websocket": {"enabled": true}})),
            ),
            None,
        )
        .await?;
    writer
        .create_endpoint(
            &StoredProviderCatalogEndpoint::new(
                ENDPOINT_ID.to_string(),
                PROVIDER_ID.to_string(),
                "openai:responses".to_string(),
                Some("openai".to_string()),
                Some("responses".to_string()),
                true,
            )?
            .with_transport_fields(
                upstream_base_url.trim_end_matches('/').to_string(),
                None,
                None,
                Some(0),
                Some("/v1/responses".to_string()),
                None,
                None,
                None,
            )?,
        )
        .await?;
    writer
        .create_key(
            &StoredProviderCatalogKey::new(
                PROVIDER_KEY_ID.to_string(),
                PROVIDER_ID.to_string(),
                "Responses WebSocket E2E".to_string(),
                "api_key".to_string(),
                Some(json!({"streaming": true})),
                true,
            )?
            .with_transport_fields(
                Some(json!(["openai:responses"])),
                encrypt_python_fernet_plaintext(DEVELOPMENT_ENCRYPTION_KEY, PROVIDER_API_KEY)?,
                None,
                None,
                Some(json!({"openai:responses": 1})),
                Some(json!([PUBLIC_MODEL, UPSTREAM_MODEL])),
                None,
                None,
                None,
            )?
            .with_health_fields(
                Some(json!({"openai:responses": {"status": "healthy"}})),
                Some(json!({"openai:responses": {"state": "closed"}})),
            ),
        )
        .await?;
    Ok(())
}

async fn seed_models(backends: &DataBackends) -> Result<(), Box<dyn std::error::Error>> {
    let writer = backends
        .write()
        .global_models()
        .ok_or("global model writer unavailable")?;
    writer
        .create_admin_global_model(&CreateAdminGlobalModelRecord::new(
            GLOBAL_MODEL_ID.to_string(),
            PUBLIC_MODEL.to_string(),
            PUBLIC_MODEL.to_string(),
            true,
            Some(0.0),
            None,
            Some(json!({"streaming": true, "chat": true})),
            Some(json!({"model_mappings": [UPSTREAM_MODEL]})),
        )?)
        .await?;
    writer
        .create_admin_provider_model(&UpsertAdminProviderModelRecord::new(
            PROVIDER_MODEL_ID.to_string(),
            PROVIDER_ID.to_string(),
            GLOBAL_MODEL_ID.to_string(),
            UPSTREAM_MODEL.to_string(),
            Some(json!([{
                "name": UPSTREAM_MODEL,
                "priority": 0,
                "api_formats": ["openai:responses"],
                "endpoint_ids": [ENDPOINT_ID]
            }])),
            Some(0.0),
            None,
            Some(false),
            Some(false),
            Some(true),
            Some(false),
            Some(false),
            true,
            true,
            Some(json!({"responses_websocket_e2e": true})),
        )?)
        .await?;
    Ok(())
}

async fn seed_user(backends: &DataBackends) -> Result<String, Box<dyn std::error::Error>> {
    let users = backends.read().users().ok_or("user reader unavailable")?;
    let user = users
        .create_local_auth_user_with_settings(
            Some("responses-ws-e2e@example.test".to_string()),
            true,
            "responses-ws-e2e".to_string(),
            "disabled-password".to_string(),
            "user".to_string(),
            Some(vec![PROVIDER_ID.to_string()]),
            Some(vec!["openai:responses".to_string()]),
            Some(vec![PUBLIC_MODEL.to_string()]),
            None,
        )
        .await?
        .ok_or("failed to create E2E user")?;
    let wallets = backends
        .read()
        .wallets()
        .ok_or("wallet reader unavailable")?;
    wallets
        .initialize_auth_user_wallet(&user.id, 0.0, true)
        .await?;
    Ok(user.id)
}

async fn seed_client_api_key(
    backends: &DataBackends,
    user_id: &str,
) -> Result<(), Box<dyn std::error::Error>> {
    backends
        .write()
        .auth_api_keys()
        .ok_or("auth API key writer unavailable")?
        .create_standalone_api_key(CreateStandaloneApiKeyRecord {
            user_id: user_id.to_string(),
            api_key_id: API_KEY_ID.to_string(),
            key_hash: sha256_hex(CLIENT_API_KEY),
            key_encrypted: Some(CLIENT_API_KEY.to_string()),
            name: Some("Responses WebSocket E2E".to_string()),
            allowed_providers: Some(vec![PROVIDER_ID.to_string()]),
            allowed_api_formats: Some(vec!["openai:responses".to_string()]),
            allowed_models: Some(vec![PUBLIC_MODEL.to_string()]),
            ip_rules: None,
            rate_limit: Some(0),
            concurrent_limit: None,
            force_capabilities: None,
            is_active: true,
            expires_at_unix_secs: None,
            auto_delete_on_expiry: false,
            total_requests: 0,
            total_tokens: 0,
            total_cost_usd: 0.0,
        })
        .await?;
    backends
        .read()
        .wallets()
        .ok_or("wallet reader unavailable")?
        .initialize_auth_api_key_wallet(API_KEY_ID, 0.0, true)
        .await?;
    if backends
        .read()
        .wallets()
        .ok_or("wallet reader unavailable")?
        .find(WalletLookupKey::ApiKeyId(API_KEY_ID))
        .await?
        .is_none()
    {
        return Err("failed to initialize E2E API key wallet".into());
    }
    Ok(())
}

fn sha256_hex(value: &str) -> String {
    let mut hasher = sha2::Sha256::new();
    hasher.update(value.as_bytes());
    format!("{:x}", hasher.finalize())
}
