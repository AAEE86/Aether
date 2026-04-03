use sqlx::Row;

use crate::gateway::{
    AdminWalletPaymentOrderRecord, AdminWalletTransactionRecord, AppState, GatewayError,
};

use super::{admin_wallet_build_order_no, admin_wallet_snapshot_from_row};

impl AppState {
    pub(crate) async fn admin_adjust_wallet_balance(
        &self,
        wallet_id: &str,
        amount_usd: f64,
        balance_type: &str,
        operator_id: Option<&str>,
        description: Option<&str>,
    ) -> Result<
        Option<(
            aether_data::repository::wallet::StoredWalletSnapshot,
            AdminWalletTransactionRecord,
        )>,
        GatewayError,
    > {
        #[cfg(test)]
        if let Some(store) = self.auth_wallet_store.as_ref() {
            let mut guard = store.lock().expect("auth wallet store should lock");
            let Some(wallet) = guard.get_mut(wallet_id) else {
                return Ok(None);
            };

            let before_recharge = wallet.balance;
            let before_gift = wallet.gift_balance;
            let before_total = before_recharge + before_gift;
            let mut after_recharge = before_recharge;
            let mut after_gift = before_gift;

            if amount_usd > 0.0 {
                if balance_type.eq_ignore_ascii_case("gift") {
                    after_gift += amount_usd;
                } else {
                    after_recharge += amount_usd;
                }
            } else {
                let mut remaining = -amount_usd;
                let consume_positive_bucket = |balance: &mut f64, to_consume: &mut f64| {
                    if *to_consume <= 0.0 {
                        return;
                    }
                    let available = (*balance).max(0.0);
                    let consumed = available.min(*to_consume);
                    *balance -= consumed;
                    *to_consume -= consumed;
                };
                if balance_type.eq_ignore_ascii_case("gift") {
                    consume_positive_bucket(&mut after_gift, &mut remaining);
                    consume_positive_bucket(&mut after_recharge, &mut remaining);
                } else {
                    consume_positive_bucket(&mut after_recharge, &mut remaining);
                    consume_positive_bucket(&mut after_gift, &mut remaining);
                }
                if remaining > 0.0 {
                    after_recharge -= remaining;
                }
            }

            wallet.balance = after_recharge;
            wallet.gift_balance = after_gift;
            wallet.total_adjusted += amount_usd;
            wallet.updated_at_unix_secs = chrono::Utc::now().timestamp().max(0) as u64;

            let transaction = AdminWalletTransactionRecord {
                id: uuid::Uuid::new_v4().to_string(),
                wallet_id: wallet.id.clone(),
                category: "adjust".to_string(),
                reason_code: "adjust_admin".to_string(),
                amount: amount_usd,
                balance_before: before_total,
                balance_after: after_recharge + after_gift,
                recharge_balance_before: before_recharge,
                recharge_balance_after: after_recharge,
                gift_balance_before: before_gift,
                gift_balance_after: after_gift,
                link_type: Some("admin_action".to_string()),
                link_id: Some(wallet.id.clone()),
                operator_id: operator_id.map(ToOwned::to_owned),
                description: Some(
                    description
                        .filter(|value| !value.trim().is_empty())
                        .unwrap_or("管理员调账")
                        .to_string(),
                ),
                created_at_unix_secs: chrono::Utc::now().timestamp().max(0) as u64,
            };
            return Ok(Some((wallet.clone(), transaction)));
        }

        let Some(pool) = self.postgres_pool() else {
            return Ok(None);
        };
        let mut tx = pool
            .begin()
            .await
            .map_err(|err| GatewayError::Internal(err.to_string()))?;
        let Some(row) = sqlx::query(
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
  CAST(total_adjusted AS DOUBLE PRECISION) AS total_adjusted
FROM wallets
WHERE id = $1
FOR UPDATE
            "#,
        )
        .bind(wallet_id)
        .fetch_optional(&mut *tx)
        .await
        .map_err(|err| GatewayError::Internal(err.to_string()))?
        else {
            let _ = tx.rollback().await;
            return Ok(None);
        };

        let before_recharge = row
            .try_get::<f64, _>("balance")
            .map_err(|err| GatewayError::Internal(err.to_string()))?;
        let before_gift = row
            .try_get::<f64, _>("gift_balance")
            .map_err(|err| GatewayError::Internal(err.to_string()))?;
        let before_total = before_recharge + before_gift;
        let mut after_recharge = before_recharge;
        let mut after_gift = before_gift;

        if amount_usd > 0.0 {
            if balance_type.eq_ignore_ascii_case("gift") {
                after_gift += amount_usd;
            } else {
                after_recharge += amount_usd;
            }
        } else {
            let mut remaining = -amount_usd;
            let consume_positive_bucket = |balance: &mut f64, to_consume: &mut f64| {
                if *to_consume <= 0.0 {
                    return;
                }
                let available = (*balance).max(0.0);
                let consumed = available.min(*to_consume);
                *balance -= consumed;
                *to_consume -= consumed;
            };
            if balance_type.eq_ignore_ascii_case("gift") {
                consume_positive_bucket(&mut after_gift, &mut remaining);
                consume_positive_bucket(&mut after_recharge, &mut remaining);
            } else {
                consume_positive_bucket(&mut after_recharge, &mut remaining);
                consume_positive_bucket(&mut after_gift, &mut remaining);
            }
            if remaining > 0.0 {
                after_recharge -= remaining;
            }
        }

        let wallet_row = sqlx::query(
            r#"
UPDATE wallets
SET
  balance = $2,
  gift_balance = $3,
  total_adjusted = total_adjusted + $4,
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
        .bind(wallet_id)
        .bind(after_recharge)
        .bind(after_gift)
        .bind(amount_usd)
        .fetch_one(&mut *tx)
        .await
        .map_err(|err| GatewayError::Internal(err.to_string()))?;
        let wallet = admin_wallet_snapshot_from_row(&wallet_row)?;

        let transaction_id = uuid::Uuid::new_v4().to_string();
        let created_at = chrono::Utc::now().timestamp().max(0) as u64;
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
  'adjust',
  'adjust_admin',
  $3,
  $4,
  $5,
  $6,
  $7,
  $8,
  $9,
  'admin_action',
  $10,
  $11,
  $12,
  NOW()
)
            "#,
        )
        .bind(&transaction_id)
        .bind(wallet_id)
        .bind(amount_usd)
        .bind(before_total)
        .bind(after_recharge + after_gift)
        .bind(before_recharge)
        .bind(after_recharge)
        .bind(before_gift)
        .bind(after_gift)
        .bind(wallet_id)
        .bind(operator_id)
        .bind(
            description
                .filter(|value| !value.trim().is_empty())
                .unwrap_or("管理员调账"),
        )
        .execute(&mut *tx)
        .await
        .map_err(|err| GatewayError::Internal(err.to_string()))?;
        tx.commit()
            .await
            .map_err(|err| GatewayError::Internal(err.to_string()))?;

        Ok(Some((
            wallet,
            AdminWalletTransactionRecord {
                id: transaction_id,
                wallet_id: wallet_id.to_string(),
                category: "adjust".to_string(),
                reason_code: "adjust_admin".to_string(),
                amount: amount_usd,
                balance_before: before_total,
                balance_after: after_recharge + after_gift,
                recharge_balance_before: before_recharge,
                recharge_balance_after: after_recharge,
                gift_balance_before: before_gift,
                gift_balance_after: after_gift,
                link_type: Some("admin_action".to_string()),
                link_id: Some(wallet_id.to_string()),
                operator_id: operator_id.map(ToOwned::to_owned),
                description: Some(
                    description
                        .filter(|value| !value.trim().is_empty())
                        .unwrap_or("管理员调账")
                        .to_string(),
                ),
                created_at_unix_secs: created_at,
            },
        )))
    }

    pub(crate) async fn admin_create_manual_wallet_recharge(
        &self,
        wallet_id: &str,
        amount_usd: f64,
        payment_method: &str,
        operator_id: Option<&str>,
        description: Option<&str>,
    ) -> Result<
        Option<(
            aether_data::repository::wallet::StoredWalletSnapshot,
            AdminWalletPaymentOrderRecord,
        )>,
        GatewayError,
    > {
        #[cfg(test)]
        if let Some(store) = self.auth_wallet_store.as_ref() {
            let mut guard = store.lock().expect("auth wallet store should lock");
            let Some(wallet) = guard.get_mut(wallet_id) else {
                return Ok(None);
            };
            wallet.balance += amount_usd;
            wallet.total_recharged += amount_usd;
            wallet.updated_at_unix_secs = chrono::Utc::now().timestamp().max(0) as u64;
            let now = chrono::Utc::now();
            let created_at = now.timestamp().max(0) as u64;
            let order = AdminWalletPaymentOrderRecord {
                id: uuid::Uuid::new_v4().to_string(),
                order_no: admin_wallet_build_order_no(now),
                wallet_id: wallet.id.clone(),
                user_id: wallet.user_id.clone(),
                amount_usd,
                pay_amount: None,
                pay_currency: None,
                exchange_rate: None,
                refunded_amount_usd: 0.0,
                refundable_amount_usd: amount_usd,
                payment_method: payment_method.to_string(),
                gateway_order_id: None,
                status: "credited".to_string(),
                gateway_response: Some(serde_json::json!({
                    "source": "manual",
                    "operator_id": operator_id,
                    "description": description,
                })),
                created_at_unix_secs: created_at,
                paid_at_unix_secs: Some(created_at),
                credited_at_unix_secs: Some(created_at),
                expires_at_unix_secs: None,
            };
            return Ok(Some((wallet.clone(), order)));
        }

        let Some(pool) = self.postgres_pool() else {
            return Ok(None);
        };
        let mut tx = pool
            .begin()
            .await
            .map_err(|err| GatewayError::Internal(err.to_string()))?;
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
  CAST(total_adjusted AS DOUBLE PRECISION) AS total_adjusted
FROM wallets
WHERE id = $1
FOR UPDATE
            "#,
        )
        .bind(wallet_id)
        .fetch_optional(&mut *tx)
        .await
        .map_err(|err| GatewayError::Internal(err.to_string()))?
        else {
            let _ = tx.rollback().await;
            return Ok(None);
        };

        let before_recharge = wallet_row
            .try_get::<f64, _>("balance")
            .map_err(|err| GatewayError::Internal(err.to_string()))?;
        let before_gift = wallet_row
            .try_get::<f64, _>("gift_balance")
            .map_err(|err| GatewayError::Internal(err.to_string()))?;
        let user_id = wallet_row
            .try_get::<Option<String>, _>("user_id")
            .map_err(|err| GatewayError::Internal(err.to_string()))?;
        let now = chrono::Utc::now();
        let created_at = now.timestamp().max(0) as u64;
        let order_id = uuid::Uuid::new_v4().to_string();
        let order_no = admin_wallet_build_order_no(now);
        let gateway_response = serde_json::json!({
            "source": "manual",
            "operator_id": operator_id,
            "description": description,
        });

        sqlx::query(
            r#"
INSERT INTO payment_orders (
  id,
  order_no,
  wallet_id,
  user_id,
  amount_usd,
  refunded_amount_usd,
  refundable_amount_usd,
  payment_method,
  status,
  gateway_response,
  created_at,
  paid_at,
  credited_at
)
VALUES (
  $1,
  $2,
  $3,
  $4,
  $5,
  0,
  $5,
  $6,
  'credited',
  $7,
  NOW(),
  NOW(),
  NOW()
)
            "#,
        )
        .bind(&order_id)
        .bind(&order_no)
        .bind(wallet_id)
        .bind(user_id.as_deref())
        .bind(amount_usd)
        .bind(payment_method)
        .bind(&gateway_response)
        .execute(&mut *tx)
        .await
        .map_err(|err| GatewayError::Internal(err.to_string()))?;

        let after_recharge = before_recharge + amount_usd;
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
        .bind(wallet_id)
        .bind(after_recharge)
        .bind(amount_usd)
        .fetch_one(&mut *tx)
        .await
        .map_err(|err| GatewayError::Internal(err.to_string()))?;
        let wallet = admin_wallet_snapshot_from_row(&wallet_row)?;

        let reason_code = if matches!(payment_method, "card_code" | "gift_code" | "card_recharge") {
            "topup_card_code"
        } else {
            "topup_admin_manual"
        };
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
  $3,
  $4,
  $5,
  $6,
  $7,
  $8,
  $9,
  $9,
  'payment_order',
  $10,
  $11,
  $12,
  NOW()
)
            "#,
        )
        .bind(uuid::Uuid::new_v4().to_string())
        .bind(wallet_id)
        .bind(reason_code)
        .bind(amount_usd)
        .bind(before_recharge + before_gift)
        .bind(after_recharge + before_gift)
        .bind(before_recharge)
        .bind(after_recharge)
        .bind(before_gift)
        .bind(&order_id)
        .bind(operator_id)
        .bind(
            description
                .filter(|value| !value.trim().is_empty())
                .unwrap_or("管理员充值"),
        )
        .execute(&mut *tx)
        .await
        .map_err(|err| GatewayError::Internal(err.to_string()))?;

        tx.commit()
            .await
            .map_err(|err| GatewayError::Internal(err.to_string()))?;

        Ok(Some((
            wallet,
            AdminWalletPaymentOrderRecord {
                id: order_id,
                order_no,
                wallet_id: wallet_id.to_string(),
                user_id,
                amount_usd,
                pay_amount: None,
                pay_currency: None,
                exchange_rate: None,
                refunded_amount_usd: 0.0,
                refundable_amount_usd: amount_usd,
                payment_method: payment_method.to_string(),
                gateway_order_id: None,
                status: "credited".to_string(),
                gateway_response: Some(gateway_response),
                created_at_unix_secs: created_at,
                paid_at_unix_secs: Some(created_at),
                credited_at_unix_secs: Some(created_at),
                expires_at_unix_secs: None,
            },
        )))
    }
}
