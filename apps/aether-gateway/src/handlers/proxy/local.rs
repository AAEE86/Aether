use super::super::{admin, internal, public};
use crate::gateway::{AppState, GatewayError, GatewayPublicRequestContext};
use axum::body::{Body, Bytes};
use axum::http::Response;

pub(super) async fn maybe_build_local_internal_proxy_response(
    state: &AppState,
    request_context: &GatewayPublicRequestContext,
    remote_addr: &std::net::SocketAddr,
    request_body: Option<&Bytes>,
) -> Result<Option<Response<Body>>, GatewayError> {
    internal::maybe_build_local_internal_proxy_response_impl(
        state,
        request_context,
        remote_addr,
        request_body,
    )
    .await
}

pub(super) async fn maybe_build_local_admin_proxy_response(
    state: &AppState,
    request_context: &GatewayPublicRequestContext,
    request_body: Option<&Bytes>,
) -> Result<Option<Response<Body>>, GatewayError> {
    let Some(decision) = request_context.control_decision.as_ref() else {
        return Ok(None);
    };
    if decision.route_class.as_deref() != Some("admin_proxy") {
        return Ok(None);
    }
    if decision.admin_principal.is_none() {
        return Ok(None);
    }

    if let Some(response) =
        admin::maybe_build_local_admin_provider_oauth_response(state, request_context, request_body)
            .await?
    {
        return Ok(Some(response));
    }

    if let Some(response) =
        public::maybe_build_local_admin_announcements_response(state, request_context, request_body)
            .await?
    {
        return Ok(Some(response));
    }

    if let Some(response) =
        admin::maybe_build_local_admin_core_response(state, request_context, request_body).await?
    {
        return Ok(Some(response));
    }

    if let Some(response) =
        admin::maybe_build_local_admin_global_models_response(state, request_context, request_body)
            .await?
    {
        return Ok(Some(response));
    }

    if let Some(response) = admin::maybe_build_local_admin_provider_models_response(
        state,
        request_context,
        request_body,
    )
    .await?
    {
        return Ok(Some(response));
    }

    if let Some(response) =
        admin::maybe_build_local_admin_providers_response(state, request_context, request_body)
            .await?
    {
        return Ok(Some(response));
    }
    if let Some(response) =
        admin::maybe_build_local_admin_provider_ops_response(state, request_context, request_body)
            .await?
    {
        return Ok(Some(response));
    }
    if let Some(response) =
        admin::maybe_build_local_admin_adaptive_response(state, request_context, request_body)
            .await?
    {
        return Ok(Some(response));
    }
    if let Some(response) = admin::maybe_build_local_admin_provider_strategy_response(
        state,
        request_context,
        request_body,
    )
    .await?
    {
        return Ok(Some(response));
    }
    if let Some(response) =
        admin::maybe_build_local_admin_pool_response(state, request_context, request_body).await?
    {
        return Ok(Some(response));
    }
    if let Some(response) =
        admin::maybe_build_local_admin_billing_response(state, request_context, request_body)
            .await?
    {
        return Ok(Some(response));
    }
    if let Some(response) =
        admin::maybe_build_local_admin_payments_response(state, request_context, request_body)
            .await?
    {
        return Ok(Some(response));
    }
    if let Some(response) =
        admin::maybe_build_local_admin_provider_query_response(state, request_context, request_body)
            .await?
    {
        return Ok(Some(response));
    }
    if let Some(response) =
        admin::maybe_build_local_admin_security_response(state, request_context, request_body)
            .await?
    {
        return Ok(Some(response));
    }
    if let Some(response) =
        admin::maybe_build_local_admin_stats_response(state, request_context).await?
    {
        return Ok(Some(response));
    }
    if let Some(response) =
        admin::maybe_build_local_admin_monitoring_response(state, request_context).await?
    {
        return Ok(Some(response));
    }
    if let Some(response) =
        admin::maybe_build_local_admin_usage_response(state, request_context, request_body).await?
    {
        return Ok(Some(response));
    }
    if let Some(response) =
        admin::maybe_build_local_admin_video_tasks_response(state, request_context).await?
    {
        return Ok(Some(response));
    }
    if let Some(response) =
        admin::maybe_build_local_admin_proxy_nodes_response(state, request_context).await?
    {
        return Ok(Some(response));
    }
    if let Some(response) =
        admin::maybe_build_local_admin_wallets_response(state, request_context, request_body)
            .await?
    {
        return Ok(Some(response));
    }
    if let Some(response) =
        admin::maybe_build_local_admin_api_keys_response(state, request_context, request_body)
            .await?
    {
        return Ok(Some(response));
    }
    if let Some(response) =
        admin::maybe_build_local_admin_ldap_response(state, request_context, request_body).await?
    {
        return Ok(Some(response));
    }
    if let Some(response) =
        admin::maybe_build_local_admin_gemini_files_response(state, request_context, request_body)
            .await?
    {
        return Ok(Some(response));
    }
    if let Some(response) =
        admin::maybe_build_local_admin_users_response(state, request_context, request_body).await?
    {
        return Ok(Some(response));
    }
    if let Some(response) =
        admin::maybe_build_local_admin_endpoints_response(state, request_context, request_body)
            .await?
    {
        return Ok(Some(response));
    }

    Ok(None)
}
