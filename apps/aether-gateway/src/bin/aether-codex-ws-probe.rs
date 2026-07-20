//! Credential-safe compatibility probe for the Codex Responses WebSocket path.
//!
//! The probe intentionally reads the access token only from the process
//! environment and never includes credential values, account IDs, response IDs,
//! or response bodies in its output. It verifies that one upstream WebSocket
//! accepts two sequential `response.create` warmups where the second request
//! continues from the first response ID.

use std::env;
use std::time::{Duration, Instant};

use aether_ai_formats::{CODEX_CLIENT_ORIGINATOR, CODEX_CLIENT_USER_AGENT};
use clap::Parser;
use http::header::{AUTHORIZATION, USER_AGENT};
use http::{HeaderMap, HeaderName, HeaderValue};
use serde::Serialize;
use serde_json::{json, Value};
use url::Url;
use wreq::ws::message::Message as WreqWsMessage;

const ACCESS_TOKEN_ENV: &str = "AETHER_CODEX_WS_PROBE_ACCESS_TOKEN";
const ACCOUNT_ID_ENV: &str = "AETHER_CODEX_WS_PROBE_ACCOUNT_ID";
const MODEL_ENV: &str = "AETHER_CODEX_WS_PROBE_MODEL";
const URL_ENV: &str = "AETHER_CODEX_WS_PROBE_URL";
const MAX_FRAME_SIZE: usize = 1 << 20;
const MAX_EVENTS_PER_TURN: usize = 16;

#[derive(Parser)]
#[command(
    name = "aether-codex-ws-probe",
    about = "Verify a Codex Responses WebSocket endpoint without exposing credentials"
)]
struct Args {
    /// WebSocket endpoint. If omitted, AETHER_CODEX_WS_PROBE_URL is used.
    #[arg(long)]
    url: Option<String>,

    /// Per-turn receive timeout in seconds.
    #[arg(long, default_value_t = 20, value_parser = clap::value_parser!(u64).range(1..=120))]
    timeout_secs: u64,
}

struct ProbeConfig {
    url: Url,
    access_token: String,
    account_id: String,
    model: String,
    turn_timeout: Duration,
}

#[derive(Debug, Clone, Copy)]
enum ProbeFailure {
    MissingConfiguration,
    InvalidEndpoint,
    ClientBuild,
    Handshake,
    Upgrade,
    Send,
    ReceiveTimeout,
    Receive,
    RemoteError,
    MissingResponseId,
    UnexpectedFrame,
}

impl ProbeFailure {
    const fn code(self) -> &'static str {
        match self {
            Self::MissingConfiguration => "missing_configuration",
            Self::InvalidEndpoint => "invalid_endpoint",
            Self::ClientBuild => "client_build_failed",
            Self::Handshake => "handshake_failed",
            Self::Upgrade => "upgrade_failed",
            Self::Send => "send_failed",
            Self::ReceiveTimeout => "receive_timeout",
            Self::Receive => "receive_failed",
            Self::RemoteError => "upstream_error_event",
            Self::MissingResponseId => "response_id_not_observed",
            Self::UnexpectedFrame => "unexpected_frame",
        }
    }
}

#[derive(Serialize)]
struct ProbeReport {
    status: &'static str,
    target_host: Option<String>,
    handshake_status: Option<u16>,
    sent_header_names: Vec<&'static str>,
    received_header_names: Vec<String>,
    observed_event_types: Vec<String>,
    continuation_confirmed: bool,
    elapsed_ms: u64,
    error: Option<&'static str>,
}

impl ProbeReport {
    fn failed(config: Option<&ProbeConfig>, started_at: Instant, error: ProbeFailure) -> Self {
        Self {
            status: "failed",
            target_host: config.and_then(target_host),
            handshake_status: None,
            sent_header_names: sent_header_names(),
            received_header_names: Vec::new(),
            observed_event_types: Vec::new(),
            continuation_confirmed: false,
            elapsed_ms: started_at.elapsed().as_millis() as u64,
            error: Some(error.code()),
        }
    }
}

#[tokio::main]
async fn main() {
    let started_at = Instant::now();
    let args = Args::parse();
    let config = match ProbeConfig::from_args(args) {
        Ok(config) => config,
        Err(error) => {
            print_report(&ProbeReport::failed(None, started_at, error));
            std::process::exit(2);
        }
    };

    match run_probe(&config, started_at).await {
        Ok(report) => print_report(&report),
        Err(error) => {
            print_report(&ProbeReport::failed(Some(&config), started_at, error));
            std::process::exit(1);
        }
    }
}

impl ProbeConfig {
    fn from_args(args: Args) -> Result<Self, ProbeFailure> {
        let raw_url = args.url.or_else(|| env::var(URL_ENV).ok());
        let Some(raw_url) = raw_url
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty())
        else {
            return Err(ProbeFailure::MissingConfiguration);
        };
        let url = parse_probe_url(raw_url)?;
        Ok(Self {
            url,
            access_token: required_env(ACCESS_TOKEN_ENV)?,
            account_id: required_env(ACCOUNT_ID_ENV)?,
            model: required_env(MODEL_ENV)?,
            turn_timeout: Duration::from_secs(args.timeout_secs),
        })
    }
}

fn required_env(name: &str) -> Result<String, ProbeFailure> {
    env::var(name)
        .ok()
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
        .ok_or(ProbeFailure::MissingConfiguration)
}

fn parse_probe_url(raw: &str) -> Result<Url, ProbeFailure> {
    let url = Url::parse(raw).map_err(|_| ProbeFailure::InvalidEndpoint)?;
    if !matches!(url.scheme(), "ws" | "wss")
        || url.host_str().is_none()
        || !url.username().is_empty()
        || url.password().is_some()
        || url.query().is_some()
        || url.fragment().is_some()
    {
        return Err(ProbeFailure::InvalidEndpoint);
    }
    Ok(url)
}

async fn run_probe(config: &ProbeConfig, started_at: Instant) -> Result<ProbeReport, ProbeFailure> {
    let client = wreq::Client::builder()
        .connect_timeout(config.turn_timeout)
        .timeout(config.turn_timeout)
        .build()
        .map_err(|_| ProbeFailure::ClientBuild)?;
    let response = client
        .websocket(config.url.as_str())
        .headers(handshake_headers(config)?)
        .max_frame_size(MAX_FRAME_SIZE)
        .max_message_size(MAX_FRAME_SIZE)
        .send()
        .await
        .map_err(|_| ProbeFailure::Handshake)?;
    let handshake_status = response.status().as_u16();
    let received_header_names = response
        .headers()
        .keys()
        .map(|name| name.as_str().to_string())
        .collect();
    let mut socket = response
        .into_websocket()
        .await
        .map_err(|_| ProbeFailure::Upgrade)?;
    let mut observed_event_types = Vec::new();

    send_warmup(&mut socket, &config.model, None).await?;
    let first_response_id =
        receive_response_id(&mut socket, config.turn_timeout, &mut observed_event_types).await?;

    send_warmup(&mut socket, &config.model, Some(&first_response_id)).await?;
    let _second_response_id =
        receive_response_id(&mut socket, config.turn_timeout, &mut observed_event_types).await?;

    Ok(ProbeReport {
        status: "passed",
        target_host: target_host(config),
        handshake_status: Some(handshake_status),
        sent_header_names: sent_header_names(),
        received_header_names,
        observed_event_types,
        continuation_confirmed: true,
        elapsed_ms: started_at.elapsed().as_millis() as u64,
        error: None,
    })
}

fn handshake_headers(config: &ProbeConfig) -> Result<HeaderMap, ProbeFailure> {
    let authorization = HeaderValue::from_str(format!("Bearer {}", config.access_token).as_str())
        .map_err(|_| ProbeFailure::MissingConfiguration)?;
    let account_id = HeaderValue::from_str(config.account_id.as_str())
        .map_err(|_| ProbeFailure::MissingConfiguration)?;
    let mut headers = HeaderMap::new();
    headers.insert(AUTHORIZATION, authorization);
    headers.insert(HeaderName::from_static("chatgpt-account-id"), account_id);
    headers.insert(
        USER_AGENT,
        HeaderValue::from_static(CODEX_CLIENT_USER_AGENT),
    );
    headers.insert(
        HeaderName::from_static("originator"),
        HeaderValue::from_static(CODEX_CLIENT_ORIGINATOR),
    );
    Ok(headers)
}

fn target_host(config: &ProbeConfig) -> Option<String> {
    config.url.host_str().map(|host| match config.url.port() {
        Some(port) => format!("{host}:{port}"),
        None => host.to_string(),
    })
}

fn sent_header_names() -> Vec<&'static str> {
    vec![
        "authorization",
        "chatgpt-account-id",
        "user-agent",
        "originator",
    ]
}

async fn send_warmup(
    socket: &mut wreq::ws::WebSocket,
    model: &str,
    previous_response_id: Option<&str>,
) -> Result<(), ProbeFailure> {
    let mut event = json!({
        "type": "response.create",
        "model": model,
        "store": false,
        "generate": false,
        "input": [],
        "tools": [],
    });
    if let Some(previous_response_id) = previous_response_id {
        event["previous_response_id"] = Value::String(previous_response_id.to_string());
    }
    socket
        .send(WreqWsMessage::text(event.to_string()))
        .await
        .map_err(|_| ProbeFailure::Send)
}

async fn receive_response_id(
    socket: &mut wreq::ws::WebSocket,
    timeout: Duration,
    observed_event_types: &mut Vec<String>,
) -> Result<String, ProbeFailure> {
    for _ in 0..MAX_EVENTS_PER_TURN {
        let message = tokio::time::timeout(timeout, socket.recv())
            .await
            .map_err(|_| ProbeFailure::ReceiveTimeout)?
            .ok_or(ProbeFailure::MissingResponseId)?
            .map_err(|_| ProbeFailure::Receive)?;
        match message {
            WreqWsMessage::Text(text) => {
                let event: Value = serde_json::from_str(text.as_str())
                    .map_err(|_| ProbeFailure::UnexpectedFrame)?;
                let event_type = event
                    .get("type")
                    .and_then(Value::as_str)
                    .map(safe_event_label)
                    .unwrap_or_else(|| "unknown".to_string());
                let is_remote_error = event_type == "error";
                observed_event_types.push(event_type);
                if is_remote_error {
                    return Err(ProbeFailure::RemoteError);
                }
                if let Some(response_id) = event
                    .pointer("/response/id")
                    .and_then(Value::as_str)
                    .map(str::trim)
                    .filter(|value| !value.is_empty())
                {
                    return Ok(response_id.to_string());
                }
            }
            WreqWsMessage::Ping(_) | WreqWsMessage::Pong(_) => continue,
            WreqWsMessage::Close(_) => return Err(ProbeFailure::MissingResponseId),
            _ => return Err(ProbeFailure::UnexpectedFrame),
        }
    }
    Err(ProbeFailure::MissingResponseId)
}

fn safe_event_label(value: &str) -> String {
    let trimmed = value.trim();
    if trimmed.is_empty()
        || trimmed.len() > 80
        || !trimmed
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'_' | b'-'))
    {
        return "unknown".to_string();
    }
    trimmed.to_string()
}

fn print_report(report: &ProbeReport) {
    match serde_json::to_string(report) {
        Ok(json) => println!("{json}"),
        Err(_) => println!("{{\"status\":\"failed\",\"error\":\"report_serialization_failed\"}}"),
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;
    use std::time::{Duration, Instant};

    use axum::extract::ws::{Message, WebSocket, WebSocketUpgrade};
    use axum::extract::State;
    use axum::http::HeaderMap;
    use axum::response::IntoResponse;
    use axum::routing::get;
    use axum::Router;
    use futures_util::{SinkExt, StreamExt};
    use serde_json::Value;
    use tokio::sync::{oneshot, Mutex};

    use super::{parse_probe_url, run_probe, ProbeConfig};

    #[derive(Default)]
    struct MockState {
        observed: Mutex<Option<oneshot::Sender<ObservedClientMessages>>>,
    }

    struct ObservedClientMessages {
        authorization_present: bool,
        account_header_present: bool,
        first: Value,
        second: Value,
    }

    #[tokio::test]
    async fn probe_confirms_sequential_response_continuation_without_exposing_values() {
        let (url, observed, server) = spawn_mock_server().await;
        let config = ProbeConfig {
            url: parse_probe_url(url.as_str()).expect("mock URL should be valid"),
            access_token: "test-token-that-must-not-be-reported".to_string(),
            account_id: "test-account-id".to_string(),
            model: "gpt-test".to_string(),
            turn_timeout: Duration::from_secs(2),
        };

        let report = run_probe(&config, Instant::now())
            .await
            .expect("probe should complete against mock server");
        let client_messages = observed.await.expect("mock should observe client messages");
        server.abort();

        assert_eq!(report.status, "passed");
        assert!(report.continuation_confirmed);
        assert!(report
            .observed_event_types
            .contains(&"response.created".to_string()));
        assert!(client_messages.authorization_present);
        assert!(client_messages.account_header_present);
        assert_eq!(client_messages.first["type"], "response.create");
        assert_eq!(client_messages.first["generate"], false);
        assert_eq!(client_messages.first["store"], false);
        assert_eq!(client_messages.second["previous_response_id"], "resp-first");
        let report_json = serde_json::to_string(&report).expect("report should serialize");
        assert!(!report_json.contains("test-token-that-must-not-be-reported"));
        assert!(!report_json.contains("test-account-id"));
        assert!(!report_json.contains("resp-first"));
    }

    #[test]
    fn probe_url_rejects_credentials_and_query_strings() {
        assert!(parse_probe_url("wss://example.test/v1/responses").is_ok());
        assert!(parse_probe_url("https://example.test/v1/responses").is_err());
        assert!(parse_probe_url("wss://token@example.test/v1/responses").is_err());
        assert!(parse_probe_url("wss://example.test/v1/responses?token=secret").is_err());
    }

    async fn spawn_mock_server() -> (
        String,
        oneshot::Receiver<ObservedClientMessages>,
        tokio::task::JoinHandle<()>,
    ) {
        let (observed_tx, observed_rx) = oneshot::channel();
        let state = Arc::new(MockState {
            observed: Mutex::new(Some(observed_tx)),
        });
        let app = Router::new()
            .route("/backend-api/codex/responses", get(mock_websocket))
            .with_state(state);
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("mock listener should bind");
        let address = listener
            .local_addr()
            .expect("mock listener should expose address");
        let server = tokio::spawn(async move {
            axum::serve(listener, app)
                .await
                .expect("mock server should run");
        });
        (
            format!("ws://{address}/backend-api/codex/responses"),
            observed_rx,
            server,
        )
    }

    async fn mock_websocket(
        ws: WebSocketUpgrade,
        State(state): State<Arc<MockState>>,
        headers: HeaderMap,
    ) -> impl IntoResponse {
        let authorization_present = headers
            .get("authorization")
            .and_then(|value| value.to_str().ok())
            .is_some_and(|value| value.starts_with("Bearer "));
        let account_header_present = headers.contains_key("chatgpt-account-id");
        ws.on_upgrade(move |socket| async move {
            serve_mock_socket(socket, state, authorization_present, account_header_present).await;
        })
    }

    async fn serve_mock_socket(
        socket: WebSocket,
        state: Arc<MockState>,
        authorization_present: bool,
        account_header_present: bool,
    ) {
        let (mut sender, mut receiver) = socket.split();
        let first = receive_json(&mut receiver).await;
        let _ = sender
            .send(Message::Text(
                serde_json::json!({
                    "type": "response.created",
                    "response": {"id": "resp-first"}
                })
                .to_string()
                .into(),
            ))
            .await;
        let second = receive_json(&mut receiver).await;
        let _ = sender
            .send(Message::Text(
                serde_json::json!({
                    "type": "response.created",
                    "response": {"id": "resp-second"}
                })
                .to_string()
                .into(),
            ))
            .await;
        if let Some(observed) = state.observed.lock().await.take() {
            let _ = observed.send(ObservedClientMessages {
                authorization_present,
                account_header_present,
                first,
                second,
            });
        }
    }

    async fn receive_json(receiver: &mut futures_util::stream::SplitStream<WebSocket>) -> Value {
        let message = receiver
            .next()
            .await
            .expect("client should send a message")
            .expect("client message should be valid");
        let Message::Text(text) = message else {
            panic!("expected text message");
        };
        serde_json::from_str(text.as_str()).expect("client message should be JSON")
    }
}
