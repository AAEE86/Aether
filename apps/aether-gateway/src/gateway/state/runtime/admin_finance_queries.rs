use sqlx::Row;

use crate::gateway::state::AdminPaymentCallbackRecord;
use crate::gateway::{
    AdminWalletMutationOutcome, AdminWalletPaymentOrderRecord, AdminWalletRefundRecord,
    AdminWalletTransactionRecord, AppState, GatewayError,
};

use super::admin_wallet_payment_order_from_row;

impl AppState {
    pub(crate) async fn list_admin_payment_orders(
        &self,
        status: Option<&str>,
        payment_method: Option<&str>,
        limit: usize,
        offset: usize,
    ) -> Result<Option<(Vec<AdminWalletPaymentOrderRecord>, u64)>, GatewayError> {
        #[cfg(test)]
        if let Some(store) = self.admin_wallet_payment_order_store.as_ref() {
            let now_unix_secs = chrono::Utc::now().timestamp().max(0) as u64;
            let mut items = store
                .lock()
                .expect("admin wallet payment order store should lock")
                .values()
                .filter(|order| {
                    payment_method.is_none_or(|expected| order.payment_method == expected)
                        && status.is_none_or(|expected| {
                            let effective_status = if order.status == "pending"
                                && order
                                    .expires_at_unix_secs
                                    .is_some_and(|value| value < now_unix_secs)
                            {
                                "expired"
                            } else {
                                order.status.as_str()
                            };
                            effective_status == expected
                        })
                })
                .cloned()
                .collect::<Vec<_>>();
            items.sort_by(|left, right| {
                right
                    .created_at_unix_secs
                    .cmp(&left.created_at_unix_secs)
                    .then_with(|| right.id.cmp(&left.id))
            });
            let total = items.len() as u64;
            let items = items
                .into_iter()
                .skip(offset)
                .take(limit)
                .collect::<Vec<_>>();
            return Ok(Some((items, total)));
        }

        let Some(pool) = self.postgres_pool() else {
            return Ok(None);
        };
        let count_row = sqlx::query(
            r#"
SELECT COUNT(*) AS total
FROM payment_orders
WHERE ($1::TEXT IS NULL OR payment_method = $1)
  AND (
    $2::TEXT IS NULL
    OR (
      CASE
        WHEN status = 'pending' AND expires_at IS NOT NULL AND expires_at < NOW() THEN 'expired'
        ELSE status
      END
    ) = $2
  )
            "#,
        )
        .bind(payment_method)
        .bind(status)
        .fetch_one(&pool)
        .await
        .map_err(|err| GatewayError::Internal(err.to_string()))?;
        let total = count_row
            .try_get::<i64, _>("total")
            .map_err(|err| GatewayError::Internal(err.to_string()))?
            .max(0) as u64;
        let rows = sqlx::query(
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
WHERE ($1::TEXT IS NULL OR payment_method = $1)
  AND (
    $2::TEXT IS NULL
    OR (
      CASE
        WHEN status = 'pending' AND expires_at IS NOT NULL AND expires_at < NOW() THEN 'expired'
        ELSE status
      END
    ) = $2
  )
ORDER BY created_at DESC
OFFSET $3
LIMIT $4
            "#,
        )
        .bind(payment_method)
        .bind(status)
        .bind(i64::try_from(offset).map_err(|err| GatewayError::Internal(err.to_string()))?)
        .bind(i64::try_from(limit).map_err(|err| GatewayError::Internal(err.to_string()))?)
        .fetch_all(&pool)
        .await
        .map_err(|err| GatewayError::Internal(err.to_string()))?;
        let items = rows
            .iter()
            .map(admin_wallet_payment_order_from_row)
            .collect::<Result<Vec<_>, GatewayError>>()?;
        Ok(Some((items, total)))
    }

    pub(crate) async fn list_admin_payment_callbacks(
        &self,
        payment_method: Option<&str>,
        limit: usize,
        offset: usize,
    ) -> Result<Option<(Vec<AdminPaymentCallbackRecord>, u64)>, GatewayError> {
        #[cfg(not(test))]
        let _ = (payment_method, limit, offset);

        #[cfg(test)]
        if let Some(store) = self.admin_payment_callback_store.as_ref() {
            let mut items = store
                .lock()
                .expect("admin payment callback store should lock")
                .values()
                .filter(|callback| {
                    payment_method.is_none_or(|expected| callback.payment_method == expected)
                })
                .cloned()
                .collect::<Vec<_>>();
            items.sort_by(|left, right| {
                right
                    .created_at_unix_secs
                    .cmp(&left.created_at_unix_secs)
                    .then_with(|| right.id.cmp(&left.id))
            });
            let total = items.len() as u64;
            let items = items
                .into_iter()
                .skip(offset)
                .take(limit)
                .collect::<Vec<_>>();
            return Ok(Some((items, total)));
        }

        Ok(None)
    }

    pub(crate) async fn list_admin_wallet_transactions(
        &self,
        wallet_id: &str,
        limit: usize,
        offset: usize,
    ) -> Result<Option<(Vec<AdminWalletTransactionRecord>, u64)>, GatewayError> {
        #[cfg(not(test))]
        let _ = (wallet_id, limit, offset);

        #[cfg(test)]
        if let Some(store) = self.admin_wallet_transaction_store.as_ref() {
            let mut items = store
                .lock()
                .expect("admin wallet transaction store should lock")
                .values()
                .filter(|transaction| transaction.wallet_id == wallet_id)
                .cloned()
                .collect::<Vec<_>>();
            items.sort_by(|left, right| {
                right
                    .created_at_unix_secs
                    .cmp(&left.created_at_unix_secs)
                    .then_with(|| right.id.cmp(&left.id))
            });
            let total = items.len() as u64;
            let items = items
                .into_iter()
                .skip(offset)
                .take(limit)
                .collect::<Vec<_>>();
            return Ok(Some((items, total)));
        }

        Ok(None)
    }

    pub(crate) async fn list_admin_wallet_refunds(
        &self,
        wallet_id: &str,
        limit: usize,
        offset: usize,
    ) -> Result<Option<(Vec<AdminWalletRefundRecord>, u64)>, GatewayError> {
        #[cfg(not(test))]
        let _ = (wallet_id, limit, offset);

        #[cfg(test)]
        if let Some(store) = self.admin_wallet_refund_store.as_ref() {
            let mut items = store
                .lock()
                .expect("admin wallet refund store should lock")
                .values()
                .filter(|refund| refund.wallet_id == wallet_id)
                .cloned()
                .collect::<Vec<_>>();
            items.sort_by(|left, right| {
                right
                    .created_at_unix_secs
                    .cmp(&left.created_at_unix_secs)
                    .then_with(|| right.id.cmp(&left.id))
            });
            let total = items.len() as u64;
            let items = items
                .into_iter()
                .skip(offset)
                .take(limit)
                .collect::<Vec<_>>();
            return Ok(Some((items, total)));
        }

        Ok(None)
    }

    pub(crate) async fn list_admin_wallet_refund_requests(
        &self,
        status: Option<&str>,
        limit: usize,
        offset: usize,
    ) -> Result<Option<(Vec<AdminWalletRefundRecord>, u64)>, GatewayError> {
        #[cfg(not(test))]
        let _ = (status, limit, offset);

        #[cfg(test)]
        if let (Some(wallet_store), Some(refund_store)) = (
            self.auth_wallet_store.as_ref(),
            self.admin_wallet_refund_store.as_ref(),
        ) {
            let wallets = wallet_store
                .lock()
                .expect("auth wallet store should lock")
                .clone();
            let mut items = refund_store
                .lock()
                .expect("admin wallet refund store should lock")
                .values()
                .filter(|refund| status.is_none_or(|expected| refund.status == expected))
                .filter(|refund| {
                    wallets
                        .get(&refund.wallet_id)
                        .is_some_and(|wallet| wallet.user_id.is_some())
                })
                .cloned()
                .collect::<Vec<_>>();
            items.sort_by(|left, right| {
                right
                    .created_at_unix_secs
                    .cmp(&left.created_at_unix_secs)
                    .then_with(|| right.id.cmp(&left.id))
            });
            let total = items.len() as u64;
            let items = items
                .into_iter()
                .skip(offset)
                .take(limit)
                .collect::<Vec<_>>();
            return Ok(Some((items, total)));
        }

        Ok(None)
    }

    pub(crate) async fn read_admin_payment_order(
        &self,
        order_id: &str,
    ) -> Result<AdminWalletMutationOutcome<AdminWalletPaymentOrderRecord>, GatewayError> {
        #[cfg(test)]
        if let Some(store) = self.admin_wallet_payment_order_store.as_ref() {
            return Ok(store
                .lock()
                .expect("admin wallet payment order store should lock")
                .get(order_id)
                .cloned()
                .map(AdminWalletMutationOutcome::Applied)
                .unwrap_or(AdminWalletMutationOutcome::NotFound));
        }

        let Some(pool) = self.postgres_pool() else {
            return Ok(AdminWalletMutationOutcome::Unavailable);
        };
        let row = sqlx::query(
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
LIMIT 1
            "#,
        )
        .bind(order_id)
        .fetch_optional(&pool)
        .await
        .map_err(|err| GatewayError::Internal(err.to_string()))?;
        match row {
            Some(row) => Ok(AdminWalletMutationOutcome::Applied(
                admin_wallet_payment_order_from_row(&row)?,
            )),
            None => Ok(AdminWalletMutationOutcome::NotFound),
        }
    }
}
