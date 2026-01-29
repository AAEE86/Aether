"""
OAuth 认证模块

提供可配置的 OAuth 登录/绑定能力。
"""

from typing import TYPE_CHECKING

from src.core.modules.base import (
    ModuleCategory,
    ModuleDefinition,
    ModuleHealth,
    ModuleMetadata,
)

if TYPE_CHECKING:
    from sqlalchemy.orm import Session


def _get_router():
    """延迟导入路由（避免启动时加载重依赖/副作用）。"""
    # 延迟 discover，避免 alembic/mypy 等场景导入时触发 entry_points 解析
    from src.services.auth.oauth.registry import get_oauth_provider_registry

    get_oauth_provider_registry().discover_providers()

    from src.api.oauth import router

    return router


async def _health_check() -> ModuleHealth:
    # v1：不做外部网络探测，避免启动时阻塞
    return ModuleHealth.UNKNOWN


def _validate_config(db: Session) -> tuple[bool, str]:
    """
    验证 OAuth 配置是否可以启用模块

    检查项：
    1. 至少有一个已启用的 Provider 配置
    2. 已启用的 Provider 必须有 client_id 和 client_secret
    """
    from src.models.database import OAuthProvider

    # 查找所有已启用的 Provider
    enabled_providers = (
        db.query(OAuthProvider)
        .filter(OAuthProvider.is_enabled.is_(True))
        .all()
    )

    if not enabled_providers:
        return False, "请先配置并启用至少一个 OAuth Provider"

    # 检查每个已启用的 Provider 配置完整性
    for provider in enabled_providers:
        if not provider.client_id:
            return False, f"Provider [{provider.display_name}] 未配置 Client ID"
        if not provider.client_secret_encrypted:
            return False, f"Provider [{provider.display_name}] 未配置 Client Secret"
        if not provider.redirect_uri:
            return False, f"Provider [{provider.display_name}] 未配置回调地址"

    return True, ""


oauth_module = ModuleDefinition(
    metadata=ModuleMetadata(
        name="oauth",
        display_name="OAuth 登录",
        description="支持通过第三方 OAuth Provider 登录/绑定账号",
        category=ModuleCategory.AUTH,
        env_key="OAUTH_AVAILABLE",
        default_available=True,
        required_packages=["httpx", "redis"],
        api_prefix="/api/oauth",
        admin_route="/admin/oauth",
        admin_menu_icon="Key",
        admin_menu_group="system",
        admin_menu_order=55,
    ),
    router_factory=_get_router,
    health_check=_health_check,
    validate_config=_validate_config,
)

