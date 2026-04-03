use super::{
    admin_billing_optional_bool_filter, admin_billing_optional_epoch_value,
    admin_billing_optional_filter, admin_billing_pages, admin_billing_parse_page,
    admin_billing_parse_page_size, admin_billing_validate_safe_expression,
    build_admin_billing_bad_request_response, build_admin_billing_not_found_response,
    build_admin_billing_read_only_response, default_admin_billing_json_object,
    default_admin_billing_true, normalize_admin_billing_optional_text,
    normalize_admin_billing_required_text,
};
use crate::gateway::handlers::unix_secs_to_rfc3339;
use crate::gateway::{AppState, GatewayError, GatewayPublicRequestContext};
use axum::{
    body::{Body, Bytes},
    http,
    response::{IntoResponse, Response},
    Json,
};
use serde::Deserialize;
use serde_json::json;
use sqlx::Row;

fn default_admin_billing_rule_task_type() -> String {
    "chat".to_string()
}

#[derive(Debug, Deserialize)]
struct AdminBillingRuleUpsertRequest {
    name: String,
    #[serde(default = "default_admin_billing_rule_task_type")]
    task_type: String,
    #[serde(default)]
    global_model_id: Option<String>,
    #[serde(default)]
    model_id: Option<String>,
    expression: String,
    #[serde(default = "default_admin_billing_json_object")]
    variables: serde_json::Value,
    #[serde(default = "default_admin_billing_json_object")]
    dimension_mappings: serde_json::Value,
    #[serde(default = "default_admin_billing_true")]
    is_enabled: bool,
}

fn build_admin_billing_rule_payload_from_record(
    record: &crate::gateway::AdminBillingRuleRecord,
) -> serde_json::Value {
    json!({
        "id": record.id,
        "name": record.name,
        "task_type": record.task_type,
        "global_model_id": record.global_model_id,
        "model_id": record.model_id,
        "expression": record.expression,
        "variables": record.variables,
        "dimension_mappings": record.dimension_mappings,
        "is_enabled": record.is_enabled,
        "created_at": unix_secs_to_rfc3339(record.created_at_unix_secs),
        "updated_at": unix_secs_to_rfc3339(record.updated_at_unix_secs),
    })
}

fn admin_billing_rule_id_from_path(request_path: &str) -> Option<String> {
    let value = request_path
        .strip_prefix("/api/admin/billing/rules/")?
        .trim()
        .trim_matches('/')
        .to_string();
    if value.is_empty() || value.contains('/') {
        None
    } else {
        Some(value)
    }
}

fn parse_admin_billing_rule_request(
    request_body: Option<&Bytes>,
) -> Result<crate::gateway::AdminBillingRuleWriteInput, Response<Body>> {
    let Some(request_body) = request_body else {
        return Err(build_admin_billing_bad_request_response("请求体不能为空"));
    };
    let request = match serde_json::from_slice::<AdminBillingRuleUpsertRequest>(request_body) {
        Ok(value) => value,
        Err(err) => {
            return Err(build_admin_billing_bad_request_response(format!(
                "Invalid request body: {err}"
            )))
        }
    };

    let name = match normalize_admin_billing_required_text(&request.name, "name", 100) {
        Ok(value) => value,
        Err(detail) => return Err(build_admin_billing_bad_request_response(detail)),
    };
    let task_type = request.task_type.trim().to_ascii_lowercase();
    if !matches!(task_type.as_str(), "chat" | "video" | "image" | "audio") {
        return Err(build_admin_billing_bad_request_response(
            "task_type must be one of chat, video, image, audio",
        ));
    }
    let global_model_id = match normalize_admin_billing_optional_text(request.global_model_id, 64) {
        Ok(value) => value,
        Err(detail) => return Err(build_admin_billing_bad_request_response(detail)),
    };
    let model_id = match normalize_admin_billing_optional_text(request.model_id, 64) {
        Ok(value) => value,
        Err(detail) => return Err(build_admin_billing_bad_request_response(detail)),
    };
    if global_model_id.is_some() == model_id.is_some() {
        return Err(build_admin_billing_bad_request_response(
            "Exactly one of global_model_id or model_id must be provided",
        ));
    }
    let expression = request.expression.trim().to_string();
    if let Err(detail) = admin_billing_validate_safe_expression(&expression) {
        return Err(build_admin_billing_bad_request_response(format!(
            "Invalid expression: {detail}"
        )));
    }

    let Some(variables) = request.variables.as_object() else {
        return Err(build_admin_billing_bad_request_response(
            "variables must be a JSON object",
        ));
    };
    for (key, value) in variables {
        if key.trim().is_empty() {
            return Err(build_admin_billing_bad_request_response(
                "variables keys must be non-empty strings",
            ));
        }
        if value.is_boolean() || !value.is_number() {
            return Err(build_admin_billing_bad_request_response(format!(
                "variables['{key}'] must be a number"
            )));
        }
    }

    let Some(dimension_mappings) = request.dimension_mappings.as_object() else {
        return Err(build_admin_billing_bad_request_response(
            "dimension_mappings must be a JSON object",
        ));
    };
    for (key, value) in dimension_mappings {
        if key.trim().is_empty() {
            return Err(build_admin_billing_bad_request_response(
                "dimension_mappings keys must be non-empty strings",
            ));
        }
        let Some(mapping) = value.as_object() else {
            return Err(build_admin_billing_bad_request_response(format!(
                "dimension_mappings['{key}'] must be an object"
            )));
        };
        if !mapping.contains_key("source") {
            return Err(build_admin_billing_bad_request_response(format!(
                "dimension_mappings['{key}'].source is required"
            )));
        }
    }

    Ok(crate::gateway::AdminBillingRuleWriteInput {
        name,
        task_type,
        global_model_id,
        model_id,
        expression,
        variables: serde_json::Value::Object(variables.clone()),
        dimension_mappings: serde_json::Value::Object(dimension_mappings.clone()),
        is_enabled: request.is_enabled,
    })
}

fn admin_billing_rule_payload(
    row: &sqlx::postgres::PgRow,
) -> Result<serde_json::Value, GatewayError> {
    Ok(json!({
        "id": row.try_get::<String, _>("id").map_err(|err| GatewayError::Internal(err.to_string()))?,
        "name": row.try_get::<String, _>("name").map_err(|err| GatewayError::Internal(err.to_string()))?,
        "task_type": row.try_get::<String, _>("task_type").map_err(|err| GatewayError::Internal(err.to_string()))?,
        "global_model_id": row.try_get::<Option<String>, _>("global_model_id").map_err(|err| GatewayError::Internal(err.to_string()))?,
        "model_id": row.try_get::<Option<String>, _>("model_id").map_err(|err| GatewayError::Internal(err.to_string()))?,
        "expression": row.try_get::<String, _>("expression").map_err(|err| GatewayError::Internal(err.to_string()))?,
        "variables": row.try_get::<Option<serde_json::Value>, _>("variables").map_err(|err| GatewayError::Internal(err.to_string()))?.unwrap_or_else(|| json!({})),
        "dimension_mappings": row.try_get::<Option<serde_json::Value>, _>("dimension_mappings").map_err(|err| GatewayError::Internal(err.to_string()))?.unwrap_or_else(|| json!({})),
        "is_enabled": row.try_get::<bool, _>("is_enabled").map_err(|err| GatewayError::Internal(err.to_string()))?,
        "created_at": admin_billing_optional_epoch_value(row, "created_at_unix_secs")?,
        "updated_at": admin_billing_optional_epoch_value(row, "updated_at_unix_secs")?,
    }))
}

async fn build_admin_list_billing_rules_response(
    state: &AppState,
    request_context: &GatewayPublicRequestContext,
) -> Result<Response<Body>, GatewayError> {
    let query = request_context.request_query_string.as_deref();
    let page = match admin_billing_parse_page(query) {
        Ok(value) => value,
        Err(detail) => return Ok(build_admin_billing_bad_request_response(detail)),
    };
    let page_size = match admin_billing_parse_page_size(query) {
        Ok(value) => value,
        Err(detail) => return Ok(build_admin_billing_bad_request_response(detail)),
    };
    let task_type = admin_billing_optional_filter(query, "task_type");
    let is_enabled = match admin_billing_optional_bool_filter(query, "is_enabled") {
        Ok(value) => value,
        Err(detail) => return Ok(build_admin_billing_bad_request_response(detail)),
    };

    let mut total = 0_u64;
    let mut items = Vec::new();
    if let Some((records, record_total)) = state
        .list_admin_billing_rules(task_type.as_deref(), is_enabled, page, page_size)
        .await?
    {
        total = record_total;
        items = records
            .iter()
            .map(build_admin_billing_rule_payload_from_record)
            .collect::<Vec<_>>();
    } else if let Some(pool) = state.postgres_pool() {
        let count_row = sqlx::query(
            r#"
SELECT COUNT(*) AS total
FROM billing_rules
WHERE ($1::TEXT IS NULL OR task_type = $1)
  AND ($2::BOOL IS NULL OR is_enabled = $2)
            "#,
        )
        .bind(task_type.as_deref())
        .bind(is_enabled)
        .fetch_one(&pool)
        .await
        .map_err(|err| GatewayError::Internal(err.to_string()))?;
        total = count_row
            .try_get::<i64, _>("total")
            .map_err(|err| GatewayError::Internal(err.to_string()))?
            .max(0) as u64;

        let offset = u64::from(page.saturating_sub(1) * page_size);
        let rows = sqlx::query(
            r#"
SELECT
  id,
  name,
  task_type,
  global_model_id,
  model_id,
  expression,
  variables,
  dimension_mappings,
  is_enabled,
  CAST(EXTRACT(EPOCH FROM created_at) AS BIGINT) AS created_at_unix_secs,
  CAST(EXTRACT(EPOCH FROM updated_at) AS BIGINT) AS updated_at_unix_secs
FROM billing_rules
WHERE ($1::TEXT IS NULL OR task_type = $1)
  AND ($2::BOOL IS NULL OR is_enabled = $2)
ORDER BY updated_at DESC
OFFSET $3
LIMIT $4
            "#,
        )
        .bind(task_type.as_deref())
        .bind(is_enabled)
        .bind(i64::try_from(offset).map_err(|err| GatewayError::Internal(err.to_string()))?)
        .bind(i64::from(page_size))
        .fetch_all(&pool)
        .await
        .map_err(|err| GatewayError::Internal(err.to_string()))?;

        items = rows
            .iter()
            .map(admin_billing_rule_payload)
            .collect::<Result<Vec<_>, GatewayError>>()?;
    }

    Ok(Json(json!({
        "items": items,
        "total": total,
        "page": page,
        "page_size": page_size,
        "pages": admin_billing_pages(total, page_size),
    }))
    .into_response())
}

async fn build_admin_get_billing_rule_response(
    state: &AppState,
    request_context: &GatewayPublicRequestContext,
) -> Result<Response<Body>, GatewayError> {
    let Some(rule_id) = admin_billing_rule_id_from_path(&request_context.request_path) else {
        return Ok(build_admin_billing_bad_request_response("缺少 rule_id"));
    };
    if let Some(record) = state.read_admin_billing_rule(&rule_id).await? {
        return Ok(Json(build_admin_billing_rule_payload_from_record(&record)).into_response());
    }
    let Some(pool) = state.postgres_pool() else {
        return Ok(build_admin_billing_not_found_response(
            "Billing rule not found",
        ));
    };

    let row = sqlx::query(
        r#"
SELECT
  id,
  name,
  task_type,
  global_model_id,
  model_id,
  expression,
  variables,
  dimension_mappings,
  is_enabled,
  CAST(EXTRACT(EPOCH FROM created_at) AS BIGINT) AS created_at_unix_secs,
  CAST(EXTRACT(EPOCH FROM updated_at) AS BIGINT) AS updated_at_unix_secs
FROM billing_rules
WHERE id = $1
        "#,
    )
    .bind(&rule_id)
    .fetch_optional(&pool)
    .await
    .map_err(|err| GatewayError::Internal(err.to_string()))?;

    match row {
        Some(row) => Ok(Json(admin_billing_rule_payload(&row)?).into_response()),
        None => Ok(build_admin_billing_not_found_response(
            "Billing rule not found",
        )),
    }
}

async fn build_admin_create_billing_rule_response(
    state: &AppState,
    request_body: Option<&Bytes>,
) -> Result<Response<Body>, GatewayError> {
    let input = match parse_admin_billing_rule_request(request_body) {
        Ok(value) => value,
        Err(response) => return Ok(response),
    };
    match state.create_admin_billing_rule(&input).await? {
        crate::gateway::LocalMutationOutcome::Applied(record) => {
            Ok(Json(build_admin_billing_rule_payload_from_record(&record)).into_response())
        }
        crate::gateway::LocalMutationOutcome::Invalid(detail) => {
            Ok(build_admin_billing_bad_request_response(detail))
        }
        crate::gateway::LocalMutationOutcome::NotFound => Ok(
            build_admin_billing_not_found_response("Billing rule not found"),
        ),
        crate::gateway::LocalMutationOutcome::Unavailable => Ok(
            build_admin_billing_read_only_response("当前为只读模式，无法创建计费规则"),
        ),
    }
}

async fn build_admin_update_billing_rule_response(
    state: &AppState,
    request_context: &GatewayPublicRequestContext,
    request_body: Option<&Bytes>,
) -> Result<Response<Body>, GatewayError> {
    let Some(rule_id) = admin_billing_rule_id_from_path(&request_context.request_path) else {
        return Ok(build_admin_billing_bad_request_response("缺少 rule_id"));
    };
    let input = match parse_admin_billing_rule_request(request_body) {
        Ok(value) => value,
        Err(response) => return Ok(response),
    };
    match state.update_admin_billing_rule(&rule_id, &input).await? {
        crate::gateway::LocalMutationOutcome::Applied(record) => {
            Ok(Json(build_admin_billing_rule_payload_from_record(&record)).into_response())
        }
        crate::gateway::LocalMutationOutcome::NotFound => Ok(
            build_admin_billing_not_found_response("Billing rule not found"),
        ),
        crate::gateway::LocalMutationOutcome::Invalid(detail) => {
            Ok(build_admin_billing_bad_request_response(detail))
        }
        crate::gateway::LocalMutationOutcome::Unavailable => Ok(
            build_admin_billing_read_only_response("当前为只读模式，无法更新计费规则"),
        ),
    }
}

pub(super) async fn maybe_build_local_admin_billing_rules_response(
    state: &AppState,
    request_context: &GatewayPublicRequestContext,
    request_body: Option<&Bytes>,
) -> Result<Option<Response<Body>>, GatewayError> {
    let Some(decision) = request_context.control_decision.as_ref() else {
        return Ok(None);
    };
    let path = request_context.request_path.as_str();

    match decision.route_kind.as_deref() {
        Some("list_rules")
            if request_context.request_method == http::Method::GET
                && matches!(
                    path,
                    "/api/admin/billing/rules" | "/api/admin/billing/rules/"
                ) =>
        {
            Ok(Some(
                build_admin_list_billing_rules_response(state, request_context).await?,
            ))
        }
        Some("get_rule")
            if request_context.request_method == http::Method::GET
                && path.starts_with("/api/admin/billing/rules/") =>
        {
            Ok(Some(
                build_admin_get_billing_rule_response(state, request_context).await?,
            ))
        }
        Some("create_rule")
            if request_context.request_method == http::Method::POST
                && matches!(
                    path,
                    "/api/admin/billing/rules" | "/api/admin/billing/rules/"
                ) =>
        {
            Ok(Some(
                build_admin_create_billing_rule_response(state, request_body).await?,
            ))
        }
        Some("update_rule")
            if request_context.request_method == http::Method::PUT
                && path.starts_with("/api/admin/billing/rules/") =>
        {
            Ok(Some(
                build_admin_update_billing_rule_response(state, request_context, request_body)
                    .await?,
            ))
        }
        _ => Ok(None),
    }
}
