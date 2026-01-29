"""
速率限制策略基类
定义速率限制策略的接口
"""

from abc import abstractmethod
from dataclasses import dataclass
from datetime import datetime
from typing import Any

from ..common import BasePlugin


@dataclass
class RateLimitResult:
    """
    速率限制检查结果
    """

    allowed: bool
    remaining: int
    reset_at: datetime | None = None
    retry_after: int | None = None
    message: str | None = None
    headers: dict[str, str] | None = None

    def __post_init__(self):
        if self.headers is None:
            self.headers = {}
            if self.remaining is not None:
                self.headers["X-RateLimit-Remaining"] = str(self.remaining)
            if self.reset_at:
                self.headers["X-RateLimit-Reset"] = str(int(self.reset_at.timestamp()))
            if self.retry_after:
                self.headers["Retry-After"] = str(self.retry_after)


class RateLimitStrategy(BasePlugin):
    """
    速率限制策略基类
    所有速率限制策略必须继承此类
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
        初始化速率限制策略

        Args:
            name: 策略名称
            priority: 优先级（数字越大优先级越高）
            version: 插件版本
            author: 插件作者
            description: 插件描述
            api_version: API版本
            dependencies: 依赖的其他插件
            provides: 提供的服务
            config: 配置字典
        """
        super().__init__(
            name=name,
            priority=priority,
            version=version,
            author=author,
            description=description,
            api_version=api_version,
            dependencies=dependencies,
            provides=provides,
            config=config,
        )

    @abstractmethod
    async def check_limit(self, key: str, **kwargs) -> RateLimitResult:
        """
        检查速率限制

        Args:
            key: 限制键（如用户ID、API Key ID等）
            **kwargs: 额外参数

        Returns:
            速率限制检查结果
        """
        pass

    @abstractmethod
    async def consume(self, key: str, amount: int = 1, **kwargs) -> bool:
        """
        消费配额

        Args:
            key: 限制键
            amount: 消费数量
            **kwargs: 额外参数

        Returns:
            是否成功消费
        """
        pass

    @abstractmethod
    async def reset(self, key: str):
        """
        重置限制

        Args:
            key: 限制键
        """
        pass

    @abstractmethod
    async def get_stats(self, key: str) -> dict[str, Any]:
        """
        获取统计信息

        Args:
            key: 限制键

        Returns:
            统计信息字典
        """
        pass
