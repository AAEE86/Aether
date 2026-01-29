"""
上游模型获取公共模块

提供从上游 API 获取模型列表的公共函数，供以下场景使用：
- 定时任务自动获取（fetch_scheduler.py）
- 管理后台手动查询（provider_query.py）
"""

import asyncio

import httpx

from src.core.api_format import APIFormat, get_extra_headers_from_endpoint
from src.core.logger import logger
from src.models.database import ProviderEndpoint
from src.utils.ssl_utils import get_ssl_context

# 并发请求限制
MAX_CONCURRENT_REQUESTS = 5

# 只对这些基础格式获取模型列表，CLI 格式使用相同的上游 API
MODEL_FETCH_FORMATS = [APIFormat.OPENAI, APIFormat.CLAUDE, APIFormat.GEMINI]


def _get_adapter_for_format(api_format: str) -> type | None:
    """根据 API 格式获取对应的 Adapter 类"""
    from src.api.handlers.base.chat_adapter_base import get_adapter_class
    from src.api.handlers.base.cli_adapter_base import get_cli_adapter_class

    adapter_class = get_adapter_class(api_format)
    if adapter_class:
        return adapter_class
    cli_adapter_class = get_cli_adapter_class(api_format)
    if cli_adapter_class:
        return cli_adapter_class
    return None


def build_all_format_configs(
    api_key_value: str,
    format_to_endpoint: dict[str, ProviderEndpoint],
) -> list[dict]:
    """
    构建所有 API 格式的端点配置

    从所有 APIFormat 枚举值构建配置，如果该格式有专门的端点配置则使用，
    否则使用基础端点的 base_url 尝试。

    Args:
        api_key_value: 解密后的 API Key
        format_to_endpoint: API 格式到端点的映射

    Returns:
        端点配置列表，每个配置包含 api_key, base_url, api_format, extra_headers
    """
    if not format_to_endpoint:
        return []

    # 获取任意一个端点的 base_url 作为基础（用于尝试所有格式）
    # 优先使用 OPENAI 格式的端点，因为它最通用
    base_endpoint = (
        format_to_endpoint.get("OPENAI")
        or format_to_endpoint.get("CLAUDE")
        or format_to_endpoint.get("GEMINI")
        or next(iter(format_to_endpoint.values()))
    )
    base_url = base_endpoint.base_url
    extra_headers = get_extra_headers_from_endpoint(base_endpoint)

    # 只对基础 API 格式获取模型，CLI 格式使用相同的上游 API
    endpoint_configs: list[dict] = []
    for fmt in MODEL_FETCH_FORMATS:
        fmt_value = fmt.value
        # 如果该格式有专门的端点配置，使用其 base_url 和 headers
        if fmt_value in format_to_endpoint:
            ep = format_to_endpoint[fmt_value]
            endpoint_configs.append(
                {
                    "api_key": api_key_value,
                    "base_url": ep.base_url,
                    "api_format": fmt_value,
                    "extra_headers": get_extra_headers_from_endpoint(ep),
                }
            )
        else:
            # 没有专门配置，使用基础端点的 base_url 尝试
            endpoint_configs.append(
                {
                    "api_key": api_key_value,
                    "base_url": base_url,
                    "api_format": fmt_value,
                    "extra_headers": extra_headers,
                }
            )

    return endpoint_configs


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

    async def fetch_one(
        client: httpx.AsyncClient, config: dict
    ) -> tuple[list, str | None, bool]:
        base_url = config["base_url"]
        if not base_url:
            return [], None, False
        base_url = base_url.rstrip("/")
        api_format = config["api_format"]
        api_key_value = config["api_key"]
        extra_headers = config.get("extra_headers")

        try:
            adapter_class = _get_adapter_for_format(api_format)
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
            logger.warning(f"获取 {api_format} 模型超时")
            return [], f"{api_format}: timeout", False
        except Exception:
            logger.exception(f"获取 {api_format} 模型出错")
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
