use crate::gateway::handlers::{query_param_value, unix_secs_to_rfc3339};
use crate::gateway::{AppState, GatewayError, GatewayPublicRequestContext};
use aether_data::repository::proxy_nodes::{StoredProxyNode, StoredProxyNodeEvent};
use axum::{
    body::Body,
    http,
    response::{IntoResponse, Response},
    Json,
};
use serde_json::json;

const ADMIN_PROXY_NODES_DATA_UNAVAILABLE_DETAIL: &str = "Admin proxy nodes data unavailable";

fn build_admin_proxy_nodes_data_unavailable_response() -> Response<Body> {
    (
        http::StatusCode::SERVICE_UNAVAILABLE,
        Json(json!({ "detail": ADMIN_PROXY_NODES_DATA_UNAVAILABLE_DETAIL })),
    )
        .into_response()
}

fn mask_admin_proxy_node_password(password: Option<&str>) -> Option<String> {
    let password = password?;
    if password.is_empty() {
        return None;
    }
    if password.len() < 8 {
        return Some("****".to_string());
    }
    Some(format!(
        "{}****{}",
        &password[..2],
        &password[password.len() - 2..]
    ))
}

fn build_admin_proxy_node_payload(node: &StoredProxyNode) -> serde_json::Value {
    let mut payload = serde_json::Map::from_iter([
        ("id".to_string(), json!(node.id)),
        ("name".to_string(), json!(node.name)),
        ("ip".to_string(), json!(node.ip)),
        ("port".to_string(), json!(node.port)),
        ("region".to_string(), json!(node.region)),
        ("status".to_string(), json!(node.status)),
        ("is_manual".to_string(), json!(node.is_manual)),
        ("tunnel_mode".to_string(), json!(node.tunnel_mode)),
        ("tunnel_connected".to_string(), json!(node.tunnel_connected)),
        (
            "tunnel_connected_at".to_string(),
            json!(node
                .tunnel_connected_at_unix_secs
                .and_then(unix_secs_to_rfc3339)),
        ),
        ("registered_by".to_string(), json!(node.registered_by)),
        (
            "last_heartbeat_at".to_string(),
            json!(node
                .last_heartbeat_at_unix_secs
                .and_then(unix_secs_to_rfc3339)),
        ),
        (
            "heartbeat_interval".to_string(),
            json!(node.heartbeat_interval),
        ),
        (
            "active_connections".to_string(),
            json!(node.active_connections),
        ),
        ("total_requests".to_string(), json!(node.total_requests)),
        ("avg_latency_ms".to_string(), json!(node.avg_latency_ms)),
        ("failed_requests".to_string(), json!(node.failed_requests)),
        ("dns_failures".to_string(), json!(node.dns_failures)),
        ("stream_errors".to_string(), json!(node.stream_errors)),
        ("proxy_metadata".to_string(), json!(node.proxy_metadata)),
        ("hardware_info".to_string(), json!(node.hardware_info)),
        (
            "estimated_max_concurrency".to_string(),
            json!(node.estimated_max_concurrency),
        ),
        ("remote_config".to_string(), json!(node.remote_config)),
        ("config_version".to_string(), json!(node.config_version)),
        (
            "created_at".to_string(),
            json!(node.created_at_unix_secs.and_then(unix_secs_to_rfc3339)),
        ),
        (
            "updated_at".to_string(),
            json!(node.updated_at_unix_secs.and_then(unix_secs_to_rfc3339)),
        ),
    ]);

    if node.is_manual {
        payload.insert("proxy_url".to_string(), json!(node.proxy_url));
        payload.insert("proxy_username".to_string(), json!(node.proxy_username));
        payload.insert(
            "proxy_password".to_string(),
            json!(mask_admin_proxy_node_password(
                node.proxy_password.as_deref()
            )),
        );
    }

    serde_json::Value::Object(payload)
}

fn build_admin_proxy_node_event_payload(event: &StoredProxyNodeEvent) -> serde_json::Value {
    json!({
        "id": event.id,
        "event_type": event.event_type,
        "detail": event.detail,
        "created_at": event.created_at_unix_secs.and_then(unix_secs_to_rfc3339),
    })
}

fn admin_proxy_node_event_node_id_from_path(request_path: &str) -> Option<&str> {
    let node_id = request_path.strip_prefix("/api/admin/proxy-nodes/")?;
    let node_id = node_id.strip_suffix("/events")?;
    if node_id.is_empty() || node_id.contains('/') {
        return None;
    }
    Some(node_id)
}

pub(crate) async fn maybe_build_local_admin_proxy_nodes_response(
    state: &AppState,
    request_context: &GatewayPublicRequestContext,
) -> Result<Option<Response<Body>>, GatewayError> {
    let Some(decision) = request_context.control_decision.as_ref() else {
        return Ok(None);
    };

    if decision.route_family.as_deref() != Some("proxy_nodes_manage") {
        return Ok(None);
    }

    if decision.route_kind.as_deref() == Some("list_nodes")
        && request_context.request_method == http::Method::GET
        && matches!(
            request_context.request_path.as_str(),
            "/api/admin/proxy-nodes" | "/api/admin/proxy-nodes/"
        )
    {
        if !state.data.has_proxy_node_reader() {
            return Ok(Some(build_admin_proxy_nodes_data_unavailable_response()));
        }

        let skip = query_param_value(request_context.request_query_string.as_deref(), "skip")
            .and_then(|value| value.parse::<usize>().ok())
            .unwrap_or(0);
        let limit = query_param_value(request_context.request_query_string.as_deref(), "limit")
            .and_then(|value| value.parse::<usize>().ok())
            .filter(|value| *value > 0 && *value <= 1000)
            .unwrap_or(100);
        let status = query_param_value(request_context.request_query_string.as_deref(), "status")
            .map(|value| value.trim().to_ascii_lowercase())
            .filter(|value| !value.is_empty());

        if let Some(status) = status.as_deref() {
            if !matches!(status, "offline" | "online") {
                return Ok(Some(
                    (
                        http::StatusCode::BAD_REQUEST,
                        Json(json!({
                            "detail": "status 必须是以下之一: ['offline', 'online']"
                        })),
                    )
                        .into_response(),
                ));
            }
        }

        let mut nodes = state.list_proxy_nodes().await?;
        nodes.sort_by(|left, right| left.name.cmp(&right.name));

        let filtered = nodes
            .into_iter()
            .filter(|node| {
                status
                    .as_deref()
                    .map(|value| node.status.eq_ignore_ascii_case(value))
                    .unwrap_or(true)
            })
            .collect::<Vec<_>>();
        let total = filtered.len();
        let items = filtered
            .into_iter()
            .skip(skip)
            .take(limit)
            .map(|node| build_admin_proxy_node_payload(&node))
            .collect::<Vec<_>>();

        return Ok(Some(
            Json(json!({
                "items": items,
                "total": total,
                "skip": skip,
                "limit": limit,
            }))
            .into_response(),
        ));
    }

    if decision.route_kind.as_deref() == Some("list_node_events")
        && request_context.request_method == http::Method::GET
    {
        if !state.data.has_proxy_node_reader() {
            return Ok(Some(build_admin_proxy_nodes_data_unavailable_response()));
        }

        let Some(node_id) = admin_proxy_node_event_node_id_from_path(&request_context.request_path)
        else {
            return Ok(Some(
                (
                    http::StatusCode::NOT_FOUND,
                    Json(json!({ "detail": "Proxy node 不存在" })),
                )
                    .into_response(),
            ));
        };

        if state.find_proxy_node(node_id).await?.is_none() {
            return Ok(Some(
                (
                    http::StatusCode::NOT_FOUND,
                    Json(json!({ "detail": "Proxy node 不存在" })),
                )
                    .into_response(),
            ));
        }

        let limit = query_param_value(request_context.request_query_string.as_deref(), "limit")
            .and_then(|value| value.parse::<usize>().ok())
            .filter(|value| *value > 0 && *value <= 200)
            .unwrap_or(50);
        let items = state
            .list_proxy_node_events(node_id, limit)
            .await?
            .into_iter()
            .map(|event| build_admin_proxy_node_event_payload(&event))
            .collect::<Vec<_>>();

        return Ok(Some(Json(json!({ "items": items })).into_response()));
    }

    Ok(Some(build_admin_proxy_nodes_data_unavailable_response()))
}
