use super::{
    build_admin_payments_data_unavailable_response,
    payments_callbacks::maybe_build_local_admin_payment_callbacks_response,
    payments_orders::maybe_build_local_admin_payment_orders_response,
};
use crate::gateway::{AppState, GatewayError, GatewayPublicRequestContext};
use axum::{body::Body, http, response::Response};

pub(super) async fn maybe_build_local_admin_payments_response(
    state: &AppState,
    request_context: &GatewayPublicRequestContext,
    request_body: Option<&axum::body::Bytes>,
) -> Result<Option<Response<Body>>, GatewayError> {
    let Some(decision) = request_context.control_decision.as_ref() else {
        return Ok(None);
    };

    if decision.route_family.as_deref() != Some("payments_manage") {
        return Ok(None);
    }

    let normalized_path = request_context.request_path.trim_end_matches('/');
    let path = if normalized_path.is_empty() {
        request_context.request_path.as_str()
    } else {
        normalized_path
    };
    let is_payments_route = (request_context.request_method == http::Method::GET
        && path == "/api/admin/payments/orders")
        || (request_context.request_method == http::Method::GET
            && path.starts_with("/api/admin/payments/orders/")
            && path.matches('/').count() == 5)
        || (request_context.request_method == http::Method::POST
            && path.starts_with("/api/admin/payments/orders/")
            && path.ends_with("/expire")
            && path.matches('/').count() == 6)
        || (request_context.request_method == http::Method::POST
            && path.starts_with("/api/admin/payments/orders/")
            && path.ends_with("/credit")
            && path.matches('/').count() == 6)
        || (request_context.request_method == http::Method::POST
            && path.starts_with("/api/admin/payments/orders/")
            && path.ends_with("/fail")
            && path.matches('/').count() == 6)
        || (request_context.request_method == http::Method::GET
            && path == "/api/admin/payments/callbacks");

    if !is_payments_route {
        return Ok(None);
    }

    let route_kind = decision.route_kind.as_deref();
    if let Some(response) = maybe_build_local_admin_payment_orders_response(
        state,
        request_context,
        request_body,
        route_kind,
    )
    .await?
    {
        return Ok(Some(response));
    }
    if let Some(response) =
        maybe_build_local_admin_payment_callbacks_response(state, request_context, route_kind)
            .await?
    {
        return Ok(Some(response));
    }

    Ok(Some(build_admin_payments_data_unavailable_response()))
}
