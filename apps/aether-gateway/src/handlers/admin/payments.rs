use crate::gateway::{AppState, GatewayError, GatewayPublicRequestContext};
use axum::{body::Body, response::Response};

#[path = "payment/postgres.rs"]
mod payment_postgres;
#[path = "payments/callbacks.rs"]
mod payments_callbacks;
#[path = "payments/orders.rs"]
mod payments_orders;
#[path = "payments/routes.rs"]
mod payments_routes;
#[path = "payments/shared.rs"]
mod payments_shared;

use self::payments_shared::{
    admin_payment_operator_id, admin_payment_order_id_from_detail_path,
    admin_payment_order_id_from_suffix_path, build_admin_payment_callback_payload,
    build_admin_payment_callback_payload_from_record, build_admin_payment_order_not_found_response,
    build_admin_payment_order_payload, build_admin_payment_orders_page_response,
    build_admin_payments_backend_unavailable_response, build_admin_payments_bad_request_response,
    build_admin_payments_data_unavailable_response, normalize_admin_payment_currency,
    normalize_admin_payment_optional_string, normalize_admin_payment_positive_number,
    parse_admin_payments_limit, parse_admin_payments_offset, AdminPaymentOrderCreditRequest,
};

pub(crate) async fn maybe_build_local_admin_payments_response(
    state: &AppState,
    request_context: &GatewayPublicRequestContext,
    request_body: Option<&axum::body::Bytes>,
) -> Result<Option<Response<Body>>, GatewayError> {
    payments_routes::maybe_build_local_admin_payments_response(state, request_context, request_body)
        .await
}
