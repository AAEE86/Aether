use http::Uri;

use super::{classify_control_route, headers};

#[test]
fn classifies_admin_payments_list_orders_as_admin_proxy_route() {
    let headers = headers(&[]);
    let uri: Uri = "/api/admin/payments/orders"
        .parse()
        .expect("uri should parse");
    let decision =
        classify_control_route(&http::Method::GET, &uri, &headers).expect("route should classify");

    assert_eq!(decision.route_class.as_deref(), Some("admin_proxy"));
    assert_eq!(decision.route_family.as_deref(), Some("payments_manage"));
    assert_eq!(decision.route_kind.as_deref(), Some("list_orders"));
    assert_eq!(
        decision.auth_endpoint_signature.as_deref(),
        Some("admin:payments")
    );
    assert!(!decision.is_execution_runtime_candidate());
}

#[test]
fn classifies_admin_payments_get_order_as_admin_proxy_route() {
    let headers = headers(&[]);
    let uri: Uri = "/api/admin/payments/orders/order-1"
        .parse()
        .expect("uri should parse");
    let decision =
        classify_control_route(&http::Method::GET, &uri, &headers).expect("route should classify");

    assert_eq!(decision.route_class.as_deref(), Some("admin_proxy"));
    assert_eq!(decision.route_family.as_deref(), Some("payments_manage"));
    assert_eq!(decision.route_kind.as_deref(), Some("get_order"));
    assert_eq!(
        decision.auth_endpoint_signature.as_deref(),
        Some("admin:payments")
    );
    assert!(!decision.is_execution_runtime_candidate());
}

#[test]
fn classifies_admin_payments_trailing_slash_routes_as_admin_proxy_route() {
    let headers = headers(&[]);

    let detail_uri: Uri = "/api/admin/payments/orders/order-1/"
        .parse()
        .expect("uri should parse");
    let detail = classify_control_route(&http::Method::GET, &detail_uri, &headers)
        .expect("detail route should classify");
    assert_eq!(detail.route_family.as_deref(), Some("payments_manage"));
    assert_eq!(detail.route_kind.as_deref(), Some("get_order"));

    let credit_uri: Uri = "/api/admin/payments/orders/order-1/credit/"
        .parse()
        .expect("uri should parse");
    let credit = classify_control_route(&http::Method::POST, &credit_uri, &headers)
        .expect("credit route should classify");
    assert_eq!(credit.route_family.as_deref(), Some("payments_manage"));
    assert_eq!(credit.route_kind.as_deref(), Some("credit_order"));
}

#[test]
fn classifies_admin_payments_expire_order_as_admin_proxy_route() {
    let headers = headers(&[]);
    let uri: Uri = "/api/admin/payments/orders/order-1/expire"
        .parse()
        .expect("uri should parse");
    let decision =
        classify_control_route(&http::Method::POST, &uri, &headers).expect("route should classify");

    assert_eq!(decision.route_class.as_deref(), Some("admin_proxy"));
    assert_eq!(decision.route_family.as_deref(), Some("payments_manage"));
    assert_eq!(decision.route_kind.as_deref(), Some("expire_order"));
    assert_eq!(
        decision.auth_endpoint_signature.as_deref(),
        Some("admin:payments")
    );
    assert!(!decision.is_execution_runtime_candidate());
}

#[test]
fn classifies_admin_payments_credit_order_as_admin_proxy_route() {
    let headers = headers(&[]);
    let uri: Uri = "/api/admin/payments/orders/order-1/credit"
        .parse()
        .expect("uri should parse");
    let decision =
        classify_control_route(&http::Method::POST, &uri, &headers).expect("route should classify");

    assert_eq!(decision.route_class.as_deref(), Some("admin_proxy"));
    assert_eq!(decision.route_family.as_deref(), Some("payments_manage"));
    assert_eq!(decision.route_kind.as_deref(), Some("credit_order"));
    assert_eq!(
        decision.auth_endpoint_signature.as_deref(),
        Some("admin:payments")
    );
    assert!(!decision.is_execution_runtime_candidate());
}

#[test]
fn classifies_admin_payments_fail_order_as_admin_proxy_route() {
    let headers = headers(&[]);
    let uri: Uri = "/api/admin/payments/orders/order-1/fail"
        .parse()
        .expect("uri should parse");
    let decision =
        classify_control_route(&http::Method::POST, &uri, &headers).expect("route should classify");

    assert_eq!(decision.route_class.as_deref(), Some("admin_proxy"));
    assert_eq!(decision.route_family.as_deref(), Some("payments_manage"));
    assert_eq!(decision.route_kind.as_deref(), Some("fail_order"));
    assert_eq!(
        decision.auth_endpoint_signature.as_deref(),
        Some("admin:payments")
    );
    assert!(!decision.is_execution_runtime_candidate());
}

#[test]
fn classifies_admin_payments_callbacks_as_admin_proxy_route() {
    let headers = headers(&[]);
    let uri: Uri = "/api/admin/payments/callbacks"
        .parse()
        .expect("uri should parse");
    let decision =
        classify_control_route(&http::Method::GET, &uri, &headers).expect("route should classify");

    assert_eq!(decision.route_class.as_deref(), Some("admin_proxy"));
    assert_eq!(decision.route_family.as_deref(), Some("payments_manage"));
    assert_eq!(decision.route_kind.as_deref(), Some("list_callbacks"));
    assert_eq!(
        decision.auth_endpoint_signature.as_deref(),
        Some("admin:payments")
    );
    assert!(!decision.is_execution_runtime_candidate());
}
