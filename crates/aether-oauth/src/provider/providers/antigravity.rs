use super::generic::{
    provider_account_state_from_metadata, template_for_provider_type, GenericProviderOAuthAdapter,
};
use crate::provider::ProviderOAuthAdapter;

#[derive(Debug, Clone)]
pub struct AntigravityProviderOAuthAdapter {
    inner: GenericProviderOAuthAdapter,
}

impl Default for AntigravityProviderOAuthAdapter {
    fn default() -> Self {
        Self {
            inner: GenericProviderOAuthAdapter::new(
                template_for_provider_type("antigravity")
                    .expect("antigravity template should exist"),
            ),
        }
    }
}

#[async_trait::async_trait]
impl ProviderOAuthAdapter for AntigravityProviderOAuthAdapter {
    fn provider_type(&self) -> &'static str {
        self.inner.provider_type()
    }

    fn capabilities(&self) -> crate::provider::ProviderOAuthCapabilities {
        crate::provider::ProviderOAuthCapabilities {
            supports_account_probe: true,
            ..self.inner.capabilities()
        }
    }

    fn build_authorize_url(
        &self,
        ctx: &crate::provider::ProviderOAuthTransportContext,
        state: &str,
        code_challenge: Option<&str>,
    ) -> Result<crate::core::OAuthAuthorizeResponse, crate::core::OAuthError> {
        let mut response = self.inner.build_authorize_url(ctx, state, code_challenge)?;
        let mut url = url::Url::parse(&response.authorize_url).map_err(|_| {
            crate::core::OAuthError::invalid_request("authorize_url must be absolute")
        })?;
        {
            let mut query = url.query_pairs_mut();
            query.append_pair("access_type", "offline");
            query.append_pair("prompt", "consent");
        }
        response.authorize_url = url.to_string();
        Ok(response)
    }

    async fn exchange_code(
        &self,
        executor: &dyn crate::network::OAuthHttpExecutor,
        ctx: &crate::provider::ProviderOAuthTransportContext,
        code: &str,
        state: &str,
        pkce_verifier: Option<&str>,
    ) -> Result<crate::provider::ProviderOAuthTokenSet, crate::core::OAuthError> {
        self.inner
            .exchange_code(executor, ctx, code, state, pkce_verifier)
            .await
    }

    async fn import_credentials(
        &self,
        executor: &dyn crate::network::OAuthHttpExecutor,
        ctx: &crate::provider::ProviderOAuthTransportContext,
        input: crate::provider::ProviderOAuthImportInput,
    ) -> Result<crate::provider::ProviderOAuthTokenSet, crate::core::OAuthError> {
        self.inner.import_credentials(executor, ctx, input).await
    }

    async fn refresh(
        &self,
        executor: &dyn crate::network::OAuthHttpExecutor,
        ctx: &crate::provider::ProviderOAuthTransportContext,
        account: &crate::provider::ProviderOAuthAccount,
    ) -> Result<crate::provider::ProviderOAuthTokenSet, crate::core::OAuthError> {
        self.inner.refresh(executor, ctx, account).await
    }

    fn resolve_request_auth(
        &self,
        account: &crate::provider::ProviderOAuthAccount,
    ) -> Result<crate::provider::ProviderOAuthRequestAuth, crate::core::OAuthError> {
        self.inner.resolve_request_auth(account)
    }

    fn account_fingerprint(
        &self,
        account: &crate::provider::ProviderOAuthAccount,
    ) -> Option<String> {
        self.inner.account_fingerprint(account)
    }

    async fn probe_account_state(
        &self,
        _executor: &dyn crate::network::OAuthHttpExecutor,
        _ctx: &crate::provider::ProviderOAuthTransportContext,
        account: &crate::provider::ProviderOAuthAccount,
    ) -> Result<Option<crate::provider::ProviderOAuthProbeResult>, crate::core::OAuthError> {
        Ok(Some(provider_account_state_from_metadata(
            "antigravity",
            account,
        )))
    }
}

#[cfg(test)]
mod tests {
    use super::AntigravityProviderOAuthAdapter;
    use crate::network::{OAuthHttpExecutor, OAuthHttpRequest, OAuthHttpResponse};
    use crate::provider::{
        ProviderOAuthAccount, ProviderOAuthAdapter, ProviderOAuthTransportContext,
    };
    use async_trait::async_trait;
    use serde_json::json;
    use std::collections::BTreeMap;

    struct UnusedExecutor;

    fn transport_context() -> ProviderOAuthTransportContext {
        ProviderOAuthTransportContext {
            provider_id: String::new(),
            provider_type: "antigravity".to_string(),
            endpoint_id: None,
            key_id: None,
            auth_type: Some("oauth".to_string()),
            decrypted_api_key: None,
            decrypted_auth_config: None,
            provider_config: None,
            endpoint_config: None,
            key_config: None,
            network: crate::network::OAuthNetworkContext::provider_operation(None),
        }
    }

    #[async_trait]
    impl OAuthHttpExecutor for UnusedExecutor {
        async fn execute(
            &self,
            _request: OAuthHttpRequest,
        ) -> Result<OAuthHttpResponse, crate::core::OAuthError> {
            unreachable!("metadata probe should not execute network requests")
        }
    }

    #[test]
    fn antigravity_authorize_requests_offline_refresh_token() {
        let adapter = AntigravityProviderOAuthAdapter::default();
        let response = adapter
            .build_authorize_url(&transport_context(), "state-1", Some("challenge-1"))
            .expect("authorize url should build");
        let url = url::Url::parse(&response.authorize_url).expect("authorize url should parse");
        let query = url.query_pairs().collect::<BTreeMap<_, _>>();

        assert_eq!(
            query.get("access_type").map(|value| value.as_ref()),
            Some("offline")
        );
        assert_eq!(
            query.get("prompt").map(|value| value.as_ref()),
            Some("consent")
        );
        assert_eq!(
            query.get("code_challenge").map(|value| value.as_ref()),
            Some("challenge-1")
        );
    }

    #[tokio::test]
    async fn antigravity_probe_marks_forbidden_metadata_invalid() {
        let adapter = AntigravityProviderOAuthAdapter::default();
        let ctx = transport_context();
        let account = ProviderOAuthAccount {
            provider_type: "antigravity".to_string(),
            access_token: "access-token".to_string(),
            auth_config: json!({
                "email": "ag@example.com",
                "antigravity": {
                    "is_forbidden": true,
                    "forbidden_reason": "project blocked"
                }
            }),
            expires_at_unix_secs: Some(2000),
            identity: BTreeMap::new(),
        };

        let probe = adapter
            .probe_account_state(&UnusedExecutor, &ctx, &account)
            .await
            .expect("probe should succeed")
            .expect("probe should return state");

        assert!(!probe.state.is_valid);
        assert_eq!(probe.state.email.as_deref(), Some("ag@example.com"));
        assert_eq!(
            probe.state.invalid_reason.as_deref(),
            Some("project blocked")
        );
    }
}
