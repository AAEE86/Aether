"""
签到操作抽象基类
"""

from abc import abstractmethod
from typing import Any

import httpx

from src.services.provider_ops.actions.base import ProviderAction
from src.services.provider_ops.types import (
    ActionResult,
    CheckinInfo,
    ProviderActionType,
)


class CheckinAction(ProviderAction):
    """
    签到操作抽象基类

    子类必须实现 execute() 方法来处理特定平台的签到逻辑。
    """

    action_type = ProviderActionType.CHECKIN
    display_name = "签到"
    description = "每日签到领取额度"
    default_cache_ttl = 3600  # 签到结果缓存 1 小时

    @abstractmethod
    async def execute(self, client: httpx.AsyncClient) -> ActionResult:
        """
        执行签到

        子类必须实现此方法。

        Args:
            client: 已认证的 HTTP 客户端

        Returns:
            ActionResult，其中 data 字段为 CheckinInfo
        """
        pass

    def _create_checkin_info(
        self,
        reward: float | None = None,
        streak_days: int | None = None,
        message: str | None = None,
        extra: dict[str, Any] | None = None,
    ) -> CheckinInfo:
        """
        创建签到信息对象

        辅助方法，用于创建统一格式的 CheckinInfo。

        Args:
            reward: 签到奖励额度
            streak_days: 连续签到天数
            message: 签到消息
            extra: 额外信息

        Returns:
            CheckinInfo 对象
        """
        return CheckinInfo(
            reward=reward,
            streak_days=streak_days,
            message=message,
            extra=extra if extra is not None else {},
        )

    @classmethod
    def get_config_schema(cls) -> dict[str, Any]:
        """获取操作配置 schema（子类可重写）"""
        return {
            "type": "object",
            "properties": {},
            "required": [],
        }
