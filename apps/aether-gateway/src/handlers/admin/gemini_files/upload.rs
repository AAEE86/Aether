use super::{
    admin_gemini_files_error_response, admin_gemini_files_key_capable,
    ADMIN_GEMINI_FILES_DATA_UNAVAILABLE_DETAIL,
};
use crate::gateway::handlers::{is_admin_gemini_files_upload_root, query_param_value};
use crate::gateway::{AppState, GatewayError, GatewayPublicRequestContext};
use aether_contracts::{ExecutionPlan, ExecutionResult, RequestBody};
use aether_data::repository::provider_catalog::{
    StoredProviderCatalogEndpoint, StoredProviderCatalogKey,
};
use axum::body::{Body, Bytes};
use axum::http::{self, Response};
use axum::response::IntoResponse;
use axum::Json;
use base64::Engine as _;
use serde_json::json;
use std::collections::{BTreeMap, BTreeSet};

#[derive(Debug, Clone)]
struct AdminGeminiFilesUploadRequest {
    display_name: String,
    mime_type: String,
    body_bytes: Vec<u8>,
    body_bytes_b64: String,
}

#[derive(Debug, Clone)]
struct AdminGeminiFilesUploadExecutionSuccess {
    file_name: String,
    display_name: Option<String>,
    mime_type: Option<String>,
}

pub(super) async fn maybe_build_local_admin_gemini_files_upload_response(
    state: &AppState,
    request_context: &GatewayPublicRequestContext,
    request_body: Option<&Bytes>,
) -> Result<Option<Response<Body>>, GatewayError> {
    match request_context
        .control_decision
        .as_ref()
        .and_then(|decision| decision.route_kind.as_deref())
    {
        Some("upload")
            if request_context.request_method == http::Method::POST
                && is_admin_gemini_files_upload_root(&request_context.request_path) =>
        {
            if !state.has_gemini_file_mapping_data_writer() {
                return Ok(Some(admin_gemini_files_error_response(
                    http::StatusCode::SERVICE_UNAVAILABLE,
                    ADMIN_GEMINI_FILES_DATA_UNAVAILABLE_DETAIL,
                )));
            }
            let upload =
                match admin_gemini_files_parse_upload_request(request_context, request_body) {
                    Ok(upload) => upload,
                    Err(detail) => {
                        return Ok(Some(admin_gemini_files_error_response(
                            http::StatusCode::BAD_REQUEST,
                            detail,
                        )));
                    }
                };
            let key_ids = admin_gemini_files_query_key_ids(request_context);
            if key_ids.is_empty() {
                return Ok(Some(admin_gemini_files_error_response(
                    http::StatusCode::BAD_REQUEST,
                    "key_ids 不能为空",
                )));
            }
            let response = admin_gemini_files_upload_across_keys(
                state,
                "",
                request_context.trace_id.as_str(),
                &upload,
                &key_ids,
            )
            .await?;
            Ok(Some(Json(response).into_response()))
        }
        _ => Ok(None),
    }
}

fn admin_gemini_files_query_key_ids(request_context: &GatewayPublicRequestContext) -> Vec<String> {
    let mut key_ids = Vec::new();
    let mut seen = BTreeSet::new();
    let Some(raw) = query_param_value(request_context.request_query_string.as_deref(), "key_ids")
    else {
        return key_ids;
    };
    for key_id in raw.split(',') {
        let trimmed = key_id.trim();
        if trimmed.is_empty() || !seen.insert(trimmed.to_string()) {
            continue;
        }
        key_ids.push(trimmed.to_string());
    }
    key_ids
}

fn admin_gemini_files_parse_upload_request(
    request_context: &GatewayPublicRequestContext,
    request_body: Option<&axum::body::Bytes>,
) -> Result<AdminGeminiFilesUploadRequest, String> {
    let content_type = request_context
        .request_content_type
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .ok_or_else(|| "Content-Type 缺失".to_string())?;
    let boundary = admin_gemini_files_multipart_boundary(content_type)?;
    let body = request_body
        .filter(|body| !body.is_empty())
        .ok_or_else(|| "上传文件不能为空".to_string())?;
    let (display_name, mime_type, body_bytes) =
        admin_gemini_files_extract_file_part(body.as_ref(), &boundary)?;
    Ok(AdminGeminiFilesUploadRequest {
        display_name,
        mime_type,
        body_bytes_b64: base64::engine::general_purpose::STANDARD.encode(&body_bytes),
        body_bytes,
    })
}

fn admin_gemini_files_multipart_boundary(content_type: &str) -> Result<String, String> {
    let normalized = content_type.trim();
    if !normalized
        .to_ascii_lowercase()
        .starts_with("multipart/form-data")
    {
        return Err("Content-Type 必须是 multipart/form-data".to_string());
    }
    for part in normalized.split(';').skip(1) {
        let Some((key, value)) = part.trim().split_once('=') else {
            continue;
        };
        if !key.trim().eq_ignore_ascii_case("boundary") {
            continue;
        }
        let boundary = value.trim().trim_matches('"').trim();
        if !boundary.is_empty() {
            return Ok(boundary.to_string());
        }
    }
    Err("multipart boundary 缺失".to_string())
}

fn admin_gemini_files_extract_file_part(
    body: &[u8],
    boundary: &str,
) -> Result<(String, String, Vec<u8>), String> {
    let boundary_marker = format!("--{boundary}");
    let next_boundary_marker = format!("\r\n--{boundary}");
    let boundary_bytes = boundary_marker.as_bytes();
    let next_boundary_bytes = next_boundary_marker.as_bytes();

    let mut cursor = 0usize;
    while cursor < body.len() {
        if !body[cursor..].starts_with(boundary_bytes) {
            return Err("multipart body 格式无效".to_string());
        }
        cursor += boundary_bytes.len();
        if body[cursor..].starts_with(b"--") {
            break;
        }
        if !body[cursor..].starts_with(b"\r\n") {
            return Err("multipart body 缺少头部分隔符".to_string());
        }
        cursor += 2;
        let Some(headers_end_rel) = admin_gemini_files_find_subslice(&body[cursor..], b"\r\n\r\n")
        else {
            return Err("multipart part 缺少头部".to_string());
        };
        let headers_end = cursor + headers_end_rel;
        let headers_text = std::str::from_utf8(&body[cursor..headers_end])
            .map_err(|_| "multipart part 头部编码无效".to_string())?;
        cursor = headers_end + 4;
        let Some(next_boundary_rel) =
            admin_gemini_files_find_subslice(&body[cursor..], next_boundary_bytes)
        else {
            return Err("multipart body 缺少结束边界".to_string());
        };
        let content_end = cursor + next_boundary_rel;
        let content = &body[cursor..content_end];
        cursor = content_end + 2;

        let Some((field_name, file_name, mime_type)) =
            admin_gemini_files_parse_part_headers(headers_text)
        else {
            continue;
        };
        if field_name != "file" {
            continue;
        }
        return Ok((
            file_name.unwrap_or_else(|| "uploaded-file".to_string()),
            mime_type.unwrap_or_else(|| "application/octet-stream".to_string()),
            content.to_vec(),
        ));
    }

    Err("multipart body 中缺少 file 字段".to_string())
}

fn admin_gemini_files_parse_part_headers(
    headers_text: &str,
) -> Option<(String, Option<String>, Option<String>)> {
    let mut field_name = None;
    let mut file_name = None;
    let mut mime_type = None;

    for line in headers_text.split("\r\n") {
        let Some((header_name, header_value)) = line.split_once(':') else {
            continue;
        };
        let header_name = header_name.trim();
        let header_value = header_value.trim();
        if header_name.eq_ignore_ascii_case("content-disposition") {
            for part in header_value.split(';').skip(1) {
                let Some((key, value)) = part.trim().split_once('=') else {
                    continue;
                };
                let key = key.trim();
                let value = value.trim().trim_matches('"').trim();
                if key.eq_ignore_ascii_case("name") && !value.is_empty() {
                    field_name = Some(value.to_string());
                } else if key.eq_ignore_ascii_case("filename") && !value.is_empty() {
                    file_name = Some(value.to_string());
                }
            }
        } else if header_name.eq_ignore_ascii_case("content-type") && !header_value.is_empty() {
            mime_type = Some(header_value.to_string());
        }
    }

    field_name.map(|field_name| (field_name, file_name, mime_type))
}

fn admin_gemini_files_find_subslice(haystack: &[u8], needle: &[u8]) -> Option<usize> {
    if haystack.is_empty() || needle.is_empty() || haystack.len() < needle.len() {
        return None;
    }
    haystack
        .windows(needle.len())
        .position(|window| window == needle)
}

async fn admin_gemini_files_upload_across_keys(
    state: &AppState,
    execution_runtime_base_url: &str,
    trace_id: &str,
    upload: &AdminGeminiFilesUploadRequest,
    requested_key_ids: &[String],
) -> Result<serde_json::Value, GatewayError> {
    let keys = state
        .read_provider_catalog_keys_by_ids(requested_key_ids)
        .await?;
    let key_by_id = keys
        .iter()
        .map(|key| (key.id.as_str(), key))
        .collect::<BTreeMap<_, _>>();
    let provider_ids = keys
        .iter()
        .map(|key| key.provider_id.clone())
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect::<Vec<_>>();
    let endpoints = state
        .list_provider_catalog_endpoints_by_provider_ids(&provider_ids)
        .await?;
    let endpoints_by_provider_id = endpoints.into_iter().fold(
        BTreeMap::<String, Vec<StoredProviderCatalogEndpoint>>::new(),
        |mut out, endpoint| {
            out.entry(endpoint.provider_id.clone())
                .or_default()
                .push(endpoint);
            out
        },
    );

    let mut results = Vec::new();
    let mut success_count = 0usize;
    let mut fail_count = 0usize;

    for key_id in requested_key_ids {
        let Some(key) = key_by_id.get(key_id.as_str()) else {
            fail_count += 1;
            results.push(json!({
                "key_id": key_id,
                "key_name": serde_json::Value::Null,
                "success": false,
                "file_name": serde_json::Value::Null,
                "error": "Key 不存在",
            }));
            continue;
        };

        let key_name = Some(key.name.clone());
        let outcome = admin_gemini_files_upload_single_key(
            state,
            execution_runtime_base_url,
            trace_id,
            upload,
            key,
            endpoints_by_provider_id.get(&key.provider_id),
        )
        .await;

        match outcome {
            Ok(success) => {
                success_count += 1;
                results.push(json!({
                    "key_id": key.id,
                    "key_name": key_name,
                    "success": true,
                    "file_name": success.file_name,
                    "error": serde_json::Value::Null,
                }));
            }
            Err(error) => {
                fail_count += 1;
                results.push(json!({
                    "key_id": key.id,
                    "key_name": key_name,
                    "success": false,
                    "file_name": serde_json::Value::Null,
                    "error": error,
                }));
            }
        }
    }

    Ok(json!({
        "display_name": upload.display_name,
        "mime_type": upload.mime_type,
        "size_bytes": upload.body_bytes.len(),
        "results": results,
        "success_count": success_count,
        "fail_count": fail_count,
    }))
}

async fn admin_gemini_files_upload_single_key(
    state: &AppState,
    _execution_runtime_base_url: &str,
    trace_id: &str,
    upload: &AdminGeminiFilesUploadRequest,
    key: &StoredProviderCatalogKey,
    endpoints: Option<&Vec<StoredProviderCatalogEndpoint>>,
) -> Result<AdminGeminiFilesUploadExecutionSuccess, String> {
    if !admin_gemini_files_key_capable(key) {
        return Err("Key 不支持 Gemini Files".to_string());
    }
    let Some(endpoint) = endpoints.and_then(|endpoints| {
        endpoints.iter().find(|endpoint| {
            endpoint.is_active
                && endpoint
                    .api_format
                    .trim()
                    .eq_ignore_ascii_case("gemini:chat")
        })
    }) else {
        return Err("找不到有效的 gemini:chat 端点".to_string());
    };
    let transport = state
        .read_provider_transport_snapshot(&key.provider_id, &endpoint.id, &key.id)
        .await
        .map_err(|err| format!("{err:?}"))?
        .ok_or_else(|| "无法读取 Key 传输配置".to_string())?;
    if !crate::gateway::provider_transport::supports_local_gemini_transport_with_network(
        &transport,
        "gemini:chat",
    ) {
        return Err("Key 传输配置不支持 Gemini Files 上传".to_string());
    }
    if transport.endpoint.body_rules.is_some() {
        return Err("Gemini Files 二进制上传暂不支持 endpoint body_rules".to_string());
    }
    let (auth_header, auth_value) =
        crate::gateway::provider_transport::resolve_local_gemini_auth(&transport)
            .ok_or_else(|| "Key 缺少可用的 Gemini 认证信息".to_string())?;

    let mut provider_request_headers =
        crate::gateway::provider_transport::build_passthrough_headers_with_auth(
            &http::HeaderMap::new(),
            &auth_header,
            &auth_value,
            &BTreeMap::new(),
        );
    provider_request_headers.insert("content-type".to_string(), upload.mime_type.clone());
    let original_request_body = json!({
        "body_bytes_b64": upload.body_bytes_b64,
    });
    if !crate::gateway::provider_transport::apply_local_header_rules(
        &mut provider_request_headers,
        transport.endpoint.header_rules.as_ref(),
        &[auth_header.as_str(), "content-type"],
        &original_request_body,
        Some(&original_request_body),
    ) {
        return Err("Key 端点 header_rules 应用失败".to_string());
    }

    let upload_path = transport
        .endpoint
        .custom_path
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .unwrap_or("/upload/v1beta/files");
    let upload_query = if upload_path.contains("uploadType=") {
        None
    } else {
        Some("uploadType=resumable")
    };
    let upstream_url = crate::gateway::provider_transport::build_gemini_files_passthrough_url(
        &transport.endpoint.base_url,
        upload_path,
        upload_query,
    )
    .ok_or_else(|| "无法构建 Gemini Files 上传地址".to_string())?;

    let plan = ExecutionPlan {
        request_id: format!("{trace_id}:admin-gemini-upload:{}", key.id),
        candidate_id: None,
        provider_name: Some(transport.provider.name.clone()),
        provider_id: transport.provider.id.clone(),
        endpoint_id: transport.endpoint.id.clone(),
        key_id: transport.key.id.clone(),
        method: "POST".to_string(),
        url: upstream_url,
        headers: provider_request_headers,
        content_type: Some(upload.mime_type.clone()),
        content_encoding: None,
        body: RequestBody {
            json_body: None,
            body_bytes_b64: Some(upload.body_bytes_b64.clone()),
            body_ref: None,
        },
        stream: false,
        client_api_format: "gemini:files".to_string(),
        provider_api_format: "gemini:files".to_string(),
        model_name: Some("gemini-files".to_string()),
        proxy: crate::gateway::provider_transport::resolve_transport_proxy_snapshot_with_tunnel_affinity(state, &transport).await,
        tls_profile: crate::gateway::provider_transport::resolve_transport_tls_profile(&transport),
        timeouts: crate::gateway::provider_transport::resolve_transport_execution_timeouts(
            &transport,
        ),
    };

    let result = admin_gemini_files_execute_upload_plan(state, trace_id, &plan)
        .await
        .map_err(|error| format!("{error:?}"))?;
    if result.status_code >= 400 {
        return Err(admin_gemini_files_execution_error_message(&result));
    }
    let body_json = admin_gemini_files_execution_json_body(&result)
        .ok_or_else(|| "上传成功但上游响应缺少 JSON body".to_string())?;
    let success = admin_gemini_files_upload_success_from_body(&body_json, upload)
        .ok_or_else(|| admin_gemini_files_execution_error_message(&result))?;
    crate::gateway::usage::store_local_gemini_file_mapping(
        state,
        success.file_name.as_str(),
        key.id.as_str(),
        None,
        success
            .display_name
            .as_deref()
            .or(Some(upload.display_name.as_str())),
        success
            .mime_type
            .as_deref()
            .or(Some(upload.mime_type.as_str())),
    )
    .await
    .map_err(|err| format!("上传成功但本地映射写入失败: {err:?}"))?;
    Ok(success)
}

async fn admin_gemini_files_execute_upload_plan(
    state: &AppState,
    trace_id: &str,
    plan: &ExecutionPlan,
) -> Result<ExecutionResult, GatewayError> {
    crate::gateway::execute_execution_runtime_sync_plan(state, Some(trace_id), plan).await
}

fn admin_gemini_files_execution_json_body(result: &ExecutionResult) -> Option<serde_json::Value> {
    if let Some(body_json) = result
        .body
        .as_ref()
        .and_then(|body| body.json_body.as_ref())
    {
        return Some(body_json.clone());
    }
    let content_type = result
        .headers
        .iter()
        .find(|(key, _)| key.eq_ignore_ascii_case("content-type"))
        .map(|(_, value)| value.trim().to_ascii_lowercase());
    if !content_type
        .as_deref()
        .is_some_and(|value| value.starts_with("application/json"))
    {
        return None;
    }
    let body_bytes_b64 = result
        .body
        .as_ref()
        .and_then(|body| body.body_bytes_b64.as_deref())?;
    let decoded = base64::engine::general_purpose::STANDARD
        .decode(body_bytes_b64)
        .ok()?;
    serde_json::from_slice(&decoded).ok()
}

fn admin_gemini_files_upload_success_from_body(
    body_json: &serde_json::Value,
    upload: &AdminGeminiFilesUploadRequest,
) -> Option<AdminGeminiFilesUploadExecutionSuccess> {
    let file_object = body_json
        .get("file")
        .and_then(serde_json::Value::as_object)
        .or_else(|| body_json.as_object())?;
    let file_name = file_object
        .get("name")
        .and_then(serde_json::Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())?;
    let display_name = file_object
        .get("displayName")
        .or_else(|| file_object.get("display_name"))
        .and_then(serde_json::Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(ToOwned::to_owned)
        .or_else(|| Some(upload.display_name.clone()));
    let mime_type = file_object
        .get("mimeType")
        .or_else(|| file_object.get("mime_type"))
        .and_then(serde_json::Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(ToOwned::to_owned)
        .or_else(|| Some(upload.mime_type.clone()));
    Some(AdminGeminiFilesUploadExecutionSuccess {
        file_name: file_name.to_string(),
        display_name,
        mime_type,
    })
}

fn admin_gemini_files_execution_error_message(result: &ExecutionResult) -> String {
    if let Some(body_json) = admin_gemini_files_execution_json_body(result) {
        if let Some(message) = body_json
            .get("error")
            .and_then(serde_json::Value::as_object)
            .and_then(|error| error.get("message"))
            .and_then(serde_json::Value::as_str)
            .map(str::trim)
            .filter(|value| !value.is_empty())
        {
            return message.to_string();
        }
        if let Some(message) = body_json
            .get("message")
            .and_then(serde_json::Value::as_str)
            .map(str::trim)
            .filter(|value| !value.is_empty())
        {
            return message.to_string();
        }
    }
    if let Some(error) = result
        .error
        .as_ref()
        .map(|error| error.message.trim())
        .filter(|value| !value.is_empty())
    {
        return error.to_string();
    }
    format!("上传失败，状态码 {}", result.status_code)
}
