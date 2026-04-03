use http::Uri;

use super::{classify_control_route, headers};

#[test]
fn classifies_admin_video_tasks_list_as_admin_proxy_route() {
    let headers = headers(&[]);
    let uri: Uri = "/api/admin/video-tasks?status=completed&page=2"
        .parse()
        .expect("uri should parse");
    let decision =
        classify_control_route(&http::Method::GET, &uri, &headers).expect("route should classify");

    assert_eq!(decision.route_class.as_deref(), Some("admin_proxy"));
    assert_eq!(decision.route_family.as_deref(), Some("video_tasks_manage"));
    assert_eq!(decision.route_kind.as_deref(), Some("list_tasks"));
    assert_eq!(
        decision.auth_endpoint_signature.as_deref(),
        Some("admin:video_tasks")
    );
    assert!(!decision.is_execution_runtime_candidate());
}

#[test]
fn classifies_admin_video_tasks_stats_as_admin_proxy_route() {
    let headers = headers(&[]);
    let uri: Uri = "/api/admin/video-tasks/stats"
        .parse()
        .expect("uri should parse");
    let decision =
        classify_control_route(&http::Method::GET, &uri, &headers).expect("route should classify");

    assert_eq!(decision.route_class.as_deref(), Some("admin_proxy"));
    assert_eq!(decision.route_family.as_deref(), Some("video_tasks_manage"));
    assert_eq!(decision.route_kind.as_deref(), Some("stats"));
    assert_eq!(
        decision.auth_endpoint_signature.as_deref(),
        Some("admin:video_tasks")
    );
    assert!(!decision.is_execution_runtime_candidate());
}

#[test]
fn classifies_admin_video_tasks_detail_as_admin_proxy_route() {
    let headers = headers(&[]);
    let uri: Uri = "/api/admin/video-tasks/task-123"
        .parse()
        .expect("uri should parse");
    let decision =
        classify_control_route(&http::Method::GET, &uri, &headers).expect("route should classify");

    assert_eq!(decision.route_class.as_deref(), Some("admin_proxy"));
    assert_eq!(decision.route_family.as_deref(), Some("video_tasks_manage"));
    assert_eq!(decision.route_kind.as_deref(), Some("detail"));
    assert_eq!(
        decision.auth_endpoint_signature.as_deref(),
        Some("admin:video_tasks")
    );
    assert!(!decision.is_execution_runtime_candidate());
}

#[test]
fn classifies_admin_video_tasks_cancel_as_admin_proxy_route() {
    let headers = headers(&[]);
    let uri: Uri = "/api/admin/video-tasks/task-123/cancel"
        .parse()
        .expect("uri should parse");
    let decision =
        classify_control_route(&http::Method::POST, &uri, &headers).expect("route should classify");

    assert_eq!(decision.route_class.as_deref(), Some("admin_proxy"));
    assert_eq!(decision.route_family.as_deref(), Some("video_tasks_manage"));
    assert_eq!(decision.route_kind.as_deref(), Some("cancel"));
    assert_eq!(
        decision.auth_endpoint_signature.as_deref(),
        Some("admin:video_tasks")
    );
    assert!(!decision.is_execution_runtime_candidate());
}

#[test]
fn classifies_admin_video_tasks_video_as_admin_proxy_route() {
    let headers = headers(&[]);
    let uri: Uri = "/api/admin/video-tasks/task-123/video"
        .parse()
        .expect("uri should parse");
    let decision =
        classify_control_route(&http::Method::GET, &uri, &headers).expect("route should classify");

    assert_eq!(decision.route_class.as_deref(), Some("admin_proxy"));
    assert_eq!(decision.route_family.as_deref(), Some("video_tasks_manage"));
    assert_eq!(decision.route_kind.as_deref(), Some("video"));
    assert_eq!(
        decision.auth_endpoint_signature.as_deref(),
        Some("admin:video_tasks")
    );
    assert!(!decision.is_execution_runtime_candidate());
}
