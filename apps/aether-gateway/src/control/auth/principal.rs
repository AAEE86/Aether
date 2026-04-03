use super::credentials::hash_api_key;
use super::types::{
    GatewayExtractedCredentials, GatewayPrimaryCredential, GatewayPrincipalCandidate,
};

pub(super) fn derive_principal_candidate(
    credentials: &GatewayExtractedCredentials,
) -> Option<GatewayPrincipalCandidate> {
    if let Some(trusted_headers) = credentials.trusted_headers.clone() {
        return Some(GatewayPrincipalCandidate::TrustedHeaders(trusted_headers));
    }

    match credentials.primary.as_ref()? {
        GatewayPrimaryCredential::ProviderApiKey { raw, carrier } => {
            Some(GatewayPrincipalCandidate::ApiKeyHash {
                key_hash: hash_api_key(raw),
                carrier: *carrier,
            })
        }
        GatewayPrimaryCredential::BearerToken { raw, carrier } => {
            Some(GatewayPrincipalCandidate::DeferredBearerToken {
                raw: raw.clone(),
                carrier: *carrier,
            })
        }
        GatewayPrimaryCredential::CookieHeader { raw, carrier } => {
            Some(GatewayPrincipalCandidate::DeferredCookieHeader {
                raw: raw.clone(),
                carrier: *carrier,
            })
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{derive_principal_candidate, GatewayPrincipalCandidate};
    use crate::gateway::control::auth::types::{
        GatewayCredentialBundle, GatewayCredentialCarrier, GatewayExtractedCredentials,
        GatewayPrimaryCredential, GatewayTrustedAuthHeaders,
    };

    #[test]
    fn derives_trusted_headers_candidate_before_primary_credential() {
        let candidate = derive_principal_candidate(&GatewayExtractedCredentials {
            trusted_headers: Some(GatewayTrustedAuthHeaders {
                user_id: "user-1".to_string(),
                api_key_id: "key-1".to_string(),
                balance_remaining: Some(1.5),
                access_allowed: Some(true),
            }),
            trusted_admin_headers: None,
            bundle: GatewayCredentialBundle::default(),
            primary: Some(GatewayPrimaryCredential::ProviderApiKey {
                raw: "sk-test".to_string(),
                carrier: GatewayCredentialCarrier::AuthorizationBearer,
            }),
        });

        assert!(matches!(
            candidate,
            Some(GatewayPrincipalCandidate::TrustedHeaders(_))
        ));
    }

    #[test]
    fn derives_api_key_hash_candidate_for_provider_api_key() {
        let candidate = derive_principal_candidate(&GatewayExtractedCredentials {
            trusted_headers: None,
            trusted_admin_headers: None,
            bundle: GatewayCredentialBundle::default(),
            primary: Some(GatewayPrimaryCredential::ProviderApiKey {
                raw: "sk-test".to_string(),
                carrier: GatewayCredentialCarrier::XApiKey,
            }),
        });

        match candidate {
            Some(GatewayPrincipalCandidate::ApiKeyHash { key_hash, carrier }) => {
                assert_eq!(
                    key_hash,
                    "f3abf2a6cc4f00987743db5f544ba345b4899ae31f326d8ee9c4816de153c9e0"
                );
                assert_eq!(carrier, GatewayCredentialCarrier::XApiKey);
            }
            other => panic!("unexpected candidate: {other:?}"),
        }
    }

    #[test]
    fn derives_deferred_cookie_candidate_for_cookie_credentials() {
        let candidate = derive_principal_candidate(&GatewayExtractedCredentials {
            trusted_headers: None,
            trusted_admin_headers: None,
            bundle: GatewayCredentialBundle::default(),
            primary: Some(GatewayPrimaryCredential::CookieHeader {
                raw: "session=test".to_string(),
                carrier: GatewayCredentialCarrier::CookieHeader,
            }),
        });

        assert_eq!(
            candidate,
            Some(GatewayPrincipalCandidate::DeferredCookieHeader {
                raw: "session=test".to_string(),
                carrier: GatewayCredentialCarrier::CookieHeader,
            })
        );
    }
}
