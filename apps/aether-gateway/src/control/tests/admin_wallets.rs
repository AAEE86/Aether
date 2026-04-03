use http::Uri;

use super::{classify_control_route, headers};

#[test]
fn classifies_admin_wallets_list_as_admin_proxy_route() {
    let headers = headers(&[]);
    let uri: Uri = "/api/admin/wallets?status=active&limit=20"
        .parse()
        .expect("uri should parse");
    let decision =
        classify_control_route(&http::Method::GET, &uri, &headers).expect("route should classify");

    assert_eq!(decision.route_class.as_deref(), Some("admin_proxy"));
    assert_eq!(decision.route_family.as_deref(), Some("wallets_manage"));
    assert_eq!(decision.route_kind.as_deref(), Some("list_wallets"));
    assert_eq!(
        decision.auth_endpoint_signature.as_deref(),
        Some("admin:wallets")
    );
    assert!(!decision.is_execution_runtime_candidate());
}

#[test]
fn classifies_admin_wallets_ledger_as_admin_proxy_route() {
    let headers = headers(&[]);
    let uri: Uri = "/api/admin/wallets/ledger?owner_type=user"
        .parse()
        .expect("uri should parse");
    let decision =
        classify_control_route(&http::Method::GET, &uri, &headers).expect("route should classify");

    assert_eq!(decision.route_class.as_deref(), Some("admin_proxy"));
    assert_eq!(decision.route_family.as_deref(), Some("wallets_manage"));
    assert_eq!(decision.route_kind.as_deref(), Some("ledger"));
    assert_eq!(
        decision.auth_endpoint_signature.as_deref(),
        Some("admin:wallets")
    );
    assert!(!decision.is_execution_runtime_candidate());
}

#[test]
fn classifies_admin_wallets_refund_requests_as_admin_proxy_route() {
    let headers = headers(&[]);
    let uri: Uri = "/api/admin/wallets/refund-requests?status=pending_approval"
        .parse()
        .expect("uri should parse");
    let decision =
        classify_control_route(&http::Method::GET, &uri, &headers).expect("route should classify");

    assert_eq!(decision.route_class.as_deref(), Some("admin_proxy"));
    assert_eq!(decision.route_family.as_deref(), Some("wallets_manage"));
    assert_eq!(decision.route_kind.as_deref(), Some("list_refund_requests"));
    assert_eq!(
        decision.auth_endpoint_signature.as_deref(),
        Some("admin:wallets")
    );
    assert!(!decision.is_execution_runtime_candidate());
}

#[test]
fn classifies_admin_wallets_detail_as_admin_proxy_route() {
    let headers = headers(&[]);
    let uri: Uri = "/api/admin/wallets/wallet-123"
        .parse()
        .expect("uri should parse");
    let decision =
        classify_control_route(&http::Method::GET, &uri, &headers).expect("route should classify");

    assert_eq!(decision.route_class.as_deref(), Some("admin_proxy"));
    assert_eq!(decision.route_family.as_deref(), Some("wallets_manage"));
    assert_eq!(decision.route_kind.as_deref(), Some("wallet_detail"));
    assert_eq!(
        decision.auth_endpoint_signature.as_deref(),
        Some("admin:wallets")
    );
    assert!(!decision.is_execution_runtime_candidate());
}

#[test]
fn classifies_admin_wallets_transactions_as_admin_proxy_route() {
    let headers = headers(&[]);
    let uri: Uri = "/api/admin/wallets/wallet-123/transactions?limit=50"
        .parse()
        .expect("uri should parse");
    let decision =
        classify_control_route(&http::Method::GET, &uri, &headers).expect("route should classify");

    assert_eq!(decision.route_class.as_deref(), Some("admin_proxy"));
    assert_eq!(decision.route_family.as_deref(), Some("wallets_manage"));
    assert_eq!(
        decision.route_kind.as_deref(),
        Some("list_wallet_transactions")
    );
    assert_eq!(
        decision.auth_endpoint_signature.as_deref(),
        Some("admin:wallets")
    );
    assert!(!decision.is_execution_runtime_candidate());
}

#[test]
fn classifies_admin_wallets_transactions_with_empty_wallet_id_as_admin_proxy_route() {
    let headers = headers(&[]);
    let uri: Uri = "/api/admin/wallets//transactions"
        .parse()
        .expect("uri should parse");
    let decision =
        classify_control_route(&http::Method::GET, &uri, &headers).expect("route should classify");

    assert_eq!(decision.route_class.as_deref(), Some("admin_proxy"));
    assert_eq!(decision.route_family.as_deref(), Some("wallets_manage"));
    assert_eq!(
        decision.route_kind.as_deref(),
        Some("list_wallet_transactions")
    );
    assert_eq!(
        decision.auth_endpoint_signature.as_deref(),
        Some("admin:wallets")
    );
    assert!(!decision.is_execution_runtime_candidate());
}

#[test]
fn classifies_admin_wallets_refunds_as_admin_proxy_route() {
    let headers = headers(&[]);
    let uri: Uri = "/api/admin/wallets/wallet-123/refunds?limit=50"
        .parse()
        .expect("uri should parse");
    let decision =
        classify_control_route(&http::Method::GET, &uri, &headers).expect("route should classify");

    assert_eq!(decision.route_class.as_deref(), Some("admin_proxy"));
    assert_eq!(decision.route_family.as_deref(), Some("wallets_manage"));
    assert_eq!(decision.route_kind.as_deref(), Some("list_wallet_refunds"));
    assert_eq!(
        decision.auth_endpoint_signature.as_deref(),
        Some("admin:wallets")
    );
    assert!(!decision.is_execution_runtime_candidate());
}

#[test]
fn classifies_admin_wallets_adjust_as_admin_proxy_route() {
    let headers = headers(&[]);
    let uri: Uri = "/api/admin/wallets/wallet-123/adjust"
        .parse()
        .expect("uri should parse");
    let decision =
        classify_control_route(&http::Method::POST, &uri, &headers).expect("route should classify");

    assert_eq!(decision.route_class.as_deref(), Some("admin_proxy"));
    assert_eq!(decision.route_family.as_deref(), Some("wallets_manage"));
    assert_eq!(decision.route_kind.as_deref(), Some("adjust_balance"));
    assert_eq!(
        decision.auth_endpoint_signature.as_deref(),
        Some("admin:wallets")
    );
    assert!(!decision.is_execution_runtime_candidate());
}

#[test]
fn classifies_admin_wallets_recharge_as_admin_proxy_route() {
    let headers = headers(&[]);
    let uri: Uri = "/api/admin/wallets/wallet-123/recharge"
        .parse()
        .expect("uri should parse");
    let decision =
        classify_control_route(&http::Method::POST, &uri, &headers).expect("route should classify");

    assert_eq!(decision.route_class.as_deref(), Some("admin_proxy"));
    assert_eq!(decision.route_family.as_deref(), Some("wallets_manage"));
    assert_eq!(decision.route_kind.as_deref(), Some("recharge_balance"));
    assert_eq!(
        decision.auth_endpoint_signature.as_deref(),
        Some("admin:wallets")
    );
    assert!(!decision.is_execution_runtime_candidate());
}

#[test]
fn classifies_admin_wallets_process_refund_as_admin_proxy_route() {
    let headers = headers(&[]);
    let uri: Uri = "/api/admin/wallets/wallet-123/refunds/refund-1/process"
        .parse()
        .expect("uri should parse");
    let decision =
        classify_control_route(&http::Method::POST, &uri, &headers).expect("route should classify");

    assert_eq!(decision.route_class.as_deref(), Some("admin_proxy"));
    assert_eq!(decision.route_family.as_deref(), Some("wallets_manage"));
    assert_eq!(decision.route_kind.as_deref(), Some("process_refund"));
    assert_eq!(
        decision.auth_endpoint_signature.as_deref(),
        Some("admin:wallets")
    );
    assert!(!decision.is_execution_runtime_candidate());
}

#[test]
fn classifies_admin_wallets_process_refund_with_empty_refund_id_as_admin_proxy_route() {
    let headers = headers(&[]);
    let uri: Uri = "/api/admin/wallets/wallet-123/refunds//process"
        .parse()
        .expect("uri should parse");
    let decision =
        classify_control_route(&http::Method::POST, &uri, &headers).expect("route should classify");

    assert_eq!(decision.route_class.as_deref(), Some("admin_proxy"));
    assert_eq!(decision.route_family.as_deref(), Some("wallets_manage"));
    assert_eq!(decision.route_kind.as_deref(), Some("process_refund"));
    assert_eq!(
        decision.auth_endpoint_signature.as_deref(),
        Some("admin:wallets")
    );
    assert!(!decision.is_execution_runtime_candidate());
}

#[test]
fn classifies_admin_wallets_complete_refund_as_admin_proxy_route() {
    let headers = headers(&[]);
    let uri: Uri = "/api/admin/wallets/wallet-123/refunds/refund-1/complete"
        .parse()
        .expect("uri should parse");
    let decision =
        classify_control_route(&http::Method::POST, &uri, &headers).expect("route should classify");

    assert_eq!(decision.route_class.as_deref(), Some("admin_proxy"));
    assert_eq!(decision.route_family.as_deref(), Some("wallets_manage"));
    assert_eq!(decision.route_kind.as_deref(), Some("complete_refund"));
    assert_eq!(
        decision.auth_endpoint_signature.as_deref(),
        Some("admin:wallets")
    );
    assert!(!decision.is_execution_runtime_candidate());
}

#[test]
fn classifies_admin_wallets_fail_refund_as_admin_proxy_route() {
    let headers = headers(&[]);
    let uri: Uri = "/api/admin/wallets/wallet-123/refunds/refund-1/fail"
        .parse()
        .expect("uri should parse");
    let decision =
        classify_control_route(&http::Method::POST, &uri, &headers).expect("route should classify");

    assert_eq!(decision.route_class.as_deref(), Some("admin_proxy"));
    assert_eq!(decision.route_family.as_deref(), Some("wallets_manage"));
    assert_eq!(decision.route_kind.as_deref(), Some("fail_refund"));
    assert_eq!(
        decision.auth_endpoint_signature.as_deref(),
        Some("admin:wallets")
    );
    assert!(!decision.is_execution_runtime_candidate());
}
