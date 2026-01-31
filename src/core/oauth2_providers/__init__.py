"""
OAuth2 Provider 认证模块

提供对多种 OAuth2 反代渠道的支持：
- Codex (OpenAI CLI)
- ClaudeCode (Anthropic CLI)
- GeminiCli (Google Cloud)
- Antigravity (Google Sandbox)

使用方式：
    from src.core.oauth2_providers import OAuth2ProviderRegistry

    # 获取 Provider
    provider = OAuth2ProviderRegistry.get_provider("codex")

    # 获取 Token
    from src.core.oauth2_providers.token_store import OAuth2TokenStore
    access_token = await OAuth2TokenStore.get_access_token(key)
"""

from __future__ import annotations

from typing import TYPE_CHECKING

from src.core.logger import logger

if TYPE_CHECKING:
    from src.core.oauth2_providers.base import OAuth2AuthProvider

# OAuth2 认证类型常量
OAUTH2_AUTH_TYPES = frozenset({"codex", "claude_code", "gemini_cli", "antigravity"})


class OAuth2ProviderRegistry:
    """
    OAuth2 Provider 注册表

    采用懒加载模式，在首次访问时自动发现并注册所有 Provider。
    """

    _providers: dict[str, "OAuth2AuthProvider"] = {}
    _discovered: bool = False

    @classmethod
    def discover_providers(cls) -> None:
        """
        自动发现并注册所有内置 Provider

        此方法是幂等的，多次调用只会执行一次发现过程。
        """
        if cls._discovered:
            return
        cls._discovered = True

        # 导入并注册内置 Provider
        from src.core.oauth2_providers.providers.codex import CodexProvider
        from src.core.oauth2_providers.providers.claude_code import ClaudeCodeProvider
        from src.core.oauth2_providers.providers.gemini_cli import GeminiCliProvider
        from src.core.oauth2_providers.providers.antigravity import AntigravityProvider

        for provider_cls in [CodexProvider, ClaudeCodeProvider, GeminiCliProvider, AntigravityProvider]:
            provider = provider_cls()
            cls._providers[provider.config.provider_id] = provider
            logger.debug(f"[OAuth2Registry] Registered provider: {provider.config.provider_id}")

    @classmethod
    def get_provider(cls, provider_id: str) -> "OAuth2AuthProvider | None":
        """
        获取指定 ID 的 Provider

        Args:
            provider_id: Provider ID (codex, claude_code, gemini_cli, antigravity)

        Returns:
            Provider 实例，如果不存在则返回 None
        """
        cls.discover_providers()
        return cls._providers.get(provider_id)

    @classmethod
    def get_all_providers(cls) -> list["OAuth2AuthProvider"]:
        """获取所有已注册的 Provider"""
        cls.discover_providers()
        return list(cls._providers.values())

    @classmethod
    def is_oauth2_auth_type(cls, auth_type: str) -> bool:
        """
        检查指定的 auth_type 是否为 OAuth2 Provider

        Args:
            auth_type: 认证类型

        Returns:
            如果是 OAuth2 类型则返回 True
        """
        return auth_type in OAUTH2_AUTH_TYPES

    @classmethod
    def get_provider_info(cls) -> list[dict]:
        """
        获取所有 Provider 的配置信息（用于 API 响应）

        Returns:
            Provider 配置信息列表
        """
        cls.discover_providers()
        return [
            {
                "provider_id": p.config.provider_id,
                "display_name": p.config.display_name,
                "api_format": p.config.api_format,
                "pkce_required": p.config.pkce_required,
            }
            for p in cls._providers.values()
        ]


# 导出常用类
__all__ = [
    "OAuth2ProviderRegistry",
    "OAUTH2_AUTH_TYPES",
]
