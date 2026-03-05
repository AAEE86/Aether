"""
按 api_format 从上游获取模型列表的实现。

该模块用于 services 层（如自动抓取模型、管理后台查询），避免依赖 api/handlers 的 Adapter 注册表，
从而消除 services→api 的反向依赖。
"""

from __future__ import annotations

from typing import Any

import httpx

from src.config.settings import config
from src.core.api_format.headers import (
    BROWSER_FINGERPRINT_HEADERS,
    build_adapter_headers_for_endpoint,
)
from src.core.logger import logger
from src.services.provider.transport import redact_url_for_log


def _build_v1_models_url(base_url: str) -> str:
    """构建 /v1/models URL（OpenAI 和 Claude 共用）。"""
    base_url = str(base_url or "").rstrip("/")
    if base_url.endswith("/v1"):
        return f"{base_url}/models"
    return f"{base_url}/v1/models"


async def _fetch_openai_models(
    client: httpx.AsyncClient,
    base_url: str,
    api_key: str,
    *,
    api_format: str,
    extra_headers: dict[str, str] | None,
) -> tuple[list[dict], str | None]:
    headers = build_adapter_headers_for_endpoint(api_format, api_key, extra_headers)
    models_url = _build_v1_models_url(base_url)

    try:
        response = await client.get(models_url, headers=headers)
        logger.debug("OpenAI models request to {}: status={}", models_url, response.status_code)
        if response.status_code == 200:
            data = response.json()
            models: list[dict] = []
            if isinstance(data, dict) and isinstance(data.get("data"), list):
                models = [m for m in data["data"] if isinstance(m, dict)]
            elif isinstance(data, list):
                models = [m for m in data if isinstance(m, dict)]

            for m in models:
                m.setdefault("api_format", api_format)
            return models, None

        error_body = response.text[:500] if response.text else "(empty)"
        error_msg = f"HTTP {response.status_code}: {error_body}"
        logger.warning("OpenAI models request to {} failed: {}", models_url, error_msg)
        return [], error_msg
    except Exception as e:
        error_msg = f"Request error: {str(e)}"
        logger.warning("Failed to fetch models from {}: {}", models_url, e)
        return [], error_msg


async def _fetch_claude_models_paginated(
    client: httpx.AsyncClient,
    base_url: str,
    headers: dict[str, str],
    *,
    api_format: str,
) -> tuple[list[dict], str | None]:
    models_url = _build_v1_models_url(base_url)

    try:
        all_models: list[dict] = []
        seen_ids: set[str] = set()

        after_id: str | None = None
        limit = 100
        max_pages = 20

        for _ in range(max_pages):
            params: dict[str, Any] = {"limit": limit}
            if after_id:
                params["after_id"] = after_id

            response = await client.get(models_url, headers=headers, params=params)
            logger.debug(
                "Claude models request to {}: status={}, after_id={}",
                models_url,
                response.status_code,
                after_id,
            )
            if response.status_code != 200:
                error_body = response.text[:500] if response.text else "(empty)"
                error_msg = f"HTTP {response.status_code}: {error_body}"
                logger.warning("Claude models request to {} failed: {}", models_url, error_msg)
                return [], error_msg

            data = response.json()
            page_models: list[dict] = []
            if isinstance(data, dict) and isinstance(data.get("data"), list):
                page_models = [m for m in data["data"] if isinstance(m, dict)]
            elif isinstance(data, list):
                page_models = [m for m in data if isinstance(m, dict)]

            for m in page_models:
                mid = m.get("id")
                if isinstance(mid, str) and mid and mid in seen_ids:
                    continue
                if isinstance(mid, str) and mid:
                    seen_ids.add(mid)
                m.setdefault("api_format", api_format)
                all_models.append(m)

            if not isinstance(data, dict):
                break

            has_more = bool(data.get("has_more"))
            last_id = data.get("last_id")
            if not has_more:
                break
            if not isinstance(last_id, str) or not last_id:
                break
            if after_id == last_id:
                break
            after_id = last_id

        return all_models, None
    except Exception as e:
        error_msg = f"Request error: {str(e)}"
        logger.warning("Failed to fetch Claude models from {}: {}", models_url, e)
        return [], error_msg


async def _fetch_claude_models(
    client: httpx.AsyncClient,
    base_url: str,
    api_key: str,
    *,
    api_format: str,
    extra_headers: dict[str, str] | None,
    force_bearer_fallback: bool,
) -> tuple[list[dict], str | None]:
    headers = build_adapter_headers_for_endpoint(api_format, api_key, extra_headers)
    if force_bearer_fallback and "authorization" not in {k.lower() for k in headers}:
        headers["Authorization"] = f"Bearer {api_key}"
    return await _fetch_claude_models_paginated(
        client,
        base_url,
        headers,
        api_format=api_format,
    )


async def _fetch_gemini_models(
    client: httpx.AsyncClient,
    base_url: str,
    api_key: str,
    *,
    api_format: str,
    extra_headers: dict[str, str] | None,
) -> tuple[list[dict], str | None]:
    base_url_clean = str(base_url or "").rstrip("/")
    if base_url_clean.endswith("/v1beta"):
        models_url = f"{base_url_clean}/models?key={api_key}"
    else:
        models_url = f"{base_url_clean}/v1beta/models?key={api_key}"

    headers: dict[str, str] = {**BROWSER_FINGERPRINT_HEADERS}
    if extra_headers:
        headers.update(extra_headers)

    try:
        response = await client.get(models_url, headers=headers)
        logger.debug(
            "Gemini models request to {}: status={}",
            redact_url_for_log(models_url),
            response.status_code,
        )
        if response.status_code == 200:
            data = response.json()
            if isinstance(data, dict) and isinstance(data.get("models"), list):
                out: list[dict] = []
                for m in data["models"]:
                    if not isinstance(m, dict):
                        continue
                    out.append(
                        {
                            "id": str(m.get("name", "")).replace("models/", ""),
                            "owned_by": "google",
                            "display_name": m.get("displayName", ""),
                            "api_format": api_format,
                        }
                    )
                return out, None
            return [], None

        error_body = response.text[:500] if response.text else "(empty)"
        error_msg = f"HTTP {response.status_code}: {error_body}"
        logger.warning(
            "Gemini models request to {} failed: {}",
            redact_url_for_log(models_url),
            error_msg,
        )
        return [], error_msg
    except Exception as e:
        sanitized_error = redact_url_for_log(str(e))
        error_msg = f"Request error: {sanitized_error}"
        logger.warning(
            "Failed to fetch Gemini models from {}: {}",
            redact_url_for_log(models_url),
            sanitized_error,
        )
        return [], error_msg


async def fetch_models_for_api_format(
    client: httpx.AsyncClient,
    *,
    api_format: str,
    base_url: str,
    api_key: str,
    extra_headers: dict[str, str] | None = None,
) -> tuple[list[dict], str | None]:
    """按 api_format 获取模型列表。"""
    fmt = str(api_format or "").strip().lower()
    if not fmt:
        return [], "Unknown API format: (empty)"

    if fmt == "openai:chat":
        return await _fetch_openai_models(
            client, base_url, api_key, api_format=fmt, extra_headers=extra_headers
        )

    if fmt in {"openai:cli", "openai:compact"}:
        cli_headers = {"User-Agent": config.internal_user_agent_openai_cli}
        if extra_headers:
            cli_headers.update(extra_headers)
        return await _fetch_openai_models(
            client, base_url, api_key, api_format=fmt, extra_headers=cli_headers
        )

    if fmt == "claude:chat":
        return await _fetch_claude_models(
            client,
            base_url,
            api_key,
            api_format=fmt,
            extra_headers=extra_headers,
            force_bearer_fallback=True,
        )

    if fmt == "claude:cli":
        cli_headers = {"User-Agent": config.internal_user_agent_claude_cli}
        if extra_headers:
            cli_headers.update(extra_headers)
        return await _fetch_claude_models(
            client,
            base_url,
            api_key,
            api_format=fmt,
            extra_headers=cli_headers,
            force_bearer_fallback=False,
        )

    if fmt == "gemini:chat":
        return await _fetch_gemini_models(
            client,
            base_url,
            api_key,
            api_format=fmt,
            extra_headers=extra_headers,
        )

    if fmt == "gemini:cli":
        cli_headers = {"User-Agent": config.internal_user_agent_gemini_cli}
        if extra_headers:
            cli_headers.update(extra_headers)
        return await _fetch_gemini_models(
            client,
            base_url,
            api_key,
            api_format=fmt,
            extra_headers=cli_headers,
        )

    return [], f"Unknown API format: {api_format}"


__all__ = [
    "fetch_models_for_api_format",
]
