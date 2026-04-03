use sqlx::Row;

use crate::gateway::{
    AdminWalletMutationOutcome, AdminWalletPaymentOrderRecord, AppState, GatewayError,
};

use super::{
    admin_payment_gateway_response_map, admin_wallet_payment_order_from_row,
    admin_wallet_snapshot_from_row,
};

impl AppState {
    pub(crate) async fn admin_expire_payment_order(
        &self,
        order_id: &str,
    ) -> Result<AdminWalletMutationOutcome<(AdminWalletPaymentOrderRecord, bool)>, GatewayError>
    {
        #[cfg(test)]
        if let Some(store) = self.admin_wallet_payment_order_store.as_ref() {
            let mut guard = store
                .lock()
                .expect("admin wallet payment order store should lock");
            let Some(order) = guard.get_mut(order_id) else {
                return Ok(AdminWalletMutationOutcome::NotFound);
            };
            if order.status == "credited" {
                return Ok(AdminWalletMutationOutcome::Invalid(
                    "credited order cannot be expired".to_string(),
                ));
            }
            if order.status == "expired" {
                return Ok(AdminWalletMutationOutcome::Applied((order.clone(), false)));
            }
            if order.status != "pending" {
                return Ok(AdminWalletMutationOutcome::Invalid(format!(
                    "only pending order can be expired: {}",
                    order.status
                )));
            }
            let mut gateway_response =
                admin_payment_gateway_response_map(order.gateway_response.take());
            gateway_response.insert(
                "expire_reason".to_string(),
                serde_json::Value::String("admin_mark_expired".to_string()),
            );
            gateway_response.insert(
                "expired_at".to_string(),
                serde_json::Value::String(chrono::Utc::now().to_rfc3339()),
            );
            order.status = "expired".to_string();
            order.gateway_response = Some(serde_json::Value::Object(gateway_response));
            return Ok(AdminWalletMutationOutcome::Applied((order.clone(), true)));
        }

        let Some(pool) = self.postgres_pool() else {
            return Ok(AdminWalletMutationOutcome::Unavailable);
        };
        let mut tx = pool
            .begin()
            .await
            .map_err(|err| GatewayError::Internal(err.to_string()))?;
        let Some(row) = sqlx::query(
            r#"
SELECT
  id,
  order_no,
  wallet_id,
  user_id,
  CAST(amount_usd AS DOUBLE PRECISION) AS amount_usd,
  CAST(pay_amount AS DOUBLE PRECISION) AS pay_amount,
  pay_currency,
  CAST(exchange_rate AS DOUBLE PRECISION) AS exchange_rate,
  CAST(refunded_amount_usd AS DOUBLE PRECISION) AS refunded_amount_usd,
  CAST(refundable_amount_usd AS DOUBLE PRECISION) AS refundable_amount_usd,
  payment_method,
  gateway_order_id,
  gateway_response,
  status,
  CAST(EXTRACT(EPOCH FROM created_at) AS BIGINT) AS created_at_unix_secs,
  CAST(EXTRACT(EPOCH FROM paid_at) AS BIGINT) AS paid_at_unix_secs,
  CAST(EXTRACT(EPOCH FROM credited_at) AS BIGINT) AS credited_at_unix_secs,
  CAST(EXTRACT(EPOCH FROM expires_at) AS BIGINT) AS expires_at_unix_secs
FROM payment_orders
WHERE id = $1
FOR UPDATE
            "#,
        )
        .bind(order_id)
        .fetch_optional(&mut *tx)
        .await
        .map_err(|err| GatewayError::Internal(err.to_string()))?
        else {
            let _ = tx.rollback().await;
            return Ok(AdminWalletMutationOutcome::NotFound);
        };
        let order = admin_wallet_payment_order_from_row(&row)?;
        if order.status == "credited" {
            let _ = tx.rollback().await;
            return Ok(AdminWalletMutationOutcome::Invalid(
                "credited order cannot be expired".to_string(),
            ));
        }
        if order.status == "expired" {
            tx.commit()
                .await
                .map_err(|err| GatewayError::Internal(err.to_string()))?;
            return Ok(AdminWalletMutationOutcome::Applied((order, false)));
        }
        if order.status != "pending" {
            let _ = tx.rollback().await;
            return Ok(AdminWalletMutationOutcome::Invalid(format!(
                "only pending order can be expired: {}",
                order.status
            )));
        }
        let mut gateway_response =
            admin_payment_gateway_response_map(order.gateway_response.clone());
        gateway_response.insert(
            "expire_reason".to_string(),
            serde_json::Value::String("admin_mark_expired".to_string()),
        );
        gateway_response.insert(
            "expired_at".to_string(),
            serde_json::Value::String(chrono::Utc::now().to_rfc3339()),
        );
        let row = sqlx::query(
            r#"
UPDATE payment_orders
SET
  status = 'expired',
  gateway_response = $2
WHERE id = $1
RETURNING
  id,
  order_no,
  wallet_id,
  user_id,
  CAST(amount_usd AS DOUBLE PRECISION) AS amount_usd,
  CAST(pay_amount AS DOUBLE PRECISION) AS pay_amount,
  pay_currency,
  CAST(exchange_rate AS DOUBLE PRECISION) AS exchange_rate,
  CAST(refunded_amount_usd AS DOUBLE PRECISION) AS refunded_amount_usd,
  CAST(refundable_amount_usd AS DOUBLE PRECISION) AS refundable_amount_usd,
  payment_method,
  gateway_order_id,
  gateway_response,
  status,
  CAST(EXTRACT(EPOCH FROM created_at) AS BIGINT) AS created_at_unix_secs,
  CAST(EXTRACT(EPOCH FROM paid_at) AS BIGINT) AS paid_at_unix_secs,
  CAST(EXTRACT(EPOCH FROM credited_at) AS BIGINT) AS credited_at_unix_secs,
  CAST(EXTRACT(EPOCH FROM expires_at) AS BIGINT) AS expires_at_unix_secs
            "#,
        )
        .bind(order_id)
        .bind(serde_json::Value::Object(gateway_response))
        .fetch_one(&mut *tx)
        .await
        .map_err(|err| GatewayError::Internal(err.to_string()))?;
        tx.commit()
            .await
            .map_err(|err| GatewayError::Internal(err.to_string()))?;
        Ok(AdminWalletMutationOutcome::Applied((
            admin_wallet_payment_order_from_row(&row)?,
            true,
        )))
    }

    pub(crate) async fn admin_fail_payment_order(
        &self,
        order_id: &str,
    ) -> Result<AdminWalletMutationOutcome<AdminWalletPaymentOrderRecord>, GatewayError> {
        #[cfg(test)]
        if let Some(store) = self.admin_wallet_payment_order_store.as_ref() {
            let mut guard = store
                .lock()
                .expect("admin wallet payment order store should lock");
            let Some(order) = guard.get_mut(order_id) else {
                return Ok(AdminWalletMutationOutcome::NotFound);
            };
            if order.status == "credited" {
                return Ok(AdminWalletMutationOutcome::Invalid(
                    "credited order cannot be failed".to_string(),
                ));
            }
            let mut gateway_response =
                admin_payment_gateway_response_map(order.gateway_response.take());
            gateway_response.insert(
                "failure_reason".to_string(),
                serde_json::Value::String("admin_mark_failed".to_string()),
            );
            gateway_response.insert(
                "failed_at".to_string(),
                serde_json::Value::String(chrono::Utc::now().to_rfc3339()),
            );
            order.status = "failed".to_string();
            order.gateway_response = Some(serde_json::Value::Object(gateway_response));
            return Ok(AdminWalletMutationOutcome::Applied(order.clone()));
        }

        let Some(pool) = self.postgres_pool() else {
            return Ok(AdminWalletMutationOutcome::Unavailable);
        };
        let mut tx = pool
            .begin()
            .await
            .map_err(|err| GatewayError::Internal(err.to_string()))?;
        let Some(row) = sqlx::query(
            r#"
SELECT
  id,
  order_no,
  wallet_id,
  user_id,
  CAST(amount_usd AS DOUBLE PRECISION) AS amount_usd,
  CAST(pay_amount AS DOUBLE PRECISION) AS pay_amount,
  pay_currency,
  CAST(exchange_rate AS DOUBLE PRECISION) AS exchange_rate,
  CAST(refunded_amount_usd AS DOUBLE PRECISION) AS refunded_amount_usd,
  CAST(refundable_amount_usd AS DOUBLE PRECISION) AS refundable_amount_usd,
  payment_method,
  gateway_order_id,
  gateway_response,
  status,
  CAST(EXTRACT(EPOCH FROM created_at) AS BIGINT) AS created_at_unix_secs,
  CAST(EXTRACT(EPOCH FROM paid_at) AS BIGINT) AS paid_at_unix_secs,
  CAST(EXTRACT(EPOCH FROM credited_at) AS BIGINT) AS credited_at_unix_secs,
  CAST(EXTRACT(EPOCH FROM expires_at) AS BIGINT) AS expires_at_unix_secs
FROM payment_orders
WHERE id = $1
FOR UPDATE
            "#,
        )
        .bind(order_id)
        .fetch_optional(&mut *tx)
        .await
        .map_err(|err| GatewayError::Internal(err.to_string()))?
        else {
            let _ = tx.rollback().await;
            return Ok(AdminWalletMutationOutcome::NotFound);
        };
        let order = admin_wallet_payment_order_from_row(&row)?;
        if order.status == "credited" {
            let _ = tx.rollback().await;
            return Ok(AdminWalletMutationOutcome::Invalid(
                "credited order cannot be failed".to_string(),
            ));
        }
        let mut gateway_response =
            admin_payment_gateway_response_map(order.gateway_response.clone());
        gateway_response.insert(
            "failure_reason".to_string(),
            serde_json::Value::String("admin_mark_failed".to_string()),
        );
        gateway_response.insert(
            "failed_at".to_string(),
            serde_json::Value::String(chrono::Utc::now().to_rfc3339()),
        );
        let row = sqlx::query(
            r#"
UPDATE payment_orders
SET
  status = 'failed',
  gateway_response = $2
WHERE id = $1
RETURNING
  id,
  order_no,
  wallet_id,
  user_id,
  CAST(amount_usd AS DOUBLE PRECISION) AS amount_usd,
  CAST(pay_amount AS DOUBLE PRECISION) AS pay_amount,
  pay_currency,
  CAST(exchange_rate AS DOUBLE PRECISION) AS exchange_rate,
  CAST(refunded_amount_usd AS DOUBLE PRECISION) AS refunded_amount_usd,
  CAST(refundable_amount_usd AS DOUBLE PRECISION) AS refundable_amount_usd,
  payment_method,
  gateway_order_id,
  gateway_response,
  status,
  CAST(EXTRACT(EPOCH FROM created_at) AS BIGINT) AS created_at_unix_secs,
  CAST(EXTRACT(EPOCH FROM paid_at) AS BIGINT) AS paid_at_unix_secs,
  CAST(EXTRACT(EPOCH FROM credited_at) AS BIGINT) AS credited_at_unix_secs,
  CAST(EXTRACT(EPOCH FROM expires_at) AS BIGINT) AS expires_at_unix_secs
            "#,
        )
        .bind(order_id)
        .bind(serde_json::Value::Object(gateway_response))
        .fetch_one(&mut *tx)
        .await
        .map_err(|err| GatewayError::Internal(err.to_string()))?;
        tx.commit()
            .await
            .map_err(|err| GatewayError::Internal(err.to_string()))?;
        Ok(AdminWalletMutationOutcome::Applied(
            admin_wallet_payment_order_from_row(&row)?,
        ))
    }

    #[allow(clippy::too_many_arguments)]
    pub(crate) async fn admin_credit_payment_order(
        &self,
        order_id: &str,
        gateway_order_id: Option<&str>,
        pay_amount: Option<f64>,
        pay_currency: Option<&str>,
        exchange_rate: Option<f64>,
        gateway_response_patch: Option<serde_json::Value>,
        operator_id: Option<&str>,
    ) -> Result<AdminWalletMutationOutcome<(AdminWalletPaymentOrderRecord, bool)>, GatewayError>
    {
        #[cfg(test)]
        if let (Some(order_store), Some(wallet_store)) = (
            self.admin_wallet_payment_order_store.as_ref(),
            self.auth_wallet_store.as_ref(),
        ) {
            let mut orders = order_store
                .lock()
                .expect("admin wallet payment order store should lock");
            let Some(order) = orders.get_mut(order_id) else {
                return Ok(AdminWalletMutationOutcome::NotFound);
            };
            if order.status == "credited" {
                return Ok(AdminWalletMutationOutcome::Applied((order.clone(), false)));
            }
            if matches!(order.status.as_str(), "failed" | "expired" | "refunded") {
                return Ok(AdminWalletMutationOutcome::Invalid(format!(
                    "payment order is not creditable: {}",
                    order.status
                )));
            }
            if order
                .expires_at_unix_secs
                .is_some_and(|value| value < chrono::Utc::now().timestamp().max(0) as u64)
            {
                return Ok(AdminWalletMutationOutcome::Invalid(
                    "payment order expired".to_string(),
                ));
            }

            let mut wallets = wallet_store.lock().expect("auth wallet store should lock");
            let Some(wallet) = wallets.get_mut(&order.wallet_id) else {
                return Ok(AdminWalletMutationOutcome::Invalid(
                    "wallet not found".to_string(),
                ));
            };
            if wallet.status != "active" {
                return Ok(AdminWalletMutationOutcome::Invalid(
                    "wallet is not active".to_string(),
                ));
            }

            let now_unix_secs = chrono::Utc::now().timestamp().max(0) as u64;
            let mut gateway_response =
                admin_payment_gateway_response_map(order.gateway_response.take());
            if let Some(serde_json::Value::Object(map)) = gateway_response_patch {
                gateway_response.extend(map);
            }
            gateway_response.insert("manual_credit".to_string(), serde_json::Value::Bool(true));
            gateway_response.insert(
                "credited_by".to_string(),
                operator_id
                    .map(|value| serde_json::Value::String(value.to_string()))
                    .unwrap_or(serde_json::Value::Null),
            );

            wallet.balance += order.amount_usd;
            wallet.total_recharged += order.amount_usd;
            wallet.updated_at_unix_secs = now_unix_secs;

            if let Some(value) = gateway_order_id {
                order.gateway_order_id = Some(value.to_string());
            }
            if let Some(value) = pay_amount {
                order.pay_amount = Some(value);
            }
            if let Some(value) = pay_currency {
                order.pay_currency = Some(value.to_string());
            }
            if let Some(value) = exchange_rate {
                order.exchange_rate = Some(value);
            }
            order.status = "credited".to_string();
            order.paid_at_unix_secs = order.paid_at_unix_secs.or(Some(now_unix_secs));
            order.credited_at_unix_secs = Some(now_unix_secs);
            order.refundable_amount_usd = order.amount_usd;
            order.gateway_response = Some(serde_json::Value::Object(gateway_response));
            return Ok(AdminWalletMutationOutcome::Applied((order.clone(), true)));
        }

        let Some(pool) = self.postgres_pool() else {
            return Ok(AdminWalletMutationOutcome::Unavailable);
        };
        let mut tx = pool
            .begin()
            .await
            .map_err(|err| GatewayError::Internal(err.to_string()))?;
        let Some(order_row) = sqlx::query(
            r#"
SELECT
  id,
  order_no,
  wallet_id,
  user_id,
  CAST(amount_usd AS DOUBLE PRECISION) AS amount_usd,
  CAST(pay_amount AS DOUBLE PRECISION) AS pay_amount,
  pay_currency,
  CAST(exchange_rate AS DOUBLE PRECISION) AS exchange_rate,
  CAST(refunded_amount_usd AS DOUBLE PRECISION) AS refunded_amount_usd,
  CAST(refundable_amount_usd AS DOUBLE PRECISION) AS refundable_amount_usd,
  payment_method,
  gateway_order_id,
  gateway_response,
  status,
  CAST(EXTRACT(EPOCH FROM created_at) AS BIGINT) AS created_at_unix_secs,
  CAST(EXTRACT(EPOCH FROM paid_at) AS BIGINT) AS paid_at_unix_secs,
  CAST(EXTRACT(EPOCH FROM credited_at) AS BIGINT) AS credited_at_unix_secs,
  CAST(EXTRACT(EPOCH FROM expires_at) AS BIGINT) AS expires_at_unix_secs
FROM payment_orders
WHERE id = $1
FOR UPDATE
            "#,
        )
        .bind(order_id)
        .fetch_optional(&mut *tx)
        .await
        .map_err(|err| GatewayError::Internal(err.to_string()))?
        else {
            let _ = tx.rollback().await;
            return Ok(AdminWalletMutationOutcome::NotFound);
        };
        let order = admin_wallet_payment_order_from_row(&order_row)?;
        if order.status == "credited" {
            tx.commit()
                .await
                .map_err(|err| GatewayError::Internal(err.to_string()))?;
            return Ok(AdminWalletMutationOutcome::Applied((order, false)));
        }
        if matches!(order.status.as_str(), "failed" | "expired" | "refunded") {
            let _ = tx.rollback().await;
            return Ok(AdminWalletMutationOutcome::Invalid(format!(
                "payment order is not creditable: {}",
                order.status
            )));
        }
        if order
            .expires_at_unix_secs
            .is_some_and(|value| value < chrono::Utc::now().timestamp().max(0) as u64)
        {
            let _ = tx.rollback().await;
            return Ok(AdminWalletMutationOutcome::Invalid(
                "payment order expired".to_string(),
            ));
        }

        let Some(wallet_row) = sqlx::query(
            r#"
SELECT
  id,
  user_id,
  api_key_id,
  CAST(balance AS DOUBLE PRECISION) AS balance,
  CAST(gift_balance AS DOUBLE PRECISION) AS gift_balance,
  limit_mode,
  currency,
  status,
  CAST(total_recharged AS DOUBLE PRECISION) AS total_recharged,
  CAST(total_consumed AS DOUBLE PRECISION) AS total_consumed,
  CAST(total_refunded AS DOUBLE PRECISION) AS total_refunded,
  CAST(total_adjusted AS DOUBLE PRECISION) AS total_adjusted,
  CAST(EXTRACT(EPOCH FROM updated_at) AS BIGINT) AS updated_at_unix_secs
FROM wallets
WHERE id = $1
FOR UPDATE
            "#,
        )
        .bind(&order.wallet_id)
        .fetch_optional(&mut *tx)
        .await
        .map_err(|err| GatewayError::Internal(err.to_string()))?
        else {
            let _ = tx.rollback().await;
            return Ok(AdminWalletMutationOutcome::Invalid(
                "wallet not found".to_string(),
            ));
        };
        let wallet_status = wallet_row
            .try_get::<String, _>("status")
            .map_err(|err| GatewayError::Internal(err.to_string()))?;
        if wallet_status != "active" {
            let _ = tx.rollback().await;
            return Ok(AdminWalletMutationOutcome::Invalid(
                "wallet is not active".to_string(),
            ));
        }

        let before_recharge = wallet_row
            .try_get::<f64, _>("balance")
            .map_err(|err| GatewayError::Internal(err.to_string()))?;
        let before_gift = wallet_row
            .try_get::<f64, _>("gift_balance")
            .map_err(|err| GatewayError::Internal(err.to_string()))?;
        let before_total = before_recharge + before_gift;
        let after_recharge = before_recharge + order.amount_usd;
        let now_unix_secs = chrono::Utc::now().timestamp().max(0) as u64;

        let wallet_row = sqlx::query(
            r#"
UPDATE wallets
SET
  balance = $2,
  total_recharged = total_recharged + $3,
  updated_at = NOW()
WHERE id = $1
RETURNING
  id,
  user_id,
  api_key_id,
  CAST(balance AS DOUBLE PRECISION) AS balance,
  CAST(gift_balance AS DOUBLE PRECISION) AS gift_balance,
  limit_mode,
  currency,
  status,
  CAST(total_recharged AS DOUBLE PRECISION) AS total_recharged,
  CAST(total_consumed AS DOUBLE PRECISION) AS total_consumed,
  CAST(total_refunded AS DOUBLE PRECISION) AS total_refunded,
  CAST(total_adjusted AS DOUBLE PRECISION) AS total_adjusted,
  CAST(EXTRACT(EPOCH FROM updated_at) AS BIGINT) AS updated_at_unix_secs
            "#,
        )
        .bind(&order.wallet_id)
        .bind(after_recharge)
        .bind(order.amount_usd)
        .fetch_one(&mut *tx)
        .await
        .map_err(|err| GatewayError::Internal(err.to_string()))?;
        let _wallet = admin_wallet_snapshot_from_row(&wallet_row)?;

        sqlx::query(
            r#"
INSERT INTO wallet_transactions (
  id,
  wallet_id,
  category,
  reason_code,
  amount,
  balance_before,
  balance_after,
  recharge_balance_before,
  recharge_balance_after,
  gift_balance_before,
  gift_balance_after,
  link_type,
  link_id,
  operator_id,
  description,
  created_at
)
VALUES (
  $1,
  $2,
  'recharge',
  'topup_gateway',
  $3,
  $4,
  $5,
  $6,
  $7,
  $8,
  $8,
  'payment_order',
  $9,
  NULL,
  $10,
  NOW()
)
            "#,
        )
        .bind(uuid::Uuid::new_v4().to_string())
        .bind(&order.wallet_id)
        .bind(order.amount_usd)
        .bind(before_total)
        .bind(after_recharge + before_gift)
        .bind(before_recharge)
        .bind(after_recharge)
        .bind(before_gift)
        .bind(order_id)
        .bind(format!("充值到账({})", order.payment_method))
        .execute(&mut *tx)
        .await
        .map_err(|err| GatewayError::Internal(err.to_string()))?;

        let mut gateway_response =
            admin_payment_gateway_response_map(order.gateway_response.clone());
        if let Some(serde_json::Value::Object(map)) = gateway_response_patch {
            gateway_response.extend(map);
        }
        gateway_response.insert("manual_credit".to_string(), serde_json::Value::Bool(true));
        gateway_response.insert(
            "credited_by".to_string(),
            operator_id
                .map(|value| serde_json::Value::String(value.to_string()))
                .unwrap_or(serde_json::Value::Null),
        );
        let next_gateway_order_id = gateway_order_id
            .map(ToOwned::to_owned)
            .or(order.gateway_order_id.clone());
        let next_pay_amount = pay_amount.or(order.pay_amount);
        let next_pay_currency = pay_currency
            .map(ToOwned::to_owned)
            .or(order.pay_currency.clone());
        let next_exchange_rate = exchange_rate.or(order.exchange_rate);
        let next_paid_at_unix_secs = order.paid_at_unix_secs.or(Some(now_unix_secs));

        let row = sqlx::query(
            r#"
UPDATE payment_orders
SET
  gateway_order_id = $2,
  gateway_response = $3,
  pay_amount = $4,
  pay_currency = $5,
  exchange_rate = $6,
  status = 'credited',
  paid_at = COALESCE(to_timestamp($7), NOW()),
  credited_at = NOW(),
  refundable_amount_usd = amount_usd
WHERE id = $1
RETURNING
  id,
  order_no,
  wallet_id,
  user_id,
  CAST(amount_usd AS DOUBLE PRECISION) AS amount_usd,
  CAST(pay_amount AS DOUBLE PRECISION) AS pay_amount,
  pay_currency,
  CAST(exchange_rate AS DOUBLE PRECISION) AS exchange_rate,
  CAST(refunded_amount_usd AS DOUBLE PRECISION) AS refunded_amount_usd,
  CAST(refundable_amount_usd AS DOUBLE PRECISION) AS refundable_amount_usd,
  payment_method,
  gateway_order_id,
  gateway_response,
  status,
  CAST(EXTRACT(EPOCH FROM created_at) AS BIGINT) AS created_at_unix_secs,
  CAST(EXTRACT(EPOCH FROM paid_at) AS BIGINT) AS paid_at_unix_secs,
  CAST(EXTRACT(EPOCH FROM credited_at) AS BIGINT) AS credited_at_unix_secs,
  CAST(EXTRACT(EPOCH FROM expires_at) AS BIGINT) AS expires_at_unix_secs
            "#,
        )
        .bind(order_id)
        .bind(next_gateway_order_id)
        .bind(serde_json::Value::Object(gateway_response))
        .bind(next_pay_amount)
        .bind(next_pay_currency)
        .bind(next_exchange_rate)
        .bind(i64::try_from(next_paid_at_unix_secs.unwrap_or(now_unix_secs)).unwrap_or_default())
        .fetch_one(&mut *tx)
        .await
        .map_err(|err| GatewayError::Internal(err.to_string()))?;
        tx.commit()
            .await
            .map_err(|err| GatewayError::Internal(err.to_string()))?;
        Ok(AdminWalletMutationOutcome::Applied((
            admin_wallet_payment_order_from_row(&row)?,
            true,
        )))
    }
}
