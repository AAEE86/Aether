use super::{module_available_from_env, system_config_bool};
use crate::gateway::{AppState, GatewayError};
use serde_json::json;

#[derive(Clone, Copy)]
struct PublicAuthModuleDefinition {
    name: &'static str,
    display_name: &'static str,
    env_key: &'static str,
    default_available: bool,
}

pub(crate) struct AdminModuleDefinition {
    pub(crate) name: &'static str,
    pub(crate) display_name: &'static str,
    description: &'static str,
    category: &'static str,
    pub(crate) env_key: &'static str,
    pub(crate) default_available: bool,
    admin_route: Option<&'static str>,
    admin_menu_icon: Option<&'static str>,
    admin_menu_group: Option<&'static str>,
    admin_menu_order: i32,
}

const PUBLIC_AUTH_MODULE_DEFINITIONS: &[PublicAuthModuleDefinition] = &[
    PublicAuthModuleDefinition {
        name: "oauth",
        display_name: "OAuth 登录",
        env_key: "OAUTH_AVAILABLE",
        default_available: true,
    },
    PublicAuthModuleDefinition {
        name: "ldap",
        display_name: "LDAP 认证",
        env_key: "LDAP_AVAILABLE",
        default_available: true,
    },
];

const ADMIN_MODULE_DEFINITIONS: &[AdminModuleDefinition] = &[
    AdminModuleDefinition {
        name: "oauth",
        display_name: "OAuth 登录",
        description: "支持通过第三方 OAuth Provider 登录/绑定账号",
        category: "auth",
        env_key: "OAUTH_AVAILABLE",
        default_available: true,
        admin_route: Some("/admin/oauth"),
        admin_menu_icon: Some("Key"),
        admin_menu_group: Some("system"),
        admin_menu_order: 55,
    },
    AdminModuleDefinition {
        name: "ldap",
        display_name: "LDAP 认证",
        description: "支持通过 LDAP/Active Directory 进行用户认证",
        category: "auth",
        env_key: "LDAP_AVAILABLE",
        default_available: true,
        admin_route: Some("/admin/ldap"),
        admin_menu_icon: Some("Users"),
        admin_menu_group: Some("system"),
        admin_menu_order: 50,
    },
    AdminModuleDefinition {
        name: "management_tokens",
        display_name: "访问令牌",
        description: "管理 API 访问令牌，支持细粒度权限控制和 IP 白名单",
        category: "security",
        env_key: "MANAGEMENT_TOKENS_AVAILABLE",
        default_available: true,
        admin_route: Some("/admin/management-tokens"),
        admin_menu_icon: None,
        admin_menu_group: None,
        admin_menu_order: 0,
    },
    AdminModuleDefinition {
        name: "notification_email",
        display_name: "异常通知",
        description: "为 5xx 异常发送邮件通知，可在模块管理中启用或禁用",
        category: "integration",
        env_key: "NOTIFICATION_EMAIL_AVAILABLE",
        default_available: true,
        admin_route: None,
        admin_menu_icon: Some("Mail"),
        admin_menu_group: Some("system"),
        admin_menu_order: 58,
    },
    AdminModuleDefinition {
        name: "gemini_files",
        display_name: "文件缓存",
        description: "管理 Gemini Files API 上传的文件，支持文件上传、查看和删除",
        category: "integration",
        env_key: "GEMINI_FILES_AVAILABLE",
        default_available: true,
        admin_route: Some("/admin/gemini-files"),
        admin_menu_icon: Some("FileUp"),
        admin_menu_group: Some("system"),
        admin_menu_order: 60,
    },
    AdminModuleDefinition {
        name: "proxy_nodes",
        display_name: "代理节点",
        description: "添加Http/Socket代理节点, 或使用Aether-Proxy自动连接代理节点.",
        category: "integration",
        env_key: "PROXY_NODES_AVAILABLE",
        default_available: true,
        admin_route: Some("/admin/proxy-nodes"),
        admin_menu_icon: Some("Server"),
        admin_menu_group: Some("system"),
        admin_menu_order: 60,
    },
];

#[derive(Debug, Clone, serde::Deserialize)]
pub(crate) struct AdminSetModuleEnabledRequest {
    pub(crate) enabled: bool,
}

pub(crate) struct AdminModuleRuntimeState {
    oauth_providers: Vec<aether_data::repository::auth_modules::StoredOAuthProviderModuleConfig>,
    ldap_config: Option<aether_data::repository::auth_modules::StoredLdapModuleConfig>,
    gemini_files_has_capable_key: bool,
    smtp_configured: bool,
}

pub(crate) fn oauth_module_config_is_valid(
    providers: &[aether_data::repository::auth_modules::StoredOAuthProviderModuleConfig],
) -> bool {
    !providers.is_empty()
        && providers.iter().all(|provider| {
            !provider.client_id.trim().is_empty()
                && provider
                    .client_secret_encrypted
                    .as_deref()
                    .map(str::trim)
                    .filter(|value| !value.is_empty())
                    .is_some()
                && !provider.redirect_uri.trim().is_empty()
        })
}

pub(crate) fn ldap_module_config_is_valid(
    config: Option<&aether_data::repository::auth_modules::StoredLdapModuleConfig>,
) -> bool {
    let Some(config) = config else {
        return false;
    };
    !config.server_url.trim().is_empty()
        && !config.bind_dn.trim().is_empty()
        && !config.base_dn.trim().is_empty()
        && config
            .bind_password_encrypted
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .is_some()
}

pub(crate) async fn build_public_auth_modules_status_payload(
    state: &AppState,
) -> Result<serde_json::Value, GatewayError> {
    let oauth_providers = state.list_enabled_oauth_module_providers().await?;
    let ldap_config = state.get_ldap_module_config().await?;
    let oauth_active = oauth_module_config_is_valid(&oauth_providers);
    let ldap_active = ldap_module_config_is_valid(ldap_config.as_ref());

    let mut items = Vec::new();
    for module in PUBLIC_AUTH_MODULE_DEFINITIONS {
        if !module_available_from_env(module.env_key, module.default_available) {
            continue;
        }
        let enabled = state
            .read_system_config_json_value(&format!("module.{}.enabled", module.name))
            .await
            .ok()
            .flatten();
        let enabled = system_config_bool(enabled.as_ref(), false);
        let active = match module.name {
            "oauth" => enabled && oauth_active,
            "ldap" => enabled && ldap_active,
            _ => false,
        };
        items.push(json!({
            "name": module.name,
            "display_name": module.display_name,
            "active": active,
        }));
    }

    Ok(serde_json::Value::Array(items))
}

pub(crate) fn admin_module_by_name(name: &str) -> Option<&'static AdminModuleDefinition> {
    ADMIN_MODULE_DEFINITIONS
        .iter()
        .find(|module| module.name == name)
}

pub(crate) async fn build_admin_module_runtime_state(
    state: &AppState,
) -> Result<AdminModuleRuntimeState, GatewayError> {
    let oauth_providers = state.list_enabled_oauth_module_providers().await?;
    let ldap_config = state.get_ldap_module_config().await?;

    let provider_ids = state
        .list_provider_catalog_providers(false)
        .await
        .ok()
        .unwrap_or_default()
        .into_iter()
        .map(|provider| provider.id)
        .collect::<Vec<_>>();
    let gemini_files_has_capable_key = if provider_ids.is_empty() {
        false
    } else {
        state
            .list_provider_catalog_keys_by_provider_ids(&provider_ids)
            .await
            .ok()
            .unwrap_or_default()
            .into_iter()
            .any(|key| {
                key.is_active
                    && key
                        .capabilities
                        .as_ref()
                        .and_then(|value| value.get("gemini_files"))
                        .and_then(serde_json::Value::as_bool)
                        == Some(true)
            })
    };

    let smtp_host = state.read_system_config_json_value("smtp_host").await?;
    let smtp_from_email = state
        .read_system_config_json_value("smtp_from_email")
        .await?;
    let smtp_configured = smtp_host
        .as_ref()
        .and_then(serde_json::Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .is_some()
        && smtp_from_email
            .as_ref()
            .and_then(serde_json::Value::as_str)
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .is_some();

    Ok(AdminModuleRuntimeState {
        oauth_providers,
        ldap_config,
        gemini_files_has_capable_key,
        smtp_configured,
    })
}

pub(crate) fn build_admin_module_validation_result(
    module: &AdminModuleDefinition,
    runtime: &AdminModuleRuntimeState,
) -> (bool, Option<String>) {
    match module.name {
        "oauth" => {
            if runtime.oauth_providers.is_empty() {
                return (
                    false,
                    Some("请先配置并启用至少一个 OAuth Provider".to_string()),
                );
            }
            for provider in &runtime.oauth_providers {
                if provider.client_id.trim().is_empty() {
                    return (
                        false,
                        Some(format!(
                            "Provider [{}] 未配置 Client ID",
                            provider.display_name
                        )),
                    );
                }
                if provider
                    .client_secret_encrypted
                    .as_deref()
                    .map(str::trim)
                    .filter(|value| !value.is_empty())
                    .is_none()
                {
                    return (
                        false,
                        Some(format!(
                            "Provider [{}] 未配置 Client Secret",
                            provider.display_name
                        )),
                    );
                }
                if provider.redirect_uri.trim().is_empty() {
                    return (
                        false,
                        Some(format!(
                            "Provider [{}] 未配置回调地址",
                            provider.display_name
                        )),
                    );
                }
            }
            (true, None)
        }
        "ldap" => {
            let Some(config) = runtime.ldap_config.as_ref() else {
                return (false, Some("请先配置 LDAP 连接信息".to_string()));
            };
            if config.server_url.trim().is_empty() {
                return (false, Some("请配置 LDAP 服务器地址".to_string()));
            }
            if config.bind_dn.trim().is_empty() {
                return (false, Some("请配置绑定 DN".to_string()));
            }
            if config.base_dn.trim().is_empty() {
                return (false, Some("请配置搜索基准 DN".to_string()));
            }
            if config
                .bind_password_encrypted
                .as_deref()
                .map(str::trim)
                .filter(|value| !value.is_empty())
                .is_none()
            {
                return (false, Some("请配置绑定密码".to_string()));
            }
            (true, None)
        }
        "notification_email" => {
            if runtime.smtp_configured {
                (true, None)
            } else {
                (false, Some("请先完成邮件配置（SMTP）".to_string()))
            }
        }
        "gemini_files" => {
            if runtime.gemini_files_has_capable_key {
                (true, None)
            } else {
                (
                    false,
                    Some("至少启用一个具有「Gemini 文件 API」能力的 Key".to_string()),
                )
            }
        }
        "management_tokens" | "proxy_nodes" => (true, None),
        _ => (true, None),
    }
}

pub(crate) fn build_admin_module_health(
    module: &AdminModuleDefinition,
    runtime: &AdminModuleRuntimeState,
) -> &'static str {
    match module.name {
        "management_tokens" | "proxy_nodes" => "healthy",
        "gemini_files" => {
            if runtime.gemini_files_has_capable_key {
                "healthy"
            } else {
                "degraded"
            }
        }
        _ => "unknown",
    }
}

pub(crate) async fn build_admin_module_status_payload(
    state: &AppState,
    module: &AdminModuleDefinition,
    runtime: &AdminModuleRuntimeState,
) -> Result<serde_json::Value, GatewayError> {
    let available = module_available_from_env(module.env_key, module.default_available);
    let enabled = if available {
        let enabled = state
            .read_system_config_json_value(&format!("module.{}.enabled", module.name))
            .await?;
        system_config_bool(enabled.as_ref(), false)
    } else {
        false
    };
    let (config_validated, config_error) = if available {
        build_admin_module_validation_result(module, runtime)
    } else {
        (false, None)
    };
    let active = available && enabled && config_validated;
    let health = if available {
        build_admin_module_health(module, runtime)
    } else {
        "unknown"
    };
    Ok(json!({
        "name": module.name,
        "available": available,
        "enabled": enabled,
        "active": active,
        "config_validated": config_validated,
        "config_error": if config_validated { serde_json::Value::Null } else { json!(config_error) },
        "display_name": module.display_name,
        "description": module.description,
        "category": module.category,
        "admin_route": if available { json!(module.admin_route) } else { serde_json::Value::Null },
        "admin_menu_icon": module.admin_menu_icon,
        "admin_menu_group": module.admin_menu_group,
        "admin_menu_order": module.admin_menu_order,
        "health": health,
    }))
}

pub(crate) async fn build_admin_modules_status_payload(
    state: &AppState,
) -> Result<serde_json::Value, GatewayError> {
    let runtime = build_admin_module_runtime_state(state).await?;
    let mut payload = serde_json::Map::new();
    for module in ADMIN_MODULE_DEFINITIONS {
        payload.insert(
            module.name.to_string(),
            build_admin_module_status_payload(state, module, &runtime).await?,
        );
    }
    Ok(serde_json::Value::Object(payload))
}

pub(crate) fn admin_module_name_from_status_path(request_path: &str) -> Option<String> {
    request_path
        .strip_prefix("/api/admin/modules/status/")
        .map(str::trim)
        .filter(|value| !value.is_empty() && !value.contains('/'))
        .map(ToOwned::to_owned)
}

pub(crate) fn admin_module_name_from_enabled_path(request_path: &str) -> Option<String> {
    request_path
        .strip_prefix("/api/admin/modules/status/")
        .and_then(|value| value.strip_suffix("/enabled"))
        .map(str::trim)
        .filter(|value| !value.is_empty() && !value.contains('/'))
        .map(ToOwned::to_owned)
}
