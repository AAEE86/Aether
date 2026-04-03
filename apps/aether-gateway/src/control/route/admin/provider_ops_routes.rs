use axum::http;

use super::{classified, ClassifiedRoute};

pub(super) fn classify_admin_provider_ops_routes(
    method: &http::Method,
    normalized_path: &str,
) -> Option<ClassifiedRoute> {
    if method == http::Method::GET
        && normalized_path.starts_with("/api/admin/provider-ops/providers/")
        && normalized_path.ends_with("/status")
        && normalized_path.matches('/').count() == 6
    {
        Some(classified(
            "admin_proxy",
            "provider_ops_manage",
            "get_provider_status",
            "admin:provider_ops",
            false,
        ))
    } else if method == http::Method::GET
        && normalized_path.starts_with("/api/admin/provider-ops/providers/")
        && normalized_path.ends_with("/config")
        && normalized_path.matches('/').count() == 6
    {
        Some(classified(
            "admin_proxy",
            "provider_ops_manage",
            "get_provider_config",
            "admin:provider_ops",
            false,
        ))
    } else if method == http::Method::PUT
        && normalized_path.starts_with("/api/admin/provider-ops/providers/")
        && normalized_path.ends_with("/config")
        && normalized_path.matches('/').count() == 6
    {
        Some(classified(
            "admin_proxy",
            "provider_ops_manage",
            "save_provider_config",
            "admin:provider_ops",
            false,
        ))
    } else if method == http::Method::DELETE
        && normalized_path.starts_with("/api/admin/provider-ops/providers/")
        && normalized_path.ends_with("/config")
        && normalized_path.matches('/').count() == 6
    {
        Some(classified(
            "admin_proxy",
            "provider_ops_manage",
            "delete_provider_config",
            "admin:provider_ops",
            false,
        ))
    } else if method == http::Method::POST
        && normalized_path.starts_with("/api/admin/provider-ops/providers/")
        && normalized_path.ends_with("/connect")
        && normalized_path.matches('/').count() == 6
    {
        Some(classified(
            "admin_proxy",
            "provider_ops_manage",
            "connect_provider",
            "admin:provider_ops",
            false,
        ))
    } else if method == http::Method::POST
        && normalized_path.starts_with("/api/admin/provider-ops/providers/")
        && normalized_path.ends_with("/verify")
        && normalized_path.matches('/').count() == 6
    {
        Some(classified(
            "admin_proxy",
            "provider_ops_manage",
            "verify_provider",
            "admin:provider_ops",
            false,
        ))
    } else if method == http::Method::POST
        && normalized_path.starts_with("/api/admin/provider-ops/providers/")
        && normalized_path.ends_with("/disconnect")
        && normalized_path.matches('/').count() == 6
    {
        Some(classified(
            "admin_proxy",
            "provider_ops_manage",
            "disconnect_provider",
            "admin:provider_ops",
            false,
        ))
    } else if method == http::Method::GET
        && normalized_path.starts_with("/api/admin/provider-ops/providers/")
        && normalized_path.ends_with("/balance")
        && normalized_path.matches('/').count() == 6
    {
        Some(classified(
            "admin_proxy",
            "provider_ops_manage",
            "get_provider_balance",
            "admin:provider_ops",
            false,
        ))
    } else if method == http::Method::POST
        && normalized_path.starts_with("/api/admin/provider-ops/providers/")
        && normalized_path.ends_with("/balance")
        && normalized_path.matches('/').count() == 6
    {
        Some(classified(
            "admin_proxy",
            "provider_ops_manage",
            "refresh_provider_balance",
            "admin:provider_ops",
            false,
        ))
    } else if method == http::Method::POST
        && normalized_path.starts_with("/api/admin/provider-ops/providers/")
        && normalized_path.ends_with("/checkin")
        && normalized_path.matches('/').count() == 6
    {
        Some(classified(
            "admin_proxy",
            "provider_ops_manage",
            "provider_checkin",
            "admin:provider_ops",
            false,
        ))
    } else if method == http::Method::POST
        && normalized_path.starts_with("/api/admin/provider-ops/providers/")
        && normalized_path.contains("/actions/")
        && normalized_path.matches('/').count() == 7
    {
        Some(classified(
            "admin_proxy",
            "provider_ops_manage",
            "execute_provider_action",
            "admin:provider_ops",
            false,
        ))
    } else if method == http::Method::POST
        && matches!(
            normalized_path,
            "/api/admin/provider-ops/batch/balance" | "/api/admin/provider-ops/batch/balance/"
        )
    {
        Some(classified(
            "admin_proxy",
            "provider_ops_manage",
            "batch_balance",
            "admin:provider_ops",
            false,
        ))
    } else {
        None
    }
}
