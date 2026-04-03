use super::super::{normalize_json_array, normalize_json_object, normalize_string_list};
use super::{
    admin_provider_model_effective_capability, admin_provider_model_effective_input_price,
    admin_provider_model_effective_output_price, admin_provider_model_name_exists,
    normalize_optional_price, normalize_required_trimmed_string,
    resolve_admin_global_model_by_id_or_err,
};
use crate::gateway::handlers::{
    AdminGlobalModelCreateRequest, AdminGlobalModelUpdateRequest, AdminImportProviderModelsRequest,
    AdminProviderModelCreateRequest, AdminProviderModelUpdateRequest,
};
use crate::gateway::AppState;
use aether_data::repository::global_models::{
    AdminProviderModelListQuery, CreateAdminGlobalModelRecord, StoredAdminGlobalModel,
    StoredAdminProviderModel, UpdateAdminGlobalModelRecord, UpsertAdminProviderModelRecord,
};
use serde_json::json;
use std::collections::{BTreeMap, BTreeSet};
use uuid::Uuid;

pub(crate) async fn build_admin_global_model_create_record(
    state: &AppState,
    payload: AdminGlobalModelCreateRequest,
) -> Result<CreateAdminGlobalModelRecord, String> {
    let name = normalize_required_trimmed_string(&payload.name, "name")?;
    let display_name = normalize_required_trimmed_string(&payload.display_name, "display_name")?;
    if state
        .get_admin_global_model_by_name(&name)
        .await
        .map_err(|err| format!("{err:?}"))?
        .is_some()
    {
        return Err(format!("GlobalModel '{name}' 已存在"));
    }
    let default_price_per_request = normalize_optional_price(
        payload.default_price_per_request,
        "default_price_per_request",
    )?;
    let default_tiered_pricing =
        normalize_json_object(payload.default_tiered_pricing, "default_tiered_pricing")?;
    let supported_capabilities =
        normalize_string_list(payload.supported_capabilities).map(|value| json!(value));
    let config = normalize_json_object(payload.config, "config")?;
    CreateAdminGlobalModelRecord::new(
        Uuid::new_v4().to_string(),
        name,
        display_name,
        payload.is_active.unwrap_or(true),
        default_price_per_request,
        default_tiered_pricing,
        supported_capabilities,
        config,
    )
    .map_err(|err| err.to_string())
}

pub(crate) async fn build_admin_global_model_update_record(
    _state: &AppState,
    existing: &StoredAdminGlobalModel,
    raw_payload: &serde_json::Map<String, serde_json::Value>,
    payload: AdminGlobalModelUpdateRequest,
) -> Result<UpdateAdminGlobalModelRecord, String> {
    let display_name = if let Some(value) = raw_payload.get("display_name") {
        let Some(display_name) = payload.display_name.as_deref() else {
            return Err(if value.is_null() {
                "display_name 不能为空".to_string()
            } else {
                "display_name 必须是字符串".to_string()
            });
        };
        normalize_required_trimmed_string(display_name, "display_name")?
    } else {
        existing.display_name.clone()
    };

    let default_price_per_request = if raw_payload.contains_key("default_price_per_request") {
        normalize_optional_price(
            payload.default_price_per_request,
            "default_price_per_request",
        )?
    } else {
        existing.default_price_per_request
    };

    let default_tiered_pricing = if raw_payload.contains_key("default_tiered_pricing") {
        normalize_json_object(payload.default_tiered_pricing, "default_tiered_pricing")?
    } else {
        existing.default_tiered_pricing.clone()
    };

    let supported_capabilities = if raw_payload.contains_key("supported_capabilities") {
        normalize_string_list(payload.supported_capabilities).map(|value| json!(value))
    } else {
        existing.supported_capabilities.clone()
    };

    let config = if raw_payload.contains_key("config") {
        normalize_json_object(payload.config, "config")?
    } else {
        existing.config.clone()
    };

    UpdateAdminGlobalModelRecord::new(
        existing.id.clone(),
        display_name,
        payload.is_active.unwrap_or(existing.is_active),
        default_price_per_request,
        default_tiered_pricing,
        supported_capabilities,
        config,
    )
    .map_err(|err| err.to_string())
}

pub(crate) async fn build_admin_provider_model_create_record(
    state: &AppState,
    provider_id: &str,
    payload: AdminProviderModelCreateRequest,
) -> Result<UpsertAdminProviderModelRecord, String> {
    let provider_model_name =
        normalize_required_trimmed_string(&payload.provider_model_name, "provider_model_name")?;
    if admin_provider_model_name_exists(state, provider_id, &provider_model_name, None)
        .await
        .map_err(|err| format!("{err:?}"))?
    {
        return Err(format!("模型 '{provider_model_name}' 已存在"));
    }
    let global_model_id =
        normalize_required_trimmed_string(&payload.global_model_id, "global_model_id")?;
    resolve_admin_global_model_by_id_or_err(state, &global_model_id).await?;
    let price_per_request =
        normalize_optional_price(payload.price_per_request, "price_per_request")?;
    let tiered_pricing = normalize_json_object(payload.tiered_pricing, "tiered_pricing")?;
    let provider_model_mappings =
        normalize_json_array(payload.provider_model_mappings, "provider_model_mappings")?;
    let config = normalize_json_object(payload.config, "config")?;
    UpsertAdminProviderModelRecord::new(
        Uuid::new_v4().to_string(),
        provider_id.to_string(),
        global_model_id,
        provider_model_name,
        provider_model_mappings,
        price_per_request,
        tiered_pricing,
        payload.supports_vision,
        payload.supports_function_calling,
        payload.supports_streaming,
        payload.supports_extended_thinking,
        None,
        payload.is_active.unwrap_or(true),
        true,
        config,
    )
    .map_err(|err| err.to_string())
}

pub(crate) async fn build_admin_provider_model_update_record(
    state: &AppState,
    existing: &StoredAdminProviderModel,
    raw_payload: &serde_json::Map<String, serde_json::Value>,
    payload: AdminProviderModelUpdateRequest,
) -> Result<UpsertAdminProviderModelRecord, String> {
    let provider_model_name = if let Some(value) = raw_payload.get("provider_model_name") {
        let Some(name) = payload.provider_model_name.as_deref() else {
            return Err(if value.is_null() {
                "provider_model_name 不能为空".to_string()
            } else {
                "provider_model_name 必须是字符串".to_string()
            });
        };
        let name = normalize_required_trimmed_string(name, "provider_model_name")?;
        if admin_provider_model_name_exists(state, &existing.provider_id, &name, Some(&existing.id))
            .await
            .map_err(|err| format!("{err:?}"))?
        {
            return Err(format!("模型 '{name}' 已存在"));
        }
        name
    } else {
        existing.provider_model_name.clone()
    };

    let global_model_id = if let Some(value) = raw_payload.get("global_model_id") {
        let Some(global_model_id) = payload.global_model_id.as_deref() else {
            return Err(if value.is_null() {
                "global_model_id 不能为空".to_string()
            } else {
                "global_model_id 必须是字符串".to_string()
            });
        };
        let global_model_id =
            normalize_required_trimmed_string(global_model_id, "global_model_id")?;
        resolve_admin_global_model_by_id_or_err(state, &global_model_id).await?;
        global_model_id
    } else {
        existing.global_model_id.clone()
    };

    let price_per_request = if raw_payload.contains_key("price_per_request") {
        normalize_optional_price(payload.price_per_request, "price_per_request")?
    } else {
        existing.price_per_request
    };
    let tiered_pricing = if raw_payload.contains_key("tiered_pricing") {
        normalize_json_object(payload.tiered_pricing, "tiered_pricing")?
    } else {
        existing.tiered_pricing.clone()
    };
    let provider_model_mappings = if raw_payload.contains_key("provider_model_mappings") {
        normalize_json_array(payload.provider_model_mappings, "provider_model_mappings")?
    } else {
        existing.provider_model_mappings.clone()
    };
    let config = if raw_payload.contains_key("config") {
        normalize_json_object(payload.config, "config")?
    } else {
        existing.config.clone()
    };

    UpsertAdminProviderModelRecord::new(
        existing.id.clone(),
        existing.provider_id.clone(),
        global_model_id,
        provider_model_name,
        provider_model_mappings,
        price_per_request,
        tiered_pricing,
        if raw_payload.contains_key("supports_vision") {
            payload.supports_vision
        } else {
            existing.supports_vision
        },
        if raw_payload.contains_key("supports_function_calling") {
            payload.supports_function_calling
        } else {
            existing.supports_function_calling
        },
        if raw_payload.contains_key("supports_streaming") {
            payload.supports_streaming
        } else {
            existing.supports_streaming
        },
        if raw_payload.contains_key("supports_extended_thinking") {
            payload.supports_extended_thinking
        } else {
            existing.supports_extended_thinking
        },
        existing.supports_image_generation,
        payload.is_active.unwrap_or(existing.is_active),
        payload.is_available.unwrap_or(existing.is_available),
        config,
    )
    .map_err(|err| err.to_string())
}

pub(crate) async fn build_admin_provider_available_source_models_payload(
    state: &AppState,
    provider_id: &str,
) -> Option<serde_json::Value> {
    if !state.has_global_model_data_reader() || !state.has_provider_catalog_data_reader() {
        return None;
    }
    let provider = state
        .read_provider_catalog_providers_by_ids(&[provider_id.to_string()])
        .await
        .ok()?
        .into_iter()
        .next()?;
    let models = state
        .list_admin_provider_available_source_models(&provider.id)
        .await
        .ok()?;
    let mut by_global_model = BTreeMap::<String, StoredAdminProviderModel>::new();
    for model in models {
        by_global_model
            .entry(model.global_model_id.clone())
            .or_insert(model);
    }
    let mut payload_models = by_global_model
        .into_values()
        .map(|model| {
            json!({
                "global_model_name": model.global_model_name,
                "display_name": model.global_model_display_name,
                "provider_model_name": model.provider_model_name,
                "model_id": model.id,
                "price": {
                    "input_price_per_1m": admin_provider_model_effective_input_price(&model),
                    "output_price_per_1m": admin_provider_model_effective_output_price(&model),
                    "cache_creation_price_per_1m": serde_json::Value::Null,
                    "cache_read_price_per_1m": serde_json::Value::Null,
                    "price_per_request": model.price_per_request.or(model.global_model_default_price_per_request),
                },
                "capabilities": json!({
                    "supports_vision": admin_provider_model_effective_capability(&model, "vision"),
                    "supports_function_calling": admin_provider_model_effective_capability(&model, "function_calling"),
                    "supports_streaming": admin_provider_model_effective_capability(&model, "streaming"),
                }),
                "is_active": model.is_active,
            })
        })
        .collect::<Vec<_>>();
    let total = payload_models.len();
    payload_models.sort_by(|left, right| {
        left.get("global_model_name")
            .and_then(serde_json::Value::as_str)
            .cmp(
                &right
                    .get("global_model_name")
                    .and_then(serde_json::Value::as_str),
            )
    });
    Some(json!({
        "models": payload_models,
        "total": total,
    }))
}

pub(crate) async fn build_admin_batch_assign_global_models_payload(
    state: &AppState,
    provider_id: &str,
    global_model_ids: Vec<String>,
) -> Result<serde_json::Value, String> {
    let existing_models = state
        .list_admin_provider_models(&AdminProviderModelListQuery {
            provider_id: provider_id.to_string(),
            is_active: None,
            offset: 0,
            limit: 10_000,
        })
        .await
        .map_err(|err| format!("{err:?}"))?;
    let existing_global_model_ids = existing_models
        .into_iter()
        .map(|model| model.global_model_id)
        .collect::<BTreeSet<_>>();

    let mut success = Vec::new();
    let mut errors = Vec::new();
    for global_model_id in global_model_ids {
        let global_model_id = global_model_id.trim().to_string();
        if global_model_id.is_empty() {
            continue;
        }
        let global_model =
            match resolve_admin_global_model_by_id_or_err(state, &global_model_id).await {
                Ok(model) => model,
                Err(detail) => {
                    errors.push(json!({
                        "global_model_id": global_model_id,
                        "error": detail,
                    }));
                    continue;
                }
            };
        if existing_global_model_ids.contains(&global_model.id) {
            errors.push(json!({
                "global_model_id": global_model.id,
                "error": "Model already exists",
            }));
            continue;
        }
        let record = UpsertAdminProviderModelRecord::new(
            Uuid::new_v4().to_string(),
            provider_id.to_string(),
            global_model.id.clone(),
            global_model.name.clone(),
            None,
            None,
            None,
            None,
            None,
            None,
            None,
            None,
            true,
            true,
            None,
        )
        .map_err(|err| err.to_string())?;
        match state.create_admin_provider_model(&record).await {
            Ok(Some(created)) => success.push(json!({
                "global_model_id": global_model.id,
                "global_model_name": global_model.name,
                "provider_model_id": created.id,
            })),
            Ok(None) => errors.push(json!({
                "global_model_id": global_model.id,
                "error": "Create provider model failed",
            })),
            Err(err) => errors.push(json!({
                "global_model_id": global_model.id,
                "error": format!("{err:?}"),
            })),
        }
    }
    Ok(json!({
        "success": success,
        "errors": errors,
    }))
}

pub(crate) async fn build_admin_import_provider_models_payload(
    state: &AppState,
    provider_id: &str,
    payload: AdminImportProviderModelsRequest,
) -> Result<serde_json::Value, String> {
    let default_pricing = json!({
        "tiers": [{
            "up_to": null,
            "input_price_per_1m": 0.0,
            "output_price_per_1m": 0.0,
        }]
    });
    let tiered_pricing = normalize_json_object(payload.tiered_pricing, "tiered_pricing")?;

    let existing_models = state
        .list_admin_provider_models(&AdminProviderModelListQuery {
            provider_id: provider_id.to_string(),
            is_active: None,
            offset: 0,
            limit: 10_000,
        })
        .await
        .map_err(|err| format!("{err:?}"))?;
    let mut existing_by_name = existing_models
        .iter()
        .map(|model| (model.provider_model_name.clone(), model.clone()))
        .collect::<BTreeMap<_, _>>();

    let mut success = Vec::new();
    let mut errors = Vec::new();

    for model_id in payload.model_ids {
        let trimmed = model_id.trim();
        if trimmed.is_empty() || trimmed.len() > 100 {
            errors.push(json!({
                "model_id": if trimmed.is_empty() { "<empty>" } else { trimmed },
                "error": "Invalid model_id: must be 1-100 characters",
            }));
            continue;
        }

        if let Some(existing) = existing_by_name.get(trimmed) {
            success.push(json!({
                "model_id": trimmed,
                "global_model_id": existing.global_model_id,
                "global_model_name": existing.global_model_name,
                "provider_model_id": existing.id,
                "created_global_model": false,
            }));
            continue;
        }

        let mut created_global_model = false;
        let global_model = if let Some(existing) = state
            .get_admin_global_model_by_name(trimmed)
            .await
            .map_err(|err| format!("{err:?}"))?
        {
            existing
        } else {
            let created = state
                .create_admin_global_model(
                    &CreateAdminGlobalModelRecord::new(
                        Uuid::new_v4().to_string(),
                        trimmed.to_string(),
                        trimmed.to_string(),
                        true,
                        payload.price_per_request,
                        tiered_pricing
                            .clone()
                            .or_else(|| Some(default_pricing.clone())),
                        None,
                        None,
                    )
                    .map_err(|err| err.to_string())?,
                )
                .await
                .map_err(|err| format!("{err:?}"))?;
            let Some(created) = created else {
                errors.push(json!({"model_id": trimmed, "error": "Create GlobalModel failed"}));
                continue;
            };
            created_global_model = true;
            created
        };

        let record = UpsertAdminProviderModelRecord::new(
            Uuid::new_v4().to_string(),
            provider_id.to_string(),
            global_model.id.clone(),
            trimmed.to_string(),
            None,
            payload.price_per_request,
            tiered_pricing.clone(),
            None,
            None,
            None,
            None,
            None,
            true,
            true,
            None,
        )
        .map_err(|err| err.to_string())?;

        match state.create_admin_provider_model(&record).await {
            Ok(Some(created)) => {
                existing_by_name.insert(trimmed.to_string(), created.clone());
                success.push(json!({
                    "model_id": trimmed,
                    "global_model_id": global_model.id,
                    "global_model_name": global_model.name,
                    "provider_model_id": created.id,
                    "created_global_model": created_global_model,
                }));
            }
            Ok(None) => {
                errors.push(json!({"model_id": trimmed, "error": "Create model failed"}));
            }
            Err(err) => {
                errors.push(json!({"model_id": trimmed, "error": format!("{err:?}")}));
            }
        }
    }

    Ok(json!({ "success": success, "errors": errors }))
}
