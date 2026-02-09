"""
上游模型获取公共模块

提供从上游 API 获取模型列表的公共函数，供以下场景使用：
- 定时任务自动获取（fetch_scheduler.py）
- 管理后台手动查询（provider_query.py）
"""

import asyncio
from dataclasses import dataclass
from typing import Any, Awaitable, Callable

import httpx

from src.core.api_format import get_extra_headers_from_endpoint
from src.core.logger import logger
from src.models.database import ProviderEndpoint
from src.utils.ssl_utils import get_ssl_context

# 并发请求限制
MAX_CONCURRENT_REQUESTS = 5

# 只对这些基础 endpoint signature 获取模型列表，CLI 使用相同的上游 API
MODEL_FETCH_FORMATS = ["openai:chat", "claude:chat", "gemini:chat"]

# Return tuple signature:
# (models, errors, has_success, upstream_metadata)
_ModelsFetcher = Callable[
    ["UpstreamModelsFetchContext", float],
    Awaitable[tuple[list[dict], list[str], bool, dict[str, Any] | None]],
]


@dataclass(frozen=True)
class UpstreamModelsFetchContext:
    """上游模型获取上下文（Key 级别）。"""

    provider_type: str
    api_key_value: str
    format_to_endpoint: dict[str, Any]
    proxy_config: dict[str, Any] | None = None
    auth_config: dict[str, Any] | None = None


class UpstreamModelsFetcherRegistry:
    """按 provider_type 注册上游模型获取策略，避免到处写特判。"""

    _fetchers: dict[str, _ModelsFetcher] = {}

    @classmethod
    def register(cls, *, provider_types: list[str], fetcher: _ModelsFetcher) -> None:
        for pt in provider_types:
            if not pt:
                continue
            cls._fetchers[pt.lower()] = fetcher

    @classmethod
    def get(cls, provider_type: str) -> _ModelsFetcher | None:
        if not provider_type:
            return None
        return cls._fetchers.get(provider_type.lower())


async def _fetch_models_default(
    ctx: UpstreamModelsFetchContext,
    timeout_seconds: float,
) -> tuple[list[dict], list[str], bool, dict[str, Any] | None]:
    endpoint_configs = build_all_format_configs(ctx.api_key_value, ctx.format_to_endpoint)
    models, errors, has_success = await fetch_models_from_endpoints(
        endpoint_configs, timeout=timeout_seconds
    )
    return models, errors, has_success, None


async def fetch_models_for_key(
    ctx: UpstreamModelsFetchContext,
    *,
    timeout_seconds: float = 30.0,
) -> tuple[list[dict], list[str], bool, dict[str, Any] | None]:
    """统一入口：按 provider_type 选择策略获取模型列表（可附带 upstream_metadata）。"""
    # Ensure provider plugins (including custom model fetchers) are registered.
    from src.services.provider.envelope import ensure_providers_bootstrapped

    ensure_providers_bootstrapped()

    fetcher = UpstreamModelsFetcherRegistry.get(ctx.provider_type) or _fetch_models_default
    return await fetcher(ctx, timeout_seconds)


def merge_upstream_metadata(
    current: dict[str, Any] | None,
    incoming: dict[str, Any],
) -> dict[str, Any]:
    """合并上游元数据，对 quota_by_model 做模型级深度合并。

    上游 API 在配额耗尽后可能不再返回该模型的 quotaInfo，因此需要：
    1. 保留旧数据中已有的 reset_time（当新数据缺少时）
    2. 保留旧数据中存在但新数据中缺失的模型条目（标记为 100% 已用）
    """
    merged: dict[str, Any] = dict(current) if isinstance(current, dict) else {}
    for ns_key, ns_val in incoming.items():
        old_ns = merged.get(ns_key)
        if (
            isinstance(ns_val, dict)
            and isinstance(old_ns, dict)
            and "quota_by_model" in ns_val
            and "quota_by_model" in old_ns
        ):
            old_qbm = old_ns["quota_by_model"]
            new_qbm = ns_val["quota_by_model"]
            if isinstance(old_qbm, dict) and isinstance(new_qbm, dict):
                # 保留新数据中已有模型的旧 reset_time
                for model_id, new_info in new_qbm.items():
                    if not isinstance(new_info, dict):
                        continue
                    old_info = old_qbm.get(model_id)
                    if (
                        isinstance(old_info, dict)
                        and "reset_time" in old_info
                        and "reset_time" not in new_info
                    ):
                        new_info["reset_time"] = old_info["reset_time"]
                # 保留新数据中缺失但旧数据中存在的模型（配额耗尽后上游可能不返回）
                for model_id, old_info in old_qbm.items():
                    if model_id not in new_qbm and isinstance(old_info, dict):
                        exhausted = dict(old_info)
                        exhausted["remaining_fraction"] = 0.0
                        exhausted["used_percent"] = 100.0
                        new_qbm[model_id] = exhausted
        merged[ns_key] = ns_val
    return merged


# Provider-specific fetchers are registered by plugin.register_all()
# (called from envelope.py bootstrap)


def get_adapter_for_format(api_format: str) -> type | None:
    """根据 API 格式获取对应的 Adapter 类"""
    from src.api.handlers.base.chat_adapter_base import get_adapter_class
    from src.api.handlers.base.cli_adapter_base import get_cli_adapter_class

    return get_adapter_class(api_format) or get_cli_adapter_class(api_format)


def build_all_format_configs(
    api_key_value: str,
    format_to_endpoint: dict[str, ProviderEndpoint],
) -> list[dict]:
    """
    构建所有 API 格式的端点配置

    只对实际配置了端点的格式构建请求配置，不同端点的 base_url 可能不同，
    不应使用某个端点的 base_url 去尝试其他格式。

    Args:
        api_key_value: 解密后的 API Key
        format_to_endpoint: API 格式到端点的映射

    Returns:
        端点配置列表，每个配置包含 api_key, base_url, api_format, extra_headers
    """
    if not format_to_endpoint:
        return []

    # 只对基础 API 格式获取模型，CLI 格式使用相同的上游 API
    return [
        {
            "api_key": api_key_value,
            "base_url": ep.base_url,
            "api_format": fmt,
            "extra_headers": get_extra_headers_from_endpoint(ep),
        }
        for fmt in MODEL_FETCH_FORMATS
        if (ep := format_to_endpoint.get(fmt)) is not None
    ]


async def fetch_models_from_endpoints(
    endpoint_configs: list[dict],
    timeout: float = 30.0,
) -> tuple[list[dict], list[str], bool]:
    """
    从多个端点并发获取模型

    Args:
        endpoint_configs: 端点配置列表，每个配置包含 api_key, base_url, api_format, extra_headers
        timeout: 请求超时时间（秒）

    Returns:
        (模型列表, 错误列表, 是否有成功)
    """
    all_models: list[dict] = []
    errors: list[str] = []
    has_success = False
    semaphore = asyncio.Semaphore(MAX_CONCURRENT_REQUESTS)

    async def fetch_one(client: httpx.AsyncClient, config: dict) -> tuple[list, str | None, bool]:
        base_url = config["base_url"]
        if not base_url:
            return [], None, False
        base_url = base_url.rstrip("/")
        api_format = config["api_format"]
        api_key_value = config["api_key"]
        extra_headers = config.get("extra_headers")

        try:
            adapter_class = get_adapter_for_format(api_format)
            if not adapter_class:
                return [], f"Unknown API format: {api_format}", False

            async with semaphore:
                models, error = await adapter_class.fetch_models(  # type: ignore[attr-defined]
                    client, base_url, api_key_value, extra_headers
                )

            for m in models:
                if "api_format" not in m:
                    m["api_format"] = api_format

            # 即使返回空列表，只要没有错误也算成功
            success = error is None
            return models, error, success
        except httpx.TimeoutException:
            logger.warning("获取 {} 模型超时", api_format)
            return [], f"{api_format}: timeout", False
        except Exception:
            logger.exception("获取 {} 模型出错", api_format)
            return [], f"{api_format}: error", False

    async with httpx.AsyncClient(timeout=timeout, verify=get_ssl_context()) as client:
        results = await asyncio.gather(*[fetch_one(client, c) for c in endpoint_configs])
        for models, error, success in results:
            all_models.extend(models)
            if error:
                errors.append(error)
            if success:
                has_success = True

    return all_models, errors, has_success
