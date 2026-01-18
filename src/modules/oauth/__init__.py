"""
通用 OAuth 认证模块

提供多 OAuth 提供商（如 LinuxDo、GitHub、Google 等）的用户认证支持
"""

from src.core.modules.base import (
    ModuleCategory,
    ModuleDefinition,
    ModuleHealth,
    ModuleMetadata,
)


def _get_router():
    """延迟导入路由（避免启动时加载重依赖）"""
    from src.api.admin.oauth import router

    return router


async def _health_check() -> ModuleHealth:
    """健康检查 - 简化版，不依赖数据库连接"""
    return ModuleHealth.UNKNOWN


# OAuth 通用模块定义
oauth_module = ModuleDefinition(
    metadata=ModuleMetadata(
        name="oauth",
        display_name="OAuth 登录",
        description="支持通过多种 OAuth 提供商进行用户认证（如 LinuxDo、GitHub、Google 等）",
        category=ModuleCategory.AUTH,
        # 可用性控制
        env_key="OAUTH_AVAILABLE",
        default_available=True,
        required_packages=[],  # 使用已有的 httpx
        # 路由配置
        api_prefix="/api/admin/oauth",
        # 前端配置
        admin_route="/admin/oauth",
        admin_menu_icon="KeyRound",
        admin_menu_group="system",
        admin_menu_order=51,
    ),
    router_factory=_get_router,
    health_check=_health_check,
)
