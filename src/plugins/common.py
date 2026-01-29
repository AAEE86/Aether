"""
插件系统通用定义
包含所有插件类型共享的类和接口
"""

from abc import ABC
from dataclasses import dataclass
from enum import Enum
from typing import Any

from src.core.logger import logger


class HealthStatus(Enum):
    """插件健康状态"""

    HEALTHY = "healthy"
    DEGRADED = "degraded"
    UNHEALTHY = "unhealthy"


@dataclass
class PluginMetadata:
    """插件元数据"""

    name: str
    version: str = "1.0.0"
    author: str = "Unknown"
    description: str = ""
    api_version: str = "1.0"
    dependencies: list[str] = None
    provides: list[str] = None

    def __post_init__(self):
        if self.dependencies is None:
            self.dependencies = []
        if self.provides is None:
            self.provides = []


class BasePlugin(ABC):
    """
    所有插件的基类
    定义插件的基本生命周期和元数据管理
    """

    def __init__(
        self,
        name: str,
        priority: int = 0,
        version: str = "1.0.0",
        author: str = "Unknown",
        description: str = "",
        api_version: str = "1.0",
        dependencies: list[str] = None,
        provides: list[str] = None,
        config: dict[str, Any] = None,
    ):
        """
        初始化插件

        Args:
            name: 插件名称
            priority: 优先级（数字越大优先级越高）
            version: 插件版本
            author: 插件作者
            description: 插件描述
            api_version: API版本
            dependencies: 依赖的其他插件
            provides: 提供的服务
            config: 配置字典
        """
        self.name = name
        self.priority = priority
        self.enabled = True
        self.config = config or {}
        self.metadata = PluginMetadata(
            name=name,
            version=version,
            author=author,
            description=description,
            api_version=api_version,
            dependencies=dependencies or [],
            provides=provides or [],
        )
        self._initialized = False

    async def initialize(self) -> bool:
        """
        初始化插件

        Returns:
            初始化成功返回True，失败返回False
        """
        if self._initialized:
            return True

        try:
            await self._do_initialize()
            self._initialized = True
            return True
        except Exception as e:
            logger.warning(f"Failed to initialize plugin {self.name}: {e}")
            return False

    async def _do_initialize(self):
        """
        子类可以重写此方法来实现特定的初始化逻辑
        """
        pass

    async def shutdown(self):
        """
        关闭插件，清理资源
        """
        if not self._initialized:
            return

        try:
            await self._do_shutdown()
        except Exception as e:
            logger.warning(f"Error during plugin {self.name} shutdown: {e}")
        finally:
            self._initialized = False

    async def _do_shutdown(self):
        """
        子类可以重写此方法来实现特定的清理逻辑
        """
        pass

    async def health_check(self) -> HealthStatus:
        """
        检查插件健康状态

        Returns:
            插件健康状态
        """
        if not self._initialized or not self.enabled:
            return HealthStatus.UNHEALTHY

        try:
            return await self._do_health_check()
        except Exception:
            return HealthStatus.UNHEALTHY

    async def _do_health_check(self) -> HealthStatus:
        """
        子类可以重写此方法来实现特定的健康检查逻辑
        默认实现：如果插件已初始化且启用，则认为健康
        """
        return (
            HealthStatus.HEALTHY if (self._initialized and self.enabled) else HealthStatus.UNHEALTHY
        )

    def configure(self, config: dict[str, Any]):
        """
        配置插件

        Args:
            config: 配置字典
        """
        self.config.update(config)
        self.enabled = config.get("enabled", True)

    def get_metadata(self) -> PluginMetadata:
        """
        获取插件元数据

        Returns:
            插件元数据
        """
        return self.metadata

    @property
    def is_initialized(self) -> bool:
        """检查插件是否已初始化"""
        return self._initialized

    def validate_dependencies(self, available_plugins: dict[str, list[str]]) -> list[str]:
        """
        验证插件依赖是否满足

        Args:
            available_plugins: 可用插件字典 {plugin_type: [plugin_names]}

        Returns:
            缺失的依赖列表
        """
        missing_deps = []
        for dep in self.metadata.dependencies:
            found = False
            for plugin_type, plugin_names in available_plugins.items():
                if dep in plugin_names:
                    found = True
                    break
            if not found:
                missing_deps.append(dep)
        return missing_deps

    def __repr__(self):
        return f"<{self.__class__.__name__}(name={self.name}, priority={self.priority}, enabled={self.enabled}, version={self.metadata.version})>"
