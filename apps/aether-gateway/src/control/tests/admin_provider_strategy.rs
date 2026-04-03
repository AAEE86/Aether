use http::Uri;

use super::{classify_control_route, headers};

#[test]
fn classifies_admin_provider_strategy_list_as_admin_proxy_route() {
    let headers = headers(&[]);
    let uri: Uri = "/api/admin/provider-strategy/strategies"
        .parse()
        .expect("uri should parse");
    let decision =
        classify_control_route(&http::Method::GET, &uri, &headers).expect("route should classify");

    assert_eq!(decision.route_class.as_deref(), Some("admin_proxy"));
    assert_eq!(
        decision.route_family.as_deref(),
        Some("provider_strategy_manage")
    );
    assert_eq!(decision.route_kind.as_deref(), Some("list_strategies"));
    assert_eq!(
        decision.auth_endpoint_signature.as_deref(),
        Some("admin:provider_strategy")
    );
    assert!(!decision.is_execution_runtime_candidate());
}

#[test]
fn classifies_admin_provider_strategy_reset_quota_as_admin_proxy_route() {
    let headers = headers(&[]);
    let uri: Uri = "/api/admin/provider-strategy/providers/provider-openai/quota"
        .parse()
        .expect("uri should parse");
    let decision = classify_control_route(&http::Method::DELETE, &uri, &headers)
        .expect("route should classify");

    assert_eq!(decision.route_class.as_deref(), Some("admin_proxy"));
    assert_eq!(
        decision.route_family.as_deref(),
        Some("provider_strategy_manage")
    );
    assert_eq!(decision.route_kind.as_deref(), Some("reset_provider_quota"));
    assert_eq!(
        decision.auth_endpoint_signature.as_deref(),
        Some("admin:provider_strategy")
    );
    assert!(!decision.is_execution_runtime_candidate());
}

#[test]
fn classifies_admin_provider_strategy_update_billing_as_admin_proxy_route() {
    let headers = headers(&[]);
    let uri: Uri = "/api/admin/provider-strategy/providers/provider-openai/billing"
        .parse()
        .expect("uri should parse");
    let decision =
        classify_control_route(&http::Method::PUT, &uri, &headers).expect("route should classify");

    assert_eq!(decision.route_class.as_deref(), Some("admin_proxy"));
    assert_eq!(
        decision.route_family.as_deref(),
        Some("provider_strategy_manage")
    );
    assert_eq!(
        decision.route_kind.as_deref(),
        Some("update_provider_billing")
    );
    assert_eq!(
        decision.auth_endpoint_signature.as_deref(),
        Some("admin:provider_strategy")
    );
    assert!(!decision.is_execution_runtime_candidate());
}

#[test]
fn classifies_admin_provider_strategy_stats_as_admin_proxy_route() {
    let headers = headers(&[]);
    let uri: Uri = "/api/admin/provider-strategy/providers/provider-openai/stats?hours=48"
        .parse()
        .expect("uri should parse");
    let decision =
        classify_control_route(&http::Method::GET, &uri, &headers).expect("route should classify");

    assert_eq!(decision.route_class.as_deref(), Some("admin_proxy"));
    assert_eq!(
        decision.route_family.as_deref(),
        Some("provider_strategy_manage")
    );
    assert_eq!(decision.route_kind.as_deref(), Some("get_provider_stats"));
    assert_eq!(
        decision.auth_endpoint_signature.as_deref(),
        Some("admin:provider_strategy")
    );
    assert!(!decision.is_execution_runtime_candidate());
}
