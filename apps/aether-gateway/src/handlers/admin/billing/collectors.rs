use super::{
    admin_billing_optional_bool_filter, admin_billing_optional_epoch_value,
    admin_billing_optional_filter, admin_billing_pages, admin_billing_parse_page,
    admin_billing_parse_page_size, admin_billing_validate_safe_expression,
    build_admin_billing_bad_request_response, build_admin_billing_not_found_response,
    build_admin_billing_read_only_response, default_admin_billing_true,
    normalize_admin_billing_optional_text, normalize_admin_billing_required_text,
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

fn default_admin_billing_collector_value_type() -> String {
    "float".to_string()
}

#[derive(Debug, Deserialize)]
struct AdminBillingCollectorUpsertRequest {
    api_format: String,
    task_type: String,
    dimension_name: String,
    source_type: String,
    #[serde(default)]
    source_path: Option<String>,
    #[serde(default = "default_admin_billing_collector_value_type")]
    value_type: String,
    #[serde(default)]
    transform_expression: Option<String>,
    #[serde(default)]
    default_value: Option<String>,
    #[serde(default)]
    priority: i32,
    #[serde(default = "default_admin_billing_true")]
    is_enabled: bool,
}

fn build_admin_billing_collector_payload_from_record(
    record: &crate::gateway::AdminBillingCollectorRecord,
) -> serde_json::Value {
    json!({
        "id": record.id,
        "api_format": record.api_format,
        "task_type": record.task_type,
        "dimension_name": record.dimension_name,
        "source_type": record.source_type,
        "source_path": record.source_path,
        "value_type": record.value_type,
        "transform_expression": record.transform_expression,
        "default_value": record.default_value,
        "priority": record.priority,
        "is_enabled": record.is_enabled,
        "created_at": unix_secs_to_rfc3339(record.created_at_unix_secs),
        "updated_at": unix_secs_to_rfc3339(record.updated_at_unix_secs),
    })
}

fn admin_billing_collector_id_from_path(request_path: &str) -> Option<String> {
    let value = request_path
        .strip_prefix("/api/admin/billing/collectors/")?
        .trim()
        .trim_matches('/')
        .to_string();
    if value.is_empty() || value.contains('/') {
        None
    } else {
        Some(value)
    }
}

fn admin_billing_collector_payload(
    row: &sqlx::postgres::PgRow,
) -> Result<serde_json::Value, GatewayError> {
    Ok(json!({
        "id": row.try_get::<String, _>("id").map_err(|err| GatewayError::Internal(err.to_string()))?,
        "api_format": row.try_get::<String, _>("api_format").map_err(|err| GatewayError::Internal(err.to_string()))?,
        "task_type": row.try_get::<String, _>("task_type").map_err(|err| GatewayError::Internal(err.to_string()))?,
        "dimension_name": row.try_get::<String, _>("dimension_name").map_err(|err| GatewayError::Internal(err.to_string()))?,
        "source_type": row.try_get::<String, _>("source_type").map_err(|err| GatewayError::Internal(err.to_string()))?,
        "source_path": row.try_get::<Option<String>, _>("source_path").map_err(|err| GatewayError::Internal(err.to_string()))?,
        "value_type": row.try_get::<String, _>("value_type").map_err(|err| GatewayError::Internal(err.to_string()))?,
        "transform_expression": row.try_get::<Option<String>, _>("transform_expression").map_err(|err| GatewayError::Internal(err.to_string()))?,
        "default_value": row.try_get::<Option<String>, _>("default_value").map_err(|err| GatewayError::Internal(err.to_string()))?,
        "priority": row.try_get::<i32, _>("priority").map_err(|err| GatewayError::Internal(err.to_string()))?,
        "is_enabled": row.try_get::<bool, _>("is_enabled").map_err(|err| GatewayError::Internal(err.to_string()))?,
        "created_at": admin_billing_optional_epoch_value(row, "created_at_unix_secs")?,
        "updated_at": admin_billing_optional_epoch_value(row, "updated_at_unix_secs")?,
    }))
}

async fn build_admin_list_dimension_collectors_response(
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
    let api_format = admin_billing_optional_filter(query, "api_format");
    let task_type = admin_billing_optional_filter(query, "task_type");
    let dimension_name = admin_billing_optional_filter(query, "dimension_name");
    let is_enabled = match admin_billing_optional_bool_filter(query, "is_enabled") {
        Ok(value) => value,
        Err(detail) => return Ok(build_admin_billing_bad_request_response(detail)),
    };

    if let Some((items, total)) = state
        .list_admin_billing_collectors(
            api_format.as_deref(),
            task_type.as_deref(),
            dimension_name.as_deref(),
            is_enabled,
            page,
            page_size,
        )
        .await?
    {
        return Ok(Json(json!({
            "items": items
                .iter()
                .map(build_admin_billing_collector_payload_from_record)
                .collect::<Vec<_>>(),
            "total": total,
            "page": page,
            "page_size": page_size,
            "pages": admin_billing_pages(total, page_size),
        }))
        .into_response());
    }

    let mut total = 0_u64;
    let mut items = Vec::new();
    if let Some(pool) = state.postgres_pool() {
        let count_row = sqlx::query(
            r#"
SELECT COUNT(*) AS total
FROM dimension_collectors
WHERE ($1::TEXT IS NULL OR api_format = $1)
  AND ($2::TEXT IS NULL OR task_type = $2)
  AND ($3::TEXT IS NULL OR dimension_name = $3)
  AND ($4::BOOL IS NULL OR is_enabled = $4)
            "#,
        )
        .bind(api_format.as_deref())
        .bind(task_type.as_deref())
        .bind(dimension_name.as_deref())
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
  api_format,
  task_type,
  dimension_name,
  source_type,
  source_path,
  value_type,
  transform_expression,
  default_value,
  priority,
  is_enabled,
  CAST(EXTRACT(EPOCH FROM created_at) AS BIGINT) AS created_at_unix_secs,
  CAST(EXTRACT(EPOCH FROM updated_at) AS BIGINT) AS updated_at_unix_secs
FROM dimension_collectors
WHERE ($1::TEXT IS NULL OR api_format = $1)
  AND ($2::TEXT IS NULL OR task_type = $2)
  AND ($3::TEXT IS NULL OR dimension_name = $3)
  AND ($4::BOOL IS NULL OR is_enabled = $4)
ORDER BY updated_at DESC, priority DESC, id ASC
OFFSET $5
LIMIT $6
            "#,
        )
        .bind(api_format.as_deref())
        .bind(task_type.as_deref())
        .bind(dimension_name.as_deref())
        .bind(is_enabled)
        .bind(i64::try_from(offset).map_err(|err| GatewayError::Internal(err.to_string()))?)
        .bind(i64::from(page_size))
        .fetch_all(&pool)
        .await
        .map_err(|err| GatewayError::Internal(err.to_string()))?;

        items = rows
            .iter()
            .map(admin_billing_collector_payload)
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

async fn build_admin_get_dimension_collector_response(
    state: &AppState,
    request_context: &GatewayPublicRequestContext,
) -> Result<Response<Body>, GatewayError> {
    let Some(collector_id) = admin_billing_collector_id_from_path(&request_context.request_path)
    else {
        return Ok(build_admin_billing_bad_request_response(
            "缺少 collector_id",
        ));
    };

    if let Some(record) = state.read_admin_billing_collector(&collector_id).await? {
        return Ok(
            Json(build_admin_billing_collector_payload_from_record(&record)).into_response(),
        );
    }

    let Some(pool) = state.postgres_pool() else {
        return Ok(build_admin_billing_not_found_response(
            "Dimension collector not found",
        ));
    };

    let row = sqlx::query(
        r#"
SELECT
  id,
  api_format,
  task_type,
  dimension_name,
  source_type,
  source_path,
  value_type,
  transform_expression,
  default_value,
  priority,
  is_enabled,
  CAST(EXTRACT(EPOCH FROM created_at) AS BIGINT) AS created_at_unix_secs,
  CAST(EXTRACT(EPOCH FROM updated_at) AS BIGINT) AS updated_at_unix_secs
FROM dimension_collectors
WHERE id = $1
        "#,
    )
    .bind(&collector_id)
    .fetch_optional(&pool)
    .await
    .map_err(|err| GatewayError::Internal(err.to_string()))?;

    match row {
        Some(row) => Ok(Json(admin_billing_collector_payload(&row)?).into_response()),
        None => Ok(build_admin_billing_not_found_response(
            "Dimension collector not found",
        )),
    }
}

async fn parse_admin_billing_collector_request(
    state: &AppState,
    request_body: Option<&Bytes>,
    existing_id: Option<&str>,
) -> Result<crate::gateway::AdminBillingCollectorWriteInput, Response<Body>> {
    let Some(request_body) = request_body else {
        return Err(build_admin_billing_bad_request_response("请求体不能为空"));
    };
    let request = match serde_json::from_slice::<AdminBillingCollectorUpsertRequest>(request_body) {
        Ok(value) => value,
        Err(err) => {
            return Err(build_admin_billing_bad_request_response(format!(
                "Invalid request body: {err}"
            )))
        }
    };

    let api_format =
        match normalize_admin_billing_required_text(&request.api_format, "api_format", 50) {
            Ok(value) => value.to_ascii_uppercase(),
            Err(detail) => return Err(build_admin_billing_bad_request_response(detail)),
        };
    let task_type = match normalize_admin_billing_required_text(&request.task_type, "task_type", 20)
    {
        Ok(value) => value.to_ascii_lowercase(),
        Err(detail) => return Err(build_admin_billing_bad_request_response(detail)),
    };
    let dimension_name =
        match normalize_admin_billing_required_text(&request.dimension_name, "dimension_name", 100)
        {
            Ok(value) => value,
            Err(detail) => return Err(build_admin_billing_bad_request_response(detail)),
        };
    let source_type = request.source_type.trim().to_ascii_lowercase();
    if !matches!(
        source_type.as_str(),
        "request" | "response" | "metadata" | "computed"
    ) {
        return Err(build_admin_billing_bad_request_response(
            "source_type must be one of request, response, metadata, computed",
        ));
    }
    let value_type = request.value_type.trim().to_ascii_lowercase();
    if !matches!(value_type.as_str(), "float" | "int" | "string") {
        return Err(build_admin_billing_bad_request_response(
            "value_type must be one of float, int, string",
        ));
    }
    let source_path = match normalize_admin_billing_optional_text(request.source_path, 200) {
        Ok(value) => value,
        Err(detail) => return Err(build_admin_billing_bad_request_response(detail)),
    };
    let transform_expression =
        match normalize_admin_billing_optional_text(request.transform_expression, 4096) {
            Ok(value) => value,
            Err(detail) => return Err(build_admin_billing_bad_request_response(detail)),
        };
    let default_value = match normalize_admin_billing_optional_text(request.default_value, 100) {
        Ok(value) => value,
        Err(detail) => return Err(build_admin_billing_bad_request_response(detail)),
    };

    if source_type == "computed" {
        if source_path.is_some() {
            return Err(build_admin_billing_bad_request_response(
                "computed collector must have source_path=null",
            ));
        }
        if transform_expression.is_none() {
            return Err(build_admin_billing_bad_request_response(
                "computed collector must have transform_expression",
            ));
        }
    } else if source_path.is_none() {
        return Err(build_admin_billing_bad_request_response(
            "non-computed collector must have source_path",
        ));
    }

    if let Some(transform_expression) = transform_expression.as_deref() {
        if let Err(detail) = admin_billing_validate_safe_expression(transform_expression) {
            return Err(build_admin_billing_bad_request_response(format!(
                "Invalid transform_expression: {detail}"
            )));
        }
    }

    if default_value.is_some() && request.is_enabled {
        match state
            .admin_billing_enabled_default_value_exists(
                &api_format,
                &task_type,
                &dimension_name,
                existing_id,
            )
            .await
        {
            Ok(true) => {
                return Err(build_admin_billing_bad_request_response(
                    "default_value already exists for this (api_format, task_type, dimension_name)",
                ))
            }
            Ok(false) => {}
            Err(err) => {
                let detail = match err {
                    GatewayError::Internal(message) => message,
                    other => format!("{other:?}"),
                };
                return Err((
                    http::StatusCode::INTERNAL_SERVER_ERROR,
                    Json(json!({ "detail": detail })),
                )
                    .into_response());
            }
        }
    }

    Ok(crate::gateway::AdminBillingCollectorWriteInput {
        api_format,
        task_type,
        dimension_name,
        source_type,
        source_path,
        value_type,
        transform_expression,
        default_value,
        priority: request.priority,
        is_enabled: request.is_enabled,
    })
}

async fn build_admin_create_dimension_collector_response(
    state: &AppState,
    request_body: Option<&Bytes>,
) -> Result<Response<Body>, GatewayError> {
    let input = match parse_admin_billing_collector_request(state, request_body, None).await {
        Ok(value) => value,
        Err(response) => return Ok(response),
    };
    match state.create_admin_billing_collector(&input).await? {
        crate::gateway::LocalMutationOutcome::Applied(record) => {
            Ok(Json(build_admin_billing_collector_payload_from_record(&record)).into_response())
        }
        crate::gateway::LocalMutationOutcome::Invalid(detail) => {
            Ok(build_admin_billing_bad_request_response(detail))
        }
        crate::gateway::LocalMutationOutcome::NotFound => Ok(
            build_admin_billing_not_found_response("Dimension collector not found"),
        ),
        crate::gateway::LocalMutationOutcome::Unavailable => Ok(
            build_admin_billing_read_only_response("当前为只读模式，无法创建维度采集器"),
        ),
    }
}

async fn build_admin_update_dimension_collector_response(
    state: &AppState,
    request_context: &GatewayPublicRequestContext,
    request_body: Option<&Bytes>,
) -> Result<Response<Body>, GatewayError> {
    let Some(collector_id) = admin_billing_collector_id_from_path(&request_context.request_path)
    else {
        return Ok(build_admin_billing_bad_request_response(
            "缺少 collector_id",
        ));
    };
    let input =
        match parse_admin_billing_collector_request(state, request_body, Some(&collector_id)).await
        {
            Ok(value) => value,
            Err(response) => return Ok(response),
        };
    match state
        .update_admin_billing_collector(&collector_id, &input)
        .await?
    {
        crate::gateway::LocalMutationOutcome::Applied(record) => {
            Ok(Json(build_admin_billing_collector_payload_from_record(&record)).into_response())
        }
        crate::gateway::LocalMutationOutcome::NotFound => Ok(
            build_admin_billing_not_found_response("Dimension collector not found"),
        ),
        crate::gateway::LocalMutationOutcome::Invalid(detail) => {
            Ok(build_admin_billing_bad_request_response(detail))
        }
        crate::gateway::LocalMutationOutcome::Unavailable => Ok(
            build_admin_billing_read_only_response("当前为只读模式，无法更新维度采集器"),
        ),
    }
}

pub(super) async fn maybe_build_local_admin_billing_collectors_response(
    state: &AppState,
    request_context: &GatewayPublicRequestContext,
    request_body: Option<&Bytes>,
) -> Result<Option<Response<Body>>, GatewayError> {
    let Some(decision) = request_context.control_decision.as_ref() else {
        return Ok(None);
    };
    let path = request_context.request_path.as_str();

    match decision.route_kind.as_deref() {
        Some("list_collectors")
            if request_context.request_method == http::Method::GET
                && matches!(
                    path,
                    "/api/admin/billing/collectors" | "/api/admin/billing/collectors/"
                ) =>
        {
            Ok(Some(
                build_admin_list_dimension_collectors_response(state, request_context).await?,
            ))
        }
        Some("get_collector")
            if request_context.request_method == http::Method::GET
                && path.starts_with("/api/admin/billing/collectors/") =>
        {
            Ok(Some(
                build_admin_get_dimension_collector_response(state, request_context).await?,
            ))
        }
        Some("create_collector")
            if request_context.request_method == http::Method::POST
                && matches!(
                    path,
                    "/api/admin/billing/collectors" | "/api/admin/billing/collectors/"
                ) =>
        {
            Ok(Some(
                build_admin_create_dimension_collector_response(state, request_body).await?,
            ))
        }
        Some("update_collector")
            if request_context.request_method == http::Method::PUT
                && path.starts_with("/api/admin/billing/collectors/") =>
        {
            Ok(Some(
                build_admin_update_dimension_collector_response(
                    state,
                    request_context,
                    request_body,
                )
                .await?,
            ))
        }
        _ => Ok(None),
    }
}
