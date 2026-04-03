use crate::gateway::{AppState, GatewayError};

impl AppState {
    pub(crate) async fn find_user_session(
        &self,
        user_id: &str,
        session_id: &str,
    ) -> Result<Option<crate::gateway::gateway_data::StoredUserSessionRecord>, GatewayError> {
        #[cfg(test)]
        if let Some(store) = self.auth_session_store.as_ref() {
            let key = format!("{user_id}:{session_id}");
            return Ok(store
                .lock()
                .expect("auth session store should lock")
                .get(&key)
                .cloned());
        }

        self.data
            .find_user_session(user_id, session_id)
            .await
            .map_err(|err| GatewayError::Internal(err.to_string()))
    }

    pub(crate) async fn list_user_sessions(
        &self,
        user_id: &str,
    ) -> Result<Vec<crate::gateway::gateway_data::StoredUserSessionRecord>, GatewayError> {
        #[cfg(test)]
        if let Some(store) = self.auth_session_store.as_ref() {
            let prefix = format!("{user_id}:");
            let now = chrono::Utc::now();
            let mut sessions = store
                .lock()
                .expect("auth session store should lock")
                .iter()
                .filter(|(key, _)| key.starts_with(&prefix))
                .map(|(_, session)| session.clone())
                .filter(|session| !session.is_revoked() && !session.is_expired(now))
                .collect::<Vec<_>>();
            sessions.sort_by(|left, right| {
                right
                    .last_seen_at
                    .cmp(&left.last_seen_at)
                    .then_with(|| right.created_at.cmp(&left.created_at))
            });
            return Ok(sessions);
        }

        self.data
            .list_user_sessions(user_id)
            .await
            .map_err(|err| GatewayError::Internal(err.to_string()))
    }

    pub(crate) async fn touch_user_session(
        &self,
        user_id: &str,
        session_id: &str,
        touched_at: chrono::DateTime<chrono::Utc>,
        ip_address: Option<&str>,
        user_agent: Option<&str>,
    ) -> Result<bool, GatewayError> {
        #[cfg(test)]
        if let Some(store) = self.auth_session_store.as_ref() {
            let key = format!("{user_id}:{session_id}");
            let mut guard = store.lock().expect("auth session store should lock");
            if let Some(session) = guard.get_mut(&key) {
                session.last_seen_at = Some(touched_at);
                if let Some(ip_address) = ip_address {
                    session.ip_address = Some(ip_address.to_string());
                }
                if let Some(user_agent) = user_agent {
                    session.user_agent = Some(user_agent.chars().take(1000).collect());
                }
                session.updated_at = Some(touched_at);
                return Ok(true);
            }
            return Ok(false);
        }

        self.data
            .touch_user_session(user_id, session_id, touched_at, ip_address, user_agent)
            .await
            .map_err(|err| GatewayError::Internal(err.to_string()))
    }

    pub(crate) async fn update_user_session_device_label(
        &self,
        user_id: &str,
        session_id: &str,
        device_label: &str,
        updated_at: chrono::DateTime<chrono::Utc>,
    ) -> Result<bool, GatewayError> {
        #[cfg(test)]
        if let Some(store) = self.auth_session_store.as_ref() {
            let key = format!("{user_id}:{session_id}");
            let mut guard = store.lock().expect("auth session store should lock");
            if let Some(session) = guard.get_mut(&key) {
                session.device_label = Some(device_label.chars().take(120).collect());
                session.updated_at = Some(updated_at);
                return Ok(true);
            }
            return Ok(false);
        }

        self.data
            .update_user_session_device_label(user_id, session_id, device_label, updated_at)
            .await
            .map_err(|err| GatewayError::Internal(err.to_string()))
    }

    pub(crate) async fn create_user_session(
        &self,
        session: crate::gateway::gateway_data::StoredUserSessionRecord,
    ) -> Result<Option<crate::gateway::gateway_data::StoredUserSessionRecord>, GatewayError> {
        #[cfg(test)]
        if let Some(store) = self.auth_session_store.as_ref() {
            let now = session
                .created_at
                .or(session.updated_at)
                .or(session.last_seen_at)
                .unwrap_or_else(chrono::Utc::now);
            let mut guard = store.lock().expect("auth session store should lock");
            for existing in guard.values_mut() {
                if existing.user_id == session.user_id
                    && existing.client_device_id == session.client_device_id
                    && !existing.is_revoked()
                    && !existing.is_expired(now)
                {
                    existing.revoked_at = Some(now);
                    existing.revoke_reason = Some("replaced_by_new_login".to_string());
                    existing.updated_at = Some(now);
                }
            }
            guard.insert(
                format!("{}:{}", session.user_id, session.id),
                session.clone(),
            );
            return Ok(Some(session));
        }

        self.data
            .create_user_session(&session)
            .await
            .map_err(|err| GatewayError::Internal(err.to_string()))
    }

    pub(crate) async fn rotate_user_session_refresh_token(
        &self,
        user_id: &str,
        session_id: &str,
        previous_refresh_token_hash: &str,
        next_refresh_token_hash: &str,
        rotated_at: chrono::DateTime<chrono::Utc>,
        expires_at: chrono::DateTime<chrono::Utc>,
        ip_address: Option<&str>,
        user_agent: Option<&str>,
    ) -> Result<bool, GatewayError> {
        #[cfg(test)]
        if let Some(store) = self.auth_session_store.as_ref() {
            let key = format!("{user_id}:{session_id}");
            let mut guard = store.lock().expect("auth session store should lock");
            if let Some(session) = guard.get_mut(&key) {
                session.prev_refresh_token_hash = Some(previous_refresh_token_hash.to_string());
                session.refresh_token_hash = next_refresh_token_hash.to_string();
                session.rotated_at = Some(rotated_at);
                session.expires_at = Some(expires_at);
                session.last_seen_at = Some(rotated_at);
                if let Some(ip_address) = ip_address {
                    session.ip_address = Some(ip_address.to_string());
                }
                if let Some(user_agent) = user_agent {
                    session.user_agent = Some(user_agent.chars().take(1000).collect());
                }
                session.updated_at = Some(rotated_at);
                return Ok(true);
            }
            return Ok(false);
        }

        self.data
            .rotate_user_session_refresh_token(
                user_id,
                session_id,
                previous_refresh_token_hash,
                next_refresh_token_hash,
                rotated_at,
                expires_at,
                ip_address,
                user_agent,
            )
            .await
            .map_err(|err| GatewayError::Internal(err.to_string()))
    }

    pub(crate) async fn revoke_user_session(
        &self,
        user_id: &str,
        session_id: &str,
        revoked_at: chrono::DateTime<chrono::Utc>,
        reason: &str,
    ) -> Result<bool, GatewayError> {
        #[cfg(test)]
        if let Some(store) = self.auth_session_store.as_ref() {
            let key = format!("{user_id}:{session_id}");
            let mut guard = store.lock().expect("auth session store should lock");
            if let Some(session) = guard.get_mut(&key) {
                session.revoked_at = Some(revoked_at);
                session.revoke_reason = Some(reason.chars().take(100).collect());
                session.updated_at = Some(revoked_at);
                return Ok(true);
            }
            return Ok(false);
        }

        self.data
            .revoke_user_session(user_id, session_id, revoked_at, reason)
            .await
            .map_err(|err| GatewayError::Internal(err.to_string()))
    }

    pub(crate) async fn revoke_all_user_sessions(
        &self,
        user_id: &str,
        revoked_at: chrono::DateTime<chrono::Utc>,
        reason: &str,
    ) -> Result<u64, GatewayError> {
        #[cfg(test)]
        if let Some(store) = self.auth_session_store.as_ref() {
            let prefix = format!("{user_id}:");
            let mut revoked = 0_u64;
            let mut guard = store.lock().expect("auth session store should lock");
            for (key, session) in guard.iter_mut() {
                if !key.starts_with(&prefix) || session.revoked_at.is_some() {
                    continue;
                }
                session.revoked_at = Some(revoked_at);
                session.revoke_reason = Some(reason.chars().take(100).collect());
                session.updated_at = Some(revoked_at);
                revoked += 1;
            }
            return Ok(revoked);
        }

        self.data
            .revoke_all_user_sessions(user_id, revoked_at, reason)
            .await
            .map_err(|err| GatewayError::Internal(err.to_string()))
    }
}
