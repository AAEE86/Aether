use super::{
    provider_transport_snapshot_looks_refreshed, AppState, CachedProviderTransportSnapshot,
    GatewayError, ProviderTransportSnapshotCacheKey, PROVIDER_TRANSPORT_SNAPSHOT_CACHE_MAX_ENTRIES,
    PROVIDER_TRANSPORT_SNAPSHOT_CACHE_TTL,
};

use super::super::provider_transport;
use std::time::Duration;

use aether_crypto::encrypt_python_fernet_plaintext;

impl AppState {
    pub(in crate::gateway) fn clear_provider_transport_snapshot_cache(&self) {
        self.provider_transport_snapshot_cache
            .lock()
            .expect("provider transport snapshot cache should lock")
            .clear();
    }

    fn get_cached_provider_transport_snapshot(
        &self,
        cache_key: &ProviderTransportSnapshotCacheKey,
    ) -> Option<provider_transport::GatewayProviderTransportSnapshot> {
        let mut cache = self
            .provider_transport_snapshot_cache
            .lock()
            .expect("provider transport snapshot cache should lock");
        let cached = cache.get(cache_key).cloned()?;
        if cached.loaded_at.elapsed() <= PROVIDER_TRANSPORT_SNAPSHOT_CACHE_TTL {
            return Some(cached.snapshot);
        }
        cache.remove(cache_key);
        None
    }

    fn put_cached_provider_transport_snapshot(
        &self,
        cache_key: ProviderTransportSnapshotCacheKey,
        snapshot: provider_transport::GatewayProviderTransportSnapshot,
    ) {
        let mut cache = self
            .provider_transport_snapshot_cache
            .lock()
            .expect("provider transport snapshot cache should lock");
        if cache.len() >= PROVIDER_TRANSPORT_SNAPSHOT_CACHE_MAX_ENTRIES {
            cache.retain(|_, entry| {
                entry.loaded_at.elapsed() <= PROVIDER_TRANSPORT_SNAPSHOT_CACHE_TTL
            });
            if cache.len() >= PROVIDER_TRANSPORT_SNAPSHOT_CACHE_MAX_ENTRIES {
                cache.clear();
            }
        }
        cache.insert(
            cache_key,
            CachedProviderTransportSnapshot {
                loaded_at: std::time::Instant::now(),
                snapshot,
            },
        );
    }

    async fn read_provider_transport_snapshot_uncached(
        &self,
        provider_id: &str,
        endpoint_id: &str,
        key_id: &str,
    ) -> Result<Option<crate::gateway::gateway_data::GatewayProviderTransportSnapshot>, GatewayError>
    {
        self.data
            .read_provider_transport_snapshot(provider_id, endpoint_id, key_id)
            .await
            .map_err(|err| GatewayError::Internal(err.to_string()))
    }

    pub(crate) async fn list_enabled_oauth_module_providers(
        &self,
    ) -> Result<
        Vec<aether_data::repository::auth_modules::StoredOAuthProviderModuleConfig>,
        GatewayError,
    > {
        self.data
            .list_enabled_oauth_module_providers()
            .await
            .map_err(|err| GatewayError::Internal(err.to_string()))
    }

    pub(crate) async fn get_ldap_module_config(
        &self,
    ) -> Result<Option<aether_data::repository::auth_modules::StoredLdapModuleConfig>, GatewayError>
    {
        self.data
            .get_ldap_module_config()
            .await
            .map_err(|err| GatewayError::Internal(err.to_string()))
    }

    pub(crate) async fn upsert_ldap_module_config(
        &self,
        config: &aether_data::repository::auth_modules::StoredLdapModuleConfig,
    ) -> Result<Option<aether_data::repository::auth_modules::StoredLdapModuleConfig>, GatewayError>
    {
        self.data
            .upsert_ldap_module_config(config)
            .await
            .map_err(|err| GatewayError::Internal(err.to_string()))
    }

    pub(crate) async fn count_active_local_admin_users_with_valid_password(
        &self,
    ) -> Result<u64, GatewayError> {
        self.data
            .count_active_local_admin_users_with_valid_password()
            .await
            .map_err(|err| GatewayError::Internal(err.to_string()))
    }

    pub(crate) async fn list_oauth_provider_configs(
        &self,
    ) -> Result<
        Vec<aether_data::repository::oauth_providers::StoredOAuthProviderConfig>,
        GatewayError,
    > {
        self.data
            .list_oauth_provider_configs()
            .await
            .map_err(|err| GatewayError::Internal(err.to_string()))
    }

    pub(crate) async fn get_oauth_provider_config(
        &self,
        provider_type: &str,
    ) -> Result<
        Option<aether_data::repository::oauth_providers::StoredOAuthProviderConfig>,
        GatewayError,
    > {
        self.data
            .get_oauth_provider_config(provider_type)
            .await
            .map_err(|err| GatewayError::Internal(err.to_string()))
    }

    pub(crate) async fn count_locked_users_if_oauth_provider_disabled(
        &self,
        provider_type: &str,
        ldap_exclusive: bool,
    ) -> Result<usize, GatewayError> {
        self.data
            .count_locked_users_if_oauth_provider_disabled(provider_type, ldap_exclusive)
            .await
            .map_err(|err| GatewayError::Internal(err.to_string()))
    }

    pub(crate) async fn upsert_oauth_provider_config(
        &self,
        record: &aether_data::repository::oauth_providers::UpsertOAuthProviderConfigRecord,
    ) -> Result<
        Option<aether_data::repository::oauth_providers::StoredOAuthProviderConfig>,
        GatewayError,
    > {
        self.data
            .upsert_oauth_provider_config(record)
            .await
            .map_err(|err| GatewayError::Internal(err.to_string()))
    }

    pub(crate) async fn delete_oauth_provider_config(
        &self,
        provider_type: &str,
    ) -> Result<bool, GatewayError> {
        self.data
            .delete_oauth_provider_config(provider_type)
            .await
            .map_err(|err| GatewayError::Internal(err.to_string()))
    }

    pub(crate) fn encryption_key(&self) -> Option<&str> {
        self.data.encryption_key()
    }

    pub(crate) fn has_auth_module_writer(&self) -> bool {
        self.data.has_auth_module_writer()
    }

    pub(crate) fn provider_oauth_token_url(
        &self,
        _provider_type: &str,
        default_token_url: &str,
    ) -> String {
        #[cfg(test)]
        {
            if let Some(value) = self
                .provider_oauth_token_url_overrides
                .lock()
                .expect("provider oauth token url overrides should lock")
                .get(_provider_type.trim())
                .cloned()
            {
                return value;
            }
        }

        default_token_url.to_string()
    }

    pub(crate) fn save_provider_oauth_state_for_tests(&self, _key: &str, _value: &str) -> bool {
        #[cfg(test)]
        {
            if let Some(store) = self.provider_oauth_state_store.as_ref() {
                store
                    .lock()
                    .expect("provider oauth state store should lock")
                    .insert(_key.to_string(), _value.to_string());
                return true;
            }
        }

        false
    }

    pub(crate) fn take_provider_oauth_state_for_tests(&self, _key: &str) -> Option<String> {
        #[cfg(test)]
        {
            return self.provider_oauth_state_store.as_ref().and_then(|store| {
                store
                    .lock()
                    .expect("provider oauth state store should lock")
                    .remove(_key)
            });
        }

        #[allow(unreachable_code)]
        None
    }

    pub(crate) fn save_provider_oauth_device_session_for_tests(
        &self,
        _key: &str,
        _value: &str,
    ) -> bool {
        #[cfg(test)]
        {
            if let Some(store) = self.provider_oauth_device_session_store.as_ref() {
                store
                    .lock()
                    .expect("provider oauth device session store should lock")
                    .insert(_key.to_string(), _value.to_string());
                return true;
            }
        }

        false
    }

    pub(crate) fn load_provider_oauth_device_session_for_tests(
        &self,
        _key: &str,
    ) -> Option<String> {
        #[cfg(test)]
        {
            return self
                .provider_oauth_device_session_store
                .as_ref()
                .and_then(|store| {
                    store
                        .lock()
                        .expect("provider oauth device session store should lock")
                        .get(_key)
                        .cloned()
                });
        }

        #[allow(unreachable_code)]
        None
    }

    pub(crate) fn save_provider_oauth_batch_task_for_tests(
        &self,
        _key: &str,
        _value: &str,
    ) -> bool {
        #[cfg(test)]
        {
            if let Some(store) = self.provider_oauth_batch_task_store.as_ref() {
                store
                    .lock()
                    .expect("provider oauth batch task store should lock")
                    .insert(_key.to_string(), _value.to_string());
                return true;
            }
        }

        false
    }

    pub(crate) fn load_provider_oauth_batch_task_for_tests(&self, _key: &str) -> Option<String> {
        #[cfg(test)]
        {
            return self
                .provider_oauth_batch_task_store
                .as_ref()
                .and_then(|store| {
                    store
                        .lock()
                        .expect("provider oauth batch task store should lock")
                        .get(_key)
                        .cloned()
                });
        }

        #[allow(unreachable_code)]
        None
    }

    pub(crate) async fn read_provider_transport_snapshot(
        &self,
        provider_id: &str,
        endpoint_id: &str,
        key_id: &str,
    ) -> Result<Option<crate::gateway::gateway_data::GatewayProviderTransportSnapshot>, GatewayError>
    {
        let Some(cache_key) =
            ProviderTransportSnapshotCacheKey::new(provider_id, endpoint_id, key_id)
        else {
            return self
                .read_provider_transport_snapshot_uncached(provider_id, endpoint_id, key_id)
                .await;
        };
        if let Some(snapshot) = self.get_cached_provider_transport_snapshot(&cache_key) {
            return Ok(Some(snapshot));
        }

        let snapshot = self
            .read_provider_transport_snapshot_uncached(provider_id, endpoint_id, key_id)
            .await?;
        if let Some(snapshot) = snapshot.as_ref() {
            self.put_cached_provider_transport_snapshot(cache_key, snapshot.clone());
        }
        Ok(snapshot)
    }

    pub(crate) async fn update_provider_catalog_key_oauth_credentials(
        &self,
        key_id: &str,
        encrypted_api_key: &str,
        encrypted_auth_config: Option<&str>,
        expires_at_unix_secs: Option<u64>,
    ) -> Result<bool, GatewayError> {
        let updated = self
            .data
            .update_provider_catalog_key_oauth_credentials(
                key_id,
                encrypted_api_key,
                encrypted_auth_config,
                expires_at_unix_secs,
            )
            .await
            .map_err(|err| GatewayError::Internal(err.to_string()))?;
        if updated {
            self.clear_provider_transport_snapshot_cache();
        }
        Ok(updated)
    }

    pub(crate) async fn resolve_local_oauth_request_auth(
        &self,
        transport: &provider_transport::GatewayProviderTransportSnapshot,
    ) -> Result<Option<provider_transport::LocalResolvedOAuthRequestAuth>, GatewayError> {
        let distributed_lock = self.data.oauth_refresh_lock_runner();
        let lock_owner = format!("aether-gateway-{}", std::process::id());
        let mut current_transport = transport.clone();

        for _ in 0..2 {
            let resolution = self
                .oauth_refresh
                .resolve_with_result(
                    &self.client,
                    &current_transport,
                    distributed_lock.as_ref(),
                    Some(lock_owner.as_str()),
                )
                .await
                .map_err(|err| GatewayError::Internal(err.to_string()))?;

            if resolution
                .as_ref()
                .is_some_and(|resolution| resolution.refresh_in_flight)
            {
                let Some(reloaded_transport) = self
                    .wait_for_remote_oauth_refresh(&current_transport)
                    .await?
                else {
                    continue;
                };
                current_transport = reloaded_transport;
                continue;
            }

            if let Some(refreshed_entry) = resolution
                .as_ref()
                .and_then(|resolution| resolution.refreshed_entry.as_ref())
            {
                if let Err(err) = self
                    .persist_local_oauth_refresh_entry(&current_transport, refreshed_entry)
                    .await
                {
                    tracing::warn!(
                        key_id = %current_transport.key.id,
                        provider_type = %current_transport.provider.provider_type,
                        error = ?err,
                        "gateway local oauth refresh persistence failed"
                    );
                }
            }

            return Ok(resolution.and_then(|resolution| resolution.auth));
        }

        Ok(None)
    }

    pub(crate) async fn force_local_oauth_refresh_entry(
        &self,
        transport: &provider_transport::GatewayProviderTransportSnapshot,
    ) -> Result<
        Option<provider_transport::CachedOAuthEntry>,
        provider_transport::LocalOAuthRefreshError,
    > {
        let distributed_lock = self.data.oauth_refresh_lock_runner();
        let lock_owner = format!("aether-gateway-admin-{}", std::process::id());
        let mut current_transport = transport.clone();
        current_transport.key.decrypted_api_key = "__placeholder__".to_string();

        for _ in 0..2 {
            let resolution = self
                .oauth_refresh
                .resolve_with_result(
                    &self.client,
                    &current_transport,
                    distributed_lock.as_ref(),
                    Some(lock_owner.as_str()),
                )
                .await?;

            if resolution
                .as_ref()
                .is_some_and(|resolution| resolution.refresh_in_flight)
            {
                let Some(reloaded_transport) = self
                    .wait_for_remote_oauth_refresh(&current_transport)
                    .await
                    .map_err(
                        |err| provider_transport::LocalOAuthRefreshError::InvalidResponse {
                            provider_type: "gateway",
                            message: format!("{err:?}"),
                        },
                    )?
                else {
                    continue;
                };
                current_transport = reloaded_transport;
                current_transport.key.decrypted_api_key = "__placeholder__".to_string();
                continue;
            }

            if let Some(refreshed_entry) = resolution
                .as_ref()
                .and_then(|resolution| resolution.refreshed_entry.as_ref())
            {
                if let Err(err) = self
                    .persist_local_oauth_refresh_entry(&current_transport, refreshed_entry)
                    .await
                {
                    tracing::warn!(
                        key_id = %current_transport.key.id,
                        provider_type = %current_transport.provider.provider_type,
                        error = ?err,
                        "gateway manual oauth refresh persistence failed"
                    );
                }
                return Ok(Some(refreshed_entry.clone()));
            }

            return Ok(None);
        }

        Ok(None)
    }

    async fn persist_local_oauth_refresh_entry(
        &self,
        transport: &provider_transport::GatewayProviderTransportSnapshot,
        entry: &provider_transport::CachedOAuthEntry,
    ) -> Result<(), GatewayError> {
        let key_id = transport.key.id.trim();
        if key_id.is_empty() {
            return Ok(());
        }

        let Some(encryption_key) = self.data.encryption_key() else {
            return Ok(());
        };

        let access_token = entry
            .auth_header_value
            .trim()
            .strip_prefix("Bearer ")
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .ok_or_else(|| {
                GatewayError::Internal(
                    "local oauth refresh produced non-bearer auth header".to_string(),
                )
            })?;

        let encrypted_api_key = encrypt_python_fernet_plaintext(encryption_key, access_token)
            .map_err(|err| GatewayError::Internal(err.to_string()))?;
        let encrypted_auth_config = entry
            .metadata
            .as_ref()
            .map(|value| serde_json::to_string(value))
            .transpose()
            .map_err(|err| GatewayError::Internal(err.to_string()))?
            .map(|value| encrypt_python_fernet_plaintext(encryption_key, value.as_str()))
            .transpose()
            .map_err(|err| GatewayError::Internal(err.to_string()))?;

        self.update_provider_catalog_key_oauth_credentials(
            key_id,
            encrypted_api_key.as_str(),
            encrypted_auth_config.as_deref(),
            entry.expires_at_unix_secs,
        )
        .await?;
        Ok(())
    }

    async fn wait_for_remote_oauth_refresh(
        &self,
        transport: &provider_transport::GatewayProviderTransportSnapshot,
    ) -> Result<Option<provider_transport::GatewayProviderTransportSnapshot>, GatewayError> {
        if !self.data.has_provider_catalog_reader() {
            return Ok(None);
        }

        for _ in 0..20 {
            let Some(reloaded_transport) = self
                .read_provider_transport_snapshot_uncached(
                    &transport.provider.id,
                    &transport.endpoint.id,
                    &transport.key.id,
                )
                .await?
            else {
                return Ok(None);
            };

            if provider_transport_snapshot_looks_refreshed(transport, &reloaded_transport) {
                return Ok(Some(reloaded_transport));
            }

            tokio::time::sleep(Duration::from_millis(100)).await;
        }

        Ok(None)
    }
}
