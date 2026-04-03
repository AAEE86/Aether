use http::Uri;

use super::{classify_control_route, headers};

#[test]
fn classifies_admin_provider_query_models_as_admin_proxy_route() {
    let headers = headers(&[]);
    let uri: Uri = "/api/admin/provider-query/models"
        .parse()
        .expect("uri should parse");
    let decision =
        classify_control_route(&http::Method::POST, &uri, &headers).expect("route should classify");

    assert_eq!(decision.route_class.as_deref(), Some("admin_proxy"));
    assert_eq!(
        decision.route_family.as_deref(),
        Some("provider_query_manage")
    );
    assert_eq!(decision.route_kind.as_deref(), Some("query_models"));
    assert_eq!(
        decision.auth_endpoint_signature.as_deref(),
        Some("admin:provider_query")
    );
    assert!(!decision.is_execution_runtime_candidate());
}

#[test]
fn classifies_admin_provider_query_test_model_as_admin_proxy_route() {
    let headers = headers(&[]);
    let uri: Uri = "/api/admin/provider-query/test-model"
        .parse()
        .expect("uri should parse");
    let decision =
        classify_control_route(&http::Method::POST, &uri, &headers).expect("route should classify");

    assert_eq!(decision.route_class.as_deref(), Some("admin_proxy"));
    assert_eq!(
        decision.route_family.as_deref(),
        Some("provider_query_manage")
    );
    assert_eq!(decision.route_kind.as_deref(), Some("test_model"));
    assert_eq!(
        decision.auth_endpoint_signature.as_deref(),
        Some("admin:provider_query")
    );
    assert!(!decision.is_execution_runtime_candidate());
}

#[test]
fn classifies_admin_provider_query_test_model_failover_as_admin_proxy_route() {
    let headers = headers(&[]);
    let uri: Uri = "/api/admin/provider-query/test-model-failover"
        .parse()
        .expect("uri should parse");
    let decision =
        classify_control_route(&http::Method::POST, &uri, &headers).expect("route should classify");

    assert_eq!(decision.route_class.as_deref(), Some("admin_proxy"));
    assert_eq!(
        decision.route_family.as_deref(),
        Some("provider_query_manage")
    );
    assert_eq!(decision.route_kind.as_deref(), Some("test_model_failover"));
    assert_eq!(
        decision.auth_endpoint_signature.as_deref(),
        Some("admin:provider_query")
    );
    assert!(!decision.is_execution_runtime_candidate());
}
