"""
Gemini Files API 代理端点

代理 Google Gemini Files API，支持文件的上传、查询、删除等操作。

端点列表：
- POST /upload/v1beta/files - 上传文件（可恢复上传）
- GET /v1beta/files - 列出文件
- GET /v1beta/files/{name} - 获取文件元数据
- DELETE /v1beta/files/{name} - 删除文件

认证方式：
- x-goog-api-key 请求头
- ?key= URL 参数

参考文档：
https://ai.google.dev/api/files
"""

from __future__ import annotations

from typing import Any, Dict, Optional, Tuple
from urllib.parse import urlencode

from fastapi import APIRouter, Depends, HTTPException, Request, Response
from fastapi.responses import JSONResponse
from sqlalchemy.orm import Session

from src.clients.http_client import HTTPClientPool
from src.core.api_format import APIFormat, extract_client_api_key_with_query
from src.core.api_format.metadata import get_api_format_definition
from src.core.crypto import crypto_service
from src.core.logger import logger
from src.database import get_db
from src.models.database import ApiKey, GlobalModel, Model, Provider, ProviderEndpoint, User
from src.services.auth.service import AuthService
from src.services.cache.aware_scheduler import CacheAwareScheduler, ProviderCandidate
from src.services.gemini_files_mapping import delete_file_key_mapping, store_file_key_mapping
from src.services.provider.transport import redact_url_for_log

# 从配置获取路径前缀
_gemini_def = get_api_format_definition(APIFormat.GEMINI)

router = APIRouter(tags=["Gemini Files API"], prefix=_gemini_def.path_prefix)

# Gemini Files API 基础 URL
GEMINI_FILES_BASE_URL = "https://generativelanguage.googleapis.com"

# Gemini Files API 能力标签
REQUIRED_CAPABILITIES = {"gemini_files_api": True}

# 需要从客户端请求中移除的头部（这些会由代理重新设置或不应转发）
HEADERS_TO_REMOVE = frozenset({
    "host",
    "content-length",
    "transfer-encoding",
    "connection",
    "x-goog-api-key",
    "authorization",
})


def _extract_gemini_api_key(request: Request) -> str | None:
    """
    从请求中提取 Gemini API Key

    优先级（与 Google SDK 行为一致）：
    1. URL 参数 ?key=
    2. x-goog-api-key 请求头
    """
    return extract_client_api_key_with_query(
        dict(request.headers),
        dict(request.query_params),
        APIFormat.GEMINI,
    )


def _build_upstream_headers(
    original_headers: dict[str, str],
    upstream_api_key: str,
) -> dict[str, str]:
    """
    构建上游请求头

    Args:
        original_headers: 原始请求头
        upstream_api_key: 上游 API Key

    Returns:
        处理后的请求头字典
    """
    headers = {}

    # 透传非敏感头部
    for name, value in original_headers.items():
        if name.lower() not in HEADERS_TO_REMOVE:
            headers[name] = value

    # 设置认证头
    headers["x-goog-api-key"] = upstream_api_key

    return headers


def _build_upstream_url(
    base_url: str,
    path: str,
    query_params: dict[str, Any] | None = None,
    is_upload: bool = False,
) -> str:
    """
    构建上游 URL

    Args:
        base_url: 上游基础 URL
        path: API 路径
        query_params: 查询参数
        is_upload: 是否为上传端点

    Returns:
        完整的上游 URL
    """
    # 移除 key 参数（认证通过 header）
    effective_params = dict(query_params) if query_params else {}
    effective_params.pop("key", None)

    # 处理 base_url 可能包含 /v1beta 的情况，避免重复
    normalized_base_url = base_url.rstrip("/")
    if normalized_base_url.endswith("/v1beta"):
        normalized_base_url = normalized_base_url[:-len("/v1beta")]

    # 上传端点使用不同的路径前缀
    if is_upload:
        url = f"{normalized_base_url}/upload{path}"
    else:
        url = f"{normalized_base_url}{path}"

    if effective_params:
        query_string = urlencode(effective_params, doseq=True)
        url = f"{url}?{query_string}"

    return url


def _resolve_files_model_name(
    db: Session,
    user_api_key: ApiKey,
    user: User | None,
) -> str | None:
    """
    为 Files API 选择一个可用的模型名（用于 Key 选择与权限过滤）

    选择顺序:
    1. 用户/Key 的 allowed_models（取交集后选第一个）
    2. 任意支持 Gemini 格式的 GlobalModel
    """
    from src.core.model_permissions import merge_allowed_models

    allowed_models = merge_allowed_models(
        user_api_key.allowed_models,
        user.allowed_models if user else None,
    )
    if allowed_models is not None:
        if not allowed_models:
            return None
        return sorted(allowed_models)[0]

    row = (
        db.query(GlobalModel.name)
        .join(Model, Model.global_model_id == GlobalModel.id)
        .join(Provider, Provider.id == Model.provider_id)
        .join(ProviderEndpoint, ProviderEndpoint.provider_id == Provider.id)
        .filter(
            GlobalModel.is_active == True,
            Model.is_active == True,
            Provider.is_active == True,
            ProviderEndpoint.is_active == True,
            ProviderEndpoint.api_format == APIFormat.GEMINI.value,
        )
        .distinct()
        .order_by(GlobalModel.name.asc())
        .first()
    )
    return row[0] if row else None


async def _select_provider_candidate(
    db: Session,
    user_api_key: ApiKey,
    model_name: str,
) -> ProviderCandidate | None:
    """选择支持 Files API 的 Provider/Endpoint/Key 组合"""
    scheduler = CacheAwareScheduler()
    candidates, _global_model_id = await scheduler.list_all_candidates(
        db=db,
        api_format=APIFormat.GEMINI,
        model_name=model_name,
        affinity_key=str(user_api_key.id),
        user_api_key=user_api_key,
        max_candidates=10,
        capability_requirements=REQUIRED_CAPABILITIES,
    )
    for candidate in candidates:
        auth_type = getattr(candidate.key, "auth_type", "api_key") or "api_key"
        if auth_type == "api_key":
            return candidate
    return None


async def _resolve_upstream_context(
    request: Request,
    db: Session,
) -> tuple[str, str, str]:
    """
    解析上游 Key 与 Base URL

    仅允许系统 API Key，通过能力标签选择支持 Files API 的 Provider Key。
    """
    client_key = _extract_gemini_api_key(request)
    if not client_key:
        raise HTTPException(
            status_code=401,
            detail={
                "error": {
                    "code": 401,
                    "message": "API key required. Provide via x-goog-api-key header or ?key= parameter.",
                    "status": "UNAUTHENTICATED",
                }
            },
        )

    auth_result = AuthService.authenticate_api_key(db, client_key)
    if not auth_result:
        raise HTTPException(
            status_code=401,
            detail={
                "error": {
                    "code": 401,
                    "message": "API key not valid. Please pass a valid API key.",
                    "status": "UNAUTHENTICATED",
                }
            },
        )

    user, user_api_key = auth_result
    model_name = _resolve_files_model_name(db, user_api_key, user)
    if not model_name:
        raise HTTPException(
            status_code=503,
            detail={
                "error": {
                    "code": 503,
                    "message": "No available model for Gemini Files API routing",
                    "status": "UNAVAILABLE",
                }
            },
        )

    candidate = await _select_provider_candidate(db, user_api_key, model_name)
    if not candidate:
        raise HTTPException(
            status_code=503,
            detail={
                "error": {
                    "code": 503,
                    "message": "No available key with gemini_files_api capability",
                    "status": "UNAVAILABLE",
                }
            },
        )

    try:
        upstream_key = crypto_service.decrypt(candidate.key.api_key)
    except Exception as exc:
        logger.error(f"Failed to decrypt provider key for Gemini Files API: {exc}")
        raise HTTPException(
            status_code=500,
            detail={
                "error": {
                    "code": 500,
                    "message": "Failed to decrypt provider key",
                    "status": "INTERNAL",
                }
            },
        )

    base_url = candidate.endpoint.base_url or GEMINI_FILES_BASE_URL
    return upstream_key, base_url, str(candidate.key.id)


async def _proxy_request(
    method: str,
    upstream_url: str,
    headers: dict[str, str],
    content: bytes | None = None,
    json_body: dict[str, Any] | None = None,
    file_key_id: str | None = None,
) -> Response:
    """
    代理请求到上游 Gemini API

    Args:
        method: HTTP 方法
        upstream_url: 上游 URL
        headers: 请求头
        content: 原始请求体（二进制）
        json_body: JSON 请求体
        file_key_id: 上游 Provider Key ID，用于成功响应时存储 file→key 映射

    Returns:
        FastAPI Response 对象
    """
    client = await HTTPClientPool.get_default_client_async()

    try:
        if method.upper() == "GET":
            response = await client.get(upstream_url, headers=headers)
        elif method.upper() == "DELETE":
            response = await client.delete(upstream_url, headers=headers)
        elif method.upper() == "POST":
            if content is not None:
                response = await client.post(
                    upstream_url, headers=headers, content=content
                )
            elif json_body is not None:
                response = await client.post(
                    upstream_url, headers=headers, json=json_body
                )
            else:
                response = await client.post(upstream_url, headers=headers)
        else:
            raise HTTPException(status_code=405, detail="Method not allowed")

        # 构建响应头（排除 hop-by-hop 头部）
        response_headers = {}
        hop_by_hop = {"connection", "keep-alive", "transfer-encoding", "upgrade"}
        for name, value in response.headers.items():
            if name.lower() not in hop_by_hop:
                response_headers[name] = value

        if (
            file_key_id
            and response.status_code < 300
            and response.headers.get("content-type", "").startswith("application/json")
        ):
            try:
                payload = response.json()
                file_name = None
                if isinstance(payload, dict):
                    file_name = payload.get("name")
                    if not file_name and isinstance(payload.get("file"), dict):
                        file_name = payload["file"].get("name")
                    if file_name:
                        await store_file_key_mapping(file_name, file_key_id)
                        logger.debug(
                            f"Gemini file→key 映射已存储: {file_name} → key_id={file_key_id}"
                        )

                    # 为 list_files 响应中的所有文件建立映射
                    # 这是正确的：Gemini API 按 Key 隔离文件，返回的文件必然属于当前 Key
                    files_list = payload.get("files")
                    if isinstance(files_list, list):
                        mapped_count = 0
                        for item in files_list:
                            if isinstance(item, dict) and item.get("name"):
                                await store_file_key_mapping(item["name"], file_key_id)
                                mapped_count += 1
                        if mapped_count > 0:
                            logger.debug(
                                f"Gemini list_files 批量映射已存储: {mapped_count} 个文件 → key_id={file_key_id}"
                            )
            except (ValueError, KeyError) as e:
                logger.debug(f"Failed to store Gemini file mapping: {e}")

        return Response(
            content=response.content,
            status_code=response.status_code,
            headers=response_headers,
            media_type=response.headers.get("content-type", "application/json"),
        )

    except Exception as e:
        sanitized_error = redact_url_for_log(str(e))
        logger.error(f"Gemini Files API proxy error: {sanitized_error}")
        return JSONResponse(
            status_code=502,
            content={
                "error": {
                    "code": 502,
                    "message": "Upstream request failed",
                    "status": "BAD_GATEWAY",
                }
            },
        )


# ==============================================================================
# 文件上传端点
# ==============================================================================


@router.post("/upload/v1beta/files")
async def upload_file(
    request: Request,
    db: Session = Depends(get_db),
):
    """
    上传文件到 Gemini Files API

    支持可恢复上传协议（Resumable Upload Protocol）：
    1. 初始请求：设置元数据，获取上传 URL
    2. 上传请求：上传实际文件内容

    **认证方式**:
    - `x-goog-api-key` 请求头，或
    - `?key=` URL 参数

    **请求头（可恢复上传）**:
    - `X-Goog-Upload-Protocol: resumable`
    - `X-Goog-Upload-Command: start` | `upload, finalize`
    - `X-Goog-Upload-Header-Content-Length`: 文件大小
    - `X-Goog-Upload-Header-Content-Type`: 文件 MIME 类型

    **请求体（初始请求）**:
    ```json
    {
        "file": {
            "display_name": "文件名"
        }
    }
    ```
    """
    upstream_key, base_url, file_key_id = await _resolve_upstream_context(request, db)

    # 读取请求体
    body = await request.body()

    # 构建上游请求
    upstream_url = _build_upstream_url(
        base_url,
        "/v1beta/files",
        dict(request.query_params),
        is_upload=True,
    )

    headers = _build_upstream_headers(dict(request.headers), upstream_key)

    logger.debug(f"Gemini Files upload proxy: POST {redact_url_for_log(upstream_url)}")

    return await _proxy_request(
        "POST", upstream_url, headers, content=body, file_key_id=file_key_id
    )


# ==============================================================================
# 文件列表端点
# ==============================================================================


@router.get("/v1beta/files")
async def list_files(
    request: Request,
    db: Session = Depends(get_db),
    pageSize: int | None = None,
    pageToken: str | None = None,
):
    """
    列出已上传的文件

    **认证方式**:
    - `x-goog-api-key` 请求头，或
    - `?key=` URL 参数

    **查询参数**:
    - `pageSize`: 每页返回的文件数量（默认 10，最大 100）
    - `pageToken`: 分页令牌

    **响应格式**:
    ```json
    {
        "files": [
            {
                "name": "files/abc-123",
                "displayName": "文件名",
                "mimeType": "image/jpeg",
                "sizeBytes": "12345",
                "createTime": "2024-01-01T00:00:00Z",
                "updateTime": "2024-01-01T00:00:00Z",
                "expirationTime": "2024-01-03T00:00:00Z",
                "sha256Hash": "...",
                "uri": "https://...",
                "state": "ACTIVE"
            }
        ],
        "nextPageToken": "..."
    }
    ```
    """
    upstream_key, base_url, file_key_id = await _resolve_upstream_context(request, db)

    # 构建查询参数
    query_params = dict(request.query_params)
    if pageSize is not None:
        query_params["pageSize"] = pageSize
    if pageToken is not None:
        query_params["pageToken"] = pageToken

    # 构建上游请求
    upstream_url = _build_upstream_url(base_url, "/v1beta/files", query_params)
    headers = _build_upstream_headers(dict(request.headers), upstream_key)

    logger.debug(f"Gemini Files list proxy: GET {redact_url_for_log(upstream_url)}")

    return await _proxy_request("GET", upstream_url, headers, file_key_id=file_key_id)


# ==============================================================================
# 获取文件元数据端点
# ==============================================================================


@router.get("/v1beta/files/{file_name:path}")
async def get_file(
    file_name: str,
    request: Request,
    db: Session = Depends(get_db),
):
    """
    获取指定文件的元数据

    **认证方式**:
    - `x-goog-api-key` 请求头，或
    - `?key=` URL 参数

    **路径参数**:
    - `file_name`: 文件名（格式：files/xxx 或 xxx）

    **响应格式**:
    ```json
    {
        "name": "files/abc-123",
        "displayName": "文件名",
        "mimeType": "image/jpeg",
        "sizeBytes": "12345",
        "createTime": "2024-01-01T00:00:00Z",
        "updateTime": "2024-01-01T00:00:00Z",
        "expirationTime": "2024-01-03T00:00:00Z",
        "sha256Hash": "...",
        "uri": "https://...",
        "state": "ACTIVE"
    }
    ```
    """
    upstream_key, base_url, file_key_id = await _resolve_upstream_context(request, db)

    # 规范化文件名（确保以 files/ 开头）
    if not file_name.startswith("files/"):
        file_name = f"files/{file_name}"

    # 构建上游请求
    upstream_url = _build_upstream_url(
        base_url,
        f"/v1beta/{file_name}",
        dict(request.query_params),
    )
    headers = _build_upstream_headers(dict(request.headers), upstream_key)

    logger.debug(f"Gemini Files get proxy: GET {redact_url_for_log(upstream_url)}")

    return await _proxy_request("GET", upstream_url, headers, file_key_id=file_key_id)


# ==============================================================================
# 删除文件端点
# ==============================================================================


@router.delete("/v1beta/files/{file_name:path}")
async def delete_file(
    file_name: str,
    request: Request,
    db: Session = Depends(get_db),
):
    """
    删除指定文件

    **认证方式**:
    - `x-goog-api-key` 请求头，或
    - `?key=` URL 参数

    **路径参数**:
    - `file_name`: 文件名（格式：files/xxx 或 xxx）

    **响应格式**:
    成功时返回空 JSON 对象：`{}`
    """
    upstream_key, base_url, _file_key_id = await _resolve_upstream_context(request, db)

    # 规范化文件名（确保以 files/ 开头）
    if not file_name.startswith("files/"):
        file_name = f"files/{file_name}"

    # 构建上游请求
    upstream_url = _build_upstream_url(
        base_url,
        f"/v1beta/{file_name}",
        dict(request.query_params),
    )
    headers = _build_upstream_headers(dict(request.headers), upstream_key)

    logger.debug(f"Gemini Files delete proxy: DELETE {redact_url_for_log(upstream_url)}")

    del _file_key_id  # 显式标记：delete 端点不需要存储映射
    response = await _proxy_request("DELETE", upstream_url, headers)
    if response.status_code < 300:
        await delete_file_key_mapping(file_name)
    else:
        logger.debug(
            f"Gemini Files delete failed, skip mapping cleanup: status={response.status_code}"
        )
    return response


