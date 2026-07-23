//! Mutable state owned by one Responses WebSocket connection.
//!
//! The session loop is intentionally kept separate from these containers.  A
//! connection may survive many `response.create` turns, while the turn
//! lifecycle and upstream binding are replaced independently.

use serde_json::Value;
use std::collections::{BTreeMap, BTreeSet};
use tokio::task::JoinHandle;

use super::adapter::{ResponsesWebSocketDrainDirective, ResponsesWebSocketProtocolAdapter};
use super::binding::UpstreamBindingIdentity;
use super::request::response_create_has_previous_response_id;
use super::turn::ResponsesWebSocketTurn;
use crate::ai_serving::AiExecutionDecision;

const EXHAUSTED_KEY_EXCLUSION_FALLBACK_SECONDS: u64 = 300;

/// All mutable state associated with the physical upstream connection.
pub(super) struct BoundResponsesConnection {
    pub(super) upstream: Option<wreq::ws::WebSocket>,
    pub(super) adapter: &'static dyn ResponsesWebSocketProtocolAdapter,
    pub(super) client_model: String,
    pub(super) provider_model: String,
    pub(super) response_in_flight: bool,
    pub(super) decision_template: AiExecutionDecision,
    pub(super) binding_identity: UpstreamBindingIdentity,
    pub(super) active_turn: Option<ResponsesWebSocketTurn>,
    pub(super) active_response_create: Option<ActiveResponsesWebSocketRequest>,
    pub(super) next_turn_index: u64,
    pub(super) upstream_response_headers: BTreeMap<String, String>,
    pub(super) pending_adapter_drain: Option<ResponsesWebSocketDrainDirective>,
    pub(super) pending_adapter_observation: Option<JoinHandle<()>>,
    pub(super) exhausted_exclusions: ExhaustedResponsesWebSocketExclusions,
    pub(super) pending_turn_finalization: Option<JoinHandle<()>>,
}

/// Connection-local fallback in addition to the distributed account breaker.
/// A key and its provider account are excluded until the upstream's reset
/// deadline (or a short fallback when the terminal payload lacks one), so an
/// unusually long-lived client socket does not keep it unavailable after the
/// quota has recovered.
#[derive(Debug, Default)]
pub(super) struct ExhaustedResponsesWebSocketExclusions {
    expires_at_by_key: BTreeMap<String, u64>,
    expires_at_by_codex_account: BTreeMap<String, u64>,
}

impl ExhaustedResponsesWebSocketExclusions {
    pub(super) fn exclude(
        &mut self,
        key_id: String,
        codex_account_id: Option<String>,
        reset_at_unix_secs: Option<u64>,
        now_unix_secs: u64,
    ) -> u64 {
        self.prune(now_unix_secs);
        let requested_expiry = reset_at_unix_secs
            .filter(|reset_at| *reset_at > now_unix_secs)
            .unwrap_or_else(|| {
                now_unix_secs.saturating_add(EXHAUSTED_KEY_EXCLUSION_FALLBACK_SECONDS)
            });
        let expiry = self
            .expires_at_by_key
            .entry(key_id)
            .and_modify(|existing| *existing = (*existing).max(requested_expiry))
            .or_insert(requested_expiry);
        if let Some(account_id) = codex_account_id {
            self.expires_at_by_codex_account
                .entry(account_id)
                .and_modify(|existing| *existing = (*existing).max(requested_expiry))
                .or_insert(requested_expiry);
        }
        *expiry
    }

    pub(super) fn codex_account_ids(&mut self, now_unix_secs: u64) -> BTreeSet<String> {
        self.prune(now_unix_secs);
        self.expires_at_by_codex_account.keys().cloned().collect()
    }

    pub(super) fn key_ids(&mut self, now_unix_secs: u64) -> BTreeSet<String> {
        self.prune(now_unix_secs);
        self.expires_at_by_key.keys().cloned().collect()
    }

    pub(super) fn len(&mut self, now_unix_secs: u64) -> usize {
        self.prune(now_unix_secs);
        self.expires_at_by_key.len() + self.expires_at_by_codex_account.len()
    }

    fn prune(&mut self, now_unix_secs: u64) {
        self.expires_at_by_key
            .retain(|_, expires_at| *expires_at > now_unix_secs);
        self.expires_at_by_codex_account
            .retain(|_, expires_at| *expires_at > now_unix_secs);
    }
}

#[derive(Debug, Clone)]
pub(super) struct ActiveResponsesWebSocketRequest {
    pub(super) client_event: Value,
    pub(super) turn_index: u64,
    pub(super) logical_turn_id: String,
    pub(super) turn_attempt: u32,
    pub(super) retry_attempted: bool,
    pub(super) retry_unsafe_reason: Option<&'static str>,
}

impl ActiveResponsesWebSocketRequest {
    pub(super) fn new(client_event: Value, turn_index: u64, logical_turn_id: String) -> Self {
        Self {
            client_event,
            turn_index,
            logical_turn_id,
            turn_attempt: 1,
            retry_attempted: false,
            retry_unsafe_reason: None,
        }
    }

    pub(super) fn quota_retry_block_reason(&self) -> Option<&'static str> {
        if self.retry_attempted {
            Some("quota_retry_already_attempted")
        } else if let Some(reason) = self.retry_unsafe_reason {
            Some(reason)
        } else if response_create_has_previous_response_id(&self.client_event) {
            Some("previous_response_id")
        } else {
            None
        }
    }

    pub(super) fn mark_retry_unsafe(&mut self, reason: &'static str) {
        self.retry_unsafe_reason.get_or_insert(reason);
    }
}
