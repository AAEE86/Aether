use serde_json::{Map, Value};
use sha2::{Digest, Sha256};

use super::agent_identity::{
    codex_agent_identity_transport_credential_fingerprint, is_codex_agent_identity_transport,
};
use super::snapshot::GatewayProviderTransportSnapshot;

const OAUTH_CREDENTIAL_GENERATION_FIELD: &str = "aether_credential_generation";
const PLACEHOLDER_API_KEY: &str = "__placeholder__";

/// Removes credential metadata that is owned by the gateway refresh path.
///
/// Admin create, update, import, and export paths must call this before
/// persisting or returning user-controlled auth config. Only a successful
/// OAuth refresh may write the generation marker.
pub fn strip_server_owned_credential_generation(auth_config: &mut Map<String, Value>) {
    auth_config.remove(OAUTH_CREDENTIAL_GENERATION_FIELD);
}

/// Returns a non-secret identity for the credential generation represented by
/// a transport snapshot.
///
/// Provider key ids identify catalog rows, not the secret currently stored in
/// those rows. Responses WebSocket continuations use this fingerprint to
/// reject a row whose credential was replaced while allowing ordinary OAuth
/// access-token refreshes and Agent Identity task/assertion rotation.
pub fn provider_transport_credential_binding_fingerprint(
    transport: &GatewayProviderTransportSnapshot,
) -> String {
    let provider_type = transport.provider.provider_type.trim().to_ascii_lowercase();
    let auth_type = transport.key.auth_type.trim().to_ascii_lowercase();
    let generation = if is_codex_agent_identity_transport(transport) {
        codex_agent_identity_transport_credential_fingerprint(transport)
            .map(|value| format!("codex-agent-identity:{value}"))
            .unwrap_or_else(|| static_credential_generation(transport))
    } else if auth_type == "oauth" {
        oauth_credential_generation(transport)
    } else {
        static_credential_generation(transport)
    };

    fingerprint_fields(&[
        b"aether-provider-credential-binding-v1",
        provider_type.as_bytes(),
        auth_type.as_bytes(),
        generation.as_bytes(),
    ])
}

/// Persists the source OAuth generation into refreshed metadata. The marker is
/// derived from the pre-refresh credential and therefore survives providers
/// that rotate both access and refresh tokens in one successful refresh.
pub(crate) fn preserve_oauth_credential_generation(
    transport: &GatewayProviderTransportSnapshot,
    refreshed_auth_config: &mut Value,
) {
    if is_codex_agent_identity_transport(transport)
        || !transport.key.auth_type.trim().eq_ignore_ascii_case("oauth")
    {
        return;
    }
    let generation = oauth_credential_generation(transport);
    let Some(object) = refreshed_auth_config.as_object_mut() else {
        return;
    };
    object.insert(
        OAUTH_CREDENTIAL_GENERATION_FIELD.to_string(),
        Value::String(generation),
    );
}

fn oauth_credential_generation(transport: &GatewayProviderTransportSnapshot) -> String {
    let config = parsed_auth_config(transport);
    if let Some(generation) = config
        .as_ref()
        .and_then(Value::as_object)
        .and_then(|object| object.get(OAUTH_CREDENTIAL_GENERATION_FIELD))
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
    {
        return generation.to_string();
    }

    if let Some(refresh_token) = config
        .as_ref()
        .and_then(|config| find_non_empty_string(config, &["refresh_token", "refreshToken"]))
    {
        return format!(
            "oauth-refresh:{}",
            fingerprint_fields(&[refresh_token.as_bytes()])
        );
    }

    let access_token = transport.key.decrypted_api_key.trim();
    if !access_token.is_empty() && access_token != PLACEHOLDER_API_KEY {
        return format!(
            "oauth-access:{}",
            fingerprint_fields(&[access_token.as_bytes()])
        );
    }

    static_credential_generation(transport)
}

fn parsed_auth_config(transport: &GatewayProviderTransportSnapshot) -> Option<Value> {
    transport
        .key
        .decrypted_auth_config
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .and_then(|value| serde_json::from_str(value).ok())
}

fn find_non_empty_string<'a>(value: &'a Value, keys: &[&str]) -> Option<&'a str> {
    match value {
        Value::Object(object) => {
            for key in keys {
                if let Some(value) = object
                    .get(*key)
                    .and_then(Value::as_str)
                    .map(str::trim)
                    .filter(|value| !value.is_empty())
                {
                    return Some(value);
                }
            }
            object
                .values()
                .find_map(|value| find_non_empty_string(value, keys))
        }
        Value::Array(items) => items
            .iter()
            .find_map(|value| find_non_empty_string(value, keys)),
        _ => None,
    }
}

fn static_credential_generation(transport: &GatewayProviderTransportSnapshot) -> String {
    let secret = transport.key.decrypted_api_key.trim();
    let auth_config = transport
        .key
        .decrypted_auth_config
        .as_deref()
        .map(str::trim)
        .unwrap_or_default();
    format!(
        "static:{}",
        fingerprint_fields(&[secret.as_bytes(), auth_config.as_bytes()])
    )
}

fn fingerprint_fields(fields: &[&[u8]]) -> String {
    let mut digest = Sha256::new();
    for field in fields {
        digest.update((field.len() as u64).to_be_bytes());
        digest.update(field);
    }
    format!("{:x}", digest.finalize())
}

#[cfg(test)]
mod tests {
    use base64::{engine::general_purpose::STANDARD, Engine as _};
    use ed25519_dalek::{pkcs8::EncodePrivateKey, SigningKey};
    use serde_json::json;

    use super::{
        preserve_oauth_credential_generation, provider_transport_credential_binding_fingerprint,
        strip_server_owned_credential_generation, OAUTH_CREDENTIAL_GENERATION_FIELD,
    };
    use crate::snapshot::{
        GatewayProviderTransportEndpoint, GatewayProviderTransportKey,
        GatewayProviderTransportProvider, GatewayProviderTransportSnapshot,
    };

    fn transport(
        auth_type: &str,
        secret: &str,
        auth_config: Option<serde_json::Value>,
    ) -> GatewayProviderTransportSnapshot {
        GatewayProviderTransportSnapshot {
            provider: GatewayProviderTransportProvider {
                id: "provider-1".to_string(),
                name: "Provider".to_string(),
                provider_type: "codex".to_string(),
                website: None,
                is_active: true,
                keep_priority_on_conversion: false,
                enable_format_conversion: false,
                concurrent_limit: None,
                max_retries: None,
                proxy: None,
                request_timeout_secs: None,
                stream_first_byte_timeout_secs: None,
                config: None,
            },
            endpoint: GatewayProviderTransportEndpoint {
                id: "endpoint-1".to_string(),
                provider_id: "provider-1".to_string(),
                api_format: "openai:responses".to_string(),
                api_family: Some("openai".to_string()),
                endpoint_kind: Some("responses".to_string()),
                is_active: true,
                base_url: "https://example.test".to_string(),
                header_rules: None,
                body_rules: None,
                max_retries: None,
                custom_path: None,
                config: None,
                format_acceptance_config: None,
                proxy: None,
            },
            key: GatewayProviderTransportKey {
                id: "key-1".to_string(),
                provider_id: "provider-1".to_string(),
                name: "Key".to_string(),
                auth_type: auth_type.to_string(),
                is_active: true,
                api_formats: None,
                auth_type_by_format: None,
                allow_auth_channel_mismatch_formats: None,
                allowed_models: None,
                capabilities: None,
                rate_multipliers: None,
                global_priority_by_format: None,
                expires_at_unix_secs: None,
                proxy: None,
                fingerprint: None,
                upstream_metadata: None,
                decrypted_api_key: secret.to_string(),
                decrypted_auth_config: auth_config.map(|value| value.to_string()),
            },
        }
    }

    #[test]
    fn static_credential_replacement_changes_binding_generation() {
        let first = transport("bearer", "static-secret-a", None);
        let replacement = transport("bearer", "static-secret-b", None);

        assert_ne!(
            provider_transport_credential_binding_fingerprint(&first),
            provider_transport_credential_binding_fingerprint(&replacement)
        );
    }

    #[test]
    fn oauth_access_token_refresh_keeps_refresh_credential_generation() {
        let first = transport(
            "oauth",
            "access-a",
            Some(json!({"refresh_token": "refresh-stable", "expires_at": 1})),
        );
        let refreshed = transport(
            "oauth",
            "access-b",
            Some(json!({
                "refresh_token": "refresh-stable",
                "expires_at": 2,
                "updated_at": 2
            })),
        );

        assert_eq!(
            provider_transport_credential_binding_fingerprint(&first),
            provider_transport_credential_binding_fingerprint(&refreshed)
        );
    }

    #[test]
    fn oauth_access_credential_replacement_changes_binding_with_same_account_identity() {
        let first = transport(
            "oauth",
            "access-a",
            Some(json!({"account_id": "acct-1", "email": "alice@example.test"})),
        );
        let replacement = transport(
            "oauth",
            "access-b",
            Some(json!({"account_id": "acct-1", "email": "alice@example.test"})),
        );

        assert_ne!(
            provider_transport_credential_binding_fingerprint(&first),
            provider_transport_credential_binding_fingerprint(&replacement)
        );
    }

    #[test]
    fn persisted_generation_keeps_access_only_oauth_refresh_bound() {
        let first = transport(
            "oauth",
            "access-a",
            Some(json!({"account_id": "acct-1", "expires_at": 1})),
        );
        let first_fingerprint = provider_transport_credential_binding_fingerprint(&first);
        let mut refreshed_config = json!({"account_id": "acct-1", "expires_at": 2});
        preserve_oauth_credential_generation(&first, &mut refreshed_config);
        let refreshed = transport("oauth", "access-b", Some(refreshed_config));

        assert_eq!(
            first_fingerprint,
            provider_transport_credential_binding_fingerprint(&refreshed)
        );
    }

    #[test]
    fn persisted_generation_survives_refresh_token_rotation() {
        let first = transport(
            "oauth",
            "access-a",
            Some(json!({"refresh_token": "refresh-a"})),
        );
        let first_fingerprint = provider_transport_credential_binding_fingerprint(&first);
        let mut refreshed_config = json!({
            "refresh_token": "refresh-b",
            "updated_at": 2,
            OAUTH_CREDENTIAL_GENERATION_FIELD: "untrusted-refreshed-generation"
        });
        preserve_oauth_credential_generation(&first, &mut refreshed_config);
        let refreshed = transport("oauth", "access-b", Some(refreshed_config.clone()));

        assert!(refreshed_config[OAUTH_CREDENTIAL_GENERATION_FIELD]
            .as_str()
            .is_some_and(|value| !value.is_empty() && value != "untrusted-refreshed-generation"));
        assert_eq!(
            first_fingerprint,
            provider_transport_credential_binding_fingerprint(&refreshed)
        );

        let replacement = transport(
            "oauth",
            "access-c",
            Some(json!({"refresh_token": "refresh-c"})),
        );
        assert_ne!(
            first_fingerprint,
            provider_transport_credential_binding_fingerprint(&replacement)
        );
    }

    #[test]
    fn user_auth_config_cannot_supply_credential_generation() {
        let mut config = json!({
            "refresh_token": "replacement-refresh",
            OAUTH_CREDENTIAL_GENERATION_FIELD: "forged-generation"
        })
        .as_object()
        .cloned()
        .expect("auth config should be an object");

        strip_server_owned_credential_generation(&mut config);

        assert!(!config.contains_key(OAUTH_CREDENTIAL_GENERATION_FIELD));
        assert_eq!(
            config.get("refresh_token"),
            Some(&json!("replacement-refresh"))
        );
    }

    fn agent_identity_config(seed: u8, task_id: &str) -> serde_json::Value {
        let signing_key = SigningKey::from_bytes(&[seed; 32]);
        let private_key = signing_key.to_pkcs8_der().expect("test key should encode");
        json!({
            "provider_type": "codex",
            "auth_mode": "agentIdentity",
            "agent_runtime_id": "runtime-1",
            "agent_private_key": STANDARD.encode(private_key.as_bytes()),
            "task_id": task_id,
        })
    }

    #[test]
    fn agent_identity_uses_keypair_generation_not_rotating_task() {
        let first = transport(
            "oauth",
            "__placeholder__",
            Some(agent_identity_config(7, "task-a")),
        );
        let task_rotated = transport(
            "oauth",
            "__placeholder__",
            Some(agent_identity_config(7, "task-b")),
        );
        let key_replaced = transport(
            "oauth",
            "__placeholder__",
            Some(agent_identity_config(8, "task-c")),
        );

        assert_eq!(
            provider_transport_credential_binding_fingerprint(&first),
            provider_transport_credential_binding_fingerprint(&task_rotated)
        );
        assert_ne!(
            provider_transport_credential_binding_fingerprint(&first),
            provider_transport_credential_binding_fingerprint(&key_replaced)
        );
    }
}
