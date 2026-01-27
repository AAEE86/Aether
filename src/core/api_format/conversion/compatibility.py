"""
格式兼容性检查

用于候选筛选时判断端点是否可以处理客户端请式。
"""

from __future__ import annotations

import logging
from typing import TYPE_CHECKING, Optional, Tuple

if TYPE_CHECKING:
    from src.core.api_format.conversion.registry import FormatConversionRegistry

from src.core.api_format.utils import get_base_format

logger = logging.getLogger(__name__)


def is_format_compatible(
    client_format: str,
    endpoint_api_format: str,
    endpoint_format_acceptance_config: Optional[dict],
    is_stream: bool,
    global_conversion_enabled: bool,
    registry: Optional["FormatConversionRegistry"] = None,
) -> Tuple[bool, bool, Optional[str]]:
    """
    检查端点是否兼容客户端格式

    Args:
        client_format: 客户端请求格式
        endpoint_api_format: 端点的 API 格式
        endpoint_format_acceptance_config: 端点的格式接受配置
        is_stream: 是否是流式请求
        global_conversion_enabled: 全局格式转换开关
        registry: 转换器注册表（可选，默认使用全局单例）

    Returns:
        (is_compatible, needs_conversion, skip_reason)
        - is_compatible: 是否兼容
        - needs_conversion: 是否需要转换
        - skip_reason: 不兼容时的原因
    """
    # 延迟导入避免循环依赖
    if registry is None:
        from src.core.api_format.conversion.registry import (
            format_conversion_registry,
            register_default_normalizers,
        )

        register_default_normalizers()
        registry = format_conversion_registry

    provider_format = endpoint_api_format.upper()
    client_format_upper = client_format.upper()

    # 1. 格式完全匹配 -> 兼容，无需转换
    if provider_format == client_format_upper:
        return True, False, None

    # 2. 同族格式检查
    provider_base = get_base_format(provider_format)
    client_base = get_base_format(client_format_upper)

    if provider_base == client_base:
        # 同族格式：检查是否需要转换
        # - OPENAI 和 OPENAI_CLI 的请求/响应格式不同（Chat Completions vs Responses API），需要转换
        # - CLAUDE 和 CLAUDE_CLI、GEMINI 和 GEMINI_CLI 格式相同，只是认证不同，可透传
        if provider_base == "OPENAI":
            # OPENAI/OPENAI_CLI 同族转换：兼容但需要转换
            # 检查转换器能力（同族转换不需要全局开关和端点配置）
            if registry.can_convert_full(
                client_format_upper, provider_format, require_stream=is_stream
            ):
                return True, True, None  # 兼容，需要转换
            else:
                return (
                    False,
                    False,
                    f"不存在 {client_format} <-> {provider_format} 的完整转换器",
                )
        else:
            # CLAUDE/CLAUDE_CLI、GEMINI/GEMINI_CLI 等：格式相同，可透传
            return True, False, None

    # 3. 检查全局开关
    if not global_conversion_enabled:
        return False, False, "全局格式转换未启用"

    # 4. 检查端点配置
    if endpoint_format_acceptance_config is None:
        return False, False, "端点未配置格式转换"

    config = endpoint_format_acceptance_config
    if not isinstance(config, dict):
        return False, False, "端点格式配置无效"
    if not config.get("enabled", False):
        return False, False, "端点格式转换未启用"

    # 检查 reject_formats（优先）
    reject_formats = config.get("reject_formats", [])
    if client_format_upper in [f.upper() for f in reject_formats]:
        return False, False, f"端点拒绝 {client_format} 格式"

    # 检查 accept_formats
    accept_formats = config.get("accept_formats", [])
    if accept_formats and client_format_upper not in [f.upper() for f in accept_formats]:
        return False, False, f"端点不接受 {client_format} 格式"

    # 检查流式转换
    if is_stream and not config.get("stream_conversion", True):
        return False, False, "端点不支持流式格式转换"

    # 5. 检查转换器能力
    if not registry.can_convert_full(
        client_format_upper,
        provider_format,
        require_stream=is_stream,
    ):
        return False, False, f"不存在 {client_format} <-> {provider_format} 的完整转换器"

    return True, True, None


__all__ = [
    "is_format_compatible",
]
