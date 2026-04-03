use http::Uri;

use super::{classify_control_route, headers};

#[test]
fn classifies_admin_users_list_as_admin_proxy_route() {
    let headers = headers(&[]);
    let uri: Uri = "/api/admin/users".parse().expect("uri should parse");
    let decision =
        classify_control_route(&http::Method::GET, &uri, &headers).expect("route should classify");

    assert_eq!(decision.route_class.as_deref(), Some("admin_proxy"));
    assert_eq!(decision.route_family.as_deref(), Some("users_manage"));
    assert_eq!(decision.route_kind.as_deref(), Some("list_users"));
    assert_eq!(
        decision.auth_endpoint_signature.as_deref(),
        Some("admin:users")
    );
    assert!(!decision.is_execution_runtime_candidate());
}

#[test]
fn classifies_admin_users_create_as_admin_proxy_route() {
    let headers = headers(&[]);
    let uri: Uri = "/api/admin/users".parse().expect("uri should parse");
    let decision =
        classify_control_route(&http::Method::POST, &uri, &headers).expect("route should classify");

    assert_eq!(decision.route_class.as_deref(), Some("admin_proxy"));
    assert_eq!(decision.route_family.as_deref(), Some("users_manage"));
    assert_eq!(decision.route_kind.as_deref(), Some("create_user"));
    assert_eq!(
        decision.auth_endpoint_signature.as_deref(),
        Some("admin:users")
    );
    assert!(!decision.is_execution_runtime_candidate());
}

#[test]
fn classifies_admin_user_detail_routes_as_admin_proxy_route() {
    let headers = headers(&[]);

    let get_uri: Uri = "/api/admin/users/user-1".parse().expect("uri should parse");
    let get = classify_control_route(&http::Method::GET, &get_uri, &headers)
        .expect("route should classify");
    assert_eq!(get.route_family.as_deref(), Some("users_manage"));
    assert_eq!(get.route_kind.as_deref(), Some("get_user"));

    let put_uri: Uri = "/api/admin/users/user-1".parse().expect("uri should parse");
    let put = classify_control_route(&http::Method::PUT, &put_uri, &headers)
        .expect("route should classify");
    assert_eq!(put.route_family.as_deref(), Some("users_manage"));
    assert_eq!(put.route_kind.as_deref(), Some("update_user"));

    let delete_uri: Uri = "/api/admin/users/user-1".parse().expect("uri should parse");
    let delete = classify_control_route(&http::Method::DELETE, &delete_uri, &headers)
        .expect("route should classify");
    assert_eq!(delete.route_family.as_deref(), Some("users_manage"));
    assert_eq!(delete.route_kind.as_deref(), Some("delete_user"));
    assert_eq!(
        delete.auth_endpoint_signature.as_deref(),
        Some("admin:users")
    );
}

#[test]
fn classifies_admin_user_session_routes_as_admin_proxy_route() {
    let headers = headers(&[]);

    let list_uri: Uri = "/api/admin/users/user-1/sessions"
        .parse()
        .expect("uri should parse");
    let list = classify_control_route(&http::Method::GET, &list_uri, &headers)
        .expect("route should classify");
    assert_eq!(list.route_family.as_deref(), Some("users_manage"));
    assert_eq!(list.route_kind.as_deref(), Some("list_user_sessions"));

    let delete_all_uri: Uri = "/api/admin/users/user-1/sessions"
        .parse()
        .expect("uri should parse");
    let delete_all = classify_control_route(&http::Method::DELETE, &delete_all_uri, &headers)
        .expect("route should classify");
    assert_eq!(delete_all.route_family.as_deref(), Some("users_manage"));
    assert_eq!(
        delete_all.route_kind.as_deref(),
        Some("delete_user_sessions")
    );

    let delete_one_uri: Uri = "/api/admin/users/user-1/sessions/session-1"
        .parse()
        .expect("uri should parse");
    let delete_one = classify_control_route(&http::Method::DELETE, &delete_one_uri, &headers)
        .expect("route should classify");
    assert_eq!(delete_one.route_family.as_deref(), Some("users_manage"));
    assert_eq!(
        delete_one.route_kind.as_deref(),
        Some("delete_user_session")
    );
    assert_eq!(
        delete_one.auth_endpoint_signature.as_deref(),
        Some("admin:users")
    );
}

#[test]
fn classifies_admin_user_api_key_routes_as_admin_proxy_route() {
    let headers = headers(&[]);

    let list_uri: Uri = "/api/admin/users/user-1/api-keys"
        .parse()
        .expect("uri should parse");
    let list = classify_control_route(&http::Method::GET, &list_uri, &headers)
        .expect("route should classify");
    assert_eq!(list.route_family.as_deref(), Some("users_manage"));
    assert_eq!(list.route_kind.as_deref(), Some("list_user_api_keys"));

    let create_uri: Uri = "/api/admin/users/user-1/api-keys"
        .parse()
        .expect("uri should parse");
    let create = classify_control_route(&http::Method::POST, &create_uri, &headers)
        .expect("route should classify");
    assert_eq!(create.route_family.as_deref(), Some("users_manage"));
    assert_eq!(create.route_kind.as_deref(), Some("create_user_api_key"));

    let delete_uri: Uri = "/api/admin/users/user-1/api-keys/key-1"
        .parse()
        .expect("uri should parse");
    let delete = classify_control_route(&http::Method::DELETE, &delete_uri, &headers)
        .expect("route should classify");
    assert_eq!(delete.route_family.as_deref(), Some("users_manage"));
    assert_eq!(delete.route_kind.as_deref(), Some("delete_user_api_key"));

    let update_uri: Uri = "/api/admin/users/user-1/api-keys/key-1"
        .parse()
        .expect("uri should parse");
    let update = classify_control_route(&http::Method::PUT, &update_uri, &headers)
        .expect("route should classify");
    assert_eq!(update.route_family.as_deref(), Some("users_manage"));
    assert_eq!(update.route_kind.as_deref(), Some("update_user_api_key"));

    let lock_uri: Uri = "/api/admin/users/user-1/api-keys/key-1/lock"
        .parse()
        .expect("uri should parse");
    let lock = classify_control_route(&http::Method::PATCH, &lock_uri, &headers)
        .expect("route should classify");
    assert_eq!(lock.route_family.as_deref(), Some("users_manage"));
    assert_eq!(lock.route_kind.as_deref(), Some("lock_user_api_key"));

    let full_key_uri: Uri = "/api/admin/users/user-1/api-keys/key-1/full-key"
        .parse()
        .expect("uri should parse");
    let full_key = classify_control_route(&http::Method::GET, &full_key_uri, &headers)
        .expect("route should classify");
    assert_eq!(full_key.route_family.as_deref(), Some("users_manage"));
    assert_eq!(full_key.route_kind.as_deref(), Some("reveal_user_api_key"));
    assert_eq!(
        full_key.auth_endpoint_signature.as_deref(),
        Some("admin:users")
    );
}

#[test]
fn classifies_admin_user_api_key_mutation_routes_with_admin_users_signature() {
    let headers = headers(&[]);

    let create_uri: Uri = "/api/admin/users/user-1/api-keys"
        .parse()
        .expect("uri should parse");
    let create = classify_control_route(&http::Method::POST, &create_uri, &headers)
        .expect("route should classify");
    assert_eq!(create.route_class.as_deref(), Some("admin_proxy"));
    assert_eq!(
        create.auth_endpoint_signature.as_deref(),
        Some("admin:users")
    );

    let lock_uri: Uri = "/api/admin/users/user-1/api-keys/key-1/lock"
        .parse()
        .expect("uri should parse");
    let lock = classify_control_route(&http::Method::PATCH, &lock_uri, &headers)
        .expect("route should classify");
    assert_eq!(lock.route_class.as_deref(), Some("admin_proxy"));
    assert_eq!(lock.auth_endpoint_signature.as_deref(), Some("admin:users"));
}
