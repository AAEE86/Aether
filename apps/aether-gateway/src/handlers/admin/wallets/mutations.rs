use super::admin_wallets_shared::{
    admin_wallet_id_from_suffix_path, admin_wallet_operator_id,
    admin_wallet_refund_ids_from_suffix_path, build_admin_wallet_not_found_response,
    build_admin_wallet_payment_order_payload, build_admin_wallet_refund_not_found_response,
    build_admin_wallet_refund_payload, build_admin_wallet_summary_payload,
    build_admin_wallet_transaction_payload, build_admin_wallets_bad_request_response,
    build_admin_wallets_data_unavailable_response, normalize_admin_wallet_balance_type,
    normalize_admin_wallet_description, normalize_admin_wallet_non_zero_amount,
    normalize_admin_wallet_optional_text, normalize_admin_wallet_payment_method,
    normalize_admin_wallet_positive_amount, normalize_admin_wallet_required_text,
    resolve_admin_wallet_owner_summary, AdminWalletAdjustRequest, AdminWalletRechargeRequest,
    AdminWalletRefundCompleteRequest, AdminWalletRefundFailRequest,
    ADMIN_WALLETS_API_KEY_GIFT_ADJUST_DETAIL, ADMIN_WALLETS_API_KEY_RECHARGE_DETAIL,
    ADMIN_WALLETS_API_KEY_REFUND_DETAIL,
};
use crate::gateway::handlers::unix_secs_to_rfc3339;
use crate::gateway::{AppState, GatewayError, GatewayPublicRequestContext};
use axum::{
    body::Body,
    response::{IntoResponse, Response},
    Json,
};
use serde_json::json;

pub(super) async fn build_admin_wallet_adjust_response(
    state: &AppState,
    request_context: &GatewayPublicRequestContext,
    request_body: Option<&axum::body::Bytes>,
) -> Result<Response<Body>, GatewayError> {
    let Some(wallet_id) =
        admin_wallet_id_from_suffix_path(&request_context.request_path, "/adjust")
    else {
        return Ok(build_admin_wallets_bad_request_response("wallet_id 无效"));
    };
    let Some(request_body) = request_body else {
        return Ok(build_admin_wallets_bad_request_response("请求体不能为空"));
    };
    let payload = match serde_json::from_slice::<AdminWalletAdjustRequest>(request_body) {
        Ok(value) => value,
        Err(_) => return Ok(build_admin_wallets_bad_request_response("请求体格式无效")),
    };
    let amount_usd = match normalize_admin_wallet_non_zero_amount(payload.amount_usd, "amount_usd")
    {
        Ok(value) => value,
        Err(detail) => return Ok(build_admin_wallets_bad_request_response(detail)),
    };
    let balance_type = match normalize_admin_wallet_balance_type(payload.balance_type) {
        Ok(value) => value,
        Err(detail) => return Ok(build_admin_wallets_bad_request_response(detail)),
    };
    let description = match normalize_admin_wallet_description(payload.description) {
        Ok(value) => value,
        Err(detail) => return Ok(build_admin_wallets_bad_request_response(detail)),
    };

    let Some(existing_wallet) = state
        .find_wallet(aether_data::repository::wallet::WalletLookupKey::WalletId(
            &wallet_id,
        ))
        .await?
    else {
        return Ok(build_admin_wallet_not_found_response());
    };
    if existing_wallet.api_key_id.is_some() && balance_type == "gift" {
        return Ok(build_admin_wallets_bad_request_response(
            ADMIN_WALLETS_API_KEY_GIFT_ADJUST_DETAIL,
        ));
    }
    let operator_id = admin_wallet_operator_id(request_context);
    let has_postgres = state.postgres_pool().is_some();
    let Some((wallet, transaction)) = state
        .admin_adjust_wallet_balance(
            &wallet_id,
            amount_usd,
            &balance_type,
            operator_id.as_deref(),
            description.as_deref(),
        )
        .await?
    else {
        return if has_postgres {
            Ok(build_admin_wallet_not_found_response())
        } else {
            Ok(build_admin_wallets_data_unavailable_response())
        };
    };
    let owner = resolve_admin_wallet_owner_summary(state, &wallet).await?;
    let wallet_payload = build_admin_wallet_summary_payload(&wallet, &owner);
    let transaction_payload = build_admin_wallet_transaction_payload(
        &wallet,
        &owner,
        transaction.id,
        &transaction.category,
        &transaction.reason_code,
        transaction.amount,
        transaction.balance_before,
        transaction.balance_after,
        transaction.recharge_balance_before,
        transaction.recharge_balance_after,
        transaction.gift_balance_before,
        transaction.gift_balance_after,
        transaction.link_type.as_deref(),
        transaction.link_id.as_deref(),
        transaction.operator_id.as_deref(),
        transaction.description.as_deref(),
        unix_secs_to_rfc3339(transaction.created_at_unix_secs),
    );
    Ok(Json(json!({
        "wallet": wallet_payload,
        "transaction": transaction_payload,
    }))
    .into_response())
}

pub(super) async fn build_admin_wallet_recharge_response(
    state: &AppState,
    request_context: &GatewayPublicRequestContext,
    request_body: Option<&axum::body::Bytes>,
) -> Result<Response<Body>, GatewayError> {
    let Some(wallet_id) =
        admin_wallet_id_from_suffix_path(&request_context.request_path, "/recharge")
    else {
        return Ok(build_admin_wallets_bad_request_response("wallet_id 无效"));
    };
    let Some(request_body) = request_body else {
        return Ok(build_admin_wallets_bad_request_response("请求体不能为空"));
    };
    let payload = match serde_json::from_slice::<AdminWalletRechargeRequest>(request_body) {
        Ok(value) => value,
        Err(_) => return Ok(build_admin_wallets_bad_request_response("请求体格式无效")),
    };
    let amount_usd = match normalize_admin_wallet_positive_amount(payload.amount_usd, "amount_usd")
    {
        Ok(value) => value,
        Err(detail) => return Ok(build_admin_wallets_bad_request_response(detail)),
    };
    let payment_method = match normalize_admin_wallet_payment_method(payload.payment_method) {
        Ok(value) => value,
        Err(detail) => return Ok(build_admin_wallets_bad_request_response(detail)),
    };
    let description = match normalize_admin_wallet_description(payload.description) {
        Ok(value) => value,
        Err(detail) => return Ok(build_admin_wallets_bad_request_response(detail)),
    };

    let Some(existing_wallet) = state
        .find_wallet(aether_data::repository::wallet::WalletLookupKey::WalletId(
            &wallet_id,
        ))
        .await?
    else {
        return Ok(build_admin_wallet_not_found_response());
    };
    if existing_wallet.api_key_id.is_some() {
        return Ok(build_admin_wallets_bad_request_response(
            ADMIN_WALLETS_API_KEY_RECHARGE_DETAIL,
        ));
    }
    let operator_id = admin_wallet_operator_id(request_context);
    let has_postgres = state.postgres_pool().is_some();
    let Some((wallet, payment_order)) = state
        .admin_create_manual_wallet_recharge(
            &wallet_id,
            amount_usd,
            &payment_method,
            operator_id.as_deref(),
            description.as_deref(),
        )
        .await?
    else {
        return if has_postgres {
            Ok(build_admin_wallet_not_found_response())
        } else {
            Ok(build_admin_wallets_data_unavailable_response())
        };
    };
    let owner = resolve_admin_wallet_owner_summary(state, &wallet).await?;
    Ok(Json(json!({
        "wallet": build_admin_wallet_summary_payload(&wallet, &owner),
        "payment_order": build_admin_wallet_payment_order_payload(
            payment_order.id,
            payment_order.order_no,
            payment_order.amount_usd,
            payment_order.payment_method,
            payment_order.status,
            unix_secs_to_rfc3339(payment_order.created_at_unix_secs),
            payment_order
                .credited_at_unix_secs
                .and_then(unix_secs_to_rfc3339),
        ),
    }))
    .into_response())
}

pub(super) async fn build_admin_wallet_process_refund_response(
    state: &AppState,
    request_context: &GatewayPublicRequestContext,
) -> Result<Response<Body>, GatewayError> {
    let Some((wallet_id, refund_id)) =
        admin_wallet_refund_ids_from_suffix_path(&request_context.request_path, "/process")
    else {
        return Ok(build_admin_wallets_bad_request_response(
            "wallet_id 或 refund_id 无效",
        ));
    };

    let Some(existing_wallet) = state
        .find_wallet(aether_data::repository::wallet::WalletLookupKey::WalletId(
            &wallet_id,
        ))
        .await?
    else {
        return Ok(build_admin_wallet_not_found_response());
    };
    if existing_wallet.api_key_id.is_some() {
        return Ok(build_admin_wallets_bad_request_response(
            ADMIN_WALLETS_API_KEY_REFUND_DETAIL,
        ));
    }

    let operator_id = admin_wallet_operator_id(request_context);
    match state
        .admin_process_wallet_refund(&wallet_id, &refund_id, operator_id.as_deref())
        .await?
    {
        crate::gateway::AdminWalletMutationOutcome::Applied((wallet, refund, transaction)) => {
            let owner = resolve_admin_wallet_owner_summary(state, &wallet).await?;
            Ok(Json(json!({
                "wallet": build_admin_wallet_summary_payload(&wallet, &owner),
                "refund": build_admin_wallet_refund_payload(&wallet, &owner, &refund),
                "transaction": build_admin_wallet_transaction_payload(
                    &wallet,
                    &owner,
                    transaction.id,
                    &transaction.category,
                    &transaction.reason_code,
                    transaction.amount,
                    transaction.balance_before,
                    transaction.balance_after,
                    transaction.recharge_balance_before,
                    transaction.recharge_balance_after,
                    transaction.gift_balance_before,
                    transaction.gift_balance_after,
                    transaction.link_type.as_deref(),
                    transaction.link_id.as_deref(),
                    transaction.operator_id.as_deref(),
                    transaction.description.as_deref(),
                    unix_secs_to_rfc3339(transaction.created_at_unix_secs),
                ),
            }))
            .into_response())
        }
        crate::gateway::AdminWalletMutationOutcome::NotFound => {
            Ok(build_admin_wallet_refund_not_found_response())
        }
        crate::gateway::AdminWalletMutationOutcome::Invalid(detail) => {
            Ok(build_admin_wallets_bad_request_response(detail))
        }
        crate::gateway::AdminWalletMutationOutcome::Unavailable => {
            Ok(build_admin_wallets_data_unavailable_response())
        }
    }
}

pub(super) async fn build_admin_wallet_complete_refund_response(
    state: &AppState,
    request_context: &GatewayPublicRequestContext,
    request_body: Option<&axum::body::Bytes>,
) -> Result<Response<Body>, GatewayError> {
    let Some((wallet_id, refund_id)) =
        admin_wallet_refund_ids_from_suffix_path(&request_context.request_path, "/complete")
    else {
        return Ok(build_admin_wallets_bad_request_response(
            "wallet_id 或 refund_id 无效",
        ));
    };
    let Some(request_body) = request_body else {
        return Ok(build_admin_wallets_bad_request_response("请求体不能为空"));
    };
    let payload = match serde_json::from_slice::<AdminWalletRefundCompleteRequest>(request_body) {
        Ok(value) => value,
        Err(_) => return Ok(build_admin_wallets_bad_request_response("请求体格式无效")),
    };
    let gateway_refund_id = match normalize_admin_wallet_optional_text(
        payload.gateway_refund_id,
        "gateway_refund_id",
        128,
    ) {
        Ok(value) => value,
        Err(detail) => return Ok(build_admin_wallets_bad_request_response(detail)),
    };
    let payout_reference = match normalize_admin_wallet_optional_text(
        payload.payout_reference,
        "payout_reference",
        255,
    ) {
        Ok(value) => value,
        Err(detail) => return Ok(build_admin_wallets_bad_request_response(detail)),
    };
    if payload
        .payout_proof
        .as_ref()
        .is_some_and(|value| !value.is_object())
    {
        return Ok(build_admin_wallets_bad_request_response(
            "payout_proof 必须为对象",
        ));
    }

    let Some(wallet) = state
        .find_wallet(aether_data::repository::wallet::WalletLookupKey::WalletId(
            &wallet_id,
        ))
        .await?
    else {
        return Ok(build_admin_wallet_not_found_response());
    };
    if wallet.api_key_id.is_some() {
        return Ok(build_admin_wallets_bad_request_response(
            ADMIN_WALLETS_API_KEY_REFUND_DETAIL,
        ));
    }

    let owner = resolve_admin_wallet_owner_summary(state, &wallet).await?;
    match state
        .admin_complete_wallet_refund(
            &wallet_id,
            &refund_id,
            gateway_refund_id.as_deref(),
            payout_reference.as_deref(),
            payload.payout_proof,
        )
        .await?
    {
        crate::gateway::AdminWalletMutationOutcome::Applied(refund) => Ok(Json(json!({
            "refund": build_admin_wallet_refund_payload(&wallet, &owner, &refund),
        }))
        .into_response()),
        crate::gateway::AdminWalletMutationOutcome::NotFound => {
            Ok(build_admin_wallet_refund_not_found_response())
        }
        crate::gateway::AdminWalletMutationOutcome::Invalid(detail) => {
            let detail = if detail == "refund status must be processing before completion" {
                "只有 processing 状态的退款可以标记完成".to_string()
            } else {
                detail
            };
            Ok(build_admin_wallets_bad_request_response(detail))
        }
        crate::gateway::AdminWalletMutationOutcome::Unavailable => {
            Ok(build_admin_wallets_data_unavailable_response())
        }
    }
}

pub(super) async fn build_admin_wallet_fail_refund_response(
    state: &AppState,
    request_context: &GatewayPublicRequestContext,
    request_body: Option<&axum::body::Bytes>,
) -> Result<Response<Body>, GatewayError> {
    let Some((wallet_id, refund_id)) =
        admin_wallet_refund_ids_from_suffix_path(&request_context.request_path, "/fail")
    else {
        return Ok(build_admin_wallets_bad_request_response(
            "wallet_id 或 refund_id 无效",
        ));
    };
    let Some(request_body) = request_body else {
        return Ok(build_admin_wallets_bad_request_response("请求体不能为空"));
    };
    let payload = match serde_json::from_slice::<AdminWalletRefundFailRequest>(request_body) {
        Ok(value) => value,
        Err(_) => return Ok(build_admin_wallets_bad_request_response("请求体格式无效")),
    };
    let reason = match normalize_admin_wallet_required_text(payload.reason, "reason", 500) {
        Ok(value) => value,
        Err(detail) => return Ok(build_admin_wallets_bad_request_response(detail)),
    };

    let Some(existing_wallet) = state
        .find_wallet(aether_data::repository::wallet::WalletLookupKey::WalletId(
            &wallet_id,
        ))
        .await?
    else {
        return Ok(build_admin_wallet_not_found_response());
    };
    if existing_wallet.api_key_id.is_some() {
        return Ok(build_admin_wallets_bad_request_response(
            ADMIN_WALLETS_API_KEY_REFUND_DETAIL,
        ));
    }

    let operator_id = admin_wallet_operator_id(request_context);
    match state
        .admin_fail_wallet_refund(&wallet_id, &refund_id, &reason, operator_id.as_deref())
        .await?
    {
        crate::gateway::AdminWalletMutationOutcome::Applied((wallet, refund, transaction)) => {
            let owner = resolve_admin_wallet_owner_summary(state, &wallet).await?;
            Ok(Json(json!({
                "wallet": build_admin_wallet_summary_payload(&wallet, &owner),
                "refund": build_admin_wallet_refund_payload(&wallet, &owner, &refund),
                "transaction": transaction.map(|transaction| build_admin_wallet_transaction_payload(
                    &wallet,
                    &owner,
                    transaction.id,
                    &transaction.category,
                    &transaction.reason_code,
                    transaction.amount,
                    transaction.balance_before,
                    transaction.balance_after,
                    transaction.recharge_balance_before,
                    transaction.recharge_balance_after,
                    transaction.gift_balance_before,
                    transaction.gift_balance_after,
                    transaction.link_type.as_deref(),
                    transaction.link_id.as_deref(),
                    transaction.operator_id.as_deref(),
                    transaction.description.as_deref(),
                    unix_secs_to_rfc3339(transaction.created_at_unix_secs),
                )).unwrap_or(serde_json::Value::Null),
            }))
            .into_response())
        }
        crate::gateway::AdminWalletMutationOutcome::NotFound => {
            Ok(build_admin_wallet_refund_not_found_response())
        }
        crate::gateway::AdminWalletMutationOutcome::Invalid(detail) => {
            Ok(build_admin_wallets_bad_request_response(detail))
        }
        crate::gateway::AdminWalletMutationOutcome::Unavailable => {
            Ok(build_admin_wallets_data_unavailable_response())
        }
    }
}
