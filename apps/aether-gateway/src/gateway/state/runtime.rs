use super::super::error::GatewayError;
use super::super::{scheduler, usage};
use super::{
    AdminBillingCollectorRecord, AdminBillingCollectorWriteInput, AdminBillingPresetApplyResult,
    AdminBillingRuleRecord, AdminBillingRuleWriteInput, AdminPaymentCallbackRecord,
    AdminSecurityBlacklistEntry, AdminWalletMutationOutcome, AdminWalletPaymentOrderRecord,
    AdminWalletRefundRecord, AdminWalletTransactionRecord, AppState, LocalMutationOutcome,
    AUTH_API_KEY_LAST_USED_MAX_ENTRIES, AUTH_API_KEY_LAST_USED_TTL,
};

use sqlx::Row;

#[path = "runtime/admin_finance_queries.rs"]
mod admin_finance_queries;
#[path = "runtime/admin_payment_order_lifecycle.rs"]
mod admin_payment_order_lifecycle;
#[path = "runtime/admin_security.rs"]
mod admin_security;
#[path = "runtime/admin_wallet_balance_mutations.rs"]
mod admin_wallet_balance_mutations;
#[path = "runtime/admin_wallet_refund_lifecycle.rs"]
mod admin_wallet_refund_lifecycle;
#[path = "runtime/announcements.rs"]
mod announcements;
#[path = "runtime/api_key_exports.rs"]
mod api_key_exports;
#[path = "runtime/audit.rs"]
mod audit;
#[path = "runtime/auth_sessions.rs"]
mod auth_sessions;
#[path = "runtime/auth_user_lifecycle.rs"]
mod auth_user_lifecycle;
#[path = "runtime/auth_user_provisioning.rs"]
mod auth_user_provisioning;
#[path = "runtime/billing_admin.rs"]
mod billing_admin;
#[path = "runtime/candidate_queries.rs"]
mod candidate_queries;
#[path = "runtime/gemini_files.rs"]
mod gemini_files;
#[path = "runtime/shadow_results.rs"]
mod shadow_results;
#[path = "runtime/usage_queries.rs"]
mod usage_queries;
#[path = "runtime/user_preferences.rs"]
mod user_preferences;
#[path = "runtime/wallet_billing.rs"]
mod wallet_billing;
#[path = "runtime/wallet_reads.rs"]
mod wallet_reads;
use wallet_billing::*;

#[derive(serde::Serialize)]
pub(crate) struct AdminSecurityBlacklistEntryPayload {
    pub(crate) ip_address: String,
    pub(crate) reason: String,
    pub(crate) ttl_seconds: Option<i64>,
}

impl AppState {
    pub fn has_announcement_data_reader(&self) -> bool {
        self.data.has_announcement_reader()
    }

    pub fn has_announcement_data_writer(&self) -> bool {
        self.data.has_announcement_writer()
    }

    pub fn has_video_task_data_reader(&self) -> bool {
        self.data.has_video_task_reader()
    }

    pub fn has_video_task_data_writer(&self) -> bool {
        self.data.has_video_task_writer()
    }

    pub fn has_request_candidate_data_reader(&self) -> bool {
        self.data.has_request_candidate_reader()
    }

    pub fn has_request_candidate_data_writer(&self) -> bool {
        self.data.has_request_candidate_writer()
    }

    pub fn has_usage_data_reader(&self) -> bool {
        self.data.has_usage_reader()
    }

    pub fn has_user_data_reader(&self) -> bool {
        self.data.has_user_reader()
    }

    pub fn has_usage_data_writer(&self) -> bool {
        self.data.has_usage_writer()
    }

    pub fn has_usage_worker_backend(&self) -> bool {
        self.data.has_usage_worker_runner()
    }

    pub fn has_wallet_data_reader(&self) -> bool {
        self.data.has_wallet_reader()
    }

    pub fn has_wallet_data_writer(&self) -> bool {
        self.data.has_wallet_writer()
    }

    pub fn has_auth_user_write_capability(&self) -> bool {
        #[cfg(test)]
        if self.auth_user_store.is_some() {
            return true;
        }

        self.postgres_pool().is_some()
    }

    pub fn has_auth_wallet_write_capability(&self) -> bool {
        #[cfg(test)]
        if self.auth_wallet_store.is_some() {
            return true;
        }

        self.postgres_pool().is_some()
    }

    pub fn has_provider_quota_data_writer(&self) -> bool {
        self.data.has_provider_quota_writer()
    }

    pub fn has_shadow_result_data_writer(&self) -> bool {
        self.data.has_shadow_result_writer()
    }

    pub fn has_shadow_result_data_reader(&self) -> bool {
        self.data.has_shadow_result_reader()
    }

    pub(crate) async fn count_active_admin_users(&self) -> Result<u64, GatewayError> {
        #[cfg(test)]
        if let Some(store) = self.auth_user_store.as_ref() {
            let total = store
                .lock()
                .expect("auth user store should lock")
                .values()
                .filter(|user| {
                    user.role.eq_ignore_ascii_case("admin") && user.is_active && !user.is_deleted
                })
                .count() as u64;
            return Ok(total);
        }

        self.data
            .count_active_admin_users()
            .await
            .map_err(|err| GatewayError::Internal(err.to_string()))
    }

    pub(crate) async fn count_user_pending_refunds(
        &self,
        user_id: &str,
    ) -> Result<u64, GatewayError> {
        #[cfg(test)]
        {
            let _ = user_id;
            if self.auth_user_store.is_some() {
                return Ok(0);
            }
        }

        self.data
            .count_user_pending_refunds(user_id)
            .await
            .map_err(|err| GatewayError::Internal(err.to_string()))
    }

    pub(crate) async fn count_user_pending_payment_orders(
        &self,
        user_id: &str,
    ) -> Result<u64, GatewayError> {
        #[cfg(test)]
        {
            let _ = user_id;
            if self.auth_user_store.is_some() {
                return Ok(0);
            }
        }

        self.data
            .count_user_pending_payment_orders(user_id)
            .await
            .map_err(|err| GatewayError::Internal(err.to_string()))
    }
}
