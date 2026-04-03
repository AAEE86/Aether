use http::Uri;

use super::{classify_control_route, headers};

#[test]
fn classifies_admin_usage_stats_as_admin_proxy_route() {
    let headers = headers(&[]);
    let uri: Uri = "/api/admin/usage/stats".parse().expect("uri should parse");
    let decision =
        classify_control_route(&http::Method::GET, &uri, &headers).expect("route should classify");

    assert_eq!(decision.route_class.as_deref(), Some("admin_proxy"));
    assert_eq!(decision.route_family.as_deref(), Some("usage_manage"));
    assert_eq!(decision.route_kind.as_deref(), Some("stats"));
    assert_eq!(
        decision.auth_endpoint_signature.as_deref(),
        Some("admin:usage")
    );
    assert!(!decision.is_execution_runtime_candidate());
}

#[test]
fn classifies_admin_usage_aggregation_stats_as_admin_proxy_route() {
    let headers = headers(&[]);
    let uri: Uri = "/api/admin/usage/aggregation/stats"
        .parse()
        .expect("uri should parse");
    let decision =
        classify_control_route(&http::Method::GET, &uri, &headers).expect("route should classify");

    assert_eq!(decision.route_class.as_deref(), Some("admin_proxy"));
    assert_eq!(decision.route_family.as_deref(), Some("usage_manage"));
    assert_eq!(decision.route_kind.as_deref(), Some("aggregation_stats"));
    assert_eq!(
        decision.auth_endpoint_signature.as_deref(),
        Some("admin:usage")
    );
    assert!(!decision.is_execution_runtime_candidate());
}

#[test]
fn classifies_admin_usage_heatmap_as_admin_proxy_route() {
    let headers = headers(&[]);
    let uri: Uri = "/api/admin/usage/heatmap"
        .parse()
        .expect("uri should parse");
    let decision =
        classify_control_route(&http::Method::GET, &uri, &headers).expect("route should classify");

    assert_eq!(decision.route_class.as_deref(), Some("admin_proxy"));
    assert_eq!(decision.route_family.as_deref(), Some("usage_manage"));
    assert_eq!(decision.route_kind.as_deref(), Some("heatmap"));
    assert_eq!(
        decision.auth_endpoint_signature.as_deref(),
        Some("admin:usage")
    );
    assert!(!decision.is_execution_runtime_candidate());
}

#[test]
fn classifies_admin_usage_records_as_admin_proxy_route() {
    let headers = headers(&[]);
    let uri: Uri = "/api/admin/usage/records"
        .parse()
        .expect("uri should parse");
    let decision =
        classify_control_route(&http::Method::GET, &uri, &headers).expect("route should classify");

    assert_eq!(decision.route_class.as_deref(), Some("admin_proxy"));
    assert_eq!(decision.route_family.as_deref(), Some("usage_manage"));
    assert_eq!(decision.route_kind.as_deref(), Some("records"));
    assert_eq!(
        decision.auth_endpoint_signature.as_deref(),
        Some("admin:usage")
    );
    assert!(!decision.is_execution_runtime_candidate());
}

#[test]
fn classifies_admin_usage_active_as_admin_proxy_route() {
    let headers = headers(&[]);
    let uri: Uri = "/api/admin/usage/active".parse().expect("uri should parse");
    let decision =
        classify_control_route(&http::Method::GET, &uri, &headers).expect("route should classify");

    assert_eq!(decision.route_class.as_deref(), Some("admin_proxy"));
    assert_eq!(decision.route_family.as_deref(), Some("usage_manage"));
    assert_eq!(decision.route_kind.as_deref(), Some("active"));
    assert_eq!(
        decision.auth_endpoint_signature.as_deref(),
        Some("admin:usage")
    );
    assert!(!decision.is_execution_runtime_candidate());
}

#[test]
fn classifies_admin_usage_cache_affinity_hit_analysis_as_admin_proxy_route() {
    let headers = headers(&[]);
    let uri: Uri = "/api/admin/usage/cache-affinity/hit-analysis"
        .parse()
        .expect("uri should parse");
    let decision =
        classify_control_route(&http::Method::GET, &uri, &headers).expect("route should classify");

    assert_eq!(decision.route_class.as_deref(), Some("admin_proxy"));
    assert_eq!(decision.route_family.as_deref(), Some("usage_manage"));
    assert_eq!(
        decision.route_kind.as_deref(),
        Some("cache_affinity_hit_analysis")
    );
    assert_eq!(
        decision.auth_endpoint_signature.as_deref(),
        Some("admin:usage")
    );
    assert!(!decision.is_execution_runtime_candidate());
}

#[test]
fn classifies_admin_usage_cache_affinity_interval_timeline_as_admin_proxy_route() {
    let headers = headers(&[]);
    let uri: Uri = "/api/admin/usage/cache-affinity/interval-timeline"
        .parse()
        .expect("uri should parse");
    let decision =
        classify_control_route(&http::Method::GET, &uri, &headers).expect("route should classify");

    assert_eq!(decision.route_class.as_deref(), Some("admin_proxy"));
    assert_eq!(decision.route_family.as_deref(), Some("usage_manage"));
    assert_eq!(
        decision.route_kind.as_deref(),
        Some("cache_affinity_interval_timeline")
    );
    assert_eq!(
        decision.auth_endpoint_signature.as_deref(),
        Some("admin:usage")
    );
    assert!(!decision.is_execution_runtime_candidate());
}

#[test]
fn classifies_admin_usage_cache_affinity_ttl_analysis_as_admin_proxy_route() {
    let headers = headers(&[]);
    let uri: Uri = "/api/admin/usage/cache-affinity/ttl-analysis"
        .parse()
        .expect("uri should parse");
    let decision =
        classify_control_route(&http::Method::GET, &uri, &headers).expect("route should classify");

    assert_eq!(decision.route_class.as_deref(), Some("admin_proxy"));
    assert_eq!(decision.route_family.as_deref(), Some("usage_manage"));
    assert_eq!(
        decision.route_kind.as_deref(),
        Some("cache_affinity_ttl_analysis")
    );
    assert_eq!(
        decision.auth_endpoint_signature.as_deref(),
        Some("admin:usage")
    );
    assert!(!decision.is_execution_runtime_candidate());
}

#[test]
fn classifies_admin_usage_detail_as_admin_proxy_route() {
    let headers = headers(&[]);
    let uri: Uri = "/api/admin/usage/usage-1"
        .parse()
        .expect("uri should parse");
    let decision =
        classify_control_route(&http::Method::GET, &uri, &headers).expect("route should classify");

    assert_eq!(decision.route_class.as_deref(), Some("admin_proxy"));
    assert_eq!(decision.route_family.as_deref(), Some("usage_manage"));
    assert_eq!(decision.route_kind.as_deref(), Some("detail"));
    assert_eq!(
        decision.auth_endpoint_signature.as_deref(),
        Some("admin:usage")
    );
    assert!(!decision.is_execution_runtime_candidate());
}

#[test]
fn classifies_admin_usage_detail_with_empty_usage_id_as_admin_proxy_route() {
    let headers = headers(&[]);
    let uri: Uri = "/api/admin/usage/".parse().expect("uri should parse");
    let decision =
        classify_control_route(&http::Method::GET, &uri, &headers).expect("route should classify");

    assert_eq!(decision.route_class.as_deref(), Some("admin_proxy"));
    assert_eq!(decision.route_family.as_deref(), Some("usage_manage"));
    assert_eq!(decision.route_kind.as_deref(), Some("detail"));
    assert_eq!(
        decision.auth_endpoint_signature.as_deref(),
        Some("admin:usage")
    );
    assert!(!decision.is_execution_runtime_candidate());
}

#[test]
fn classifies_admin_usage_curl_as_admin_proxy_route() {
    let headers = headers(&[]);
    let uri: Uri = "/api/admin/usage/usage-1/curl"
        .parse()
        .expect("uri should parse");
    let decision =
        classify_control_route(&http::Method::GET, &uri, &headers).expect("route should classify");

    assert_eq!(decision.route_class.as_deref(), Some("admin_proxy"));
    assert_eq!(decision.route_family.as_deref(), Some("usage_manage"));
    assert_eq!(decision.route_kind.as_deref(), Some("curl"));
    assert_eq!(
        decision.auth_endpoint_signature.as_deref(),
        Some("admin:usage")
    );
    assert!(!decision.is_execution_runtime_candidate());
}

#[test]
fn classifies_admin_usage_curl_with_empty_usage_id_as_admin_proxy_route() {
    let headers = headers(&[]);
    let uri: Uri = "/api/admin/usage//curl".parse().expect("uri should parse");
    let decision =
        classify_control_route(&http::Method::GET, &uri, &headers).expect("route should classify");

    assert_eq!(decision.route_class.as_deref(), Some("admin_proxy"));
    assert_eq!(decision.route_family.as_deref(), Some("usage_manage"));
    assert_eq!(decision.route_kind.as_deref(), Some("curl"));
    assert_eq!(
        decision.auth_endpoint_signature.as_deref(),
        Some("admin:usage")
    );
    assert!(!decision.is_execution_runtime_candidate());
}

#[test]
fn classifies_admin_usage_replay_as_admin_proxy_route() {
    let headers = headers(&[]);
    let uri: Uri = "/api/admin/usage/usage-1/replay"
        .parse()
        .expect("uri should parse");
    let decision =
        classify_control_route(&http::Method::POST, &uri, &headers).expect("route should classify");

    assert_eq!(decision.route_class.as_deref(), Some("admin_proxy"));
    assert_eq!(decision.route_family.as_deref(), Some("usage_manage"));
    assert_eq!(decision.route_kind.as_deref(), Some("replay"));
    assert_eq!(
        decision.auth_endpoint_signature.as_deref(),
        Some("admin:usage")
    );
    assert!(!decision.is_execution_runtime_candidate());
}
