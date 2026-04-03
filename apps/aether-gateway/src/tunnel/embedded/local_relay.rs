use std::io;
use std::net::SocketAddr;
use std::time::Duration;

use aether_contracts::tunnel::TUNNEL_RELAY_FORWARDED_BY_HEADER;
use aether_runtime::{maybe_hold_axum_response_permit, AdmissionPermit};
use async_stream::stream;
use axum::body::{Body, Bytes};
use axum::extract::{ConnectInfo, Path, Request, State};
use axum::http::{HeaderMap, HeaderName, HeaderValue, Response, StatusCode};
use axum::response::IntoResponse;
use bytes::BytesMut;
use futures_util::StreamExt;
use tracing::warn;

use super::hub::{LocalBodyEvent, LocalStream};
use super::protocol;
use super::AppState;

pub const TUNNEL_ERROR_HEADER: &str = "x-aether-tunnel-error";
const MAX_RELAY_META_LEN: usize = 256 * 1024;

struct StreamGuard {
    hub: std::sync::Arc<super::hub::HubRouter>,
    stream_id: u64,
    finished: bool,
}

impl Drop for StreamGuard {
    fn drop(&mut self) {
        if !self.finished {
            self.hub
                .cancel_local_stream(self.stream_id, "local relay client dropped");
        }
    }
}

pub async fn relay_request(
    Path(node_id): Path<String>,
    State(state): State<AppState>,
    ConnectInfo(addr): ConnectInfo<SocketAddr>,
    request: Request,
) -> impl IntoResponse {
    let forwarded_by_gateway = request
        .headers()
        .get(TUNNEL_RELAY_FORWARDED_BY_HEADER)
        .and_then(|value| value.to_str().ok())
        .map(str::trim)
        .is_some_and(|value| !value.is_empty());
    if !addr.ip().is_loopback() && !forwarded_by_gateway {
        return tunnel_error_response(
            StatusCode::FORBIDDEN,
            "forbidden",
            "local relay only accepts loopback requests",
        );
    }

    let request_permit = match state.try_acquire_request_permit().await {
        Ok(permit) => permit,
        Err(super::RequestAdmissionError::Local(aether_runtime::ConcurrencyError::Saturated {
            ..
        }))
        | Err(super::RequestAdmissionError::Distributed(
            aether_runtime::DistributedConcurrencyError::Saturated { .. },
        ))
        | Err(super::RequestAdmissionError::Distributed(
            aether_runtime::DistributedConcurrencyError::Unavailable { .. },
        )) => {
            return tunnel_error_response(
                StatusCode::SERVICE_UNAVAILABLE,
                "overloaded",
                "hub relay overloaded",
            );
        }
        Err(super::RequestAdmissionError::Local(aether_runtime::ConcurrencyError::Closed {
            ..
        })) => {
            return tunnel_error_response(
                StatusCode::SERVICE_UNAVAILABLE,
                "overloaded",
                "hub relay gate closed",
            );
        }
        Err(super::RequestAdmissionError::Distributed(
            aether_runtime::DistributedConcurrencyError::InvalidConfiguration(_),
        )) => {
            return tunnel_error_response(
                StatusCode::SERVICE_UNAVAILABLE,
                "overloaded",
                "hub relay distributed gate invalid",
            );
        }
    };

    let mut body_stream = request.into_body().into_data_stream();
    let mut envelope_buf = BytesMut::new();
    let mut meta: Option<protocol::RequestMeta> = None;
    let mut stream: Option<std::sync::Arc<LocalStream>> = None;

    while let Some(chunk_result) = body_stream.next().await {
        let chunk = match chunk_result {
            Ok(chunk) => chunk,
            Err(error) => {
                if let Some(active_stream) = &stream {
                    state
                        .hub
                        .cancel_local_stream(active_stream.id, "failed to read relay request body");
                }
                warn!(error = %error, "failed to read local relay request body");
                return release_permit_response(
                    tunnel_error_response(
                        StatusCode::BAD_GATEWAY,
                        "relay",
                        "failed to read relay request body",
                    ),
                    request_permit,
                );
            }
        };

        if stream.is_none() {
            envelope_buf.extend_from_slice(&chunk);
            let Some((parsed_meta, body_offset)) = (match try_decode_envelope_meta(&envelope_buf) {
                Ok(result) => result,
                Err(error) => {
                    return release_permit_response(
                        tunnel_error_response(StatusCode::BAD_REQUEST, "bad_request", &error),
                        request_permit,
                    );
                }
            }) else {
                continue;
            };

            let opened_stream = match state.hub.open_local_stream(&node_id, &parsed_meta) {
                Ok(stream) => stream,
                Err(error) => {
                    return release_permit_response(
                        tunnel_error_response(StatusCode::SERVICE_UNAVAILABLE, "connect", &error),
                        request_permit,
                    );
                }
            };

            if envelope_buf.len() > body_offset {
                let first_body_chunk = Bytes::copy_from_slice(&envelope_buf[body_offset..]);
                if let Err(error) =
                    state
                        .hub
                        .push_local_request_body(opened_stream.id, first_body_chunk, false)
                {
                    state.hub.cancel_local_stream(opened_stream.id, &error);
                    return release_permit_response(
                        tunnel_error_response(StatusCode::SERVICE_UNAVAILABLE, "connect", &error),
                        request_permit,
                    );
                }
            }

            envelope_buf.clear();
            meta = Some(parsed_meta);
            stream = Some(opened_stream);
            continue;
        }

        let Some(active_stream) = &stream else {
            continue;
        };
        if let Err(error) = state
            .hub
            .push_local_request_body(active_stream.id, chunk, false)
        {
            state.hub.cancel_local_stream(active_stream.id, &error);
            return release_permit_response(
                tunnel_error_response(StatusCode::SERVICE_UNAVAILABLE, "connect", &error),
                request_permit,
            );
        }
    }

    let (meta, stream) = match (meta, stream) {
        (Some(meta), Some(stream)) => (meta, stream),
        _ => {
            return release_permit_response(
                tunnel_error_response(
                    StatusCode::BAD_REQUEST,
                    "bad_request",
                    "relay envelope metadata truncated",
                ),
                request_permit,
            );
        }
    };

    if let Err(error) = state
        .hub
        .push_local_request_body(stream.id, Bytes::new(), true)
    {
        state.hub.cancel_local_stream(stream.id, &error);
        return release_permit_response(
            tunnel_error_response(StatusCode::SERVICE_UNAVAILABLE, "connect", &error),
            request_permit,
        );
    }

    let request_guard = StreamGuard {
        hub: state.hub.clone(),
        stream_id: stream.id,
        finished: false,
    };

    let wait_timeout = Duration::from_secs(meta.timeout.clamp(5, 300));
    let response_head = match stream.wait_headers(wait_timeout).await {
        Ok(response) => response,
        Err(error) => {
            state.hub.cancel_local_stream(stream.id, &error);
            return release_permit_response(
                tunnel_error_response(StatusCode::GATEWAY_TIMEOUT, "timeout", &error),
                request_permit,
            );
        }
    };

    let Some(mut body_rx) = stream.take_body_receiver() else {
        state
            .hub
            .cancel_local_stream(stream.id, "missing relay response body receiver");
        return release_permit_response(
            tunnel_error_response(
                StatusCode::BAD_GATEWAY,
                "relay",
                "missing relay response body receiver",
            ),
            request_permit,
        );
    };

    let hub = state.hub.clone();
    let stream_id = stream.id;
    let body_stream = stream! {
        let mut guard = request_guard;
        guard.hub = hub;
        guard.stream_id = stream_id;
        while let Some(event) = body_rx.recv().await {
            match event {
                LocalBodyEvent::Chunk(chunk) => yield Ok::<Bytes, io::Error>(chunk),
                LocalBodyEvent::End => {
                    guard.finished = true;
                    break;
                }
                LocalBodyEvent::Error(error) => {
                    guard.finished = true;
                    yield Err(io::Error::other(error));
                    break;
                }
            }
        }
        guard.finished = true;
    };

    let mut builder = Response::builder().status(response_head.status);
    if let Some(headers) = builder.headers_mut() {
        append_headers(headers, &response_head.headers);
    }
    match builder.body(Body::from_stream(body_stream)) {
        Ok(response) => maybe_hold_axum_response_permit(response, request_permit),
        Err(error) => {
            warn!(error = %error, "failed to build relay response");
            release_permit_response(
                tunnel_error_response(
                    StatusCode::BAD_GATEWAY,
                    "relay",
                    "failed to build relay response",
                ),
                request_permit,
            )
        }
    }
}

fn release_permit_response(
    response: Response<Body>,
    _request_permit: Option<AdmissionPermit>,
) -> Response<Body> {
    response
}

fn try_decode_envelope_meta(
    buffer: &BytesMut,
) -> Result<Option<(protocol::RequestMeta, usize)>, String> {
    if buffer.len() < 4 {
        return Ok(None);
    }
    let meta_len = u32::from_be_bytes([buffer[0], buffer[1], buffer[2], buffer[3]]) as usize;
    if meta_len > MAX_RELAY_META_LEN {
        return Err("relay metadata too large".to_string());
    }
    let meta_end = 4usize
        .checked_add(meta_len)
        .ok_or_else(|| "relay envelope length overflow".to_string())?;
    if buffer.len() < meta_end {
        return Ok(None);
    }
    let meta = serde_json::from_slice::<protocol::RequestMeta>(&buffer[4..meta_end])
        .map_err(|e| format!("invalid relay metadata: {e}"))?;
    Ok(Some((meta, meta_end)))
}

fn append_headers(target: &mut HeaderMap, headers: &[(String, String)]) {
    for (name, value) in headers {
        let Ok(name) = HeaderName::from_bytes(name.as_bytes()) else {
            continue;
        };
        let Ok(value) = HeaderValue::from_str(value) else {
            continue;
        };
        target.append(name, value);
    }
}

fn tunnel_error_response(status: StatusCode, kind: &str, message: &str) -> Response<Body> {
    let mut builder = Response::builder().status(status);
    if let Some(headers) = builder.headers_mut() {
        headers.insert(
            HeaderName::from_static(TUNNEL_ERROR_HEADER),
            HeaderValue::from_str(kind).unwrap_or_else(|_| HeaderValue::from_static("relay")),
        );
        headers.insert(
            axum::http::header::CONTENT_TYPE,
            HeaderValue::from_static("text/plain; charset=utf-8"),
        );
    }
    builder
        .body(Body::from(message.to_string()))
        .unwrap_or_else(|_| Response::new(Body::from("relay error")))
}

#[cfg(test)]
mod tests {
    use super::super::{AppState, ConnConfig, ControlPlaneClient};
    use super::{relay_request, Body, Request, SocketAddr, StatusCode, TUNNEL_ERROR_HEADER};
    use aether_contracts::tunnel::TUNNEL_RELAY_FORWARDED_BY_HEADER;
    use axum::extract::{ConnectInfo, Path, State};
    use axum::response::IntoResponse;
    use std::time::Duration;

    fn test_app_state() -> AppState {
        AppState::new(
            ControlPlaneClient::disabled(),
            ConnConfig {
                ping_interval: Duration::from_secs(15),
                idle_timeout: Duration::from_secs(0),
                outbound_queue_capacity: 128,
            },
            128,
        )
    }

    #[tokio::test]
    async fn relay_rejects_non_loopback_without_forwarded_header() {
        let request = Request::builder()
            .body(Body::empty())
            .expect("request should build");
        let response = relay_request(
            Path("node-123".to_string()),
            State(test_app_state()),
            ConnectInfo(SocketAddr::from(([10, 0, 0, 1], 4242))),
            request,
        )
        .await
        .into_response();

        assert_eq!(response.status(), StatusCode::FORBIDDEN);
    }

    #[tokio::test]
    async fn relay_accepts_forwarded_gateway_request_from_non_loopback() {
        let request = Request::builder()
            .header(TUNNEL_RELAY_FORWARDED_BY_HEADER, "gateway-a")
            .body(Body::empty())
            .expect("request should build");
        let response = relay_request(
            Path("node-123".to_string()),
            State(test_app_state()),
            ConnectInfo(SocketAddr::from(([10, 0, 0, 1], 4242))),
            request,
        )
        .await
        .into_response();

        assert_eq!(response.status(), StatusCode::BAD_REQUEST);
        assert_eq!(
            response
                .headers()
                .get(TUNNEL_ERROR_HEADER)
                .and_then(|value| value.to_str().ok()),
            Some("bad_request")
        );
    }
}
