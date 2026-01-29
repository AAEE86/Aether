"""
模块注册中心

负责模块的注册、状态管理和生命周期控制
"""

from __future__ import annotations

import importlib.util
import os
from typing import TYPE_CHECKING

from src.core.logger import logger
from src.core.modules.base import (
    ModuleCategory,
    ModuleDefinition,
    ModuleHealth,
    ModuleStatus,
)

if TYPE_CHECKING:
    from sqlalchemy.orm import Session


class ModuleRegistry:
    """
    模块注册中心 - 单例模式

    职责：
    - 注册模块定义（仅元数据，不加载重依赖）
    - 检查模块可用性（环境变量 + 依赖库）
    - 管理模块启用状态（数据库配置）
    - 提供模块状态查询
    """

    _instance: ModuleRegistry | None = None

    def __init__(self):
        self._modules: dict[str, ModuleDefinition] = {}
        self._initialized: set[str] = set()

    @classmethod
    def get_instance(cls) -> ModuleRegistry:
        """获取单例实例"""
        if cls._instance is None:
            cls._instance = cls()
        return cls._instance

    @classmethod
    def reset_instance(cls) -> None:
        """重置单例（仅用于测试）"""
        cls._instance = None

    def register(self, module: ModuleDefinition) -> None:
        """
        注册模块

        仅注册元数据，不加载重依赖
        """
        name = module.metadata.name
        if name in self._modules:
            logger.warning(f"Module [{name}] already registered, skipping")
            return

        self._modules[name] = module
        logger.debug(f"Module [{name}] registered")

    def get_module(self, name: str) -> ModuleDefinition | None:
        """获取模块定义"""
        return self._modules.get(name)

    def get_all_modules(self) -> list[ModuleDefinition]:
        """获取所有已注册模块"""
        return list(self._modules.values())

    # ========== 可用性检查（部署级）==========

    def is_available(self, name: str) -> bool:
        """
        检查模块是否部署可用

        检查顺序：
        1. 模块是否已注册
        2. 环境变量是否启用
        3. 依赖的 Python 包是否安装
        4. 自定义依赖检测（如果有）
        """
        if name not in self._modules:
            return False

        module = self._modules[name]
        meta = module.metadata

        # 1. 检查环境变量
        env_value = os.getenv(meta.env_key)
        if env_value is not None:
            if env_value.lower() not in ("true", "1", "yes"):
                return False
        elif not meta.default_available:
            return False

        # 2. 检查依赖的 Python 包
        for pkg in meta.required_packages:
            if importlib.util.find_spec(pkg) is None:
                logger.debug(f"Module [{name}] unavailable: package '{pkg}' not installed")
                return False

        # 3. 自定义依赖检测
        if module.check_dependencies:
            try:
                if not module.check_dependencies():
                    logger.debug(f"Module [{name}] unavailable: custom dependency check failed")
                    return False
            except Exception as e:
                logger.warning(f"Module [{name}] dependency check error: {e}")
                return False

        return True

    def get_available_modules(self) -> list[ModuleDefinition]:
        """获取所有部署可用的模块"""
        return [m for m in self._modules.values() if self.is_available(m.metadata.name)]

    # ========== 启用状态检查（运行级）==========

    def is_enabled(self, name: str, db: Session) -> bool:
        """
        检查模块是否运行启用（数据库配置）

        Args:
            name: 模块名称
            db: 数据库会话
        """
        from src.services.system.config import SystemConfigService

        config_key = f"module.{name}.enabled"
        value = SystemConfigService.get_config(db, config_key, default=False)
        return bool(value)

    def set_enabled(self, name: str, enabled: bool, db: Session) -> None:
        """
        设置模块启用状态

        Args:
            name: 模块名称
            enabled: 是否启用
            db: 数据库会话
        """
        from src.services.system.config import SystemConfigService

        if name not in self._modules:
            raise ValueError(f"Module [{name}] not registered")

        config_key = f"module.{name}.enabled"
        module = self._modules[name]
        description = f"模块 [{module.metadata.display_name}] 启用状态"
        SystemConfigService.set_config(db, config_key, enabled, description)

    # ========== 激活状态检查 ==========

    def is_active(self, name: str, db: Session) -> bool:
        """
        检查模块是否最终激活

        激活条件：available && enabled && 依赖模块都激活
        """
        if not self.is_available(name):
            return False
        if not self.is_enabled(name, db):
            return False

        # 检查依赖模块
        module = self._modules[name]
        for dep in module.metadata.dependencies:
            if not self.is_active(dep, db):
                return False

        return True

    # ========== 配置验证 ==========

    def validate_config(self, name: str, db: Session) -> tuple[bool, str]:
        """
        验证模块配置是否有效

        Args:
            name: 模块名称
            db: 数据库会话

        Returns:
            (validated, error_message) - validated 为 True 表示配置有效
        """
        if name not in self._modules:
            return False, "模块不存在"

        module = self._modules[name]

        # 没有配置验证函数的模块，默认配置有效
        if not module.validate_config:
            return True, ""

        try:
            return module.validate_config(db)
        except Exception as e:
            logger.warning(f"Module [{name}] config validation error: {e}")
            return False, f"配置验证出错: {str(e)}"

    # ========== 状态查询 ==========

    def get_module_status(
        self, name: str, db: Session, health: ModuleHealth | None = None
    ) -> ModuleStatus | None:
        """
        获取单个模块状态

        Args:
            name: 模块名称
            db: 数据库会话
            health: 预先获取的健康状态（可选，用于异步场景）
        """
        if name not in self._modules:
            return None

        module = self._modules[name]
        meta = module.metadata
        available = self.is_available(name)

        # 获取配置验证状态
        config_validated = False
        config_error: str | None = None
        if available:
            config_validated, config_error = self.validate_config(name, db)
            if config_validated:
                config_error = None  # 验证通过时清空错误信息

        # 获取启用状态
        enabled = self.is_enabled(name, db) if available else False

        # 注意：配置验证失败时不自动禁用模块
        # 自动禁用会在查询方法中产生写操作副作用，违反幂等性原则
        # 配置验证状态通过 config_validated/config_error 字段返回，由调用方决定如何处理

        return ModuleStatus(
            name=name,
            available=available,
            enabled=enabled,
            active=self.is_active(name, db) if available else False,
            config_validated=config_validated,
            config_error=config_error,
            display_name=meta.display_name,
            description=meta.description,
            category=meta.category,
            admin_route=meta.admin_route if available else None,
            admin_menu_icon=meta.admin_menu_icon,
            admin_menu_group=meta.admin_menu_group,
            admin_menu_order=meta.admin_menu_order,
            health=health if health else ModuleHealth.UNKNOWN,
        )

    async def check_health(self, name: str) -> ModuleHealth:
        """
        执行模块健康检查

        Args:
            name: 模块名称

        Returns:
            健康状态
        """
        if name not in self._modules:
            return ModuleHealth.UNKNOWN

        module = self._modules[name]
        if not module.health_check:
            return ModuleHealth.UNKNOWN

        try:
            return await module.health_check()
        except Exception as e:
            logger.warning(f"Module [{name}] health check failed: {e}")
            return ModuleHealth.UNHEALTHY

    async def get_module_status_async(
        self, name: str, db: Session
    ) -> ModuleStatus | None:
        """异步获取模块状态（包含健康检查）"""
        if name not in self._modules:
            return None

        health = await self.check_health(name) if self.is_available(name) else ModuleHealth.UNKNOWN
        return self.get_module_status(name, db, health=health)

    async def get_all_status_async(self, db: Session) -> dict[str, ModuleStatus]:
        """异步获取所有模块状态（包含健康检查）"""
        result = {}
        for name in self._modules:
            status = await self.get_module_status_async(name, db)
            if status:
                result[name] = status
        return result

    def get_all_status(self, db: Session) -> dict[str, ModuleStatus]:
        """获取所有模块状态（同步版本，不含健康检查）"""
        result = {}
        for name in self._modules:
            status = self.get_module_status(name, db)
            if status:
                result[name] = status
        return result

    def get_available_status(self, db: Session) -> dict[str, ModuleStatus]:
        """获取所有可用模块的状态"""
        result = {}
        for name, module in self._modules.items():
            if self.is_available(name):
                status = self.get_module_status(name, db)
                if status:
                    result[name] = status
        return result

    def get_auth_modules_status(self, db: Session) -> list[ModuleStatus]:
        """获取认证模块状态（供登录页使用）"""
        result = []
        for name, module in self._modules.items():
            if module.metadata.category == ModuleCategory.AUTH:
                if self.is_available(name):
                    status = self.get_module_status(name, db)
                    if status:
                        result.append(status)
        return result


def get_module_registry() -> ModuleRegistry:
    """获取模块注册中心实例"""
    return ModuleRegistry.get_instance()
