use http::Uri;

use super::{classify_control_route, headers};

#[test]
fn classifies_admin_adaptive_keys_list_as_admin_proxy_route() {
    let headers = headers(&[]);
    let uri: Uri = "/api/admin/adaptive/keys?provider_id=provider-openai"
        .parse()
        .expect("uri should parse");
    let decision =
        classify_control_route(&http::Method::GET, &uri, &headers).expect("route should classify");

    assert_eq!(decision.route_class.as_deref(), Some("admin_proxy"));
    assert_eq!(decision.route_family.as_deref(), Some("adaptive_manage"));
    assert_eq!(decision.route_kind.as_deref(), Some("list_keys"));
    assert_eq!(
        decision.auth_endpoint_signature.as_deref(),
        Some("admin:adaptive")
    );
    assert!(!decision.is_execution_runtime_candidate());
}

#[test]
fn classifies_admin_adaptive_summary_as_admin_proxy_route() {
    let headers = headers(&[]);
    let uri: Uri = "/api/admin/adaptive/summary"
        .parse()
        .expect("uri should parse");
    let decision =
        classify_control_route(&http::Method::GET, &uri, &headers).expect("route should classify");

    assert_eq!(decision.route_class.as_deref(), Some("admin_proxy"));
    assert_eq!(decision.route_family.as_deref(), Some("adaptive_manage"));
    assert_eq!(decision.route_kind.as_deref(), Some("summary"));
    assert_eq!(
        decision.auth_endpoint_signature.as_deref(),
        Some("admin:adaptive")
    );
    assert!(!decision.is_execution_runtime_candidate());
}

#[test]
fn classifies_admin_adaptive_stats_as_admin_proxy_route() {
    let headers = headers(&[]);
    let uri: Uri = "/api/admin/adaptive/keys/key-openai/stats"
        .parse()
        .expect("uri should parse");
    let decision =
        classify_control_route(&http::Method::GET, &uri, &headers).expect("route should classify");

    assert_eq!(decision.route_class.as_deref(), Some("admin_proxy"));
    assert_eq!(decision.route_family.as_deref(), Some("adaptive_manage"));
    assert_eq!(decision.route_kind.as_deref(), Some("get_stats"));
    assert_eq!(
        decision.auth_endpoint_signature.as_deref(),
        Some("admin:adaptive")
    );
    assert!(!decision.is_execution_runtime_candidate());
}

#[test]
fn classifies_admin_adaptive_toggle_mode_as_admin_proxy_route() {
    let headers = headers(&[]);
    let uri: Uri = "/api/admin/adaptive/keys/key-openai/mode"
        .parse()
        .expect("uri should parse");
    let decision = classify_control_route(&http::Method::PATCH, &uri, &headers)
        .expect("route should classify");

    assert_eq!(decision.route_class.as_deref(), Some("admin_proxy"));
    assert_eq!(decision.route_family.as_deref(), Some("adaptive_manage"));
    assert_eq!(decision.route_kind.as_deref(), Some("toggle_mode"));
    assert_eq!(
        decision.auth_endpoint_signature.as_deref(),
        Some("admin:adaptive")
    );
    assert!(!decision.is_execution_runtime_candidate());
}

#[test]
fn classifies_admin_adaptive_set_limit_as_admin_proxy_route() {
    let headers = headers(&[]);
    let uri: Uri = "/api/admin/adaptive/keys/key-openai/limit?limit=9"
        .parse()
        .expect("uri should parse");
    let decision = classify_control_route(&http::Method::PATCH, &uri, &headers)
        .expect("route should classify");

    assert_eq!(decision.route_class.as_deref(), Some("admin_proxy"));
    assert_eq!(decision.route_family.as_deref(), Some("adaptive_manage"));
    assert_eq!(decision.route_kind.as_deref(), Some("set_limit"));
    assert_eq!(
        decision.auth_endpoint_signature.as_deref(),
        Some("admin:adaptive")
    );
    assert!(!decision.is_execution_runtime_candidate());
}

#[test]
fn classifies_admin_adaptive_reset_learning_as_admin_proxy_route() {
    let headers = headers(&[]);
    let uri: Uri = "/api/admin/adaptive/keys/key-openai/learning"
        .parse()
        .expect("uri should parse");
    let decision = classify_control_route(&http::Method::DELETE, &uri, &headers)
        .expect("route should classify");

    assert_eq!(decision.route_class.as_deref(), Some("admin_proxy"));
    assert_eq!(decision.route_family.as_deref(), Some("adaptive_manage"));
    assert_eq!(decision.route_kind.as_deref(), Some("reset_learning"));
    assert_eq!(
        decision.auth_endpoint_signature.as_deref(),
        Some("admin:adaptive")
    );
    assert!(!decision.is_execution_runtime_candidate());
}
