use http::Uri;

use super::{classify_control_route, headers};

#[test]
fn classifies_admin_security_blacklist_add_as_admin_proxy_route() {
    let headers = headers(&[]);
    let uri: Uri = "/api/admin/security/ip/blacklist"
        .parse()
        .expect("uri should parse");
    let decision =
        classify_control_route(&http::Method::POST, &uri, &headers).expect("route should classify");

    assert_eq!(decision.route_class.as_deref(), Some("admin_proxy"));
    assert_eq!(decision.route_family.as_deref(), Some("security_manage"));
    assert_eq!(decision.route_kind.as_deref(), Some("blacklist_add"));
    assert_eq!(
        decision.auth_endpoint_signature.as_deref(),
        Some("admin:security")
    );
    assert!(!decision.is_execution_runtime_candidate());
}

#[test]
fn classifies_admin_security_blacklist_remove_as_admin_proxy_route() {
    let headers = headers(&[]);
    let uri: Uri = "/api/admin/security/ip/blacklist/1.2.3.4"
        .parse()
        .expect("uri should parse");
    let decision = classify_control_route(&http::Method::DELETE, &uri, &headers)
        .expect("route should classify");

    assert_eq!(decision.route_class.as_deref(), Some("admin_proxy"));
    assert_eq!(decision.route_family.as_deref(), Some("security_manage"));
    assert_eq!(decision.route_kind.as_deref(), Some("blacklist_remove"));
    assert_eq!(
        decision.auth_endpoint_signature.as_deref(),
        Some("admin:security")
    );
    assert!(!decision.is_execution_runtime_candidate());
}

#[test]
fn classifies_admin_security_blacklist_stats_as_admin_proxy_route() {
    let headers = headers(&[]);
    let uri: Uri = "/api/admin/security/ip/blacklist/stats"
        .parse()
        .expect("uri should parse");
    let decision =
        classify_control_route(&http::Method::GET, &uri, &headers).expect("route should classify");

    assert_eq!(decision.route_class.as_deref(), Some("admin_proxy"));
    assert_eq!(decision.route_family.as_deref(), Some("security_manage"));
    assert_eq!(decision.route_kind.as_deref(), Some("blacklist_stats"));
    assert_eq!(
        decision.auth_endpoint_signature.as_deref(),
        Some("admin:security")
    );
    assert!(!decision.is_execution_runtime_candidate());
}

#[test]
fn classifies_admin_security_blacklist_list_as_admin_proxy_route() {
    let headers = headers(&[]);
    let uri: Uri = "/api/admin/security/ip/blacklist"
        .parse()
        .expect("uri should parse");
    let decision =
        classify_control_route(&http::Method::GET, &uri, &headers).expect("route should classify");

    assert_eq!(decision.route_class.as_deref(), Some("admin_proxy"));
    assert_eq!(decision.route_family.as_deref(), Some("security_manage"));
    assert_eq!(decision.route_kind.as_deref(), Some("blacklist_list"));
    assert_eq!(
        decision.auth_endpoint_signature.as_deref(),
        Some("admin:security")
    );
    assert!(!decision.is_execution_runtime_candidate());
}

#[test]
fn classifies_admin_security_whitelist_add_as_admin_proxy_route() {
    let headers = headers(&[]);
    let uri: Uri = "/api/admin/security/ip/whitelist"
        .parse()
        .expect("uri should parse");
    let decision =
        classify_control_route(&http::Method::POST, &uri, &headers).expect("route should classify");

    assert_eq!(decision.route_class.as_deref(), Some("admin_proxy"));
    assert_eq!(decision.route_family.as_deref(), Some("security_manage"));
    assert_eq!(decision.route_kind.as_deref(), Some("whitelist_add"));
    assert_eq!(
        decision.auth_endpoint_signature.as_deref(),
        Some("admin:security")
    );
    assert!(!decision.is_execution_runtime_candidate());
}

#[test]
fn classifies_admin_security_whitelist_remove_as_admin_proxy_route() {
    let headers = headers(&[]);
    let uri: Uri = "/api/admin/security/ip/whitelist/1.2.3.4"
        .parse()
        .expect("uri should parse");
    let decision = classify_control_route(&http::Method::DELETE, &uri, &headers)
        .expect("route should classify");

    assert_eq!(decision.route_class.as_deref(), Some("admin_proxy"));
    assert_eq!(decision.route_family.as_deref(), Some("security_manage"));
    assert_eq!(decision.route_kind.as_deref(), Some("whitelist_remove"));
    assert_eq!(
        decision.auth_endpoint_signature.as_deref(),
        Some("admin:security")
    );
    assert!(!decision.is_execution_runtime_candidate());
}

#[test]
fn classifies_admin_security_whitelist_list_as_admin_proxy_route() {
    let headers = headers(&[]);
    let uri: Uri = "/api/admin/security/ip/whitelist"
        .parse()
        .expect("uri should parse");
    let decision =
        classify_control_route(&http::Method::GET, &uri, &headers).expect("route should classify");

    assert_eq!(decision.route_class.as_deref(), Some("admin_proxy"));
    assert_eq!(decision.route_family.as_deref(), Some("security_manage"));
    assert_eq!(decision.route_kind.as_deref(), Some("whitelist_list"));
    assert_eq!(
        decision.auth_endpoint_signature.as_deref(),
        Some("admin:security")
    );
    assert!(!decision.is_execution_runtime_candidate());
}
