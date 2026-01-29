"""
模块基础定义

包含模块元数据、定义和状态的数据结构
"""

from dataclasses import dataclass, field
from enum import Enum
from typing import TYPE_CHECKING, Any

from collections.abc import Callable
from collections.abc import Awaitable

if TYPE_CHECKING:
    from fastapi import APIRouter
    from sqlalchemy.orm import Session


class ModuleCategory(str, Enum):
    """模块分类"""

    AUTH = "auth"  # 认证相关
    MONITORING = "monitoring"  # 监控相关
    SECURITY = "security"  # 安全相关
    INTEGRATION = "integration"  # 第三方集成


class ModuleHealth(str, Enum):
    """模块健康状态"""

    HEALTHY = "healthy"
    DEGRADED = "degraded"
    UNHEALTHY = "unhealthy"
    UNKNOWN = "unknown"


@dataclass
class ModuleMetadata:
    """
    模块元数据 - 纯数据描述，无重依赖

    用于声明式定义模块的基本信息和配置
    """

    # 基本信息
    name: str  # 唯一标识: ldap, audit_log
    display_name: str  # 显示名称: "LDAP 认证"
    description: str  # 模块描述

    # 分类
    category: ModuleCategory

    # 可用性控制（部署级）
    env_key: str  # 环境变量名: LDAP_AVAILABLE
    default_available: bool = False  # 默认是否可用
    required_packages: list[str] = field(default_factory=list)  # 依赖的 Python 包
    dependencies: list[str] = field(default_factory=list)  # 依赖的其他模块

    # 路由配置 - 模块自定义前缀
    api_prefix: str | None = None  # 如 "/api/admin/ldap"

    # 前端配置
    admin_route: str | None = None  # 管理页面路由: "/admin/ldap"
    admin_menu_icon: str | None = None  # 菜单图标
    admin_menu_group: str | None = None  # 菜单分组: "system", "security"
    admin_menu_order: int = 100  # 菜单排序（越小越靠前）


@dataclass
class ModuleDefinition:
    """
    完整模块定义

    包含元数据和生命周期钩子，钩子函数内部延迟导入重依赖
    """

    metadata: ModuleMetadata

    # 工厂函数 - 内部再 import 重依赖
    router_factory: Callable[[], APIRouter] | None = None
    service_factory: Callable[[], Any] | None = None

    # 生命周期钩子
    on_startup: Callable[[], Awaitable[None]] | None = None
    on_shutdown: Callable[[], Awaitable[None]] | None = None
    health_check: Callable[[], Awaitable[ModuleHealth]] | None = None

    # 自定义依赖检测（可选，用于检测 ldap3 等库是否安装）
    check_dependencies: Callable[[], bool] | None = None

    # 配置验证（可选，启用模块时调用，返回 (success, error_message)）
    validate_config: Callable[[Session], tuple[bool, str]] | None = None


@dataclass
class ModuleStatus:
    """
    模块运行状态

    用于 API 返回，供前端使用
    """

    name: str
    available: bool  # 部署级可用（环境变量 + 依赖库）
    enabled: bool  # 运行级启用（数据库配置）
    active: bool  # 最终激活状态 (available && enabled && dependencies_ok)
    config_validated: bool  # 配置验证通过（只有验证通过才允许启用）
    config_error: str | None  # 配置验证失败的错误信息

    # 显示信息
    display_name: str
    description: str
    category: ModuleCategory

    # 前端配置
    admin_route: str | None
    admin_menu_icon: str | None
    admin_menu_group: str | None
    admin_menu_order: int

    # 健康状态
    health: ModuleHealth = ModuleHealth.UNKNOWN
