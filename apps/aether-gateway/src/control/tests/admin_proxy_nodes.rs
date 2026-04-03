use http::Uri;

use super::{classify_control_route, headers};

#[test]
fn classifies_admin_proxy_nodes_list_as_admin_proxy_route() {
    let headers = headers(&[]);
    let uri: Uri = "/api/admin/proxy-nodes?status=online&skip=10&limit=20"
        .parse()
        .expect("uri should parse");
    let decision =
        classify_control_route(&http::Method::GET, &uri, &headers).expect("route should classify");

    assert_eq!(decision.route_class.as_deref(), Some("admin_proxy"));
    assert_eq!(decision.route_family.as_deref(), Some("proxy_nodes_manage"));
    assert_eq!(decision.route_kind.as_deref(), Some("list_nodes"));
    assert_eq!(
        decision.auth_endpoint_signature.as_deref(),
        Some("admin:proxy_nodes")
    );
    assert!(!decision.is_execution_runtime_candidate());
}

#[test]
fn classifies_admin_proxy_nodes_register_as_admin_proxy_route() {
    let headers = headers(&[]);
    let uri: Uri = "/api/admin/proxy-nodes/register"
        .parse()
        .expect("uri should parse");
    let decision =
        classify_control_route(&http::Method::POST, &uri, &headers).expect("route should classify");

    assert_eq!(decision.route_class.as_deref(), Some("admin_proxy"));
    assert_eq!(decision.route_family.as_deref(), Some("proxy_nodes_manage"));
    assert_eq!(decision.route_kind.as_deref(), Some("register_node"));
    assert_eq!(
        decision.auth_endpoint_signature.as_deref(),
        Some("admin:proxy_nodes")
    );
    assert!(!decision.is_execution_runtime_candidate());
}

#[test]
fn classifies_admin_proxy_nodes_events_as_admin_proxy_route() {
    let headers = headers(&[]);
    let uri: Uri = "/api/admin/proxy-nodes/node-1/events?limit=50"
        .parse()
        .expect("uri should parse");
    let decision =
        classify_control_route(&http::Method::GET, &uri, &headers).expect("route should classify");

    assert_eq!(decision.route_class.as_deref(), Some("admin_proxy"));
    assert_eq!(decision.route_family.as_deref(), Some("proxy_nodes_manage"));
    assert_eq!(decision.route_kind.as_deref(), Some("list_node_events"));
    assert_eq!(
        decision.auth_endpoint_signature.as_deref(),
        Some("admin:proxy_nodes")
    );
    assert!(!decision.is_execution_runtime_candidate());
}
