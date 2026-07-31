//! Responses WebSocket end-to-end coverage.
//!
//! Every test starts a protocol-aware mock upstream, seeds a throwaway SQLite
//! store, mounts the real gateway router, and drives the public
//! `/v1/responses` WebSocket the way a client would.
//!
//! The assertions deliberately reach back into the database. A turn settles its
//! billing row from a task that outlives the relay loop, so a client that saw
//! `response.completed` is not evidence that the turn was ever accounted for —
//! only the row is.

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
use aether_data_contracts::repository::usage::{StoredRequestUsageAudit, UsageAuditListQuery};
use aether_gateway::{build_router_with_state, AppState, GatewayDataConfig, UsageRuntimeConfig};
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
use tokio::net::TcpStream;
use tokio::sync::Mutex;
use tokio_tungstenite::tungstenite::client::IntoClientRequest;
use tokio_tungstenite::tungstenite::Message;
use tokio_tungstenite::{MaybeTlsStream, WebSocketStream};

type BoxError = Box<dyn std::error::Error>;
type ClientSocket = WebSocketStream<MaybeTlsStream<TcpStream>>;

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

const INPUT_TOKENS: u64 = 4;
const OUTPUT_TOKENS: u64 = 2;

/// Generous enough to absorb a loaded CI runner, short enough that a genuinely
/// lost row fails the test instead of hanging the job.
const SETTLE_TIMEOUT: Duration = Duration::from_secs(30);
const RECEIVE_TIMEOUT: Duration = Duration::from_secs(15);

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

/// The headline guarantee: a continuation stays on one physical upstream socket,
/// and both turns are billed independently.
#[tokio::test]
async fn continuation_reuses_one_upstream_connection_and_bills_both_turns() -> Result<(), BoxError>
{
    let harness = Harness::start(UpstreamBehavior::CompleteEveryTurn).await?;
    let mut client = harness.connect().await?;

    client
        .send(response_create(json!({"input": "first turn"})))
        .await?;
    let first = receive_event(&mut client, "response.completed").await?;
    assert_eq!(
        first.pointer("/response/id").and_then(Value::as_str),
        Some("resp-e2e-1")
    );

    client
        .send(response_create(json!({
            "previous_response_id": "resp-e2e-1",
            "input": "second turn"
        })))
        .await?;
    let second = receive_event(&mut client, "response.completed").await?;
    assert_eq!(
        second.pointer("/response/id").and_then(Value::as_str),
        Some("resp-e2e-2")
    );

    // The mock records each `response.create` before answering it, so both
    // turns completing means both are already on record.
    let upstream_events = harness.upstream.observed_events().await;
    assert_eq!(
        upstream_events.len(),
        2,
        "one upstream turn per client turn"
    );
    assert_eq!(
        harness.upstream.connections(),
        1,
        "the continuation must reuse the bound upstream socket"
    );
    for event in &upstream_events {
        assert_eq!(
            event.get("model").and_then(Value::as_str),
            Some(UPSTREAM_MODEL),
            "every turn is rewritten to the mapped provider model"
        );
    }
    assert_eq!(
        upstream_events[1]
            .get("previous_response_id")
            .and_then(Value::as_str),
        Some("resp-e2e-1"),
        "the continuation id survives provider body normalization"
    );
    assert_eq!(
        harness.upstream.authorization_headers().await,
        vec![Some(format!("Bearer {PROVIDER_API_KEY}"))],
        "the upstream is opened with the configured provider key"
    );

    let audits = harness
        .usage_audits_where(2, "billed turns", is_billed)
        .await?;
    assert_eq!(audits.len(), 2, "each response.create bills separately");
    for audit in &audits {
        assert!(
            audit.is_websocket(),
            "turns are recorded as WebSocket usage, metadata: {:?}",
            audit.request_metadata
        );
        assert_eq!(audit.model, PUBLIC_MODEL);
        assert_eq!(audit.input_tokens, INPUT_TOKENS);
        assert_eq!(audit.output_tokens, OUTPUT_TOKENS);
        assert_eq!(audit.total_tokens, INPUT_TOKENS + OUTPUT_TOKENS);
        assert_eq!(audit.status_code, Some(200));
    }
    assert_ne!(
        audits[0].request_id, audits[1].request_id,
        "each turn gets its own logical request identity"
    );

    Ok(())
}

/// A client that walks away mid-turn must still be billed for what it started.
///
/// This is the path with no protocol event to announce it: the relay loop owns
/// the turn, and losing the client is an exit the upstream never reports.
#[tokio::test]
async fn client_disconnect_mid_turn_still_settles_the_usage_row() -> Result<(), BoxError> {
    let harness = Harness::start(UpstreamBehavior::StallAfterCreated).await?;
    let mut client = harness.connect().await?;

    client
        .send(response_create(json!({"input": "abandoned turn"})))
        .await?;
    // Leave only once the turn is genuinely in flight upstream, so this covers
    // an interrupted turn rather than racing turn start.
    receive_event(&mut client, "response.created").await?;
    drop(client);

    let audits = harness
        .usage_audits_where(1, "settled turns", |audit| !is_pending(audit))
        .await?;
    assert_eq!(audits.len(), 1, "the abandoned turn is still accounted for");
    let audit = &audits[0];
    assert_eq!(audit.model, PUBLIC_MODEL);
    assert!(
        !is_pending(audit),
        "an abandoned turn must not be left pending: {audit:?}"
    );

    Ok(())
}

/// An upstream that dies mid-turn must surface an error and still settle.
#[tokio::test]
async fn upstream_drop_mid_turn_reports_an_error_and_settles_the_usage_row() -> Result<(), BoxError>
{
    let harness = Harness::start(UpstreamBehavior::CloseAfterCreated).await?;
    let mut client = harness.connect().await?;

    client
        .send(response_create(json!({"input": "doomed turn"})))
        .await?;
    let error = receive_error_or_close(&mut client)
        .await?
        .ok_or("gateway closed without telling the client why")?;
    assert_eq!(error.get("type").and_then(Value::as_str), Some("error"));

    let audits = harness
        .usage_audits_where(1, "settled turns", |audit| !is_pending(audit))
        .await?;
    assert_eq!(audits.len(), 1, "the failed turn is still accounted for");
    let audit = &audits[0];
    assert!(
        !is_pending(audit),
        "a failed turn must not be left pending: {audit:?}"
    );

    Ok(())
}

fn is_pending(audit: &StoredRequestUsageAudit) -> bool {
    audit.status.eq_ignore_ascii_case("pending")
}

/// A turn that finished accounting: settled, and carrying what it consumed.
fn is_billed(audit: &StoredRequestUsageAudit) -> bool {
    !is_pending(audit) && audit.total_tokens > 0
}

// ---------------------------------------------------------------------------
// Harness
// ---------------------------------------------------------------------------

/// A live gateway wired to a mock Responses WebSocket upstream over a throwaway
/// SQLite store.
struct Harness {
    database: TemporarySqlite,
    upstream: Arc<MockUpstreamState>,
    websocket_url: String,
    _upstream_server: SpawnedServer,
    _gateway_server: SpawnedServer,
}

impl Harness {
    async fn start(behavior: UpstreamBehavior) -> Result<Self, BoxError> {
        let upstream = Arc::new(MockUpstreamState::new(behavior));
        let upstream_server =
            SpawnedServer::start(mock_upstream_router(Arc::clone(&upstream))).await?;

        let database = TemporarySqlite::new();
        prepare_and_seed_database(&database.config, upstream_server.base_url()).await?;

        let data_config = GatewayDataConfig::from_database_config(database.config.clone())
            .with_encryption_key(DEVELOPMENT_ENCRYPTION_KEY);
        let state = AppState::new()?
            .with_data_config_and_background_isolation(data_config, false)?
            // The usage runtime defaults to disabled, which silently turns every
            // terminal usage write into a no-op. Without this the suite could
            // not observe billing at all. Queueing stays off so the terminal
            // write lands through the in-process path instead of Redis.
            .with_usage_runtime_config(UsageRuntimeConfig {
                enabled: true,
                ..UsageRuntimeConfig::default()
            })?;
        let gateway_server = SpawnedServer::start(build_router_with_state(state)).await?;
        let websocket_url = format!(
            "{}/v1/responses",
            gateway_server.base_url().replacen("http://", "ws://", 1)
        );

        Ok(Self {
            database,
            upstream,
            websocket_url,
            _upstream_server: upstream_server,
            _gateway_server: gateway_server,
        })
    }

    async fn connect(&self) -> Result<ClientSocket, BoxError> {
        let mut request = self.websocket_url.clone().into_client_request()?;
        request.headers_mut().insert(
            "authorization",
            http::HeaderValue::from_str(&format!("Bearer {CLIENT_API_KEY}"))?,
        );
        let (socket, response) =
            tokio::time::timeout(RECEIVE_TIMEOUT, tokio_tungstenite::connect_async(request))
                .await
                .map_err(|_| "timed out connecting to the gateway WebSocket")??;
        if response.status() != http::StatusCode::SWITCHING_PROTOCOLS {
            return Err(
                format!("unexpected gateway handshake status: {}", response.status()).into(),
            );
        }
        Ok(socket)
    }

    /// Waits until `expected` usage rows satisfy `settled`.
    ///
    /// A row is created `Pending` at turn start and reaches its final shape
    /// through several independent writes, so "no longer pending" does not imply
    /// "finished": a row can briefly read as completed with zero tokens and no
    /// WebSocket metadata before the terminal write lands. Each caller waits for
    /// the specific end state it is about to assert.
    async fn usage_audits_where(
        &self,
        expected: usize,
        what: &str,
        settled: impl Fn(&StoredRequestUsageAudit) -> bool,
    ) -> Result<Vec<StoredRequestUsageAudit>, BoxError> {
        let deadline = tokio::time::Instant::now() + SETTLE_TIMEOUT;
        loop {
            let audits = self.usage_audits().await?;
            if audits.iter().filter(|audit| settled(audit)).count() >= expected {
                return Ok(audits);
            }
            if tokio::time::Instant::now() >= deadline {
                let observed = audits
                    .iter()
                    .map(|audit| {
                        format!(
                            "{} status={} code={:?} tokens={} websocket={}",
                            audit.request_id,
                            audit.status,
                            audit.status_code,
                            audit.total_tokens,
                            audit.is_websocket()
                        )
                    })
                    .collect::<Vec<_>>();
                return Err(format!(
                    "timed out waiting for {expected} {what}; observed {}: {observed:?}",
                    audits.len()
                )
                .into());
            }
            tokio::time::sleep(Duration::from_millis(25)).await;
        }
    }

    /// Reads the persisted audit rows, oldest first.
    ///
    /// Opens its own handle per call rather than holding one for the lifetime of
    /// the harness: the gateway keeps its own pool on the same SQLite file for
    /// the whole test, and an idle second pool only adds contention.
    async fn usage_audits(&self) -> Result<Vec<StoredRequestUsageAudit>, BoxError> {
        let backends = DataBackends::from_config(DataLayerConfig::from_database(
            self.database.config.clone(),
        ))?;
        let audits = backends
            .read()
            .usage()
            .ok_or("usage reader unavailable")?
            .list_usage_audits(&UsageAuditListQuery {
                limit: Some(50),
                newest_first: false,
                ..UsageAuditListQuery::default()
            })
            .await?;
        drop(backends);
        Ok(audits)
    }
}

// ---------------------------------------------------------------------------
// Client protocol helpers
// ---------------------------------------------------------------------------

/// Builds a `response.create` frame for the seeded public model.
fn response_create(fields: Value) -> Message {
    let mut event = json!({"type": "response.create", "model": PUBLIC_MODEL});
    let object = event
        .as_object_mut()
        .expect("the literal above is an object");
    for (key, value) in fields
        .as_object()
        .expect("response.create fields must be an object")
    {
        object.insert(key.clone(), value.clone());
    }
    Message::Text(event.to_string().into())
}

/// Reads frames until `expected_type` arrives, failing fast on a gateway error.
async fn receive_event<S>(
    socket: &mut WebSocketStream<S>,
    expected_type: &str,
) -> Result<Value, BoxError>
where
    S: tokio::io::AsyncRead + tokio::io::AsyncWrite + Unpin,
{
    tokio::time::timeout(RECEIVE_TIMEOUT, async {
        loop {
            let message = socket
                .next()
                .await
                .ok_or("gateway WebSocket closed before the expected event")??;
            match message {
                Message::Text(text) => {
                    let event: Value = serde_json::from_str(text.as_ref())?;
                    match event.get("type").and_then(Value::as_str) {
                        Some("error") => {
                            return Err(format!("gateway returned an error event: {event}").into())
                        }
                        Some(event_type) if event_type == expected_type => return Ok(event),
                        _ => {}
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

/// Drains the socket until the gateway reports an error or hangs up.
///
/// Returns the error event when one arrives, `None` when the gateway closed
/// without explaining itself.
async fn receive_error_or_close<S>(
    socket: &mut WebSocketStream<S>,
) -> Result<Option<Value>, BoxError>
where
    S: tokio::io::AsyncRead + tokio::io::AsyncWrite + Unpin,
{
    tokio::time::timeout(RECEIVE_TIMEOUT, async {
        loop {
            let Some(message) = socket.next().await else {
                return Ok(None);
            };
            match message? {
                Message::Text(text) => {
                    let event: Value = serde_json::from_str(text.as_ref())?;
                    if event.get("type").and_then(Value::as_str) == Some("error") {
                        return Ok(Some(event));
                    }
                }
                Message::Ping(payload) => socket.send(Message::Pong(payload)).await?,
                Message::Close(_) => return Ok(None),
                _ => {}
            }
        }
    })
    .await
    .map_err(|_| "timed out waiting for a gateway error or close")?
}

// ---------------------------------------------------------------------------
// Mock upstream
// ---------------------------------------------------------------------------

/// How the mock upstream answers a `response.create`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum UpstreamBehavior {
    /// Announce, stream one delta, and complete — the ordinary turn.
    CompleteEveryTurn,
    /// Announce the response and then go quiet, leaving the turn in flight.
    StallAfterCreated,
    /// Announce the response and then hang up mid-turn.
    CloseAfterCreated,
}

#[derive(Debug)]
struct MockUpstreamState {
    behavior: UpstreamBehavior,
    connections: AtomicUsize,
    events: Mutex<Vec<Value>>,
    authorization_headers: Mutex<Vec<Option<String>>>,
}

impl MockUpstreamState {
    fn new(behavior: UpstreamBehavior) -> Self {
        Self {
            behavior,
            connections: AtomicUsize::new(0),
            events: Mutex::new(Vec::new()),
            authorization_headers: Mutex::new(Vec::new()),
        }
    }

    fn connections(&self) -> usize {
        self.connections.load(Ordering::Acquire)
    }

    async fn observed_events(&self) -> Vec<Value> {
        self.events.lock().await.clone()
    }

    async fn authorization_headers(&self) -> Vec<Option<String>> {
        self.authorization_headers.lock().await.clone()
    }
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
                let response_id = format!("resp-e2e-{turn}");
                match state.behavior {
                    UpstreamBehavior::CompleteEveryTurn => {
                        if send_mock_turn(&mut socket, &response_id).await.is_err() {
                            break;
                        }
                    }
                    UpstreamBehavior::StallAfterCreated => {
                        if send_mock_created(&mut socket, &response_id).await.is_err() {
                            break;
                        }
                    }
                    UpstreamBehavior::CloseAfterCreated => {
                        let _ = send_mock_created(&mut socket, &response_id).await;
                        break;
                    }
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

async fn send_mock_created(socket: &mut WebSocket, response_id: &str) -> Result<(), axum::Error> {
    send_mock_event(
        socket,
        json!({
            "type": "response.created",
            "response": {
                "id": response_id,
                "status": "in_progress",
                "model": UPSTREAM_MODEL
            }
        }),
    )
    .await
}

async fn send_mock_turn(socket: &mut WebSocket, response_id: &str) -> Result<(), axum::Error> {
    send_mock_created(socket, response_id).await?;
    send_mock_event(
        socket,
        json!({
            "type": "response.output_text.delta",
            "response_id": response_id,
            "delta": "hello"
        }),
    )
    .await?;
    send_mock_event(
        socket,
        json!({
            "type": "response.completed",
            "response": {
                "id": response_id,
                "status": "completed",
                "model": UPSTREAM_MODEL,
                "output": [],
                "usage": {
                    "input_tokens": INPUT_TOKENS,
                    "output_tokens": OUTPUT_TOKENS,
                    "total_tokens": INPUT_TOKENS + OUTPUT_TOKENS
                }
            }
        }),
    )
    .await
}

async fn send_mock_event(socket: &mut WebSocket, event: Value) -> Result<(), axum::Error> {
    socket
        .send(AxumWsMessage::Text(event.to_string().into()))
        .await
}

// ---------------------------------------------------------------------------
// Seeded data store
// ---------------------------------------------------------------------------

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

async fn prepare_and_seed_database(
    database: &SqlDatabaseConfig,
    upstream_base_url: &str,
) -> Result<(), BoxError> {
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
) -> Result<(), BoxError> {
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

async fn seed_models(backends: &DataBackends) -> Result<(), BoxError> {
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

async fn seed_user(backends: &DataBackends) -> Result<String, BoxError> {
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

async fn seed_client_api_key(backends: &DataBackends, user_id: &str) -> Result<(), BoxError> {
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
