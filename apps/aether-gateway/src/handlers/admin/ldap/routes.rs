use super::ldap_builders::{
    admin_ldap_test_connection, build_admin_ldap_test_config, build_admin_ldap_update_config,
    AdminLdapConfigTestRequest, AdminLdapConfigUpdateRequest,
};
use super::ldap_shared::*;
use crate::gateway::{AppState, GatewayError, GatewayPublicRequestContext};
use axum::{
    body::{Body, Bytes},
    http,
    response::{IntoResponse, Response},
    Json,
};
use serde_json::json;

pub(super) async fn maybe_build_local_admin_ldap_response(
    state: &AppState,
    request_context: &GatewayPublicRequestContext,
    request_body: Option<&Bytes>,
) -> Result<Option<Response<Body>>, GatewayError> {
    let Some(decision) = request_context.control_decision.as_ref() else {
        return Ok(None);
    };
    if decision.route_family.as_deref() != Some("ldap_manage") {
        return Ok(None);
    }

    match decision.route_kind.as_deref() {
        Some("get_config")
            if request_context.request_method == http::Method::GET
                && is_admin_ldap_config_root(&request_context.request_path) =>
        {
            return Ok(Some(
                Json(build_admin_ldap_config_payload(
                    state.get_ldap_module_config().await?.as_ref(),
                ))
                .into_response(),
            ));
        }
        Some("set_config")
            if request_context.request_method == http::Method::PUT
                && is_admin_ldap_config_root(&request_context.request_path) =>
        {
            if !state.has_auth_module_writer() {
                return Ok(Some(admin_ldap_unavailable_response()));
            }
            let Some(request_body) = request_body else {
                return Ok(Some(admin_ldap_bad_request_response("请求数据验证失败")));
            };
            let payload = match serde_json::from_slice::<AdminLdapConfigUpdateRequest>(request_body)
            {
                Ok(payload) => payload,
                Err(_) => return Ok(Some(admin_ldap_bad_request_response("请求数据验证失败"))),
            };
            let update = match build_admin_ldap_update_config(state, payload).await {
                Ok(config) => config,
                Err(detail) => return Ok(Some(admin_ldap_bad_request_response(detail))),
            };
            let saved = state.upsert_ldap_module_config(&update).await?;
            if saved.is_none() {
                return Ok(Some(admin_ldap_unavailable_response()));
            }
            return Ok(Some(
                Json(json!({ "message": "LDAP配置更新成功" })).into_response(),
            ));
        }
        Some("test_connection")
            if request_context.request_method == http::Method::POST
                && is_admin_ldap_test_root(&request_context.request_path) =>
        {
            let payload = match request_body {
                Some(body) if !body.is_empty() => match serde_json::from_slice::<
                    AdminLdapConfigTestRequest,
                >(body)
                {
                    Ok(value) => value,
                    Err(_) => return Ok(Some(admin_ldap_bad_request_response("请求数据验证失败"))),
                },
                _ => AdminLdapConfigTestRequest::default(),
            };
            let merged = match build_admin_ldap_test_config(state, payload).await {
                Ok(config) => config,
                Err(detail) => return Ok(Some(admin_ldap_bad_request_response(detail))),
            };
            let response = match merged {
                Some(config) => {
                    let (success, message) = admin_ldap_test_connection(config).await?;
                    json!({ "success": success, "message": message })
                }
                None => json!({
                    "success": false,
                    "message": "缺少必要字段: server_url, bind_dn, base_dn, bind_password",
                }),
            };
            return Ok(Some(Json(response).into_response()));
        }
        _ => {}
    }

    Ok(None)
}
