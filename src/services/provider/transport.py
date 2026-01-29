"""
统一的 Provider 请求构建工具。

负责:
- 根据 API 格式或端点配置生成请求 URL
- URL 脱敏（用于日志记录）
"""

import re
from typing import TYPE_CHECKING, Any
from urllib.parse import urlencode

from src.core.api_format import APIFormat, get_default_path, resolve_api_format
from src.core.logger import logger

if TYPE_CHECKING:
    from src.models.database import ProviderEndpoint


# URL 中需要脱敏的查询参数（正则模式）
_SENSITIVE_QUERY_PARAMS_PATTERN = re.compile(
    r"([?&])(key|api_key|apikey|token|secret|password|credential)=([^&]*)",
    re.IGNORECASE,
)


def redact_url_for_log(url: str) -> str:
    """
    对 URL 中的敏感查询参数进行脱敏，用于日志记录

    将 ?key=xxx 替换为 ?key=***

    Args:
        url: 原始 URL

    Returns:
        脱敏后的 URL
    """
    return _SENSITIVE_QUERY_PARAMS_PATTERN.sub(r"\1\2=***", url)


def _normalize_base_url(base_url: str, path: str) -> str:
    """
    规范化 base_url，去除末尾的斜杠和可能与 path 重复的版本前缀。

    只有当 path 以版本前缀开头时，才从 base_url 中移除该前缀，
    避免拼接出 /v1/v1/messages 这样的重复路径。

    兼容用户填写的各种格式：
    - https://api.example.com
    - https://api.example.com/
    - https://api.example.com/v1
    - https://api.example.com/v1/
    """
    base = base_url.rstrip("/")
    # 只在 path 以版本前缀开头时才去除 base_url 中的该前缀
    # 例如：base="/v1", path="/v1/messages" -> 去除 /v1
    # 例如：base="/v1", path="/chat/completions" -> 不去除（用户可能期望保留）
    for suffix in ("/v1beta", "/v1", "/v2", "/v3"):
        if base.endswith(suffix) and path.startswith(suffix):
            base = base[: -len(suffix)]
            break
    return base


def build_provider_url(
    endpoint: ProviderEndpoint,
    *,
    query_params: dict[str, Any] | None = None,
    path_params: dict[str, Any] | None = None,
    is_stream: bool = False,
) -> str:
    """
    根据 endpoint 配置生成请求 URL

    优先级：
    1. endpoint.custom_path - 自定义路径（支持模板变量如 {model}）
    2. API 格式默认路径 - 根据 api_format 自动选择

    Args:
        endpoint: 端点配置
        query_params: 查询参数
        path_params: 路径模板参数 (如 {model})
        is_stream: 是否为流式请求，用于 Gemini API 选择正确的操作方法
    """
    # 准备路径参数，添加 Gemini API 所需的 action 参数
    effective_path_params = dict(path_params) if path_params else {}

    # 为 Gemini API 格式自动添加 action 参数
    resolved_format = resolve_api_format(endpoint.api_format)
    if resolved_format in (APIFormat.GEMINI, APIFormat.GEMINI_CLI):
        if "action" not in effective_path_params:
            effective_path_params["action"] = (
                "streamGenerateContent" if is_stream else "generateContent"
            )

    # 优先使用 custom_path 字段
    if endpoint.custom_path:
        path = endpoint.custom_path
        if effective_path_params:
            try:
                path = path.format(**effective_path_params)
            except KeyError:
                # 如果模板变量不匹配，保持原路径
                pass
    else:
        # 使用 API 格式的默认路径
        path = _resolve_default_path(endpoint.api_format)
        if effective_path_params:
            try:
                path = path.format(**effective_path_params)
            except KeyError:
                # 如果模板变量不匹配，保持原路径
                pass

    if not path.startswith("/"):
        path = f"/{path}"

    # 先确定 path，再根据 path 规范化 base_url
    # base_url 在数据库中是 NOT NULL，类型标注为 Optional 是 SQLAlchemy 限制
    base = _normalize_base_url(endpoint.base_url, path)  # type: ignore[arg-type]
    url = f"{base}{path}"

    # 合并查询参数
    effective_query_params = dict(query_params) if query_params else {}

    # Gemini 格式下清除可能存在的 key 参数（避免客户端传入的认证信息泄露到上游）
    # 上游认证始终使用 header 方式，不使用 URL 参数
    if resolved_format in (APIFormat.GEMINI, APIFormat.GEMINI_CLI):
        effective_query_params.pop("key", None)
        # Gemini streamGenerateContent 官方支持 `?alt=sse` 返回 SSE（data: {...}）。
        # 网关侧统一使用 SSE 输出，优先向上游请求 SSE 以减少解析分支；同时保留 JSON-array 兜底解析。
        if is_stream:
            effective_query_params.setdefault("alt", "sse")

    # 添加查询参数
    if effective_query_params:
        query_string = urlencode(effective_query_params, doseq=True)
        if query_string:
            url = f"{url}?{query_string}"

    return url


def _resolve_default_path(api_format: str | None) -> str:
    """
    根据 API 格式返回默认路径
    """
    resolved = resolve_api_format(api_format)
    if resolved:
        return get_default_path(resolved)

    logger.warning(f"Unknown api_format '{api_format}' for endpoint, fallback to '/'")
    return "/"
