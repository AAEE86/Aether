use sqlx::Row;

use super::{
    admin_wallet_refund_from_row, admin_wallet_snapshot_from_row, AdminWalletMutationOutcome,
    AdminWalletRefundRecord, AdminWalletTransactionRecord, AppState, GatewayError,
};

impl AppState {
    pub(crate) async fn admin_process_wallet_refund(
        &self,
        wallet_id: &str,
        refund_id: &str,
        operator_id: Option<&str>,
    ) -> Result<
        AdminWalletMutationOutcome<(
            aether_data::repository::wallet::StoredWalletSnapshot,
            AdminWalletRefundRecord,
            AdminWalletTransactionRecord,
        )>,
        GatewayError,
    > {
        #[cfg(test)]
        if let (Some(wallet_store), Some(refund_store)) = (
            self.auth_wallet_store.as_ref(),
            self.admin_wallet_refund_store.as_ref(),
        ) {
            let Some(wallet) = wallet_store
                .lock()
                .expect("auth wallet store should lock")
                .get(wallet_id)
                .cloned()
            else {
                return Ok(AdminWalletMutationOutcome::NotFound);
            };
            let Some(refund) = refund_store
                .lock()
                .expect("admin wallet refund store should lock")
                .get(refund_id)
                .filter(|refund| refund.wallet_id == wallet_id)
                .cloned()
            else {
                return Ok(AdminWalletMutationOutcome::NotFound);
            };
            if !matches!(refund.status.as_str(), "approved" | "pending_approval") {
                return Ok(AdminWalletMutationOutcome::Invalid(
                    "refund status is not approvable".to_string(),
                ));
            }

            let amount_usd = refund.amount_usd;
            let mut updated_wallet = wallet.clone();
            let before_recharge = updated_wallet.balance;
            let before_gift = updated_wallet.gift_balance;
            let before_total = before_recharge + before_gift;
            let after_recharge = before_recharge - amount_usd;
            if after_recharge < 0.0 {
                return Ok(AdminWalletMutationOutcome::Invalid(
                    "refund amount exceeds refundable recharge balance".to_string(),
                ));
            }

            let mut updated_order = None;
            if let Some(payment_order_id) = refund.payment_order_id.clone() {
                let Some(order_store) = self.admin_wallet_payment_order_store.as_ref() else {
                    return Ok(AdminWalletMutationOutcome::Unavailable);
                };
                let Some(order) = order_store
                    .lock()
                    .expect("admin wallet payment order store should lock")
                    .get(&payment_order_id)
                    .cloned()
                else {
                    return Ok(AdminWalletMutationOutcome::Invalid(
                        "payment order not found".to_string(),
                    ));
                };
                if amount_usd > order.refundable_amount_usd {
                    return Ok(AdminWalletMutationOutcome::Invalid(
                        "refund amount exceeds refundable amount".to_string(),
                    ));
                }
                let mut order = order;
                order.refunded_amount_usd += amount_usd;
                order.refundable_amount_usd -= amount_usd;
                updated_order = Some(order);
            }

            let now_unix_secs = chrono::Utc::now().timestamp().max(0) as u64;
            updated_wallet.balance = after_recharge;
            updated_wallet.total_refunded = (updated_wallet.total_refunded + amount_usd).max(0.0);
            updated_wallet.updated_at_unix_secs = now_unix_secs;

            let transaction = AdminWalletTransactionRecord {
                id: uuid::Uuid::new_v4().to_string(),
                wallet_id: updated_wallet.id.clone(),
                category: "refund".to_string(),
                reason_code: "refund_out".to_string(),
                amount: -amount_usd,
                balance_before: before_total,
                balance_after: after_recharge + before_gift,
                recharge_balance_before: before_recharge,
                recharge_balance_after: after_recharge,
                gift_balance_before: before_gift,
                gift_balance_after: before_gift,
                link_type: Some("refund_request".to_string()),
                link_id: Some(refund.id.clone()),
                operator_id: operator_id.map(ToOwned::to_owned),
                description: Some("退款占款".to_string()),
                created_at_unix_secs: now_unix_secs,
            };

            let mut updated_refund = refund.clone();
            updated_refund.status = "processing".to_string();
            updated_refund.approved_by = operator_id.map(ToOwned::to_owned);
            updated_refund.processed_by = operator_id.map(ToOwned::to_owned);
            updated_refund.processed_at_unix_secs = Some(now_unix_secs);
            updated_refund.updated_at_unix_secs = now_unix_secs;

            wallet_store
                .lock()
                .expect("auth wallet store should lock")
                .insert(updated_wallet.id.clone(), updated_wallet.clone());
            refund_store
                .lock()
                .expect("admin wallet refund store should lock")
                .insert(updated_refund.id.clone(), updated_refund.clone());
            if let Some(updated_order) = updated_order {
                self.admin_wallet_payment_order_store
                    .as_ref()
                    .expect("admin wallet payment order store should exist")
                    .lock()
                    .expect("admin wallet payment order store should lock")
                    .insert(updated_order.id.clone(), updated_order);
            }

            return Ok(AdminWalletMutationOutcome::Applied((
                updated_wallet,
                updated_refund,
                transaction,
            )));
        }

        let Some(pool) = self.postgres_pool() else {
            return Ok(AdminWalletMutationOutcome::Unavailable);
        };
        let mut tx = pool
            .begin()
            .await
            .map_err(|err| GatewayError::Internal(err.to_string()))?;

        let Some(refund_row) = sqlx::query(
            r#"
SELECT
  id,
  refund_no,
  wallet_id,
  user_id,
  payment_order_id,
  source_type,
  source_id,
  refund_mode,
  CAST(amount_usd AS DOUBLE PRECISION) AS amount_usd,
  status,
  reason,
  failure_reason,
  gateway_refund_id,
  payout_method,
  payout_reference,
  payout_proof,
  requested_by,
  approved_by,
  processed_by,
  CAST(EXTRACT(EPOCH FROM created_at) AS BIGINT) AS created_at_unix_secs,
  CAST(EXTRACT(EPOCH FROM updated_at) AS BIGINT) AS updated_at_unix_secs,
  CAST(EXTRACT(EPOCH FROM processed_at) AS BIGINT) AS processed_at_unix_secs,
  CAST(EXTRACT(EPOCH FROM completed_at) AS BIGINT) AS completed_at_unix_secs
FROM refund_requests
WHERE id = $1 AND wallet_id = $2
FOR UPDATE
            "#,
        )
        .bind(refund_id)
        .bind(wallet_id)
        .fetch_optional(&mut *tx)
        .await
        .map_err(|err| GatewayError::Internal(err.to_string()))?
        else {
            let _ = tx.rollback().await;
            return Ok(AdminWalletMutationOutcome::NotFound);
        };
        let refund = admin_wallet_refund_from_row(&refund_row)?;
        if !matches!(refund.status.as_str(), "approved" | "pending_approval") {
            let _ = tx.rollback().await;
            return Ok(AdminWalletMutationOutcome::Invalid(
                "refund status is not approvable".to_string(),
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
        .bind(wallet_id)
        .fetch_optional(&mut *tx)
        .await
        .map_err(|err| GatewayError::Internal(err.to_string()))?
        else {
            let _ = tx.rollback().await;
            return Ok(AdminWalletMutationOutcome::Invalid(
                "wallet not found".to_string(),
            ));
        };
        let before_recharge = wallet_row
            .try_get::<f64, _>("balance")
            .map_err(|err| GatewayError::Internal(err.to_string()))?;
        let before_gift = wallet_row
            .try_get::<f64, _>("gift_balance")
            .map_err(|err| GatewayError::Internal(err.to_string()))?;
        let before_total = before_recharge + before_gift;
        let amount_usd = refund.amount_usd;
        let after_recharge = before_recharge - amount_usd;
        if after_recharge < 0.0 {
            let _ = tx.rollback().await;
            return Ok(AdminWalletMutationOutcome::Invalid(
                "refund amount exceeds refundable recharge balance".to_string(),
            ));
        }

        if let Some(payment_order_id) = refund.payment_order_id.as_deref() {
            let Some(order_row) = sqlx::query(
                r#"
SELECT
  id,
  CAST(refunded_amount_usd AS DOUBLE PRECISION) AS refunded_amount_usd,
  CAST(refundable_amount_usd AS DOUBLE PRECISION) AS refundable_amount_usd
FROM payment_orders
WHERE id = $1
FOR UPDATE
                "#,
            )
            .bind(payment_order_id)
            .fetch_optional(&mut *tx)
            .await
            .map_err(|err| GatewayError::Internal(err.to_string()))?
            else {
                let _ = tx.rollback().await;
                return Ok(AdminWalletMutationOutcome::Invalid(
                    "payment order not found".to_string(),
                ));
            };
            let refundable_amount = order_row
                .try_get::<f64, _>("refundable_amount_usd")
                .map_err(|err| GatewayError::Internal(err.to_string()))?;
            if amount_usd > refundable_amount {
                let _ = tx.rollback().await;
                return Ok(AdminWalletMutationOutcome::Invalid(
                    "refund amount exceeds refundable amount".to_string(),
                ));
            }
            sqlx::query(
                r#"
UPDATE payment_orders
SET
  refunded_amount_usd = refunded_amount_usd + $2,
  refundable_amount_usd = refundable_amount_usd - $2
WHERE id = $1
                "#,
            )
            .bind(payment_order_id)
            .bind(amount_usd)
            .execute(&mut *tx)
            .await
            .map_err(|err| GatewayError::Internal(err.to_string()))?;
        }

        let wallet_row = sqlx::query(
            r#"
UPDATE wallets
SET
  balance = $2,
  total_refunded = total_refunded + $3,
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

        let transaction_id = uuid::Uuid::new_v4().to_string();
        let created_at_unix_secs = chrono::Utc::now().timestamp().max(0) as u64;
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
  'refund',
  'refund_out',
  $3,
  $4,
  $5,
  $6,
  $7,
  $8,
  $9,
  'refund_request',
  $10,
  $11,
  '退款占款',
  NOW()
)
            "#,
        )
        .bind(&transaction_id)
        .bind(wallet_id)
        .bind(-amount_usd)
        .bind(before_total)
        .bind(after_recharge + before_gift)
        .bind(before_recharge)
        .bind(after_recharge)
        .bind(before_gift)
        .bind(before_gift)
        .bind(refund_id)
        .bind(operator_id)
        .execute(&mut *tx)
        .await
        .map_err(|err| GatewayError::Internal(err.to_string()))?;

        let refund_row = sqlx::query(
            r#"
UPDATE refund_requests
SET
  status = 'processing',
  approved_by = $3,
  processed_by = $3,
  processed_at = NOW(),
  updated_at = NOW()
WHERE id = $1 AND wallet_id = $2
RETURNING
  id,
  refund_no,
  wallet_id,
  user_id,
  payment_order_id,
  source_type,
  source_id,
  refund_mode,
  CAST(amount_usd AS DOUBLE PRECISION) AS amount_usd,
  status,
  reason,
  failure_reason,
  gateway_refund_id,
  payout_method,
  payout_reference,
  payout_proof,
  requested_by,
  approved_by,
  processed_by,
  CAST(EXTRACT(EPOCH FROM created_at) AS BIGINT) AS created_at_unix_secs,
  CAST(EXTRACT(EPOCH FROM updated_at) AS BIGINT) AS updated_at_unix_secs,
  CAST(EXTRACT(EPOCH FROM processed_at) AS BIGINT) AS processed_at_unix_secs,
  CAST(EXTRACT(EPOCH FROM completed_at) AS BIGINT) AS completed_at_unix_secs
            "#,
        )
        .bind(refund_id)
        .bind(wallet_id)
        .bind(operator_id)
        .fetch_one(&mut *tx)
        .await
        .map_err(|err| GatewayError::Internal(err.to_string()))?;
        let refund = admin_wallet_refund_from_row(&refund_row)?;

        tx.commit()
            .await
            .map_err(|err| GatewayError::Internal(err.to_string()))?;

        Ok(AdminWalletMutationOutcome::Applied((
            wallet,
            refund,
            AdminWalletTransactionRecord {
                id: transaction_id,
                wallet_id: wallet_id.to_string(),
                category: "refund".to_string(),
                reason_code: "refund_out".to_string(),
                amount: -amount_usd,
                balance_before: before_total,
                balance_after: after_recharge + before_gift,
                recharge_balance_before: before_recharge,
                recharge_balance_after: after_recharge,
                gift_balance_before: before_gift,
                gift_balance_after: before_gift,
                link_type: Some("refund_request".to_string()),
                link_id: Some(refund_id.to_string()),
                operator_id: operator_id.map(ToOwned::to_owned),
                description: Some("退款占款".to_string()),
                created_at_unix_secs,
            },
        )))
    }

    pub(crate) async fn admin_complete_wallet_refund(
        &self,
        wallet_id: &str,
        refund_id: &str,
        gateway_refund_id: Option<&str>,
        payout_reference: Option<&str>,
        payout_proof: Option<serde_json::Value>,
    ) -> Result<AdminWalletMutationOutcome<AdminWalletRefundRecord>, GatewayError> {
        #[cfg(test)]
        if let Some(refund_store) = self.admin_wallet_refund_store.as_ref() {
            let Some(refund) = refund_store
                .lock()
                .expect("admin wallet refund store should lock")
                .get(refund_id)
                .filter(|refund| refund.wallet_id == wallet_id)
                .cloned()
            else {
                return Ok(AdminWalletMutationOutcome::NotFound);
            };
            if refund.status != "processing" {
                return Ok(AdminWalletMutationOutcome::Invalid(
                    "refund status must be processing before completion".to_string(),
                ));
            }
            let now_unix_secs = chrono::Utc::now().timestamp().max(0) as u64;
            let mut updated_refund = refund;
            updated_refund.status = "succeeded".to_string();
            updated_refund.gateway_refund_id = gateway_refund_id.map(ToOwned::to_owned);
            updated_refund.payout_reference = payout_reference.map(ToOwned::to_owned);
            updated_refund.payout_proof = payout_proof;
            updated_refund.completed_at_unix_secs = Some(now_unix_secs);
            updated_refund.updated_at_unix_secs = now_unix_secs;
            refund_store
                .lock()
                .expect("admin wallet refund store should lock")
                .insert(updated_refund.id.clone(), updated_refund.clone());
            return Ok(AdminWalletMutationOutcome::Applied(updated_refund));
        }

        let Some(pool) = self.postgres_pool() else {
            return Ok(AdminWalletMutationOutcome::Unavailable);
        };
        let mut tx = pool
            .begin()
            .await
            .map_err(|err| GatewayError::Internal(err.to_string()))?;
        let Some(current_refund) = sqlx::query(
            r#"
SELECT status
FROM refund_requests
WHERE id = $1 AND wallet_id = $2
FOR UPDATE
            "#,
        )
        .bind(refund_id)
        .bind(wallet_id)
        .fetch_optional(&mut *tx)
        .await
        .map_err(|err| GatewayError::Internal(err.to_string()))?
        else {
            let _ = tx.rollback().await;
            return Ok(AdminWalletMutationOutcome::NotFound);
        };
        let status = current_refund
            .try_get::<String, _>("status")
            .map_err(|err| GatewayError::Internal(err.to_string()))?;
        if status != "processing" {
            let _ = tx.rollback().await;
            return Ok(AdminWalletMutationOutcome::Invalid(
                "refund status must be processing before completion".to_string(),
            ));
        }

        let refund_row = sqlx::query(
            r#"
UPDATE refund_requests
SET
  status = 'succeeded',
  gateway_refund_id = $3,
  payout_reference = $4,
  payout_proof = $5,
  completed_at = NOW(),
  updated_at = NOW()
WHERE id = $1 AND wallet_id = $2
RETURNING
  id,
  refund_no,
  wallet_id,
  user_id,
  payment_order_id,
  source_type,
  source_id,
  refund_mode,
  CAST(amount_usd AS DOUBLE PRECISION) AS amount_usd,
  status,
  reason,
  failure_reason,
  gateway_refund_id,
  payout_method,
  payout_reference,
  payout_proof,
  requested_by,
  approved_by,
  processed_by,
  CAST(EXTRACT(EPOCH FROM created_at) AS BIGINT) AS created_at_unix_secs,
  CAST(EXTRACT(EPOCH FROM updated_at) AS BIGINT) AS updated_at_unix_secs,
  CAST(EXTRACT(EPOCH FROM processed_at) AS BIGINT) AS processed_at_unix_secs,
  CAST(EXTRACT(EPOCH FROM completed_at) AS BIGINT) AS completed_at_unix_secs
            "#,
        )
        .bind(refund_id)
        .bind(wallet_id)
        .bind(gateway_refund_id)
        .bind(payout_reference)
        .bind(payout_proof)
        .fetch_one(&mut *tx)
        .await
        .map_err(|err| GatewayError::Internal(err.to_string()))?;
        let refund = admin_wallet_refund_from_row(&refund_row)?;
        tx.commit()
            .await
            .map_err(|err| GatewayError::Internal(err.to_string()))?;
        Ok(AdminWalletMutationOutcome::Applied(refund))
    }

    pub(crate) async fn admin_fail_wallet_refund(
        &self,
        wallet_id: &str,
        refund_id: &str,
        reason: &str,
        operator_id: Option<&str>,
    ) -> Result<
        AdminWalletMutationOutcome<(
            aether_data::repository::wallet::StoredWalletSnapshot,
            AdminWalletRefundRecord,
            Option<AdminWalletTransactionRecord>,
        )>,
        GatewayError,
    > {
        #[cfg(test)]
        if let (Some(wallet_store), Some(refund_store)) = (
            self.auth_wallet_store.as_ref(),
            self.admin_wallet_refund_store.as_ref(),
        ) {
            let Some(wallet) = wallet_store
                .lock()
                .expect("auth wallet store should lock")
                .get(wallet_id)
                .cloned()
            else {
                return Ok(AdminWalletMutationOutcome::NotFound);
            };
            let Some(refund) = refund_store
                .lock()
                .expect("admin wallet refund store should lock")
                .get(refund_id)
                .filter(|refund| refund.wallet_id == wallet_id)
                .cloned()
            else {
                return Ok(AdminWalletMutationOutcome::NotFound);
            };

            let now_unix_secs = chrono::Utc::now().timestamp().max(0) as u64;
            if matches!(refund.status.as_str(), "pending_approval" | "approved") {
                let mut updated_refund = refund;
                updated_refund.status = "failed".to_string();
                updated_refund.failure_reason = Some(reason.to_string());
                updated_refund.updated_at_unix_secs = now_unix_secs;
                refund_store
                    .lock()
                    .expect("admin wallet refund store should lock")
                    .insert(updated_refund.id.clone(), updated_refund.clone());
                return Ok(AdminWalletMutationOutcome::Applied((
                    wallet,
                    updated_refund,
                    None,
                )));
            }
            if refund.status != "processing" {
                return Ok(AdminWalletMutationOutcome::Invalid(format!(
                    "cannot fail refund in status: {}",
                    refund.status
                )));
            }

            let amount_usd = refund.amount_usd;
            let before_recharge = wallet.balance;
            let before_gift = wallet.gift_balance;
            let before_total = before_recharge + before_gift;
            let after_recharge = before_recharge + amount_usd;

            let mut updated_wallet = wallet.clone();
            updated_wallet.balance = after_recharge;
            updated_wallet.total_refunded = (updated_wallet.total_refunded - amount_usd).max(0.0);
            updated_wallet.updated_at_unix_secs = now_unix_secs;

            let transaction = AdminWalletTransactionRecord {
                id: uuid::Uuid::new_v4().to_string(),
                wallet_id: updated_wallet.id.clone(),
                category: "refund".to_string(),
                reason_code: "refund_revert".to_string(),
                amount: amount_usd,
                balance_before: before_total,
                balance_after: after_recharge + before_gift,
                recharge_balance_before: before_recharge,
                recharge_balance_after: after_recharge,
                gift_balance_before: before_gift,
                gift_balance_after: before_gift,
                link_type: Some("refund_request".to_string()),
                link_id: Some(refund.id.clone()),
                operator_id: operator_id.map(ToOwned::to_owned),
                description: Some("退款失败回补".to_string()),
                created_at_unix_secs: now_unix_secs,
            };

            if let Some(payment_order_id) = refund.payment_order_id.clone() {
                let Some(order_store) = self.admin_wallet_payment_order_store.as_ref() else {
                    return Ok(AdminWalletMutationOutcome::Unavailable);
                };
                let maybe_order = order_store
                    .lock()
                    .expect("admin wallet payment order store should lock")
                    .get(&payment_order_id)
                    .cloned();
                if let Some(mut order) = maybe_order {
                    order.refunded_amount_usd -= amount_usd;
                    order.refundable_amount_usd += amount_usd;
                    order_store
                        .lock()
                        .expect("admin wallet payment order store should lock")
                        .insert(order.id.clone(), order);
                }
            }

            let mut updated_refund = refund;
            updated_refund.status = "failed".to_string();
            updated_refund.failure_reason = Some(reason.to_string());
            updated_refund.updated_at_unix_secs = now_unix_secs;

            wallet_store
                .lock()
                .expect("auth wallet store should lock")
                .insert(updated_wallet.id.clone(), updated_wallet.clone());
            refund_store
                .lock()
                .expect("admin wallet refund store should lock")
                .insert(updated_refund.id.clone(), updated_refund.clone());

            return Ok(AdminWalletMutationOutcome::Applied((
                updated_wallet,
                updated_refund,
                Some(transaction),
            )));
        }

        let Some(pool) = self.postgres_pool() else {
            return Ok(AdminWalletMutationOutcome::Unavailable);
        };
        let mut tx = pool
            .begin()
            .await
            .map_err(|err| GatewayError::Internal(err.to_string()))?;
        let Some(refund_row) = sqlx::query(
            r#"
SELECT
  id,
  refund_no,
  wallet_id,
  user_id,
  payment_order_id,
  source_type,
  source_id,
  refund_mode,
  CAST(amount_usd AS DOUBLE PRECISION) AS amount_usd,
  status,
  reason,
  failure_reason,
  gateway_refund_id,
  payout_method,
  payout_reference,
  payout_proof,
  requested_by,
  approved_by,
  processed_by,
  CAST(EXTRACT(EPOCH FROM created_at) AS BIGINT) AS created_at_unix_secs,
  CAST(EXTRACT(EPOCH FROM updated_at) AS BIGINT) AS updated_at_unix_secs,
  CAST(EXTRACT(EPOCH FROM processed_at) AS BIGINT) AS processed_at_unix_secs,
  CAST(EXTRACT(EPOCH FROM completed_at) AS BIGINT) AS completed_at_unix_secs
FROM refund_requests
WHERE id = $1 AND wallet_id = $2
FOR UPDATE
            "#,
        )
        .bind(refund_id)
        .bind(wallet_id)
        .fetch_optional(&mut *tx)
        .await
        .map_err(|err| GatewayError::Internal(err.to_string()))?
        else {
            let _ = tx.rollback().await;
            return Ok(AdminWalletMutationOutcome::NotFound);
        };
        let refund = admin_wallet_refund_from_row(&refund_row)?;

        if matches!(refund.status.as_str(), "pending_approval" | "approved") {
            let refund_row = sqlx::query(
                r#"
UPDATE refund_requests
SET
  status = 'failed',
  failure_reason = $3,
  updated_at = NOW()
WHERE id = $1 AND wallet_id = $2
RETURNING
  id,
  refund_no,
  wallet_id,
  user_id,
  payment_order_id,
  source_type,
  source_id,
  refund_mode,
  CAST(amount_usd AS DOUBLE PRECISION) AS amount_usd,
  status,
  reason,
  failure_reason,
  gateway_refund_id,
  payout_method,
  payout_reference,
  payout_proof,
  requested_by,
  approved_by,
  processed_by,
  CAST(EXTRACT(EPOCH FROM created_at) AS BIGINT) AS created_at_unix_secs,
  CAST(EXTRACT(EPOCH FROM updated_at) AS BIGINT) AS updated_at_unix_secs,
  CAST(EXTRACT(EPOCH FROM processed_at) AS BIGINT) AS processed_at_unix_secs,
  CAST(EXTRACT(EPOCH FROM completed_at) AS BIGINT) AS completed_at_unix_secs
                "#,
            )
            .bind(refund_id)
            .bind(wallet_id)
            .bind(reason)
            .fetch_one(&mut *tx)
            .await
            .map_err(|err| GatewayError::Internal(err.to_string()))?;
            let refund = admin_wallet_refund_from_row(&refund_row)?;
            let wallet_row = sqlx::query(
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
                "#,
            )
            .bind(wallet_id)
            .fetch_one(&mut *tx)
            .await
            .map_err(|err| GatewayError::Internal(err.to_string()))?;
            let wallet = admin_wallet_snapshot_from_row(&wallet_row)?;
            tx.commit()
                .await
                .map_err(|err| GatewayError::Internal(err.to_string()))?;
            return Ok(AdminWalletMutationOutcome::Applied((wallet, refund, None)));
        }
        if refund.status != "processing" {
            let _ = tx.rollback().await;
            return Ok(AdminWalletMutationOutcome::Invalid(format!(
                "cannot fail refund in status: {}",
                refund.status
            )));
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
        .bind(wallet_id)
        .fetch_optional(&mut *tx)
        .await
        .map_err(|err| GatewayError::Internal(err.to_string()))?
        else {
            let _ = tx.rollback().await;
            return Ok(AdminWalletMutationOutcome::Invalid(
                "wallet not found".to_string(),
            ));
        };

        let amount_usd = refund.amount_usd;
        let before_recharge = wallet_row
            .try_get::<f64, _>("balance")
            .map_err(|err| GatewayError::Internal(err.to_string()))?;
        let before_gift = wallet_row
            .try_get::<f64, _>("gift_balance")
            .map_err(|err| GatewayError::Internal(err.to_string()))?;
        let before_total = before_recharge + before_gift;
        let after_recharge = before_recharge + amount_usd;

        let wallet_row = sqlx::query(
            r#"
UPDATE wallets
SET
  balance = $2,
  total_refunded = GREATEST(total_refunded - $3, 0),
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

        let transaction_id = uuid::Uuid::new_v4().to_string();
        let created_at_unix_secs = chrono::Utc::now().timestamp().max(0) as u64;
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
  'refund',
  'refund_revert',
  $3,
  $4,
  $5,
  $6,
  $7,
  $8,
  $9,
  'refund_request',
  $10,
  $11,
  '退款失败回补',
  NOW()
)
            "#,
        )
        .bind(&transaction_id)
        .bind(wallet_id)
        .bind(amount_usd)
        .bind(before_total)
        .bind(after_recharge + before_gift)
        .bind(before_recharge)
        .bind(after_recharge)
        .bind(before_gift)
        .bind(before_gift)
        .bind(refund_id)
        .bind(operator_id)
        .execute(&mut *tx)
        .await
        .map_err(|err| GatewayError::Internal(err.to_string()))?;

        if let Some(payment_order_id) = refund.payment_order_id.as_deref() {
            if sqlx::query(
                r#"
UPDATE payment_orders
SET
  refunded_amount_usd = refunded_amount_usd - $2,
  refundable_amount_usd = refundable_amount_usd + $2
WHERE id = $1
                "#,
            )
            .bind(payment_order_id)
            .bind(amount_usd)
            .execute(&mut *tx)
            .await
            .map_err(|err| GatewayError::Internal(err.to_string()))?
            .rows_affected()
                == 0
            {
                // Python 语义下缺失 payment_order 时直接跳过，不报错。
            }
        }

        let refund_row = sqlx::query(
            r#"
UPDATE refund_requests
SET
  status = 'failed',
  failure_reason = $3,
  updated_at = NOW()
WHERE id = $1 AND wallet_id = $2
RETURNING
  id,
  refund_no,
  wallet_id,
  user_id,
  payment_order_id,
  source_type,
  source_id,
  refund_mode,
  CAST(amount_usd AS DOUBLE PRECISION) AS amount_usd,
  status,
  reason,
  failure_reason,
  gateway_refund_id,
  payout_method,
  payout_reference,
  payout_proof,
  requested_by,
  approved_by,
  processed_by,
  CAST(EXTRACT(EPOCH FROM created_at) AS BIGINT) AS created_at_unix_secs,
  CAST(EXTRACT(EPOCH FROM updated_at) AS BIGINT) AS updated_at_unix_secs,
  CAST(EXTRACT(EPOCH FROM processed_at) AS BIGINT) AS processed_at_unix_secs,
  CAST(EXTRACT(EPOCH FROM completed_at) AS BIGINT) AS completed_at_unix_secs
            "#,
        )
        .bind(refund_id)
        .bind(wallet_id)
        .bind(reason)
        .fetch_one(&mut *tx)
        .await
        .map_err(|err| GatewayError::Internal(err.to_string()))?;
        let refund = admin_wallet_refund_from_row(&refund_row)?;
        tx.commit()
            .await
            .map_err(|err| GatewayError::Internal(err.to_string()))?;

        Ok(AdminWalletMutationOutcome::Applied((
            wallet,
            refund,
            Some(AdminWalletTransactionRecord {
                id: transaction_id,
                wallet_id: wallet_id.to_string(),
                category: "refund".to_string(),
                reason_code: "refund_revert".to_string(),
                amount: amount_usd,
                balance_before: before_total,
                balance_after: after_recharge + before_gift,
                recharge_balance_before: before_recharge,
                recharge_balance_after: after_recharge,
                gift_balance_before: before_gift,
                gift_balance_after: before_gift,
                link_type: Some("refund_request".to_string()),
                link_id: Some(refund_id.to_string()),
                operator_id: operator_id.map(ToOwned::to_owned),
                description: Some("退款失败回补".to_string()),
                created_at_unix_secs,
            }),
        )))
    }
}
