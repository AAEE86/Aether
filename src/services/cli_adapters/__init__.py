"""
CLI Provider Adapter 注册表

提供适配器的注册和查找功能。
"""

from __future__ import annotations

from typing import TYPE_CHECKING

if TYPE_CHECKING:
    from .base import CliProviderAdapter

# 适配器类映射：auth_type -> adapter class
_ADAPTER_MAP: dict[str, type["CliProviderAdapter"]] = {}

# 是否已完成发现
_discovered: bool = False


def _discover_adapters() -> None:
    """
    延迟发现并注册所有内置适配器

    使用延迟加载避免循环导入和启动时的性能开销。
    """
    global _discovered
    if _discovered:
        return
    _discovered = True

    # 导入并注册适配器
    from .claude_code_adapter import ClaudeCodeAdapter
    from .codex_adapter import CodexAdapter
    from .gemini_cli_adapter import GeminiCliAdapter
    from .antigravity_adapter import AntigravityAdapter

    _ADAPTER_MAP["claude_code"] = ClaudeCodeAdapter
    _ADAPTER_MAP["codex"] = CodexAdapter
    _ADAPTER_MAP["gemini_cli"] = GeminiCliAdapter
    _ADAPTER_MAP["antigravity"] = AntigravityAdapter


def get_cli_adapter(auth_type: str) -> "CliProviderAdapter | None":
    """
    根据 auth_type 获取 CLI 提供商适配器实例

    对于非 CLI 提供商（如标准 API Key），返回 None。

    Args:
        auth_type: 认证类型（claude_code, codex, gemini_cli, antigravity, 或其他）

    Returns:
        适配器实例，或 None（非 CLI 提供商）
    """
    _discover_adapters()
    adapter_cls = _ADAPTER_MAP.get(auth_type)
    return adapter_cls() if adapter_cls else None


def register_adapter(auth_type: str, adapter_cls: type["CliProviderAdapter"]) -> None:
    """
    注册自定义适配器

    用于扩展或测试场景。

    Args:
        auth_type: 认证类型标识
        adapter_cls: 适配器类
    """
    _ADAPTER_MAP[auth_type] = adapter_cls


def get_supported_auth_types() -> list[str]:
    """
    获取所有支持的 CLI 认证类型

    Returns:
        支持的 auth_type 列表
    """
    _discover_adapters()
    return list(_ADAPTER_MAP.keys())


# 导出公共接口
__all__ = [
    "get_cli_adapter",
    "register_adapter",
    "get_supported_auth_types",
    "CliProviderAdapter",
    "AdapterContext",
    "TransformedRequest",
]

# 从 base 模块导出类型（方便使用者导入）
from .base import AdapterContext, CliProviderAdapter, TransformedRequest
