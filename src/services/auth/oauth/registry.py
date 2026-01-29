from dataclasses import dataclass

from src.core.logger import logger
from src.services.auth.oauth.base import OAuthProviderBase


@dataclass(frozen=True)
class SupportedOAuthType:
    provider_type: str
    display_name: str
    # 默认端点（用于前端 placeholder 展示）
    default_authorization_url: str
    default_token_url: str
    default_userinfo_url: str
    default_scopes: tuple[str, ...]


class OAuthProviderRegistry:
    """Provider 注册表（支持延迟 discover）。"""

    def __init__(self) -> None:
        self._providers: dict[str, OAuthProviderBase] = {}
        self._discovered: bool = False

    def discover_providers(self) -> None:
        """发现并注册 providers（幂等）。"""
        if self._discovered:
            return
        self._discovered = True

        # 1) 内置 providers（v1：至少保证 linuxdo 可用）
        try:
            from src.services.auth.oauth.providers.linuxdo import LinuxDoOAuthProvider

            self.register(LinuxDoOAuthProvider())
        except Exception as exc:
            logger.warning("OAuth 内置 provider 加载失败: {}", exc)

        # 2) entry_points 插件（可选）
        try:
            from importlib.metadata import entry_points

            eps = entry_points()
            # Python 3.10+ 支持 select；旧接口返回 dict
            if hasattr(eps, "select"):
                candidates = list(eps.select(group="aether.oauth_providers"))  # type: ignore[attr-defined]
            else:
                candidates = list(eps.get("aether.oauth_providers", []))  # type: ignore[call-arg]

            for ep in candidates:
                try:
                    loaded = ep.load()
                    provider = loaded() if isinstance(loaded, type) else loaded
                    if not isinstance(provider, OAuthProviderBase):
                        logger.warning(
                            "OAuth provider entry_point 无效: {} (type={})", ep.name, type(provider)
                        )
                        continue
                    self.register(provider)
                except Exception as e:
                    logger.warning("OAuth provider entry_point 加载失败: {}: {}", ep.name, e)
        except Exception as exc:
            # entry_points 不可用不影响主流程
            logger.debug("OAuth entry_points discover skipped: {}", exc)

    def register(self, provider: OAuthProviderBase) -> None:
        self._providers[provider.provider_type] = provider

    def get_provider(self, provider_type: str) -> OAuthProviderBase | None:
        return self._providers.get(provider_type)

    def get_supported_types(self) -> list[SupportedOAuthType]:
        return [
            SupportedOAuthType(
                provider_type=p.provider_type,
                display_name=p.display_name,
                default_authorization_url=p.authorization_url,
                default_token_url=p.token_url,
                default_userinfo_url=p.userinfo_url,
                default_scopes=p.default_scopes,
            )
            for p in sorted(self._providers.values(), key=lambda x: x.provider_type)
        ]


_registry: OAuthProviderRegistry | None = None


def get_oauth_provider_registry() -> OAuthProviderRegistry:
    global _registry
    if _registry is None:
        _registry = OAuthProviderRegistry()
    return _registry

