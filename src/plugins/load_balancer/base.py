"""
负载均衡策略基类
定义负载均衡策略的接口
"""

from abc import abstractmethod
from dataclasses import dataclass
from typing import Any

from ..common import BasePlugin


@dataclass
class ProviderCandidate:
    """
    候选提供商信息
    """

    provider: Any  # Provider 对象
    priority: int = 0  # 优先级（数字越大优先级越高）
    weight: float = 1.0  # 权重（影响被选中的概率）
    model: Any | None = None  # Model 对象（如果需要模型信息）
    metadata: dict[str, Any] | None = None  # 额外元数据

    def __post_init__(self):
        if self.metadata is None:
            self.metadata = {}


@dataclass
class SelectionResult:
    """
    选择结果
    """

    provider: Any  # 选中的提供商
    priority: int  # 该提供商的优先级
    weight: float  # 该提供商的权重
    selection_metadata: dict[str, Any] | None = None  # 选择过程的元数据

    def __post_init__(self):
        if self.selection_metadata is None:
            self.selection_metadata = {}


class LoadBalancerStrategy(BasePlugin):
    """
    负载均衡策略基类
    所有负载均衡策略必须继承此类
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
        初始化负载均衡策略

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
    async def select(
        self, candidates: list[ProviderCandidate], context: dict[str, Any] | None = None
    ) -> SelectionResult | None:
        """
        从候选提供商中选择一个

        Args:
            candidates: 候选提供商列表
            context: 上下文信息（如请求ID、用户信息等）

        Returns:
            选择结果，如果没有可用提供商则返回 None
        """
        pass

    @abstractmethod
    async def get_stats(self) -> dict[str, Any]:
        """
        获取负载均衡统计信息

        Returns:
            统计信息字典
        """
        pass

    async def record_result(
        self,
        provider: Any,
        success: bool,
        response_time: float | None = None,
        error: Exception | None = None,
    ):
        """
        记录请求结果（用于动态调整策略）

        Args:
            provider: 提供商对象
            success: 是否成功
            response_time: 响应时间（秒）
            error: 错误信息（如果失败）
        """
        # 默认实现为空，子类可以重写来实现动态调整
        pass
