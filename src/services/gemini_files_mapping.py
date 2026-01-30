"""
Gemini Files API - 文件与 Key 绑定缓存

用于在上传文件后记录 file_id -> provider_key_id，
并在后续 generateContent 请求中优先使用同一 Key。
"""

from __future__ import annotations

from typing import Any, Dict, Optional, Set

from src.core.cache_service import CacheService

FILE_MAPPING_TTL_SECONDS = 60 * 60 * 48  # 48小时
FILE_MAPPING_CACHE_PREFIX = "gemini_files:key"


def _normalize_file_name(file_name: str) -> str:
    name = (file_name or "").strip()
    if not name:
        return ""
    return name if name.startswith("files/") else f"files/{name}"


def build_file_mapping_key(file_name: str) -> str:
    normalized = _normalize_file_name(file_name)
    return f"{FILE_MAPPING_CACHE_PREFIX}:{normalized}" if normalized else ""


async def store_file_key_mapping(file_name: str, key_id: str) -> None:
    cache_key = build_file_mapping_key(file_name)
    if not cache_key or not key_id:
        return
    await CacheService.set(cache_key, str(key_id), ttl_seconds=FILE_MAPPING_TTL_SECONDS)


async def get_file_key_mapping(file_name: str) -> str | None:
    cache_key = build_file_mapping_key(file_name)
    if not cache_key:
        return None
    value = await CacheService.get(cache_key)
    if value:
        return str(value)
    return None


async def delete_file_key_mapping(file_name: str) -> None:
    cache_key = build_file_mapping_key(file_name)
    if cache_key:
        await CacheService.delete(cache_key)


def _extract_file_name_from_uri(file_uri: str) -> str | None:
    """
    从 fileUri 提取 files/xxx 名称。

    支持两种格式：
    - 完整 URL: https://generativelanguage.googleapis.com/v1beta/files/abc123
    - 短格式: files/abc123
    """
    if not file_uri:
        return None
    # 完整 URL 格式
    if "/files/" in file_uri:
        idx = file_uri.rfind("/files/")
        return file_uri[idx + 1:]  # 提取 files/xxx 部分
    # 短格式
    if file_uri.startswith("files/"):
        return file_uri
    return None


def extract_file_names_from_request(payload: dict[str, Any] | None) -> set[str]:
    """
    从 Gemini 请求体中提取 fileUri 使用到的 files/xxx 名称集合。
    """
    results: set[str] = set()

    def walk(node: Any) -> None:
        if isinstance(node, dict):
            file_data = node.get("fileData") or node.get("file_data")
            if isinstance(file_data, dict):
                file_uri = file_data.get("fileUri") or file_data.get("file_uri")
                if isinstance(file_uri, str):
                    file_name = _extract_file_name_from_uri(file_uri)
                    if file_name:
                        results.add(file_name)
            for value in node.values():
                walk(value)
        elif isinstance(node, list):
            for item in node:
                walk(item)

    if payload:
        walk(payload)

    return results
