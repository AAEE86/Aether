use crate::gateway::handlers::{
    ADMIN_EXTERNAL_MODELS_CACHE_KEY, ADMIN_EXTERNAL_MODELS_CACHE_TTL_SECS,
    OFFICIAL_EXTERNAL_MODEL_PROVIDERS,
};
use crate::gateway::{AppState, GatewayError};
use serde_json::json;

fn mark_admin_external_models_official(mut payload: serde_json::Value) -> serde_json::Value {
    let Some(models) = payload
        .get_mut("models")
        .and_then(serde_json::Value::as_array_mut)
    else {
        return payload;
    };
    for item in models {
        let provider_name = item
            .get("provider")
            .or_else(|| item.get("provider_name"))
            .and_then(serde_json::Value::as_str)
            .unwrap_or_default()
            .to_ascii_lowercase();
        if let Some(object) = item.as_object_mut() {
            object.insert(
                "official".to_string(),
                json!(OFFICIAL_EXTERNAL_MODEL_PROVIDERS.contains(&provider_name.as_str())),
            );
        }
    }
    payload
}

pub(crate) async fn read_admin_external_models_cache(
    state: &AppState,
) -> Result<Option<serde_json::Value>, GatewayError> {
    let Some(runner) = state.redis_kv_runner() else {
        return Ok(None);
    };
    let mut connection = runner
        .client()
        .get_multiplexed_async_connection()
        .await
        .map_err(|err| GatewayError::Internal(err.to_string()))?;
    let namespaced_key = runner.keyspace().key(ADMIN_EXTERNAL_MODELS_CACHE_KEY);
    let raw = redis::cmd("GET")
        .arg(&namespaced_key)
        .query_async::<Option<String>>(&mut connection)
        .await
        .map_err(|err| GatewayError::Internal(err.to_string()))?;
    let Some(raw) = raw else {
        return Ok(None);
    };
    let payload = serde_json::from_str::<serde_json::Value>(&raw)
        .map_err(|err| GatewayError::Internal(err.to_string()))?;
    let payload = mark_admin_external_models_official(payload);
    let serialized =
        serde_json::to_string(&payload).map_err(|err| GatewayError::Internal(err.to_string()))?;
    runner
        .setex(
            ADMIN_EXTERNAL_MODELS_CACHE_KEY,
            &serialized,
            Some(ADMIN_EXTERNAL_MODELS_CACHE_TTL_SECS),
        )
        .await
        .map_err(|err| GatewayError::Internal(err.to_string()))?;
    Ok(Some(payload))
}

pub(crate) async fn clear_admin_external_models_cache(
    state: &AppState,
) -> Result<serde_json::Value, GatewayError> {
    let Some(runner) = state.redis_kv_runner() else {
        return Ok(json!({
            "cleared": false,
            "message": "Redis 未启用",
        }));
    };
    let deleted = runner
        .del(ADMIN_EXTERNAL_MODELS_CACHE_KEY)
        .await
        .map_err(|err| GatewayError::Internal(err.to_string()))?;
    Ok(json!({
        "cleared": deleted > 0,
        "message": if deleted > 0 { "缓存已清除" } else { "缓存不存在" },
    }))
}
