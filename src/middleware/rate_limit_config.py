"""
速率限制配置

提供灵活的端点速率限制策略配置
"""

from dataclasses import dataclass
from typing import Literal

RateLimitScope = Literal["server_ip", "user", "api_key", "skip"]


@dataclass
class RateLimitPolicy:
    """速率限制策略"""

    scope: RateLimitScope  # 限制范围
    limit: int  # 限制值（请求数/分钟）
    description: str = ""


class RateLimitConfig:
    """
    速率限制配置管理

    定义不同路径前缀的速率限制策略
    """

    # 默认策略配置
    POLICIES: dict[str, RateLimitPolicy] = {
        # 客户端 API 端点 - 服务器级别 IP 限制
        "/v1/": RateLimitPolicy(
            scope="server_ip", limit=60, description="Claude/OpenAI API 端点，服务器级别限制"
        ),
        # 公共 API 端点 - 服务器级别 IP 限制
        "/api/public/": RateLimitPolicy(
            scope="server_ip", limit=60, description="公共只读 API，服务器级别限制"
        ),
        # 管理后台端点 - 用户级别限制
        "/api/admin/": RateLimitPolicy(
            scope="user", limit=1000, description="管理后台，用户级别限制"
        ),
        # 认证端点 - 跳过中间件（在路由层处理）
        "/api/auth/": RateLimitPolicy(scope="skip", limit=0, description="认证端点，路由层处理"),
        # 用户端点 - 用户级别限制
        "/api/users/": RateLimitPolicy(scope="skip", limit=0, description="用户端点，路由层处理"),
        # 监控端点 - 跳过限制
        "/api/monitoring/": RateLimitPolicy(scope="skip", limit=0, description="监控端点"),
    }

    @classmethod
    def get_policy_for_path(cls, path: str) -> RateLimitPolicy | None:
        """
        根据路径获取速率限制策略

        按照最长匹配原则，优先匹配更具体的路径

        Args:
            path: 请求路径

        Returns:
            匹配的速率限制策略，如果没有匹配则返回 None
        """
        # 按路径长度降序排序，确保最长匹配优先
        sorted_prefixes = sorted(cls.POLICIES.keys(), key=len, reverse=True)

        for prefix in sorted_prefixes:
            if path.startswith(prefix):
                return cls.POLICIES[prefix]

        return None

    @classmethod
    def register_policy(cls, prefix: str, policy: RateLimitPolicy) -> None:
        """
        注册新的速率限制策略

        Args:
            prefix: 路径前缀
            policy: 速率限制策略
        """
        cls.POLICIES[prefix] = policy

    @classmethod
    def get_all_policies(cls) -> dict[str, RateLimitPolicy]:
        """获取所有策略配置"""
        return cls.POLICIES.copy()
