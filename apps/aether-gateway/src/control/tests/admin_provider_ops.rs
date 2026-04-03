use http::Uri;

use super::{classify_control_route, headers};

#[test]
fn classifies_admin_provider_ops_architectures_list_as_admin_proxy_route() {
    let headers = headers(&[]);
    let uri: Uri = "/api/admin/provider-ops/architectures?limit=20"
        .parse()
        .expect("uri should parse");
    let decision =
        classify_control_route(&http::Method::GET, &uri, &headers).expect("route should classify");

    assert_eq!(decision.route_class.as_deref(), Some("admin_proxy"));
    assert_eq!(
        decision.route_family.as_deref(),
        Some("provider_ops_manage")
    );
    assert_eq!(decision.route_kind.as_deref(), Some("list_architectures"));
    assert_eq!(
        decision.auth_endpoint_signature.as_deref(),
        Some("admin:provider_ops")
    );
    assert!(!decision.is_execution_runtime_candidate());
}

#[test]
fn classifies_admin_provider_ops_architecture_detail_as_admin_proxy_route() {
    let headers = headers(&[]);
    let uri: Uri = "/api/admin/provider-ops/architectures/generic_api"
        .parse()
        .expect("uri should parse");
    let decision =
        classify_control_route(&http::Method::GET, &uri, &headers).expect("route should classify");

    assert_eq!(decision.route_class.as_deref(), Some("admin_proxy"));
    assert_eq!(
        decision.route_family.as_deref(),
        Some("provider_ops_manage")
    );
    assert_eq!(decision.route_kind.as_deref(), Some("get_architecture"));
    assert_eq!(
        decision.auth_endpoint_signature.as_deref(),
        Some("admin:provider_ops")
    );
    assert!(!decision.is_execution_runtime_candidate());
}

#[test]
fn classifies_admin_provider_ops_provider_status_as_admin_proxy_route() {
    let headers = headers(&[]);
    let uri: Uri = "/api/admin/provider-ops/providers/provider-openai/status"
        .parse()
        .expect("uri should parse");
    let decision =
        classify_control_route(&http::Method::GET, &uri, &headers).expect("route should classify");

    assert_eq!(decision.route_class.as_deref(), Some("admin_proxy"));
    assert_eq!(
        decision.route_family.as_deref(),
        Some("provider_ops_manage")
    );
    assert_eq!(decision.route_kind.as_deref(), Some("get_provider_status"));
    assert_eq!(
        decision.auth_endpoint_signature.as_deref(),
        Some("admin:provider_ops")
    );
    assert!(!decision.is_execution_runtime_candidate());
}

#[test]
fn classifies_admin_provider_ops_provider_config_as_admin_proxy_route() {
    let headers = headers(&[]);
    let uri: Uri = "/api/admin/provider-ops/providers/provider-openai/config"
        .parse()
        .expect("uri should parse");
    let decision =
        classify_control_route(&http::Method::GET, &uri, &headers).expect("route should classify");

    assert_eq!(decision.route_class.as_deref(), Some("admin_proxy"));
    assert_eq!(
        decision.route_family.as_deref(),
        Some("provider_ops_manage")
    );
    assert_eq!(decision.route_kind.as_deref(), Some("get_provider_config"));
    assert_eq!(
        decision.auth_endpoint_signature.as_deref(),
        Some("admin:provider_ops")
    );
    assert!(!decision.is_execution_runtime_candidate());
}

#[test]
fn classifies_admin_provider_ops_save_provider_config_as_admin_proxy_route() {
    let headers = headers(&[]);
    let uri: Uri = "/api/admin/provider-ops/providers/provider-openai/config"
        .parse()
        .expect("uri should parse");
    let decision =
        classify_control_route(&http::Method::PUT, &uri, &headers).expect("route should classify");

    assert_eq!(decision.route_class.as_deref(), Some("admin_proxy"));
    assert_eq!(
        decision.route_family.as_deref(),
        Some("provider_ops_manage")
    );
    assert_eq!(decision.route_kind.as_deref(), Some("save_provider_config"));
    assert_eq!(
        decision.auth_endpoint_signature.as_deref(),
        Some("admin:provider_ops")
    );
    assert!(!decision.is_execution_runtime_candidate());
}

#[test]
fn classifies_admin_provider_ops_delete_provider_config_as_admin_proxy_route() {
    let headers = headers(&[]);
    let uri: Uri = "/api/admin/provider-ops/providers/provider-openai/config"
        .parse()
        .expect("uri should parse");
    let decision = classify_control_route(&http::Method::DELETE, &uri, &headers)
        .expect("route should classify");

    assert_eq!(decision.route_class.as_deref(), Some("admin_proxy"));
    assert_eq!(
        decision.route_family.as_deref(),
        Some("provider_ops_manage")
    );
    assert_eq!(
        decision.route_kind.as_deref(),
        Some("delete_provider_config")
    );
    assert_eq!(
        decision.auth_endpoint_signature.as_deref(),
        Some("admin:provider_ops")
    );
    assert!(!decision.is_execution_runtime_candidate());
}

#[test]
fn classifies_admin_provider_ops_disconnect_provider_as_admin_proxy_route() {
    let headers = headers(&[]);
    let uri: Uri = "/api/admin/provider-ops/providers/provider-openai/disconnect"
        .parse()
        .expect("uri should parse");
    let decision =
        classify_control_route(&http::Method::POST, &uri, &headers).expect("route should classify");

    assert_eq!(decision.route_class.as_deref(), Some("admin_proxy"));
    assert_eq!(
        decision.route_family.as_deref(),
        Some("provider_ops_manage")
    );
    assert_eq!(decision.route_kind.as_deref(), Some("disconnect_provider"));
    assert_eq!(
        decision.auth_endpoint_signature.as_deref(),
        Some("admin:provider_ops")
    );
    assert!(!decision.is_execution_runtime_candidate());
}

#[test]
fn classifies_admin_provider_ops_connect_provider_as_admin_proxy_route() {
    let headers = headers(&[]);
    let uri: Uri = "/api/admin/provider-ops/providers/provider-openai/connect"
        .parse()
        .expect("uri should parse");
    let decision =
        classify_control_route(&http::Method::POST, &uri, &headers).expect("route should classify");

    assert_eq!(decision.route_class.as_deref(), Some("admin_proxy"));
    assert_eq!(
        decision.route_family.as_deref(),
        Some("provider_ops_manage")
    );
    assert_eq!(decision.route_kind.as_deref(), Some("connect_provider"));
    assert_eq!(
        decision.auth_endpoint_signature.as_deref(),
        Some("admin:provider_ops")
    );
    assert!(!decision.is_execution_runtime_candidate());
}

#[test]
fn classifies_admin_provider_ops_verify_provider_as_admin_proxy_route() {
    let headers = headers(&[]);
    let uri: Uri = "/api/admin/provider-ops/providers/provider-openai/verify"
        .parse()
        .expect("uri should parse");
    let decision =
        classify_control_route(&http::Method::POST, &uri, &headers).expect("route should classify");

    assert_eq!(decision.route_class.as_deref(), Some("admin_proxy"));
    assert_eq!(
        decision.route_family.as_deref(),
        Some("provider_ops_manage")
    );
    assert_eq!(decision.route_kind.as_deref(), Some("verify_provider"));
    assert_eq!(
        decision.auth_endpoint_signature.as_deref(),
        Some("admin:provider_ops")
    );
    assert!(!decision.is_execution_runtime_candidate());
}

#[test]
fn classifies_admin_provider_ops_get_balance_as_admin_proxy_route() {
    let headers = headers(&[]);
    let uri: Uri = "/api/admin/provider-ops/providers/provider-openai/balance"
        .parse()
        .expect("uri should parse");
    let decision =
        classify_control_route(&http::Method::GET, &uri, &headers).expect("route should classify");

    assert_eq!(decision.route_class.as_deref(), Some("admin_proxy"));
    assert_eq!(
        decision.route_family.as_deref(),
        Some("provider_ops_manage")
    );
    assert_eq!(decision.route_kind.as_deref(), Some("get_provider_balance"));
    assert_eq!(
        decision.auth_endpoint_signature.as_deref(),
        Some("admin:provider_ops")
    );
    assert!(!decision.is_execution_runtime_candidate());
}

#[test]
fn classifies_admin_provider_ops_refresh_balance_as_admin_proxy_route() {
    let headers = headers(&[]);
    let uri: Uri = "/api/admin/provider-ops/providers/provider-openai/balance"
        .parse()
        .expect("uri should parse");
    let decision =
        classify_control_route(&http::Method::POST, &uri, &headers).expect("route should classify");

    assert_eq!(decision.route_class.as_deref(), Some("admin_proxy"));
    assert_eq!(
        decision.route_family.as_deref(),
        Some("provider_ops_manage")
    );
    assert_eq!(
        decision.route_kind.as_deref(),
        Some("refresh_provider_balance")
    );
    assert_eq!(
        decision.auth_endpoint_signature.as_deref(),
        Some("admin:provider_ops")
    );
    assert!(!decision.is_execution_runtime_candidate());
}

#[test]
fn classifies_admin_provider_ops_checkin_as_admin_proxy_route() {
    let headers = headers(&[]);
    let uri: Uri = "/api/admin/provider-ops/providers/provider-openai/checkin"
        .parse()
        .expect("uri should parse");
    let decision =
        classify_control_route(&http::Method::POST, &uri, &headers).expect("route should classify");

    assert_eq!(decision.route_class.as_deref(), Some("admin_proxy"));
    assert_eq!(
        decision.route_family.as_deref(),
        Some("provider_ops_manage")
    );
    assert_eq!(decision.route_kind.as_deref(), Some("provider_checkin"));
    assert_eq!(
        decision.auth_endpoint_signature.as_deref(),
        Some("admin:provider_ops")
    );
    assert!(!decision.is_execution_runtime_candidate());
}

#[test]
fn classifies_admin_provider_ops_execute_action_as_admin_proxy_route() {
    let headers = headers(&[]);
    let uri: Uri = "/api/admin/provider-ops/providers/provider-openai/actions/query_balance"
        .parse()
        .expect("uri should parse");
    let decision =
        classify_control_route(&http::Method::POST, &uri, &headers).expect("route should classify");

    assert_eq!(decision.route_class.as_deref(), Some("admin_proxy"));
    assert_eq!(
        decision.route_family.as_deref(),
        Some("provider_ops_manage")
    );
    assert_eq!(
        decision.route_kind.as_deref(),
        Some("execute_provider_action")
    );
    assert_eq!(
        decision.auth_endpoint_signature.as_deref(),
        Some("admin:provider_ops")
    );
    assert!(!decision.is_execution_runtime_candidate());
}

#[test]
fn classifies_admin_provider_ops_batch_balance_as_admin_proxy_route() {
    let headers = headers(&[]);
    let uri: Uri = "/api/admin/provider-ops/batch/balance"
        .parse()
        .expect("uri should parse");
    let decision =
        classify_control_route(&http::Method::POST, &uri, &headers).expect("route should classify");

    assert_eq!(decision.route_class.as_deref(), Some("admin_proxy"));
    assert_eq!(
        decision.route_family.as_deref(),
        Some("provider_ops_manage")
    );
    assert_eq!(decision.route_kind.as_deref(), Some("batch_balance"));
    assert_eq!(
        decision.auth_endpoint_signature.as_deref(),
        Some("admin:provider_ops")
    );
    assert!(!decision.is_execution_runtime_candidate());
}
