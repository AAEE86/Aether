use super::{classified, ClassifiedRoute};

pub(super) fn classify_oauth_route(
    method: &http::Method,
    normalized_path: &str,
) -> Option<ClassifiedRoute> {
    if method == http::Method::GET && normalized_path == "/api/admin/oauth/supported-types" {
        Some(classified(
            "admin_proxy",
            "oauth_manage",
            "supported_types",
            "admin:oauth",
            false,
        ))
    } else if method == http::Method::GET
        && matches!(
            normalized_path,
            "/api/admin/oauth/providers" | "/api/admin/oauth/providers/"
        )
    {
        Some(classified(
            "admin_proxy",
            "oauth_manage",
            "list_providers",
            "admin:oauth",
            false,
        ))
    } else if method == http::Method::POST
        && normalized_path.starts_with("/api/admin/oauth/providers/")
        && normalized_path.ends_with("/test")
    {
        Some(classified(
            "admin_proxy",
            "oauth_manage",
            "test_provider",
            "admin:oauth",
            false,
        ))
    } else if method == http::Method::GET
        && normalized_path.starts_with("/api/admin/oauth/providers/")
    {
        Some(classified(
            "admin_proxy",
            "oauth_manage",
            "get_provider",
            "admin:oauth",
            false,
        ))
    } else if method == http::Method::PUT
        && normalized_path.starts_with("/api/admin/oauth/providers/")
    {
        Some(classified(
            "admin_proxy",
            "oauth_manage",
            "upsert_provider",
            "admin:oauth",
            false,
        ))
    } else if method == http::Method::DELETE
        && normalized_path.starts_with("/api/admin/oauth/providers/")
    {
        Some(classified(
            "admin_proxy",
            "oauth_manage",
            "delete_provider",
            "admin:oauth",
            false,
        ))
    } else if method == http::Method::GET
        && normalized_path == "/api/admin/provider-oauth/supported-types"
    {
        Some(classified(
            "admin_proxy",
            "provider_oauth_manage",
            "supported_types",
            "admin:provider_oauth",
            false,
        ))
    } else if method == http::Method::POST
        && normalized_path.starts_with("/api/admin/provider-oauth/keys/")
        && normalized_path.ends_with("/start")
    {
        Some(classified(
            "admin_proxy",
            "provider_oauth_manage",
            "start_key_oauth",
            "admin:provider_oauth",
            false,
        ))
    } else if method == http::Method::POST
        && normalized_path.starts_with("/api/admin/provider-oauth/providers/")
        && normalized_path.ends_with("/start")
    {
        Some(classified(
            "admin_proxy",
            "provider_oauth_manage",
            "start_provider_oauth",
            "admin:provider_oauth",
            false,
        ))
    } else if method == http::Method::POST
        && normalized_path.starts_with("/api/admin/provider-oauth/keys/")
        && normalized_path.ends_with("/complete")
    {
        Some(classified(
            "admin_proxy",
            "provider_oauth_manage",
            "complete_key_oauth",
            "admin:provider_oauth",
            false,
        ))
    } else if method == http::Method::POST
        && normalized_path.starts_with("/api/admin/provider-oauth/keys/")
        && normalized_path.ends_with("/refresh")
    {
        Some(classified(
            "admin_proxy",
            "provider_oauth_manage",
            "refresh_key_oauth",
            "admin:provider_oauth",
            false,
        ))
    } else if method == http::Method::POST
        && normalized_path.starts_with("/api/admin/provider-oauth/providers/")
        && normalized_path.ends_with("/complete")
    {
        Some(classified(
            "admin_proxy",
            "provider_oauth_manage",
            "complete_provider_oauth",
            "admin:provider_oauth",
            false,
        ))
    } else if method == http::Method::POST
        && normalized_path.starts_with("/api/admin/provider-oauth/providers/")
        && normalized_path.ends_with("/import-refresh-token")
    {
        Some(classified(
            "admin_proxy",
            "provider_oauth_manage",
            "import_refresh_token",
            "admin:provider_oauth",
            false,
        ))
    } else if method == http::Method::POST
        && normalized_path.starts_with("/api/admin/provider-oauth/providers/")
        && normalized_path.ends_with("/batch-import")
    {
        Some(classified(
            "admin_proxy",
            "provider_oauth_manage",
            "batch_import_oauth",
            "admin:provider_oauth",
            false,
        ))
    } else if method == http::Method::POST
        && normalized_path.starts_with("/api/admin/provider-oauth/providers/")
        && normalized_path.ends_with("/batch-import/tasks")
    {
        Some(classified(
            "admin_proxy",
            "provider_oauth_manage",
            "start_batch_import_oauth_task",
            "admin:provider_oauth",
            false,
        ))
    } else if method == http::Method::GET
        && normalized_path.starts_with("/api/admin/provider-oauth/providers/")
        && normalized_path.contains("/batch-import/tasks/")
    {
        Some(classified(
            "admin_proxy",
            "provider_oauth_manage",
            "get_batch_import_task_status",
            "admin:provider_oauth",
            false,
        ))
    } else if method == http::Method::POST
        && normalized_path.starts_with("/api/admin/provider-oauth/providers/")
        && normalized_path.ends_with("/device-authorize")
    {
        Some(classified(
            "admin_proxy",
            "provider_oauth_manage",
            "device_authorize",
            "admin:provider_oauth",
            false,
        ))
    } else if method == http::Method::POST
        && normalized_path.starts_with("/api/admin/provider-oauth/providers/")
        && normalized_path.ends_with("/device-poll")
    {
        Some(classified(
            "admin_proxy",
            "provider_oauth_manage",
            "device_poll",
            "admin:provider_oauth",
            false,
        ))
    } else {
        None
    }
}
