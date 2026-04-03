use super::provider_query_models::{
    build_admin_provider_query_models_response,
    build_admin_provider_query_test_model_failover_response,
    build_admin_provider_query_test_model_response,
};
use super::provider_query_shared::{
    build_admin_provider_query_bad_request_response, parse_admin_provider_query_body,
    provider_query_extract_failover_models, provider_query_extract_model,
    provider_query_extract_provider_id, ADMIN_PROVIDER_QUERY_FAILOVER_MODELS_REQUIRED_DETAIL,
    ADMIN_PROVIDER_QUERY_MODEL_REQUIRED_DETAIL, ADMIN_PROVIDER_QUERY_PROVIDER_ID_REQUIRED_DETAIL,
};
use crate::gateway::{AppState, GatewayError, GatewayPublicRequestContext};
use axum::{
    body::{Body, Bytes},
    http,
    http::Response,
};

pub(super) async fn maybe_build_local_admin_provider_query_response(
    state: &AppState,
    request_context: &GatewayPublicRequestContext,
    request_body: Option<&Bytes>,
) -> Result<Option<Response<Body>>, GatewayError> {
    let Some(decision) = request_context.control_decision.as_ref() else {
        return Ok(None);
    };

    if decision.route_family.as_deref() != Some("provider_query_manage") {
        return Ok(None);
    }

    if request_context.request_method != http::Method::POST {
        return Ok(None);
    }

    let payload = match parse_admin_provider_query_body(request_body) {
        Ok(value) => value,
        Err(response) => return Ok(Some(response)),
    };

    let route_kind = decision.route_kind.as_deref().unwrap_or("query_models");
    match route_kind {
        "query_models" => Ok(Some(
            build_admin_provider_query_models_response(state, &payload).await?,
        )),
        "test_model" => {
            let Some(provider_id) = provider_query_extract_provider_id(&payload) else {
                return Ok(Some(build_admin_provider_query_bad_request_response(
                    ADMIN_PROVIDER_QUERY_PROVIDER_ID_REQUIRED_DETAIL,
                )));
            };
            let Some(model) = provider_query_extract_model(&payload) else {
                return Ok(Some(build_admin_provider_query_bad_request_response(
                    ADMIN_PROVIDER_QUERY_MODEL_REQUIRED_DETAIL,
                )));
            };
            Ok(Some(build_admin_provider_query_test_model_response(
                provider_id,
                model,
            )))
        }
        "test_model_failover" => {
            let Some(provider_id) = provider_query_extract_provider_id(&payload) else {
                return Ok(Some(build_admin_provider_query_bad_request_response(
                    ADMIN_PROVIDER_QUERY_PROVIDER_ID_REQUIRED_DETAIL,
                )));
            };
            let failover_models = provider_query_extract_failover_models(&payload);
            if failover_models.is_empty() {
                return Ok(Some(build_admin_provider_query_bad_request_response(
                    ADMIN_PROVIDER_QUERY_FAILOVER_MODELS_REQUIRED_DETAIL,
                )));
            }
            Ok(Some(
                build_admin_provider_query_test_model_failover_response(
                    provider_id,
                    failover_models,
                ),
            ))
        }
        _ => Ok(Some(
            build_admin_provider_query_models_response(state, &payload).await?,
        )),
    }
}
