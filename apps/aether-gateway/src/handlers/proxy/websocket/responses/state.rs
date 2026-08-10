//! Mutable state owned by one Responses WebSocket connection.
//!
//! The session loop is intentionally kept separate from these containers.  A
//! connection may survive many `response.create` turns, while the turn
//! lifecycle and upstream binding are replaced independently.

use std::collections::{BTreeMap, BTreeSet};

use aether_scheduler_core::SchedulerMinimalCandidateSelectionCandidate;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Arc;

use serde_json::{Number, Value};
use tokio::task::JoinHandle;

use super::adapter::{
    ResponsesProviderObserver, ResponsesPublicEventState, ResponsesPublicWireCodec,
    ResponsesWebSocketDrainDirective,
};
use super::backend::{NativeResponsesWebSocketBackend, ResponsesBackendSessionHandle};
use super::binding::UpstreamBindingIdentity;
use super::lifecycle::ResponsesSessionTerminationSignal;
use super::redaction::ResponsesWebSocketRedactionRestorer;
use super::turn_state::ResponsesTurnState;
use crate::ai_serving::{AiExecutionDecision, ResponsesWebSocketBodyNormalization};

const EXHAUSTED_KEY_EXCLUSION_FALLBACK_SECONDS: u64 = 300;

/// Evicts the one connection-local continuation entry only when a failed turn
/// actually referenced it. Independent turns and stale/foreign IDs leave the
/// latest successfully delivered chain untouched.
pub(super) fn evict_referenced_public_response_id(
    latest_public_response_id: &mut Option<String>,
    previous_response_id: Option<&str>,
) -> bool {
    if previous_response_id
        .is_some_and(|response_id| Some(response_id) == latest_public_response_id.as_deref())
    {
        *latest_public_response_id = None;
        true
    } else {
        false
    }
}

/// Allocates the public `sequence_number` values for one logical response.
///
/// The handle is shared with the outer session supervisor so a connection
/// deadline racing the relay still emits the next public sequence number.
/// Starting a new client `response.create` resets it; transparent provider
/// retries keep using the same counter because they remain one logical turn.
#[derive(Debug, Clone, Default)]
pub(crate) struct ResponsesPublicEventSequence(Arc<AtomicU64>);

/// Arbitrates the one terminal writer for a public client socket.
///
/// The outer connection supervisor races every inner bootstrap/relay path. A
/// terminal path claims this flag before its first public error or Close
/// frame. If the supervisor loses, it waits for the inner owner instead of
/// cancelling a half-written teardown and emitting a second terminal error.
#[derive(Debug, Clone, Default)]
pub(crate) struct ResponsesPublicTeardownClaim(Arc<AtomicBool>);

impl ResponsesPublicTeardownClaim {
    pub(crate) fn try_claim(&self) -> bool {
        self.0
            .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
            .is_ok()
    }

    #[cfg(test)]
    pub(crate) fn is_claimed(&self) -> bool {
        self.0.load(Ordering::Acquire)
    }
}

/// One uncommitted public event number.
///
/// Reserving only reads the counter. Dropping this value therefore rolls the
/// reservation back automatically, which is important when a socket write is
/// cancelled by the connection supervisor. Under the Responses session's
/// single-writer invariant, committing immediately after a successful write
/// advances the counter without leaving gaps.
#[derive(Debug)]
pub(crate) struct ResponsesPublicEventSequenceReservation {
    sequence: Arc<AtomicU64>,
    sequence_number: u64,
}

impl ResponsesPublicEventSequenceReservation {
    pub(crate) const fn sequence_number(&self) -> u64 {
        self.sequence_number
    }

    pub(crate) fn commit(self) {
        let next = self
            .sequence_number
            .checked_add(1)
            .expect("Responses public event sequence exhausted");
        self.sequence
            .compare_exchange(
                self.sequence_number,
                next,
                Ordering::AcqRel,
                Ordering::Acquire,
            )
            .expect("Responses public event sequence has more than one writer");
    }
}

impl ResponsesPublicEventSequence {
    pub(crate) fn reset(&self) {
        self.0.store(0, Ordering::Release);
    }

    pub(crate) fn reserve(&self) -> ResponsesPublicEventSequenceReservation {
        ResponsesPublicEventSequenceReservation {
            sequence: Arc::clone(&self.0),
            sequence_number: self.0.load(Ordering::Acquire),
        }
    }

    #[cfg(test)]
    pub(crate) fn next(&self) -> u64 {
        let reservation = self.reserve();
        let sequence_number = reservation.sequence_number();
        reservation.commit();
        sequence_number
    }

    pub(crate) fn stamp(
        &self,
        event: &mut Value,
    ) -> Result<ResponsesPublicEventSequenceReservation, ()> {
        let object = event.as_object_mut().ok_or(())?;
        let reservation = self.reserve();
        object.insert(
            "sequence_number".to_string(),
            Value::Number(Number::from(reservation.sequence_number())),
        );
        Ok(reservation)
    }
}

/// All mutable state associated with the physical upstream connection.
pub(super) struct BoundResponsesConnection {
    pub(super) backend_session: ResponsesBackendSessionHandle,
    pub(super) backend: &'static dyn NativeResponsesWebSocketBackend,
    pub(super) public_codec: &'static dyn ResponsesPublicWireCodec,
    pub(super) public_event_state: ResponsesPublicEventState,
    pub(super) provider_observer: &'static dyn ResponsesProviderObserver,
    pub(super) client_model: String,
    pub(super) provider_model: String,
    pub(super) decision_template: AiExecutionDecision,
    /// Concrete candidate identity selected after provider-pool expansion.
    /// Direct continuations revalidate exactly this provider/endpoint/key.
    pub(super) bound_candidate: SchedulerMinimalCandidateSelectionCandidate,
    /// Reproduces this binding's provider-body normalization for continuation
    /// turns, which must not re-enter the planner. Replaced whenever the
    /// binding or its decision is replaced.
    pub(super) body_normalization: ResponsesWebSocketBodyNormalization,
    pub(super) binding_identity: UpstreamBindingIdentity,
    /// 这条连接上「有没有正在进行的 logical turn」的唯一事实来源。
    pub(super) turn_state: ResponsesTurnState,
    pub(super) public_event_sequence: ResponsesPublicEventSequence,
    pub(super) public_teardown: ResponsesPublicTeardownClaim,
    /// Most recent response ID whose chainable terminal was fully flushed to
    /// this public connection. Continuations may only reference this ID;
    /// shared provider accounts are not an ownership boundary for client IDs.
    pub(super) latest_public_response_id: Option<String>,
    /// 这条连接迄今 mask 出来的映射，用于把 provider 事件里的占位符换回真实值。
    ///
    /// 刻意按连接持有而不是按 turn 持有：WS 的会话历史留在上游，continuation 只发
    /// 增量输入，所以后面几轮的响应可能回显更早那几轮的占位符（理由详见
    /// [`super::redaction`]）。上游重绑时不重置。
    pub(super) redaction_restorer: ResponsesWebSocketRedactionRestorer,
    pub(super) next_turn_index: u64,
    pub(super) upstream_response_headers: BTreeMap<String, String>,
    pub(super) pending_provider_drain: Option<ResponsesWebSocketDrainDirective>,
    pub(super) pending_provider_observation: Option<JoinHandle<()>>,
    pub(super) exhausted_exclusions: ExhaustedResponsesWebSocketExclusions,
    pub(super) pending_turn_finalization: Option<JoinHandle<()>>,
    pub(super) session_termination: ResponsesSessionTerminationSignal,
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

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::{
        evict_referenced_public_response_id, ResponsesPublicEventSequence,
        ResponsesPublicTeardownClaim,
    };

    #[test]
    fn failed_continuation_evicts_only_the_id_it_referenced() {
        let mut latest = Some("resp_latest".to_string());

        assert!(!evict_referenced_public_response_id(&mut latest, None));
        assert_eq!(latest.as_deref(), Some("resp_latest"));

        assert!(!evict_referenced_public_response_id(
            &mut latest,
            Some("resp_stale")
        ));
        assert_eq!(latest.as_deref(), Some("resp_latest"));

        assert!(evict_referenced_public_response_id(
            &mut latest,
            Some("resp_latest")
        ));
        assert_eq!(latest, None);
    }

    #[test]
    fn public_teardown_has_exactly_one_winner_across_clones() {
        let teardown = ResponsesPublicTeardownClaim::default();
        let other = teardown.clone();

        assert!(teardown.try_claim());
        assert!(!other.try_claim());
        assert!(other.is_claimed());
    }

    #[test]
    fn public_sequence_overwrites_provider_values_and_resets_for_a_new_response() {
        let sequence = ResponsesPublicEventSequence::default();
        let mut first = json!({"type": "response.created", "sequence_number": 99});
        let mut second = json!({"type": "response.completed"});

        let first_reservation = sequence.stamp(&mut first).unwrap();
        assert_eq!(first_reservation.sequence_number(), 0);
        assert_eq!(first["sequence_number"], 0);
        assert_eq!(sequence.reserve().sequence_number(), 0);
        first_reservation.commit();

        let second_reservation = sequence.stamp(&mut second).unwrap();
        assert_eq!(second_reservation.sequence_number(), 1);
        assert_eq!(second["sequence_number"], 1);
        second_reservation.commit();

        let shared = sequence.clone();
        assert_eq!(shared.next(), 2);
        sequence.reset();
        assert_eq!(shared.next(), 0);
    }

    #[test]
    fn public_sequence_rejects_non_object_events_without_consuming_a_number() {
        let sequence = ResponsesPublicEventSequence::default();
        let mut invalid = json!(["response.created"]);

        assert!(sequence.stamp(&mut invalid).is_err());
        assert_eq!(sequence.next(), 0);
    }

    #[test]
    fn dropped_public_sequence_reservation_is_reused() {
        let sequence = ResponsesPublicEventSequence::default();
        let mut cancelled = json!({"type": "response.created"});
        let reservation = sequence.stamp(&mut cancelled).unwrap();

        assert_eq!(reservation.sequence_number(), 0);
        drop(reservation);

        let mut retried = json!({"type": "error"});
        let retried_reservation = sequence.stamp(&mut retried).unwrap();
        assert_eq!(retried_reservation.sequence_number(), 0);
        assert_eq!(retried["sequence_number"], 0);
        retried_reservation.commit();
        assert_eq!(sequence.reserve().sequence_number(), 1);
    }
}
