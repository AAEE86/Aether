"""
架构注册表

管理所有可用的 Provider 架构。
"""

from __future__ import annotations

import threading

from src.core.logger import logger
from src.services.provider_ops.architectures import (
    AnyrouterArchitecture,
    CubenceArchitecture,
    GenericApiArchitecture,
    NekoCodeArchitecture,
    NewApiArchitecture,
    OneApiArchitecture,
    ProviderArchitecture,
    YesCodeArchitecture,
)


class ArchitectureRegistry:
    """
    架构注册表

    单例模式，管理所有可用的 Provider 架构。
    """

    _instance: ArchitectureRegistry | None = None
    _lock = threading.Lock()
    _initialized: bool = False

    def __new__(cls) -> ArchitectureRegistry:
        if cls._instance is None:
            with cls._lock:
                if cls._instance is None:
                    cls._instance = super().__new__(cls)
                    cls._instance._initialized = False
        return cls._instance

    def __init__(self) -> None:
        if self._initialized:
            return

        self._architectures: dict[str, ProviderArchitecture] = {}
        self._initialized = True

        # 注册内置架构
        self._register_builtin_architectures()

    def _register_builtin_architectures(self) -> None:
        """注册内置架构"""
        builtin: list[type[ProviderArchitecture]] = [
            AnyrouterArchitecture,
            CubenceArchitecture,
            GenericApiArchitecture,
            NekoCodeArchitecture,
            NewApiArchitecture,
            OneApiArchitecture,
            YesCodeArchitecture,
        ]

        for arch_cls in builtin:
            self.register(arch_cls())

    def register(self, architecture: ProviderArchitecture) -> None:
        """
        注册架构

        Args:
            architecture: 架构实例
        """
        if architecture.architecture_id in self._architectures:
            logger.warning(f"架构 {architecture.architecture_id} 已存在，将被覆盖")

        self._architectures[architecture.architecture_id] = architecture
        logger.debug(f"注册架构: {architecture.architecture_id}")

    def unregister(self, architecture_id: str) -> bool:
        """
        注销架构

        Args:
            architecture_id: 架构 ID

        Returns:
            是否成功注销
        """
        if architecture_id in self._architectures:
            del self._architectures[architecture_id]
            return True
        return False

    def get(self, architecture_id: str) -> ProviderArchitecture | None:
        """
        获取架构

        Args:
            architecture_id: 架构 ID

        Returns:
            架构实例，不存在则返回 None
        """
        return self._architectures.get(architecture_id)

    def get_or_default(self, architecture_id: str | None = None) -> ProviderArchitecture:
        """
        获取架构，如果不存在则返回默认架构

        Args:
            architecture_id: 架构 ID

        Returns:
            架构实例
        """
        if architecture_id and architecture_id in self._architectures:
            return self._architectures[architecture_id]

        # 返回默认架构（generic_api）
        return self._architectures.get("generic_api", GenericApiArchitecture())

    def list_all(self) -> list[ProviderArchitecture]:
        """获取所有已注册的架构"""
        return list(self._architectures.values())

    def list_ids(self) -> list[str]:
        """获取所有已注册的架构 ID"""
        return list(self._architectures.keys())

    def to_dict_list(self) -> list[dict]:
        """获取所有架构的字典表示（用于 API 响应）"""
        return [arch.to_dict() for arch in self._architectures.values()]


# 全局注册表实例
_registry: ArchitectureRegistry | None = None


def get_registry() -> ArchitectureRegistry:
    """获取全局注册表实例"""
    global _registry
    if _registry is None:
        _registry = ArchitectureRegistry()
    return _registry
