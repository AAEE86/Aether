use crate::request_candidate_queue::RequestCandidateEnqueueOutcome;
use crate::{AppState, GatewayError};
use aether_data_contracts::repository::{candidate_selection, candidates, quota};
use std::collections::{BTreeMap, HashMap, HashSet};
use std::sync::{Arc, RwLock};
use std::time::Duration;

const PROVIDER_QUOTA_RUNTIME_CACHE_TTL: Duration = Duration::from_secs(5);

#[derive(Debug, Default)]
pub(crate) struct RequestCandidateRuntimeOverlay {
    inner: RwLock<RequestCandidateRuntimeOverlayInner>,
}

#[derive(Debug, Default)]
struct RequestCandidateRuntimeOverlayInner {
    next_generation: u64,
    next_revision: u64,
    slots: HashMap<String, RequestCandidateRuntimeOverlaySlot>,
    request_slots: HashMap<String, HashSet<String>>,
    provider_slots: HashMap<String, HashSet<String>>,
    key_slots: HashMap<String, HashSet<String>>,
    api_key_slots: HashMap<String, HashSet<String>>,
    active_read_revisions: BTreeMap<u64, usize>,
    committed_cleanup_slots: HashSet<String>,
}

#[derive(Debug)]
struct RequestCandidateRuntimeOverlaySlot {
    persisted_base: Option<candidates::StoredRequestCandidate>,
    contributions: Vec<RequestCandidateRuntimeOverlayContribution>,
    effective: candidates::StoredRequestCandidate,
}

#[derive(Debug)]
struct RequestCandidateRuntimeOverlayContribution {
    generation: u64,
    acknowledged_revision: Option<u64>,
    candidate: candidates::StoredRequestCandidate,
    patch: candidates::UpsertRequestCandidateRecord,
}

#[derive(Debug)]
struct RequestCandidateRuntimeOverlaySnapshot {
    effective: candidates::StoredRequestCandidate,
    patches: Vec<candidates::UpsertRequestCandidateRecord>,
}

#[derive(Debug)]
struct RequestCandidateRuntimeOverlayPublish {
    slot: String,
    generation: u64,
    effective: candidates::StoredRequestCandidate,
}

#[derive(Debug)]
pub(crate) struct RequestCandidateRuntimeOverlayLease {
    overlay: Arc<RequestCandidateRuntimeOverlay>,
    published: Option<RequestCandidateRuntimeOverlayPublish>,
}

struct RequestCandidateRuntimeOverlayReadGuard {
    overlay: Arc<RequestCandidateRuntimeOverlay>,
    revision: u64,
}

impl RequestCandidateRuntimeOverlay {
    fn publish(
        &self,
        candidate: candidates::StoredRequestCandidate,
        patch: candidates::UpsertRequestCandidateRecord,
    ) -> RequestCandidateRuntimeOverlayPublish {
        let slot_key = request_candidate_runtime_slot_key(
            &candidate.request_id,
            candidate.candidate_index,
            candidate.retry_index,
        );
        let mut inner = self
            .inner
            .write()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let generation = inner.next_generation.wrapping_add(1).max(1);
        inner.next_generation = generation;
        let revision = inner.next_revision.wrapping_add(1).max(1);
        inner.next_revision = revision;

        let mut slot =
            inner
                .slots
                .remove(&slot_key)
                .unwrap_or_else(|| RequestCandidateRuntimeOverlaySlot {
                    persisted_base: None,
                    contributions: Vec::new(),
                    effective: candidate.clone(),
                });
        inner.deindex(&slot_key, &slot.effective);
        inner.committed_cleanup_slots.remove(&slot_key);
        slot.contributions
            .push(RequestCandidateRuntimeOverlayContribution {
                generation,
                acknowledged_revision: None,
                candidate,
                patch,
            });
        slot.recompute_effective();
        let effective = slot.effective.clone();
        inner.index(&slot_key, &effective);
        inner.slots.insert(slot_key.clone(), slot);

        RequestCandidateRuntimeOverlayPublish {
            slot: slot_key,
            generation,
            effective,
        }
    }

    fn rollback(&self, published: RequestCandidateRuntimeOverlayPublish) {
        self.resolve_generation(published, false);
    }

    fn acknowledge(&self, published: RequestCandidateRuntimeOverlayPublish) {
        self.resolve_generation(published, true);
    }

    fn resolve_generation(
        &self,
        published: RequestCandidateRuntimeOverlayPublish,
        persisted: bool,
    ) {
        let mut inner = self
            .inner
            .write()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let Some(mut slot) = inner.slots.remove(&published.slot) else {
            return;
        };
        inner.deindex(&published.slot, &slot.effective);
        let Some(contribution_index) = slot
            .contributions
            .iter()
            .position(|contribution| contribution.generation == published.generation)
        else {
            inner.index(&published.slot, &slot.effective);
            inner.slots.insert(published.slot, slot);
            return;
        };
        inner.committed_cleanup_slots.remove(&published.slot);
        let revision = inner.next_revision.wrapping_add(1).max(1);
        inner.next_revision = revision;
        if persisted {
            slot.contributions[contribution_index].acknowledged_revision = Some(revision);
        } else {
            slot.contributions.remove(contribution_index);
        }
        if slot.contributions.is_empty() {
            return;
        }
        let oldest_read_revision = inner.active_read_revisions.keys().next().copied();
        let all_acknowledged = slot
            .contributions
            .iter()
            .all(|contribution| contribution.acknowledged_revision.is_some());
        if all_acknowledged
            && oldest_read_revision.is_none_or(|oldest| {
                slot.contributions.iter().all(|contribution| {
                    contribution
                        .acknowledged_revision
                        .is_some_and(|acknowledged| acknowledged <= oldest)
                })
            })
        {
            return;
        }
        slot.recompute_effective();
        inner.index(&published.slot, &slot.effective);
        if all_acknowledged {
            inner.committed_cleanup_slots.insert(published.slot.clone());
        }
        inner.slots.insert(published.slot, slot);
    }

    fn reconcile_persisted(&self, persisted: &[candidates::StoredRequestCandidate]) {
        let mut inner = self
            .inner
            .write()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let allow_covered_prefix = inner.active_read_revisions.is_empty();
        reconcile_persisted_request_candidates(&mut inner, persisted, allow_covered_prefix);
    }

    fn reconcile_request_read(
        &self,
        read_guard: &RequestCandidateRuntimeOverlayReadGuard,
        persisted: &[candidates::StoredRequestCandidate],
        request_id: &str,
    ) -> Vec<(String, RequestCandidateRuntimeOverlaySnapshot)> {
        debug_assert!(std::ptr::eq(self, read_guard.overlay.as_ref()));
        let mut inner = self
            .inner
            .write()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let allow_covered_prefix = inner.has_sole_reader(read_guard.revision);
        reconcile_persisted_request_candidates(&mut inner, persisted, allow_covered_prefix);
        request_candidate_snapshots(&inner, request_id)
    }

    fn reconcile_runtime_scope_read(
        &self,
        read_guard: &RequestCandidateRuntimeOverlayReadGuard,
        persisted: &[candidates::StoredRequestCandidate],
        provider_ids: &[String],
        key_ids: &[String],
        api_key_ids: &[String],
    ) -> Vec<(String, RequestCandidateRuntimeOverlaySnapshot)> {
        debug_assert!(std::ptr::eq(self, read_guard.overlay.as_ref()));
        let mut inner = self
            .inner
            .write()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let allow_covered_prefix = inner.has_sole_reader(read_guard.revision);
        reconcile_persisted_request_candidates(&mut inner, persisted, allow_covered_prefix);
        runtime_scope_candidate_snapshots(&inner, provider_ids, key_ids, api_key_ids)
    }

    fn begin_read(self: &Arc<Self>) -> RequestCandidateRuntimeOverlayReadGuard {
        let mut inner = self
            .inner
            .write()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let revision = inner.next_revision;
        *inner.active_read_revisions.entry(revision).or_default() += 1;
        RequestCandidateRuntimeOverlayReadGuard {
            overlay: Arc::clone(self),
            revision,
        }
    }

    fn candidates_for_request(
        &self,
        request_id: &str,
    ) -> Vec<(String, RequestCandidateRuntimeOverlaySnapshot)> {
        let inner = self
            .inner
            .read()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        request_candidate_snapshots(&inner, request_id)
    }

    fn candidates_for_runtime_scopes(
        &self,
        provider_ids: &[String],
        key_ids: &[String],
        api_key_ids: &[String],
    ) -> Vec<(String, RequestCandidateRuntimeOverlaySnapshot)> {
        let inner = self
            .inner
            .read()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        runtime_scope_candidate_snapshots(&inner, provider_ids, key_ids, api_key_ids)
    }

    pub(crate) fn clear(&self) {
        self.inner
            .write()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .clear();
    }

    #[cfg(test)]
    fn is_empty(&self) -> bool {
        self.inner
            .read()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .slots
            .is_empty()
    }

    #[cfg(test)]
    fn contains_key(&self, slot_key: &str) -> bool {
        self.inner
            .read()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .slots
            .contains_key(slot_key)
    }

    #[cfg(test)]
    fn get(&self, slot_key: &str) -> Option<candidates::StoredRequestCandidate> {
        self.inner
            .read()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .slots
            .get(slot_key)
            .map(|slot| slot.effective.clone())
    }

    #[cfg(test)]
    fn contribution_count_for_slot(&self, slot_key: &str) -> usize {
        self.inner
            .read()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .slots
            .get(slot_key)
            .map(|slot| slot.contributions.len())
            .unwrap_or_default()
    }

    #[cfg(test)]
    fn contribution_count(&self) -> usize {
        self.inner
            .read()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .slots
            .values()
            .map(|slot| slot.contributions.len())
            .sum()
    }

    #[cfg(test)]
    fn publish_for_test(
        &self,
        candidate: candidates::StoredRequestCandidate,
    ) -> RequestCandidateRuntimeOverlayPublish {
        let patch = upsert_request_candidate_from_stored(&candidate);
        self.publish(candidate, patch)
    }

    #[cfg(test)]
    fn rollback_for_test(&self, published: RequestCandidateRuntimeOverlayPublish) {
        self.rollback(published);
    }

    #[cfg(test)]
    fn acknowledge_for_test(&self, published: RequestCandidateRuntimeOverlayPublish) {
        self.acknowledge(published);
    }

    #[cfg(test)]
    fn reconcile_for_test(&self, persisted: &[candidates::StoredRequestCandidate]) {
        self.reconcile_persisted(persisted);
    }
}

impl RequestCandidateRuntimeOverlayLease {
    fn new(
        overlay: Arc<RequestCandidateRuntimeOverlay>,
        candidate: candidates::StoredRequestCandidate,
        patch: candidates::UpsertRequestCandidateRecord,
    ) -> Self {
        let published = overlay.publish(candidate, patch);
        Self {
            overlay,
            published: Some(published),
        }
    }

    fn effective(&self) -> &candidates::StoredRequestCandidate {
        &self
            .published
            .as_ref()
            .expect("an active overlay lease must retain its publication")
            .effective
    }

    pub(crate) fn acknowledge(mut self) {
        if let Some(published) = self.published.take() {
            self.overlay.acknowledge(published);
        }
    }

    pub(crate) fn acknowledge_all(leases: Vec<Self>) {
        for lease in leases {
            lease.acknowledge();
        }
    }
}

impl Drop for RequestCandidateRuntimeOverlayLease {
    fn drop(&mut self) {
        if let Some(published) = self.published.take() {
            self.overlay.rollback(published);
        }
    }
}

impl Drop for RequestCandidateRuntimeOverlayReadGuard {
    fn drop(&mut self) {
        let mut inner = self
            .overlay
            .inner
            .write()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let remove_revision = inner
            .active_read_revisions
            .get_mut(&self.revision)
            .is_some_and(|readers| {
                *readers = readers.saturating_sub(1);
                *readers == 0
            });
        if remove_revision {
            inner.active_read_revisions.remove(&self.revision);
        }
        inner.prune_committed_cleanup_slots();
    }
}

impl RequestCandidateRuntimeOverlayInner {
    fn has_sole_reader(&self, revision: u64) -> bool {
        self.active_read_revisions.len() == 1
            && self.active_read_revisions.get(&revision) == Some(&1)
    }

    fn prune_committed_cleanup_slots(&mut self) {
        let oldest_read_revision = self.active_read_revisions.keys().next().copied();
        let cleanup_slots = self.committed_cleanup_slots.drain().collect::<Vec<_>>();
        for slot_key in cleanup_slots {
            let should_remove = self.slots.get(&slot_key).is_some_and(|slot| {
                !slot.contributions.is_empty()
                    && slot.contributions.iter().all(|contribution| {
                        contribution
                            .acknowledged_revision
                            .is_some_and(|acknowledged| {
                                oldest_read_revision.is_none_or(|oldest| acknowledged <= oldest)
                            })
                    })
            });
            if should_remove {
                if let Some(slot) = self.slots.remove(&slot_key) {
                    self.deindex(&slot_key, &slot.effective);
                }
            } else if self.slots.contains_key(&slot_key) {
                self.committed_cleanup_slots.insert(slot_key);
            }
        }
    }

    fn index(&mut self, slot_key: &str, candidate: &candidates::StoredRequestCandidate) {
        insert_runtime_overlay_index(&mut self.request_slots, &candidate.request_id, slot_key);
        if let Some(provider_id) = candidate.provider_id.as_deref() {
            insert_runtime_overlay_index(&mut self.provider_slots, provider_id, slot_key);
        }
        if let Some(key_id) = candidate.key_id.as_deref() {
            insert_runtime_overlay_index(&mut self.key_slots, key_id, slot_key);
        }
        if let Some(api_key_id) = candidate.api_key_id.as_deref() {
            insert_runtime_overlay_index(&mut self.api_key_slots, api_key_id, slot_key);
        }
    }

    fn deindex(&mut self, slot_key: &str, candidate: &candidates::StoredRequestCandidate) {
        remove_runtime_overlay_index(&mut self.request_slots, &candidate.request_id, slot_key);
        if let Some(provider_id) = candidate.provider_id.as_deref() {
            remove_runtime_overlay_index(&mut self.provider_slots, provider_id, slot_key);
        }
        if let Some(key_id) = candidate.key_id.as_deref() {
            remove_runtime_overlay_index(&mut self.key_slots, key_id, slot_key);
        }
        if let Some(api_key_id) = candidate.api_key_id.as_deref() {
            remove_runtime_overlay_index(&mut self.api_key_slots, api_key_id, slot_key);
        }
    }

    fn clear(&mut self) {
        self.slots.clear();
        self.request_slots.clear();
        self.provider_slots.clear();
        self.key_slots.clear();
        self.api_key_slots.clear();
        self.active_read_revisions.clear();
        self.committed_cleanup_slots.clear();
    }
}

fn reconcile_persisted_request_candidates(
    inner: &mut RequestCandidateRuntimeOverlayInner,
    persisted: &[candidates::StoredRequestCandidate],
    allow_covered_prefix: bool,
) {
    let oldest_read_revision = inner.active_read_revisions.keys().next().copied();
    for persisted_candidate in persisted {
        let slot_key = request_candidate_runtime_slot_key(
            &persisted_candidate.request_id,
            persisted_candidate.candidate_index,
            persisted_candidate.retry_index,
        );
        let Some(mut slot) = inner.slots.remove(&slot_key) else {
            continue;
        };
        inner.deindex(&slot_key, &slot.effective);
        let removable_prefix =
            slot.contributions
                .iter()
                .take_while(|contribution| {
                    let acknowledgement_is_visible = contribution
                        .acknowledged_revision
                        .is_some_and(|acknowledged| {
                            oldest_read_revision.is_none_or(|oldest| acknowledged <= oldest)
                        });
                    acknowledgement_is_visible
                        || allow_covered_prefix
                            && request_candidate_patch_is_covered(
                                persisted_candidate,
                                &contribution.patch,
                            )
                })
                .count();
        if removable_prefix > 0 {
            slot.contributions.drain(..removable_prefix);
        }
        if slot.contributions.is_empty() {
            inner.committed_cleanup_slots.remove(&slot_key);
            continue;
        }
        slot.persisted_base = Some(persisted_candidate.clone());
        slot.recompute_effective();
        inner.index(&slot_key, &slot.effective);
        inner.slots.insert(slot_key, slot);
    }
}

fn request_candidate_patch_is_covered(
    persisted: &candidates::StoredRequestCandidate,
    patch: &candidates::UpsertRequestCandidateRecord,
) -> bool {
    let mut effective = persisted.clone();
    merge_stored_request_candidate_patch(&mut effective, patch);
    effective == *persisted
}

fn request_candidate_snapshots(
    inner: &RequestCandidateRuntimeOverlayInner,
    request_id: &str,
) -> Vec<(String, RequestCandidateRuntimeOverlaySnapshot)> {
    inner
        .request_slots
        .get(request_id)
        .into_iter()
        .flatten()
        .filter_map(|slot_key| {
            inner
                .slots
                .get(slot_key)
                .map(|slot| (slot_key.clone(), slot.snapshot()))
        })
        .collect()
}

fn runtime_scope_candidate_snapshots(
    inner: &RequestCandidateRuntimeOverlayInner,
    provider_ids: &[String],
    key_ids: &[String],
    api_key_ids: &[String],
) -> Vec<(String, RequestCandidateRuntimeOverlaySnapshot)> {
    let mut slot_keys = HashSet::new();
    collect_runtime_scope_slots(&inner.provider_slots, provider_ids, &mut slot_keys);
    collect_runtime_scope_slots(&inner.key_slots, key_ids, &mut slot_keys);
    collect_runtime_scope_slots(&inner.api_key_slots, api_key_ids, &mut slot_keys);
    slot_keys
        .into_iter()
        .filter_map(|slot_key| {
            inner
                .slots
                .get(&slot_key)
                .map(|slot| (slot_key, slot.snapshot()))
        })
        .collect()
}

impl RequestCandidateRuntimeOverlaySlot {
    fn recompute_effective(&mut self) {
        let mut effective = self.persisted_base.clone();
        for contribution in &self.contributions {
            match effective.as_mut() {
                Some(effective) => {
                    merge_stored_request_candidate_patch(effective, &contribution.patch)
                }
                None => effective = Some(contribution.candidate.clone()),
            }
        }
        self.effective = effective.expect("an overlay slot must retain one contribution");
    }

    fn snapshot(&self) -> RequestCandidateRuntimeOverlaySnapshot {
        RequestCandidateRuntimeOverlaySnapshot {
            effective: self.effective.clone(),
            patches: self
                .contributions
                .iter()
                .map(|contribution| contribution.patch.clone())
                .collect(),
        }
    }
}

fn collect_runtime_scope_slots(
    index: &HashMap<String, HashSet<String>>,
    values: &[String],
    output: &mut HashSet<String>,
) {
    for value in values {
        if let Some(slots) = index.get(value) {
            output.extend(slots.iter().cloned());
        }
    }
}

fn insert_runtime_overlay_index(
    index: &mut HashMap<String, HashSet<String>>,
    value: &str,
    slot_key: &str,
) {
    if !value.is_empty() {
        index
            .entry(value.to_string())
            .or_default()
            .insert(slot_key.to_string());
    }
}

fn remove_runtime_overlay_index(
    index: &mut HashMap<String, HashSet<String>>,
    value: &str,
    slot_key: &str,
) {
    let should_remove = index.get_mut(value).is_some_and(|slots| {
        slots.remove(slot_key);
        slots.is_empty()
    });
    if should_remove {
        index.remove(value);
    }
}

impl AppState {
    pub(crate) async fn list_minimal_candidate_selection_rows_for_api_format(
        &self,
        api_format: &str,
    ) -> Result<Vec<candidate_selection::StoredMinimalCandidateSelectionRow>, GatewayError> {
        self.data
            .list_minimal_candidate_selection_rows_for_api_format(api_format)
            .await
            .map_err(|err| GatewayError::Internal(err.to_string()))
    }

    pub(crate) async fn list_minimal_candidate_selection_rows_for_api_format_and_global_model(
        &self,
        api_format: &str,
        global_model_name: &str,
    ) -> Result<Vec<candidate_selection::StoredMinimalCandidateSelectionRow>, GatewayError> {
        self.data
            .list_minimal_candidate_selection_rows(api_format, global_model_name)
            .await
            .map_err(|err| GatewayError::Internal(err.to_string()))
    }

    pub(crate) async fn list_minimal_candidate_selection_rows_for_api_format_and_requested_model(
        &self,
        api_format: &str,
        requested_model_name: &str,
    ) -> Result<Vec<candidate_selection::StoredMinimalCandidateSelectionRow>, GatewayError> {
        self.data
            .list_minimal_candidate_selection_rows_for_requested_model(
                api_format,
                requested_model_name,
            )
            .await
            .map_err(|err| GatewayError::Internal(err.to_string()))
    }

    pub(crate) async fn list_minimal_candidate_selection_rows_for_api_format_and_requested_model_page(
        &self,
        query: &candidate_selection::StoredRequestedModelCandidateRowsQuery,
    ) -> Result<Vec<candidate_selection::StoredMinimalCandidateSelectionRow>, GatewayError> {
        self.data
            .list_minimal_candidate_selection_rows_for_requested_model_page(query)
            .await
            .map_err(|err| GatewayError::Internal(err.to_string()))
    }

    pub(crate) async fn list_pool_key_candidate_rows_for_group(
        &self,
        query: &candidate_selection::StoredPoolKeyCandidateRowsQuery,
    ) -> Result<Vec<candidate_selection::StoredMinimalCandidateSelectionRow>, GatewayError> {
        self.data
            .list_pool_key_candidate_rows_for_group(query)
            .await
            .map_err(|err| GatewayError::Internal(err.to_string()))
    }

    pub(crate) async fn list_pool_key_candidate_rows_for_group_key_ids(
        &self,
        query: &candidate_selection::StoredPoolKeyCandidateRowsByKeyIdsQuery,
    ) -> Result<Vec<candidate_selection::StoredMinimalCandidateSelectionRow>, GatewayError> {
        self.data
            .list_pool_key_candidate_rows_for_group_key_ids(query)
            .await
            .map_err(|err| GatewayError::Internal(err.to_string()))
    }

    pub(crate) async fn list_pool_key_candidate_rows_for_group_key_ids_strong(
        &self,
        query: &candidate_selection::StoredPoolKeyCandidateRowsByKeyIdsQuery,
    ) -> Result<Vec<candidate_selection::StoredMinimalCandidateSelectionRow>, GatewayError> {
        self.data
            .list_pool_key_candidate_rows_for_group_key_ids_strong(query)
            .await
            .map_err(|err| GatewayError::Internal(err.to_string()))
    }

    pub(crate) async fn read_provider_quota_snapshot(
        &self,
        provider_id: &str,
    ) -> Result<Option<quota::StoredProviderQuotaSnapshot>, GatewayError> {
        let provider_id = provider_id.trim();
        if provider_id.is_empty() {
            return Ok(None);
        }
        let cache_key = provider_id.to_string();
        self.provider_quota_snapshot_cache
            .get_or_load(cache_key, PROVIDER_QUOTA_RUNTIME_CACHE_TTL, || async move {
                self.data
                    .find_provider_quota_by_provider_id(provider_id)
                    .await
                    .map_err(|err| GatewayError::Internal(err.to_string()))
            })
            .await
    }

    pub(crate) async fn read_provider_quota_snapshot_strong(
        &self,
        provider_id: &str,
    ) -> Result<Option<quota::StoredProviderQuotaSnapshot>, GatewayError> {
        let provider_id = provider_id.trim();
        if provider_id.is_empty() {
            return Ok(None);
        }
        self.data
            .find_provider_quota_by_provider_id_strong(provider_id)
            .await
            .map_err(|err| GatewayError::Internal(err.to_string()))
    }

    pub(crate) async fn read_provider_quota_snapshots(
        &self,
        provider_ids: &[String],
    ) -> Result<Vec<quota::StoredProviderQuotaSnapshot>, GatewayError> {
        self.data
            .find_provider_quotas_by_provider_ids(provider_ids)
            .await
            .map_err(|err| GatewayError::Internal(err.to_string()))
    }

    pub(crate) async fn read_recent_request_candidates(
        &self,
        limit: usize,
    ) -> Result<Vec<candidates::StoredRequestCandidate>, GatewayError> {
        self.data
            .list_recent_request_candidates(limit)
            .await
            .map_err(|err| GatewayError::Internal(err.to_string()))
    }

    /// Reads one logical request's candidates including writes that are still
    /// buffered by the asynchronous persistence queue.
    pub(crate) async fn read_request_candidates_with_runtime_overlay(
        &self,
        request_id: &str,
    ) -> Result<Vec<candidates::StoredRequestCandidate>, GatewayError> {
        let read_guard = self.request_candidate_runtime_overlay.begin_read();
        let persisted = self
            .data
            .list_request_candidates_by_request_id(request_id)
            .await
            .map_err(|err| GatewayError::Internal(err.to_string()))?;
        let runtime_overlay = self
            .request_candidate_runtime_overlay
            .reconcile_request_read(&read_guard, &persisted, request_id);
        let merged = merge_request_candidates_with_runtime_overlay(persisted, runtime_overlay);
        drop(read_guard);
        Ok(merged)
    }

    pub(crate) async fn read_runtime_scoped_request_candidates_since(
        &self,
        provider_ids: &[String],
        key_ids: &[String],
        api_key_ids: &[String],
        since_unix_secs: u64,
    ) -> Result<Vec<candidates::StoredRequestCandidate>, GatewayError> {
        let read_guard = self.request_candidate_runtime_overlay.begin_read();
        let persisted = self
            .data
            .list_runtime_scoped_request_candidates_since(
                provider_ids,
                key_ids,
                api_key_ids,
                since_unix_secs,
            )
            .await
            .map_err(|err| GatewayError::Internal(err.to_string()))?;
        let runtime_overlay = self
            .request_candidate_runtime_overlay
            .reconcile_runtime_scope_read(
                &read_guard,
                &persisted,
                provider_ids,
                key_ids,
                api_key_ids,
            );
        let merged = merge_runtime_scoped_request_candidates(
            persisted,
            runtime_overlay,
            provider_ids,
            key_ids,
            api_key_ids,
            since_unix_secs,
        );
        drop(read_guard);
        Ok(merged)
    }

    pub(crate) async fn upsert_request_candidate(
        &self,
        candidate: candidates::UpsertRequestCandidateRecord,
    ) -> Result<Option<candidates::StoredRequestCandidate>, GatewayError> {
        if let Some(queue) = self.request_candidate_queue.as_ref() {
            let runtime_candidate = stored_request_candidate_from_upsert(&candidate)?;
            // Publish before enqueueing so a fast queue worker cannot persist
            // and acknowledge the write before its runtime overlay exists.
            let lease = self.publish_stored_request_candidate_runtime_overlay(
                runtime_candidate,
                candidate.clone(),
            );
            let effective = lease.effective().clone();
            let outcome = queue
                .enqueue_or_fallback_with_overlay(candidate, lease)
                .await
                .map_err(|err| GatewayError::Internal(err.to_string()))?;
            if outcome == RequestCandidateEnqueueOutcome::Dropped {
                return Ok(None);
            }
            return Ok(Some(effective));
        }

        self.data
            .upsert_request_candidate(candidate)
            .await
            .map_err(|err| GatewayError::Internal(err.to_string()))
    }

    /// Persist a candidate status when the caller does not need the materialized row.
    ///
    /// Lifecycle updates are emitted on the hot path (in particular the first-byte
    /// `pending -> streaming` transition). Rebuilding `StoredRequestCandidate` here
    /// only to discard it adds validation and clones for every update, especially when
    /// the async queue is enabled.
    pub(crate) async fn enqueue_request_candidate_status(
        &self,
        candidate: candidates::UpsertRequestCandidateRecord,
    ) -> Result<Option<()>, GatewayError> {
        if let Some(queue) = self.request_candidate_queue.as_ref() {
            let runtime_candidate = stored_request_candidate_from_upsert(&candidate)?;
            let lease = self.publish_stored_request_candidate_runtime_overlay(
                runtime_candidate,
                candidate.clone(),
            );
            let outcome = queue
                .enqueue_or_fallback_with_overlay(candidate, lease)
                .await
                .map_err(|err| GatewayError::Internal(err.to_string()))?;
            if outcome == RequestCandidateEnqueueOutcome::Dropped {
                return Ok(None);
            }
            return Ok(Some(()));
        }

        self.data
            .upsert_request_candidate(candidate)
            .await
            .map(|stored| stored.map(|_| ()))
            .map_err(|err| GatewayError::Internal(err.to_string()))
    }

    /// Try the in-memory lifecycle lane without awaiting or touching the repository.
    /// The returned record must be persisted through `enqueue_request_candidate_status`
    /// when the queue is disabled or closed.
    pub(crate) fn try_enqueue_request_candidate_status(
        &self,
        candidate: candidates::UpsertRequestCandidateRecord,
    ) -> Result<(), candidates::UpsertRequestCandidateRecord> {
        let Some(queue) = self.request_candidate_queue.as_ref() else {
            return Err(candidate);
        };
        let runtime_candidate = match stored_request_candidate_from_upsert(&candidate) {
            Ok(candidate) => candidate,
            Err(_) => return Err(candidate),
        };
        let lease = self
            .publish_stored_request_candidate_runtime_overlay(runtime_candidate, candidate.clone());
        match queue.try_enqueue_priority_status_with_overlay(candidate, lease) {
            Ok(()) => Ok(()),
            Err(candidate) => Err(candidate),
        }
    }

    #[cfg(test)]
    fn remember_stored_request_candidate_runtime_overlay(
        &self,
        candidate: candidates::StoredRequestCandidate,
    ) -> candidates::StoredRequestCandidate {
        let patch = upsert_request_candidate_from_stored(&candidate);
        self.request_candidate_runtime_overlay
            .publish(candidate, patch)
            .effective
    }

    fn publish_stored_request_candidate_runtime_overlay(
        &self,
        candidate: candidates::StoredRequestCandidate,
        patch: candidates::UpsertRequestCandidateRecord,
    ) -> RequestCandidateRuntimeOverlayLease {
        RequestCandidateRuntimeOverlayLease::new(
            Arc::clone(&self.request_candidate_runtime_overlay),
            candidate,
            patch,
        )
    }
}

fn request_candidate_runtime_slot_key(
    request_id: &str,
    candidate_index: u32,
    retry_index: u32,
) -> String {
    format!(
        "{}:{request_id}:{candidate_index}:{retry_index}",
        request_id.len()
    )
}

fn merge_runtime_scoped_request_candidates(
    persisted: Vec<candidates::StoredRequestCandidate>,
    runtime_overlay: Vec<(String, RequestCandidateRuntimeOverlaySnapshot)>,
    provider_ids: &[String],
    key_ids: &[String],
    api_key_ids: &[String],
    since_unix_secs: u64,
) -> Vec<candidates::StoredRequestCandidate> {
    if provider_ids.is_empty() && key_ids.is_empty() && api_key_ids.is_empty() {
        return Vec::new();
    }

    let mut merged = HashMap::<String, candidates::StoredRequestCandidate>::with_capacity(
        persisted.len().saturating_add(runtime_overlay.len()),
    );
    for candidate in persisted {
        let slot = request_candidate_runtime_slot_key(
            &candidate.request_id,
            candidate.candidate_index,
            candidate.retry_index,
        );
        match merged.get_mut(&slot) {
            Some(existing) => merge_stored_request_candidate_runtime_record(existing, &candidate),
            None => {
                merged.insert(slot, candidate);
            }
        }
    }
    for (slot, overlay_candidate) in runtime_overlay {
        match merged.get_mut(&slot) {
            Some(candidate) => {
                for patch in &overlay_candidate.patches {
                    merge_stored_request_candidate_patch(candidate, patch);
                }
            }
            None => {
                merged.insert(slot, overlay_candidate.effective);
            }
        }
    }

    let mut rows = merged
        .into_values()
        .filter(request_candidate_status_affects_runtime)
        .filter(|candidate| {
            candidate
                .started_at_unix_ms
                .unwrap_or(candidate.created_at_unix_ms)
                / 1000
                >= since_unix_secs
        })
        .filter(|candidate| {
            candidate
                .provider_id
                .as_ref()
                .is_some_and(|id| provider_ids.contains(id))
                || candidate
                    .key_id
                    .as_ref()
                    .is_some_and(|id| key_ids.contains(id))
                || candidate
                    .api_key_id
                    .as_ref()
                    .is_some_and(|id| api_key_ids.contains(id))
        })
        .collect::<Vec<_>>();
    rows.sort_by(|left, right| {
        right
            .created_at_unix_ms
            .cmp(&left.created_at_unix_ms)
            .then(left.id.cmp(&right.id))
    });
    rows
}

fn merge_request_candidates_with_runtime_overlay(
    persisted: Vec<candidates::StoredRequestCandidate>,
    request_overlay: Vec<(String, RequestCandidateRuntimeOverlaySnapshot)>,
) -> Vec<candidates::StoredRequestCandidate> {
    let mut merged = HashMap::<String, candidates::StoredRequestCandidate>::with_capacity(
        persisted.len().saturating_add(request_overlay.len()),
    );
    for candidate in persisted {
        let slot = request_candidate_runtime_slot_key(
            &candidate.request_id,
            candidate.candidate_index,
            candidate.retry_index,
        );
        merged.insert(slot, candidate);
    }
    for (slot_key, candidate) in request_overlay {
        match merged.get_mut(&slot_key) {
            Some(persisted) => {
                for patch in &candidate.patches {
                    merge_stored_request_candidate_patch(persisted, patch);
                }
            }
            None => {
                merged.insert(slot_key, candidate.effective);
            }
        }
    }

    let mut candidates = merged.into_values().collect::<Vec<_>>();
    candidates.sort_by_key(|candidate| (candidate.candidate_index, candidate.retry_index));
    candidates
}

fn request_candidate_status_affects_runtime(
    candidate: &candidates::StoredRequestCandidate,
) -> bool {
    matches!(
        candidate.status,
        candidates::RequestCandidateStatus::Pending
            | candidates::RequestCandidateStatus::Streaming
            | candidates::RequestCandidateStatus::Success
            | candidates::RequestCandidateStatus::Failed
            | candidates::RequestCandidateStatus::Cancelled
    )
}

fn merge_stored_request_candidate_runtime_record(
    target: &mut candidates::StoredRequestCandidate,
    incoming: &candidates::StoredRequestCandidate,
) {
    let preserve_existing_lifecycle =
        candidates::request_candidate_lifecycle_would_regress(target.status, incoming.status);
    let incoming_lifecycle_regresses =
        candidates::request_candidate_lifecycle_would_regress(incoming.status, target.status);
    let preserve_incoming_lifecycle = !preserve_existing_lifecycle;
    if preserve_incoming_lifecycle {
        target.status = incoming.status;
    }

    clone_if_some(&mut target.user_id, &incoming.user_id);
    clone_if_some(&mut target.api_key_id, &incoming.api_key_id);
    clone_if_some(&mut target.username, &incoming.username);
    clone_if_some(&mut target.api_key_name, &incoming.api_key_name);
    clone_if_some(&mut target.provider_id, &incoming.provider_id);
    clone_if_some(&mut target.endpoint_id, &incoming.endpoint_id);
    clone_if_some(&mut target.key_id, &incoming.key_id);
    clone_if_some(&mut target.skip_reason, &incoming.skip_reason);
    target.is_cached = incoming.is_cached;
    if preserve_incoming_lifecycle {
        copy_if_some(&mut target.status_code, incoming.status_code);
        clone_if_some(&mut target.error_type, &incoming.error_type);
        clone_if_some(&mut target.error_message, &incoming.error_message);
        copy_if_some(&mut target.latency_ms, incoming.latency_ms);
        copy_if_some(
            &mut target.finished_at_unix_ms,
            incoming.finished_at_unix_ms,
        );
    }
    copy_if_some(
        &mut target.concurrent_requests,
        incoming.concurrent_requests,
    );
    merge_json_value(&mut target.extra_data, &incoming.extra_data);
    clone_if_some(
        &mut target.required_capabilities,
        &incoming.required_capabilities,
    );
    if !incoming_lifecycle_regresses {
        copy_if_some(&mut target.started_at_unix_ms, incoming.started_at_unix_ms);
    }
}

fn merge_stored_request_candidate_patch(
    target: &mut candidates::StoredRequestCandidate,
    patch: &candidates::UpsertRequestCandidateRecord,
) {
    let preserve_existing_lifecycle =
        candidates::request_candidate_lifecycle_would_regress(target.status, patch.status);
    if !preserve_existing_lifecycle {
        target.status = patch.status;
    }

    clone_if_some(&mut target.user_id, &patch.user_id);
    clone_if_some(&mut target.api_key_id, &patch.api_key_id);
    clone_if_some(&mut target.username, &patch.username);
    clone_if_some(&mut target.api_key_name, &patch.api_key_name);
    clone_if_some(&mut target.provider_id, &patch.provider_id);
    clone_if_some(&mut target.endpoint_id, &patch.endpoint_id);
    clone_if_some(&mut target.key_id, &patch.key_id);
    clone_if_some(&mut target.skip_reason, &patch.skip_reason);
    if let Some(is_cached) = patch.is_cached {
        target.is_cached = is_cached;
    }
    if !preserve_existing_lifecycle {
        copy_if_some(&mut target.status_code, patch.status_code);
        clone_if_some(&mut target.error_type, &patch.error_type);
        clone_if_some(&mut target.error_message, &patch.error_message);
        copy_if_some(&mut target.latency_ms, patch.latency_ms);
        copy_if_some(&mut target.finished_at_unix_ms, patch.finished_at_unix_ms);
    }
    copy_if_some(&mut target.concurrent_requests, patch.concurrent_requests);
    merge_json_value(&mut target.extra_data, &patch.extra_data);
    clone_if_some(
        &mut target.required_capabilities,
        &patch.required_capabilities,
    );
    copy_if_some(&mut target.started_at_unix_ms, patch.started_at_unix_ms);
}

fn copy_if_some<T: Copy>(target: &mut Option<T>, incoming: Option<T>) {
    if let Some(incoming) = incoming {
        *target = Some(incoming);
    }
}

fn clone_if_some<T: Clone>(target: &mut Option<T>, incoming: &Option<T>) {
    if let Some(incoming) = incoming {
        *target = Some(incoming.clone());
    }
}

fn merge_json_value(target: &mut Option<serde_json::Value>, incoming: &Option<serde_json::Value>) {
    match (target.as_mut(), incoming) {
        (Some(serde_json::Value::Object(target)), Some(serde_json::Value::Object(incoming))) => {
            target.extend(incoming.clone())
        }
        (_, Some(incoming)) => *target = Some(incoming.clone()),
        (_, None) => {}
    }
}

fn stored_request_candidate_from_upsert(
    candidate: &candidates::UpsertRequestCandidateRecord,
) -> Result<candidates::StoredRequestCandidate, GatewayError> {
    candidate
        .validate()
        .map_err(|err| GatewayError::Internal(err.to_string()))?;
    candidates::StoredRequestCandidate::new(
        candidate.id.clone(),
        candidate.request_id.clone(),
        candidate.user_id.clone(),
        candidate.api_key_id.clone(),
        candidate.username.clone(),
        candidate.api_key_name.clone(),
        candidate.candidate_index.try_into().unwrap_or(i32::MAX),
        candidate.retry_index.try_into().unwrap_or(i32::MAX),
        candidate.provider_id.clone(),
        candidate.endpoint_id.clone(),
        candidate.key_id.clone(),
        candidate.status,
        candidate.skip_reason.clone(),
        candidate.is_cached.unwrap_or(false),
        candidate.status_code.map(i32::from),
        candidate.error_type.clone(),
        candidate.error_message.clone(),
        candidate
            .latency_ms
            .map(|value| i32::try_from(value).unwrap_or(i32::MAX)),
        candidate
            .concurrent_requests
            .map(|value| i32::try_from(value).unwrap_or(i32::MAX)),
        candidate.extra_data.clone(),
        candidate.required_capabilities.clone(),
        candidate
            .created_at_unix_ms
            .or(candidate.started_at_unix_ms)
            .or(candidate.finished_at_unix_ms)
            .unwrap_or_else(crate::clock::current_unix_ms)
            .try_into()
            .unwrap_or(i64::MAX),
        candidate
            .started_at_unix_ms
            .map(|value| value.try_into().unwrap_or(i64::MAX)),
        candidate
            .finished_at_unix_ms
            .map(|value| value.try_into().unwrap_or(i64::MAX)),
    )
    .map_err(|err| GatewayError::Internal(err.to_string()))
}

fn upsert_request_candidate_from_stored(
    candidate: &candidates::StoredRequestCandidate,
) -> candidates::UpsertRequestCandidateRecord {
    candidates::UpsertRequestCandidateRecord {
        id: candidate.id.clone(),
        request_id: candidate.request_id.clone(),
        user_id: candidate.user_id.clone(),
        api_key_id: candidate.api_key_id.clone(),
        username: candidate.username.clone(),
        api_key_name: candidate.api_key_name.clone(),
        candidate_index: candidate.candidate_index,
        retry_index: candidate.retry_index,
        provider_id: candidate.provider_id.clone(),
        endpoint_id: candidate.endpoint_id.clone(),
        key_id: candidate.key_id.clone(),
        status: candidate.status,
        skip_reason: candidate.skip_reason.clone(),
        is_cached: Some(candidate.is_cached),
        status_code: candidate.status_code,
        error_type: candidate.error_type.clone(),
        error_message: candidate.error_message.clone(),
        latency_ms: candidate.latency_ms,
        concurrent_requests: candidate.concurrent_requests,
        extra_data: candidate.extra_data.clone(),
        required_capabilities: candidate.required_capabilities.clone(),
        created_at_unix_ms: Some(candidate.created_at_unix_ms),
        started_at_unix_ms: candidate.started_at_unix_ms,
        finished_at_unix_ms: candidate.finished_at_unix_ms,
    }
}

#[cfg(test)]
mod tests {
    use super::request_candidate_runtime_slot_key;
    use crate::data::GatewayDataState;
    use crate::request_candidate_queue::{
        RequestCandidateQueueConfig, RequestCandidateQueueFullPolicy, RequestCandidateQueueRuntime,
        RequestCandidateWriteMode,
    };
    use crate::request_candidate_runtime::prepare_execution_request_candidate_slot;
    use crate::AppState;
    use aether_contracts::{ExecutionPlan, RequestBody};
    use aether_data::repository::candidates::InMemoryRequestCandidateRepository;
    use aether_data::DataLayerError;
    use aether_data_contracts::repository::candidates::{
        RequestCandidateReadRepository, RequestCandidateStatus, RequestCandidateWriteRepository,
        StoredRequestCandidate, UpsertRequestCandidateRecord,
    };
    use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
    use std::sync::Arc;
    use std::time::Duration;

    fn execution_plan_for_queued_candidate() -> ExecutionPlan {
        ExecutionPlan {
            request_id: "request-1".to_string(),
            candidate_id: Some("candidate-queued".to_string()),
            provider_name: Some("Provider".to_string()),
            provider_id: "provider-1".to_string(),
            endpoint_id: "endpoint-1".to_string(),
            key_id: "key-1".to_string(),
            method: "POST".to_string(),
            url: "https://example.invalid/v1/responses".to_string(),
            headers: Default::default(),
            content_type: Some("application/json".to_string()),
            content_encoding: None,
            body: RequestBody::from_json(serde_json::json!({"model": "gpt-test"})),
            stream: true,
            client_api_format: "openai:responses".to_string(),
            provider_api_format: "openai:responses".to_string(),
            model_name: Some("gpt-test".to_string()),
            proxy: None,
            transport_profile: None,
            timeouts: None,
        }
    }

    struct BlockingRequestCandidateRepository {
        inner: InMemoryRequestCandidateRepository,
        write_calls: AtomicUsize,
        write_started: tokio::sync::Notify,
        release_write: tokio::sync::Notify,
    }

    impl Default for BlockingRequestCandidateRepository {
        fn default() -> Self {
            Self {
                inner: InMemoryRequestCandidateRepository::default(),
                write_calls: AtomicUsize::new(0),
                write_started: tokio::sync::Notify::new(),
                release_write: tokio::sync::Notify::new(),
            }
        }
    }

    #[async_trait::async_trait]
    impl RequestCandidateWriteRepository for BlockingRequestCandidateRepository {
        async fn upsert(
            &self,
            candidate: UpsertRequestCandidateRecord,
        ) -> Result<StoredRequestCandidate, DataLayerError> {
            self.write_calls.fetch_add(1, Ordering::AcqRel);
            self.write_started.notify_one();
            self.release_write.notified().await;
            self.inner.upsert(candidate).await
        }

        async fn upsert_many(
            &self,
            candidates: Vec<UpsertRequestCandidateRecord>,
        ) -> Result<usize, DataLayerError> {
            self.write_calls.fetch_add(1, Ordering::AcqRel);
            self.write_started.notify_one();
            self.release_write.notified().await;
            let count = candidates.len();
            for candidate in candidates {
                self.inner.upsert(candidate).await?;
            }
            Ok(count)
        }

        async fn delete_created_before(
            &self,
            created_before_unix_secs: u64,
            limit: usize,
        ) -> Result<usize, DataLayerError> {
            self.inner
                .delete_created_before(created_before_unix_secs, limit)
                .await
        }
    }

    struct FailUntilReleasedRequestCandidateRepository {
        inner: InMemoryRequestCandidateRepository,
        failing: AtomicBool,
        attempts: AtomicUsize,
        first_attempt: tokio::sync::Notify,
    }

    impl Default for FailUntilReleasedRequestCandidateRepository {
        fn default() -> Self {
            Self {
                inner: InMemoryRequestCandidateRepository::default(),
                failing: AtomicBool::new(true),
                attempts: AtomicUsize::new(0),
                first_attempt: tokio::sync::Notify::new(),
            }
        }
    }

    #[async_trait::async_trait]
    impl RequestCandidateWriteRepository for FailUntilReleasedRequestCandidateRepository {
        async fn upsert(
            &self,
            candidate: UpsertRequestCandidateRecord,
        ) -> Result<StoredRequestCandidate, DataLayerError> {
            self.inner.upsert(candidate).await
        }

        async fn upsert_many(
            &self,
            candidates: Vec<UpsertRequestCandidateRecord>,
        ) -> Result<usize, DataLayerError> {
            if self.attempts.fetch_add(1, Ordering::AcqRel) == 0 {
                self.first_attempt.notify_one();
            }
            if self.failing.load(Ordering::Acquire) {
                return Err(DataLayerError::UnexpectedValue(
                    "injected request candidate write failure".to_string(),
                ));
            }
            let count = candidates.len();
            for candidate in candidates {
                self.inner.upsert(candidate).await?;
            }
            Ok(count)
        }

        async fn delete_created_before(
            &self,
            created_before_unix_secs: u64,
            limit: usize,
        ) -> Result<usize, DataLayerError> {
            self.inner
                .delete_created_before(created_before_unix_secs, limit)
                .await
        }
    }

    fn candidate(
        id: &str,
        status: RequestCandidateStatus,
        now_unix_ms: u64,
    ) -> UpsertRequestCandidateRecord {
        UpsertRequestCandidateRecord {
            id: id.to_string(),
            request_id: "request-1".to_string(),
            user_id: Some("user-1".to_string()),
            api_key_id: Some("api-key-1".to_string()),
            username: None,
            api_key_name: None,
            candidate_index: 0,
            retry_index: 0,
            provider_id: Some("provider-1".to_string()),
            endpoint_id: Some("endpoint-1".to_string()),
            key_id: Some("key-1".to_string()),
            status,
            skip_reason: None,
            is_cached: Some(false),
            status_code: None,
            error_type: None,
            error_message: None,
            latency_ms: None,
            concurrent_requests: None,
            extra_data: None,
            required_capabilities: None,
            created_at_unix_ms: Some(now_unix_ms),
            started_at_unix_ms: Some(now_unix_ms),
            finished_at_unix_ms: None,
        }
    }

    fn queue_config() -> RequestCandidateQueueConfig {
        RequestCandidateQueueConfig {
            mode: RequestCandidateWriteMode::Async,
            capacity: 8,
            batch_size: 1,
            db_batch_size: 1,
            flush_interval: Duration::from_secs(60),
            workers: 1,
            db_write_concurrency_limit: None,
            full_policy: RequestCandidateQueueFullPolicy::Sync,
        }
    }

    fn state_with_queue(
        read_repository: Arc<InMemoryRequestCandidateRepository>,
        write_repository: Arc<dyn RequestCandidateWriteRepository>,
        config: RequestCandidateQueueConfig,
    ) -> AppState {
        let mut state = AppState::new()
            .expect("app state should build")
            .with_data_state_for_tests(
                GatewayDataState::with_request_candidate_repository_for_tests(read_repository),
            );
        state.request_candidate_queue = Some(RequestCandidateQueueRuntime::spawn(
            write_repository,
            config,
        ));
        state
    }

    async fn scoped_rows(state: &AppState, since_unix_secs: u64) -> Vec<StoredRequestCandidate> {
        state
            .read_runtime_scoped_request_candidates_since(
                &["provider-1".to_string()],
                &["key-1".to_string()],
                &["api-key-1".to_string()],
                since_unix_secs,
            )
            .await
            .expect("runtime candidate read should succeed")
    }

    #[tokio::test]
    async fn queued_candidate_is_visible_before_repository_flush_completes() {
        let read_repository = Arc::new(InMemoryRequestCandidateRepository::default());
        let write_repository = Arc::new(BlockingRequestCandidateRepository::default());
        let mut state = AppState::new()
            .expect("app state should build")
            .with_data_state_for_tests(
                GatewayDataState::with_request_candidate_repository_for_tests(Arc::clone(
                    &read_repository,
                )),
            );
        let writer: Arc<dyn RequestCandidateWriteRepository> = write_repository.clone();
        state.request_candidate_queue =
            Some(RequestCandidateQueueRuntime::spawn(writer, queue_config()));
        let now_unix_ms = crate::clock::current_unix_ms();

        state
            .enqueue_request_candidate_status(candidate(
                "candidate-queued",
                RequestCandidateStatus::Pending,
                now_unix_ms,
            ))
            .await
            .expect("queue should accept candidate");
        tokio::time::timeout(
            Duration::from_secs(1),
            write_repository.write_started.notified(),
        )
        .await
        .expect("background write should start");

        assert!(read_repository
            .list_by_request_id("request-1")
            .await
            .expect("repository read should succeed")
            .is_empty());
        let rows = scoped_rows(&state, now_unix_ms / 1000).await;
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].status, RequestCandidateStatus::Pending);

        let request_rows = state
            .read_request_candidates_with_runtime_overlay("request-1")
            .await
            .expect("request-scoped candidate read should succeed");
        assert_eq!(request_rows.len(), 1);
        assert_eq!(request_rows[0].id, "candidate-queued");

        let mut plan = execution_plan_for_queued_candidate();
        let mut report_context = Some(serde_json::json!({
            "request_id": "request-1",
            "candidate_id": "candidate-queued",
            "candidate_index": 0,
            "retry_index": 0,
        }));
        let prepared =
            prepare_execution_request_candidate_slot(&state, &mut plan, &mut report_context)
                .await
                .expect("queued planner candidate should resolve as the execution slot");
        assert!(!prepared.has_pending_write());
        assert_eq!(plan.candidate_id.as_deref(), Some("candidate-queued"));
        assert_eq!(
            report_context
                .as_ref()
                .and_then(|context| context.get("candidate_id"))
                .and_then(serde_json::Value::as_str),
            Some("candidate-queued")
        );

        write_repository.release_write.notify_waiters();
    }

    #[tokio::test]
    async fn priority_fast_path_fallback_replaces_its_overlay_generation() {
        let write_repository = Arc::new(BlockingRequestCandidateRepository::default());
        let mut state = AppState::new().expect("app state should build");
        let writer: Arc<dyn RequestCandidateWriteRepository> = write_repository.clone();
        state.request_candidate_queue = Some(RequestCandidateQueueRuntime::spawn(
            writer,
            RequestCandidateQueueConfig {
                capacity: 1,
                batch_size: 1,
                db_batch_size: 1,
                flush_interval: Duration::from_secs(60),
                workers: 1,
                ..queue_config()
            },
        ));
        let now_unix_ms = crate::clock::current_unix_ms();
        let blocker = candidate(
            "candidate-blocker",
            RequestCandidateStatus::Pending,
            now_unix_ms,
        );
        state
            .enqueue_request_candidate_status(blocker)
            .await
            .expect("blocker should enqueue");
        tokio::time::timeout(
            Duration::from_secs(1),
            write_repository.write_started.notified(),
        )
        .await
        .expect("blocker persistence should start");

        let mut queued = candidate(
            "candidate-fallback",
            RequestCandidateStatus::Streaming,
            now_unix_ms.saturating_add(1),
        );
        queued.candidate_index = 1;
        let fallback = state
            .try_enqueue_request_candidate_status(queued)
            .expect_err("full priority lane should return the raw record");
        let fallback_slot = request_candidate_runtime_slot_key("request-1", 1, 0);
        assert_eq!(
            state
                .request_candidate_runtime_overlay
                .contribution_count_for_slot(&fallback_slot),
            0,
            "the failed fast-path publication must roll back before republishing"
        );

        let state_for_fallback = state.clone();
        let fallback_task = tokio::spawn(async move {
            state_for_fallback
                .enqueue_request_candidate_status(fallback)
                .await
                .expect("fallback enqueue should succeed")
        });
        tokio::task::yield_now().await;
        assert_eq!(
            state
                .request_candidate_runtime_overlay
                .contribution_count_for_slot(&fallback_slot),
            1
        );

        write_repository.release_write.notify_waiters();
        tokio::time::timeout(Duration::from_secs(2), fallback_task)
            .await
            .expect("fallback enqueue should resume")
            .expect("fallback task should complete");
        tokio::time::timeout(
            Duration::from_secs(1),
            write_repository.write_started.notified(),
        )
        .await
        .expect("fallback persistence should start");
        write_repository.release_write.notify_waiters();
        tokio::time::timeout(Duration::from_secs(2), async {
            while state
                .request_candidate_runtime_overlay
                .contains_key(&fallback_slot)
            {
                tokio::task::yield_now().await;
            }
        })
        .await
        .expect("successful DB ACK should clear the fallback overlay without a read reconcile");
    }

    #[tokio::test]
    async fn cancelling_priority_backpressure_rolls_back_the_overlay() {
        let write_repository = Arc::new(BlockingRequestCandidateRepository::default());
        let mut state = AppState::new().expect("app state should build");
        let writer: Arc<dyn RequestCandidateWriteRepository> = write_repository.clone();
        state.request_candidate_queue = Some(RequestCandidateQueueRuntime::spawn(
            writer,
            RequestCandidateQueueConfig {
                capacity: 1,
                batch_size: 1,
                db_batch_size: 1,
                flush_interval: Duration::from_secs(60),
                workers: 1,
                ..queue_config()
            },
        ));
        let now_unix_ms = crate::clock::current_unix_ms();
        state
            .enqueue_request_candidate_status(candidate(
                "candidate-blocker",
                RequestCandidateStatus::Pending,
                now_unix_ms,
            ))
            .await
            .expect("blocker should enqueue");
        tokio::time::timeout(
            Duration::from_secs(1),
            write_repository.write_started.notified(),
        )
        .await
        .expect("blocker persistence should start");

        let mut cancelled = candidate(
            "candidate-cancelled-enqueue",
            RequestCandidateStatus::Streaming,
            now_unix_ms.saturating_add(1),
        );
        cancelled.candidate_index = 1;
        let cancelled_slot = request_candidate_runtime_slot_key("request-1", 1, 0);
        let result = tokio::time::timeout(
            Duration::from_millis(20),
            state.enqueue_request_candidate_status(cancelled),
        )
        .await;
        assert!(
            result.is_err(),
            "priority enqueue should remain backpressured"
        );
        assert!(
            !state
                .request_candidate_runtime_overlay
                .contains_key(&cancelled_slot),
            "cancelling the enqueue future must roll back its unpublished DB state"
        );

        write_repository.release_write.notify_waiters();
    }

    #[tokio::test]
    async fn successful_queue_flush_acknowledges_overlay_without_a_read_reconcile() {
        let read_repository = Arc::new(InMemoryRequestCandidateRepository::default());
        let write_repository = Arc::new(BlockingRequestCandidateRepository::default());
        let writer: Arc<dyn RequestCandidateWriteRepository> = write_repository.clone();
        let state = state_with_queue(read_repository, writer, queue_config());
        let now_unix_ms = crate::clock::current_unix_ms();

        state
            .enqueue_request_candidate_status(candidate(
                "candidate-acked",
                RequestCandidateStatus::Pending,
                now_unix_ms,
            ))
            .await
            .expect("queue should accept candidate");
        tokio::time::timeout(
            Duration::from_secs(1),
            write_repository.write_started.notified(),
        )
        .await
        .expect("background write should start");
        assert_eq!(
            state.request_candidate_runtime_overlay.contribution_count(),
            1
        );

        write_repository.release_write.notify_waiters();
        tokio::time::timeout(Duration::from_secs(1), async {
            while state.request_candidate_runtime_overlay.contribution_count() != 0 {
                tokio::time::sleep(Duration::from_millis(5)).await;
            }
        })
        .await
        .expect("successful flush should acknowledge its overlay generation");
        assert!(state.request_candidate_runtime_overlay.is_empty());
    }

    #[tokio::test]
    async fn cancelling_sync_fallback_rolls_back_only_its_overlay_generation() {
        let read_repository = Arc::new(InMemoryRequestCandidateRepository::default());
        let write_repository = Arc::new(BlockingRequestCandidateRepository::default());
        let writer: Arc<dyn RequestCandidateWriteRepository> = write_repository.clone();
        let mut config = queue_config();
        config.capacity = 1;
        config.full_policy = RequestCandidateQueueFullPolicy::Sync;
        let state = Arc::new(state_with_queue(read_repository, writer, config));
        let now_unix_ms = crate::clock::current_unix_ms();

        state
            .enqueue_request_candidate_status(candidate(
                "candidate-first",
                RequestCandidateStatus::Available,
                now_unix_ms,
            ))
            .await
            .expect("first candidate should enter the queue");
        tokio::time::timeout(
            Duration::from_secs(1),
            write_repository.write_started.notified(),
        )
        .await
        .expect("first background write should start");

        let mut second = candidate(
            "candidate-second",
            RequestCandidateStatus::Available,
            now_unix_ms.saturating_add(1),
        );
        second.candidate_index = 1;
        let state_for_second = Arc::clone(&state);
        let second_task = tokio::spawn(async move {
            state_for_second
                .enqueue_request_candidate_status(second)
                .await
        });
        tokio::time::timeout(
            Duration::from_secs(1),
            write_repository.write_started.notified(),
        )
        .await
        .expect("sync fallback write should start");
        second_task.abort();
        let _ = second_task.await;

        let second_slot = request_candidate_runtime_slot_key("request-1", 1, 0);
        assert!(!state
            .request_candidate_runtime_overlay
            .contains_key(&second_slot));
        let first_slot = request_candidate_runtime_slot_key("request-1", 0, 0);
        assert!(state
            .request_candidate_runtime_overlay
            .contains_key(&first_slot));
        write_repository.release_write.notify_waiters();
    }

    #[tokio::test]
    async fn cancelling_priority_backpressure_rolls_back_unaccepted_overlay_generation() {
        let read_repository = Arc::new(InMemoryRequestCandidateRepository::default());
        let write_repository = Arc::new(BlockingRequestCandidateRepository::default());
        let writer: Arc<dyn RequestCandidateWriteRepository> = write_repository.clone();
        let mut config = queue_config();
        config.capacity = 1;
        let state = Arc::new(state_with_queue(read_repository, writer, config));
        let now_unix_ms = crate::clock::current_unix_ms();

        state
            .enqueue_request_candidate_status(candidate(
                "candidate-first",
                RequestCandidateStatus::Pending,
                now_unix_ms,
            ))
            .await
            .expect("first lifecycle candidate should enter admission");
        tokio::time::timeout(
            Duration::from_secs(1),
            write_repository.write_started.notified(),
        )
        .await
        .expect("first lifecycle write should start");

        let mut second = candidate(
            "candidate-second",
            RequestCandidateStatus::Pending,
            now_unix_ms.saturating_add(1),
        );
        second.candidate_index = 1;
        let state_for_second = Arc::clone(&state);
        let second_task = tokio::spawn(async move {
            state_for_second
                .enqueue_request_candidate_status(second)
                .await
        });
        tokio::time::sleep(Duration::from_millis(20)).await;
        assert!(!second_task.is_finished());
        assert_eq!(write_repository.write_calls.load(Ordering::Acquire), 1);
        second_task.abort();
        let _ = second_task.await;

        let second_slot = request_candidate_runtime_slot_key("request-1", 1, 0);
        assert!(!state
            .request_candidate_runtime_overlay
            .contains_key(&second_slot));
        let first_slot = request_candidate_runtime_slot_key("request-1", 0, 0);
        assert!(state
            .request_candidate_runtime_overlay
            .contains_key(&first_slot));
        write_repository.release_write.notify_waiters();
    }

    #[tokio::test]
    async fn failed_compacted_flush_retains_all_overlay_leases_until_recovery() {
        let read_repository = Arc::new(InMemoryRequestCandidateRepository::default());
        let write_repository = Arc::new(FailUntilReleasedRequestCandidateRepository::default());
        let writer: Arc<dyn RequestCandidateWriteRepository> = write_repository.clone();
        let mut config = queue_config();
        config.batch_size = 8;
        config.flush_interval = Duration::from_millis(5);
        let state = state_with_queue(read_repository, writer, config);
        let now_unix_ms = crate::clock::current_unix_ms();

        state
            .enqueue_request_candidate_status(candidate(
                "candidate-pending",
                RequestCandidateStatus::Pending,
                now_unix_ms,
            ))
            .await
            .expect("pending update should enqueue");
        state
            .enqueue_request_candidate_status(candidate(
                "candidate-pending-newer",
                RequestCandidateStatus::Pending,
                now_unix_ms.saturating_add(1),
            ))
            .await
            .expect("newer pending update should enqueue");
        tokio::time::timeout(
            Duration::from_secs(1),
            write_repository.first_attempt.notified(),
        )
        .await
        .expect("worker should observe injected DB failure");

        assert_eq!(
            state.request_candidate_runtime_overlay.contribution_count(),
            2,
            "failed compacted writes must retain every source generation"
        );
        write_repository.failing.store(false, Ordering::Release);
        tokio::time::timeout(Duration::from_secs(2), async {
            while state.request_candidate_runtime_overlay.contribution_count() != 0 {
                tokio::time::sleep(Duration::from_millis(5)).await;
            }
        })
        .await
        .expect("recovered flush should acknowledge all compacted generations");
    }

    #[tokio::test]
    async fn normal_drop_policy_rolls_back_overlay_and_reports_no_write() {
        let read_repository = Arc::new(InMemoryRequestCandidateRepository::default());
        let write_repository = Arc::new(BlockingRequestCandidateRepository::default());
        let writer: Arc<dyn RequestCandidateWriteRepository> = write_repository.clone();
        let mut config = queue_config();
        config.capacity = 1;
        config.full_policy = RequestCandidateQueueFullPolicy::Drop;
        let state = state_with_queue(read_repository, writer, config);
        let now_unix_ms = crate::clock::current_unix_ms();

        state
            .enqueue_request_candidate_status(candidate(
                "candidate-first",
                RequestCandidateStatus::Available,
                now_unix_ms,
            ))
            .await
            .expect("first candidate should enter the queue");
        tokio::time::timeout(
            Duration::from_secs(1),
            write_repository.write_started.notified(),
        )
        .await
        .expect("first background write should start");

        let mut dropped = candidate(
            "candidate-dropped",
            RequestCandidateStatus::Available,
            now_unix_ms.saturating_add(1),
        );
        dropped.candidate_index = 1;
        let outcome = state
            .enqueue_request_candidate_status(dropped)
            .await
            .expect("drop policy is a best-effort outcome");

        assert_eq!(outcome, None);
        let dropped_slot = request_candidate_runtime_slot_key("request-1", 1, 0);
        assert!(!state
            .request_candidate_runtime_overlay
            .contains_key(&dropped_slot));
        write_repository.release_write.notify_waiters();
    }

    #[tokio::test]
    async fn request_scoped_overlay_keeps_candidate_slots_separate() {
        let now_unix_ms = crate::clock::current_unix_ms();
        let repository = Arc::new(InMemoryRequestCandidateRepository::default());
        repository
            .upsert(candidate(
                "candidate-db-slot-zero",
                RequestCandidateStatus::Failed,
                now_unix_ms,
            ))
            .await
            .expect("seed should succeed");
        let state = AppState::new()
            .expect("app state should build")
            .with_data_state_for_tests(
                GatewayDataState::with_request_candidate_repository_for_tests(repository),
            );
        let mut retry = candidate(
            "candidate-overlay-slot-one",
            RequestCandidateStatus::Pending,
            now_unix_ms.saturating_add(1),
        );
        retry.candidate_index = 1;
        retry.key_id = Some("key-2".to_string());
        state.remember_stored_request_candidate_runtime_overlay(
            super::stored_request_candidate_from_upsert(&retry)
                .expect("retry candidate should materialize"),
        );

        let rows = state
            .read_request_candidates_with_runtime_overlay("request-1")
            .await
            .expect("request-scoped candidate read should succeed");
        assert_eq!(rows.len(), 2);
        assert_eq!(rows[0].candidate_index, 0);
        assert_eq!(rows[0].id, "candidate-db-slot-zero");
        assert_eq!(rows[1].candidate_index, 1);
        assert_eq!(rows[1].id, "candidate-overlay-slot-one");
        assert_eq!(rows[1].key_id.as_deref(), Some("key-2"));
    }

    #[tokio::test]
    async fn runtime_overlay_terminal_status_overrides_stale_repository_row() {
        let now_unix_ms = crate::clock::current_unix_ms();
        let repository = Arc::new(InMemoryRequestCandidateRepository::default());
        repository
            .upsert(candidate(
                "candidate-db",
                RequestCandidateStatus::Pending,
                now_unix_ms,
            ))
            .await
            .expect("seed should succeed");
        let state = AppState::new()
            .expect("app state should build")
            .with_data_state_for_tests(
                GatewayDataState::with_request_candidate_repository_for_tests(repository),
            );
        let mut terminal = candidate(
            "candidate-overlay",
            RequestCandidateStatus::Success,
            now_unix_ms,
        );
        terminal.finished_at_unix_ms = Some(now_unix_ms.saturating_add(10));
        state.remember_stored_request_candidate_runtime_overlay(
            super::stored_request_candidate_from_upsert(&terminal)
                .expect("terminal candidate should materialize"),
        );

        let rows = scoped_rows(&state, now_unix_ms / 1000).await;
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].status, RequestCandidateStatus::Success);
        assert_eq!(rows[0].id, "candidate-db");
    }

    #[tokio::test]
    async fn late_active_overlay_cannot_revive_terminal_status() {
        let now_unix_ms = crate::clock::current_unix_ms();
        let repository = Arc::new(InMemoryRequestCandidateRepository::default());
        repository
            .upsert(candidate(
                "candidate-db",
                RequestCandidateStatus::Success,
                now_unix_ms,
            ))
            .await
            .expect("seed should succeed");
        let state = AppState::new()
            .expect("app state should build")
            .with_data_state_for_tests(
                GatewayDataState::with_request_candidate_repository_for_tests(repository),
            );
        let mut active = candidate(
            "candidate-overlay",
            RequestCandidateStatus::Streaming,
            now_unix_ms.saturating_add(10),
        );
        active.started_at_unix_ms = None;
        state.remember_stored_request_candidate_runtime_overlay(
            super::stored_request_candidate_from_upsert(&active)
                .expect("active candidate should materialize"),
        );

        let rows = scoped_rows(&state, now_unix_ms / 1000).await;
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].status, RequestCandidateStatus::Success);
        let slot = request_candidate_runtime_slot_key("request-1", 0, 0);
        assert!(
            !state.request_candidate_runtime_overlay.contains_key(&slot),
            "a persisted terminal state should evict a covered late active overlay"
        );
    }

    #[tokio::test]
    async fn persisted_candidate_evicts_an_identical_runtime_overlay() {
        let now_unix_ms = crate::clock::current_unix_ms();
        let repository = Arc::new(InMemoryRequestCandidateRepository::default());
        let stored = repository
            .upsert(candidate(
                "candidate-persisted",
                RequestCandidateStatus::Streaming,
                now_unix_ms,
            ))
            .await
            .expect("seed should succeed");
        let state = AppState::new()
            .expect("app state should build")
            .with_data_state_for_tests(
                GatewayDataState::with_request_candidate_repository_for_tests(repository),
            );
        state.remember_stored_request_candidate_runtime_overlay(stored);

        let rows = state
            .read_request_candidates_with_runtime_overlay("request-1")
            .await
            .expect("request-scoped candidate read should succeed");

        assert_eq!(rows.len(), 1);
        assert!(state.request_candidate_runtime_overlay.is_empty());
    }

    #[tokio::test]
    async fn newer_runtime_overlay_survives_a_stale_repository_read() {
        let now_unix_ms = crate::clock::current_unix_ms();
        let repository = Arc::new(InMemoryRequestCandidateRepository::default());
        repository
            .upsert(candidate(
                "candidate-shared",
                RequestCandidateStatus::Pending,
                now_unix_ms,
            ))
            .await
            .expect("seed should succeed");
        let state = AppState::new()
            .expect("app state should build")
            .with_data_state_for_tests(
                GatewayDataState::with_request_candidate_repository_for_tests(repository),
            );
        state.remember_stored_request_candidate_runtime_overlay(
            super::stored_request_candidate_from_upsert(&candidate(
                "candidate-shared",
                RequestCandidateStatus::Streaming,
                now_unix_ms.saturating_add(10),
            ))
            .expect("streaming candidate should materialize"),
        );

        let rows = state
            .read_request_candidates_with_runtime_overlay("request-1")
            .await
            .expect("request-scoped candidate read should succeed");

        assert_eq!(rows[0].status, RequestCandidateStatus::Streaming);
        let slot = request_candidate_runtime_slot_key("request-1", 0, 0);
        assert!(state.request_candidate_runtime_overlay.contains_key(&slot));
    }

    #[test]
    fn rolling_back_one_generation_preserves_a_concurrent_slot_update() {
        let now_unix_ms = crate::clock::current_unix_ms();
        let overlay = super::RequestCandidateRuntimeOverlay::default();
        let mut pending = super::stored_request_candidate_from_upsert(&candidate(
            "candidate-pending",
            RequestCandidateStatus::Pending,
            now_unix_ms,
        ))
        .expect("pending candidate should materialize");
        pending.provider_id = None;
        let failed_publish = overlay.publish_for_test(pending);

        let mut streaming = super::stored_request_candidate_from_upsert(&candidate(
            "candidate-streaming",
            RequestCandidateStatus::Streaming,
            now_unix_ms.saturating_add(1),
        ))
        .expect("streaming candidate should materialize");
        streaming.endpoint_id = Some("endpoint-new".to_string());
        overlay.publish_for_test(streaming);

        overlay.rollback_for_test(failed_publish);

        let slot = request_candidate_runtime_slot_key("request-1", 0, 0);
        let remaining = overlay
            .get(&slot)
            .expect("the independent generation must remain");
        assert_eq!(remaining.status, RequestCandidateStatus::Streaming);
        assert_eq!(remaining.provider_id.as_deref(), Some("provider-1"));
        assert_eq!(remaining.endpoint_id.as_deref(), Some("endpoint-new"));
    }

    #[test]
    fn committed_patch_preserves_none_as_inherit_while_a_newer_generation_is_pending() {
        let now_unix_ms = crate::clock::current_unix_ms();
        let overlay = super::RequestCandidateRuntimeOverlay::default();
        let mut persisted_record = candidate(
            "candidate-persisted",
            RequestCandidateStatus::Pending,
            now_unix_ms,
        );
        persisted_record.is_cached = Some(true);
        let persisted = super::stored_request_candidate_from_upsert(&persisted_record)
            .expect("persisted candidate should materialize");

        let mut committed_patch = candidate(
            "candidate-patch",
            RequestCandidateStatus::Streaming,
            now_unix_ms.saturating_add(100),
        );
        committed_patch.is_cached = None;
        committed_patch.created_at_unix_ms = None;
        committed_patch.started_at_unix_ms = Some(now_unix_ms.saturating_add(10));
        let committed_candidate = super::stored_request_candidate_from_upsert(&committed_patch)
            .expect("committed patch should materialize");
        let committed = overlay.publish(committed_candidate, committed_patch);

        let mut pending_patch = candidate(
            "candidate-pending",
            RequestCandidateStatus::Streaming,
            now_unix_ms.saturating_add(200),
        );
        pending_patch.is_cached = None;
        pending_patch.created_at_unix_ms = None;
        pending_patch.started_at_unix_ms = None;
        pending_patch.extra_data = Some(serde_json::json!({"pending": true}));
        let pending_candidate = super::stored_request_candidate_from_upsert(&pending_patch)
            .expect("pending patch should materialize");
        overlay.publish(pending_candidate, pending_patch);
        overlay.reconcile_for_test(&[persisted]);
        overlay.acknowledge_for_test(committed);

        let slot = request_candidate_runtime_slot_key("request-1", 0, 0);
        let effective = overlay
            .get(&slot)
            .expect("pending generation should remain");
        assert!(effective.is_cached);
        assert_eq!(effective.created_at_unix_ms, now_unix_ms);
        assert_eq!(effective.started_at_unix_ms, Some(now_unix_ms + 10));
        assert_eq!(
            effective.extra_data,
            Some(serde_json::json!({"pending": true}))
        );
    }

    #[test]
    fn active_read_guard_retains_a_newly_committed_overlay_prefix() {
        let now_unix_ms = crate::clock::current_unix_ms();
        let overlay = Arc::new(super::RequestCandidateRuntimeOverlay::default());
        let read_guard = overlay.begin_read();

        let mut committed_patch = candidate(
            "candidate-new",
            RequestCandidateStatus::Streaming,
            now_unix_ms.saturating_add(1),
        );
        committed_patch.extra_data = Some(serde_json::json!({"committed": true}));
        let committed_candidate = super::stored_request_candidate_from_upsert(&committed_patch)
            .expect("committed candidate should materialize");
        let committed = overlay.publish(committed_candidate, committed_patch);
        let mut pending_patch = candidate(
            "candidate-pending",
            RequestCandidateStatus::Streaming,
            now_unix_ms.saturating_add(2),
        );
        pending_patch.extra_data = Some(serde_json::json!({"pending": true}));
        let pending_candidate = super::stored_request_candidate_from_upsert(&pending_patch)
            .expect("pending candidate should materialize");
        overlay.publish(pending_candidate, pending_patch);
        overlay.acknowledge_for_test(committed);

        let slot = request_candidate_runtime_slot_key("request-1", 0, 0);
        let effective = overlay.get(&slot).expect("pending overlay should remain");
        assert_eq!(
            effective.extra_data,
            Some(serde_json::json!({"committed": true, "pending": true}))
        );
        drop(read_guard);
    }

    #[test]
    fn stale_read_reconcile_preserves_a_patch_acknowledged_during_the_read() {
        let now_unix_ms = crate::clock::current_unix_ms();
        let overlay = Arc::new(super::RequestCandidateRuntimeOverlay::default());
        let mut persisted_patch = candidate(
            "candidate-persisted",
            RequestCandidateStatus::Pending,
            now_unix_ms,
        );
        persisted_patch.extra_data = Some(serde_json::json!({"persisted": true}));
        let persisted = super::stored_request_candidate_from_upsert(&persisted_patch)
            .expect("persisted candidate should materialize");
        let read_guard = overlay.begin_read();

        let mut committed_patch = candidate(
            "candidate-committed",
            RequestCandidateStatus::Streaming,
            now_unix_ms.saturating_add(1),
        );
        committed_patch.extra_data = Some(serde_json::json!({"committed": true}));
        let committed_candidate = super::stored_request_candidate_from_upsert(&committed_patch)
            .expect("committed candidate should materialize");
        let committed = overlay.publish(committed_candidate, committed_patch);
        overlay.acknowledge_for_test(committed);

        overlay.reconcile_for_test(std::slice::from_ref(&persisted));
        let runtime_overlay = overlay.candidates_for_request("request-1");
        let rows =
            super::merge_request_candidates_with_runtime_overlay(vec![persisted], runtime_overlay);

        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].status, RequestCandidateStatus::Streaming);
        assert_eq!(
            rows[0].extra_data,
            Some(serde_json::json!({"persisted": true, "committed": true}))
        );
        drop(read_guard);
        assert!(overlay.is_empty());
    }

    #[test]
    fn newer_read_cannot_reconcile_past_an_older_active_read() {
        let now_unix_ms = crate::clock::current_unix_ms();
        let overlay = Arc::new(super::RequestCandidateRuntimeOverlay::default());
        let mut persisted_patch = candidate(
            "candidate-persisted",
            RequestCandidateStatus::Pending,
            now_unix_ms,
        );
        persisted_patch.extra_data = Some(serde_json::json!({"persisted": true}));
        let persisted = super::stored_request_candidate_from_upsert(&persisted_patch)
            .expect("persisted candidate should materialize");
        let older_read = overlay.begin_read();

        let mut committed_patch = candidate(
            "candidate-committed",
            RequestCandidateStatus::Streaming,
            now_unix_ms.saturating_add(1),
        );
        committed_patch.extra_data = Some(serde_json::json!({"committed": true}));
        let committed_candidate = super::stored_request_candidate_from_upsert(&committed_patch)
            .expect("committed candidate should materialize");
        let committed = overlay.publish(committed_candidate, committed_patch);
        overlay.acknowledge_for_test(committed);
        let newer_read = overlay.begin_read();

        overlay.reconcile_for_test(std::slice::from_ref(&persisted));

        let slot = request_candidate_runtime_slot_key("request-1", 0, 0);
        assert_eq!(
            overlay
                .get(&slot)
                .expect("older read must retain the committed patch")
                .status,
            RequestCandidateStatus::Streaming
        );
        drop(newer_read);
        assert!(overlay.contains_key(&slot));
        drop(older_read);
        assert!(!overlay.contains_key(&slot));
    }

    #[test]
    fn final_ack_stays_visible_until_the_older_read_guard_finishes() {
        let now_unix_ms = crate::clock::current_unix_ms();
        let overlay = Arc::new(super::RequestCandidateRuntimeOverlay::default());
        let read_guard = overlay.begin_read();
        let patch = candidate(
            "candidate-committed",
            RequestCandidateStatus::Pending,
            now_unix_ms,
        );
        let stored = super::stored_request_candidate_from_upsert(&patch)
            .expect("candidate should materialize");
        let published = overlay.publish(stored, patch);
        overlay.acknowledge_for_test(published);

        let slot = request_candidate_runtime_slot_key("request-1", 0, 0);
        assert!(overlay.contains_key(&slot));
        drop(read_guard);
        assert!(!overlay.contains_key(&slot));
    }

    #[test]
    fn reconcile_consumes_only_the_contiguous_committed_prefix() {
        let now_unix_ms = crate::clock::current_unix_ms();
        let overlay = Arc::new(super::RequestCandidateRuntimeOverlay::default());
        let mut first_patch = candidate(
            "candidate-first",
            RequestCandidateStatus::Pending,
            now_unix_ms,
        );
        first_patch.extra_data = Some(serde_json::json!({"value": 1}));
        let first_stored = super::stored_request_candidate_from_upsert(&first_patch)
            .expect("first candidate should materialize");
        let first = overlay.publish(first_stored, first_patch);
        let mut second_patch = candidate(
            "candidate-second",
            RequestCandidateStatus::Streaming,
            now_unix_ms.saturating_add(1),
        );
        second_patch.extra_data = Some(serde_json::json!({"value": 2}));
        let second_stored = super::stored_request_candidate_from_upsert(&second_patch)
            .expect("second candidate should materialize");
        let second = overlay.publish(second_stored, second_patch);
        let mut third_patch = candidate(
            "candidate-third",
            RequestCandidateStatus::Streaming,
            now_unix_ms.saturating_add(2),
        );
        third_patch.extra_data = Some(serde_json::json!({"pending": true}));
        let third_stored = super::stored_request_candidate_from_upsert(&third_patch)
            .expect("third candidate should materialize");
        overlay.publish(third_stored, third_patch);
        overlay.acknowledge_for_test(first);
        overlay.acknowledge_for_test(second);

        let mut persisted_patch = candidate(
            "candidate-persisted",
            RequestCandidateStatus::Streaming,
            now_unix_ms,
        );
        persisted_patch.extra_data = Some(serde_json::json!({"value": 2}));
        let persisted = super::stored_request_candidate_from_upsert(&persisted_patch)
            .expect("persisted candidate should materialize");
        overlay.reconcile_for_test(&[persisted]);

        let slot = request_candidate_runtime_slot_key("request-1", 0, 0);
        let effective = overlay.get(&slot).expect("third generation should remain");
        assert_eq!(
            effective.extra_data,
            Some(serde_json::json!({"value": 2, "pending": true}))
        );
        assert_eq!(overlay.contribution_count_for_slot(&slot), 1);
    }

    #[test]
    fn request_and_runtime_scope_indexes_only_return_matching_overlay_slots() {
        let now_unix_ms = crate::clock::current_unix_ms();
        let overlay = super::RequestCandidateRuntimeOverlay::default();
        let first = super::stored_request_candidate_from_upsert(&candidate(
            "candidate-first",
            RequestCandidateStatus::Pending,
            now_unix_ms,
        ))
        .expect("first candidate should materialize");
        overlay.publish_for_test(first);

        let mut second_record = candidate(
            "candidate-second",
            RequestCandidateStatus::Pending,
            now_unix_ms,
        );
        second_record.request_id = "request-2".to_string();
        second_record.provider_id = Some("provider-2".to_string());
        second_record.key_id = Some("key-2".to_string());
        second_record.api_key_id = Some("api-key-2".to_string());
        let second = super::stored_request_candidate_from_upsert(&second_record)
            .expect("second candidate should materialize");
        overlay.publish_for_test(second);

        let request_rows = overlay.candidates_for_request("request-1");
        assert_eq!(request_rows.len(), 1);
        assert_eq!(request_rows[0].1.effective.request_id, "request-1");

        let scoped_rows =
            overlay.candidates_for_runtime_scopes(&["provider-2".to_string()], &[], &[]);
        assert_eq!(scoped_rows.len(), 1);
        assert_eq!(scoped_rows[0].1.effective.request_id, "request-2");
    }

    #[tokio::test]
    async fn late_active_update_cannot_revive_terminal_overlay_status() {
        let now_unix_ms = crate::clock::current_unix_ms();
        let repository = Arc::new(InMemoryRequestCandidateRepository::default());
        let state = AppState::new()
            .expect("app state should build")
            .with_data_state_for_tests(
                GatewayDataState::with_request_candidate_repository_for_tests(repository),
            );
        state.remember_stored_request_candidate_runtime_overlay(
            super::stored_request_candidate_from_upsert(&candidate(
                "candidate-terminal",
                RequestCandidateStatus::Failed,
                now_unix_ms,
            ))
            .expect("terminal candidate should materialize"),
        );
        state.remember_stored_request_candidate_runtime_overlay(
            super::stored_request_candidate_from_upsert(&candidate(
                "candidate-late-active",
                RequestCandidateStatus::Pending,
                now_unix_ms.saturating_add(10),
            ))
            .expect("active candidate should materialize"),
        );

        let rows = scoped_rows(&state, now_unix_ms / 1000).await;
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].status, RequestCandidateStatus::Failed);
        let slot = request_candidate_runtime_slot_key("request-1", 0, 0);
        assert_eq!(
            state
                .request_candidate_runtime_overlay
                .get(&slot)
                .expect("overlay should contain candidate")
                .status,
            RequestCandidateStatus::Failed
        );
    }
}
