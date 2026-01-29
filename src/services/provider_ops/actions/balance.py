"""
余额查询操作抽象基类
"""

from abc import abstractmethod
from typing import Any

import httpx

from src.services.provider_ops.actions.base import ProviderAction
from src.services.provider_ops.types import (
    ActionResult,
    ActionStatus,
    BalanceInfo,
    ProviderActionType,
)


class BalanceAction(ProviderAction):
    """
    余额查询操作抽象基类

    子类必须实现 _do_query_balance() 方法来处理特定平台的余额查询逻辑。
    子类可选实现 _do_checkin() 方法来在查询余额前执行签到。
    """

    action_type = ProviderActionType.QUERY_BALANCE
    display_name = "查询余额"
    description = "查询账户余额信息"
    default_cache_ttl = 86400  # 24 小时

    async def execute(self, client: httpx.AsyncClient) -> ActionResult:
        """
        执行余额查询（模板方法）

        1. 先尝试签到（如果子类实现了 _do_checkin）
        2. 执行余额查询

        Args:
            client: 已认证的 HTTP 客户端

        Returns:
            ActionResult，其中 data 字段为 BalanceInfo
        """
        from src.core.logger import logger

        # 先尝试签到
        checkin_result = await self._do_checkin(client)

        # 执行余额查询
        result = await self._do_query_balance(client)

        # 将签到结果附加到 extra 字段
        if checkin_result and result.data and hasattr(result.data, "extra"):
            if result.data.extra is None:
                result.data.extra = {}
            # 处理 cookie_expired 标记
            if checkin_result.get("cookie_expired"):
                result.data.extra["cookie_expired"] = True
                result.data.extra["cookie_expired_message"] = checkin_result.get("message", "")
                result.status = ActionStatus.AUTH_EXPIRED
                logger.warning(f"Cookie 已失效: {checkin_result}")
            else:
                result.data.extra["checkin_success"] = checkin_result.get("success")
                result.data.extra["checkin_message"] = checkin_result.get("message", "")
                logger.debug(f"签到结果已附加到 extra: {checkin_result}")

        return result

    @abstractmethod
    async def _do_query_balance(self, client: httpx.AsyncClient) -> ActionResult:
        """
        执行余额查询（子类必须实现）

        Args:
            client: 已认证的 HTTP 客户端

        Returns:
            ActionResult，其中 data 字段为 BalanceInfo
        """
        pass

    async def _do_checkin(self, client: httpx.AsyncClient) -> dict[str, Any] | None:
        """
        执行签到（子类可选实现）

        默认实现返回 None（不签到）。
        子类可重写此方法实现平台特定的签到逻辑。

        Args:
            client: 已认证的 HTTP 客户端

        Returns:
            签到结果字典 {"success": bool, "message": str}，或 None 表示不签到
        """
        return None

    def _create_balance_info(
        self,
        total_granted: float | None = None,
        total_used: float | None = None,
        total_available: float | None = None,
        currency: str = "USD",
        extra: dict[str, Any] | None = None,
    ) -> BalanceInfo:
        """
        创建余额信息对象

        辅助方法，用于创建统一格式的 BalanceInfo。
        如果只有部分数据，会尝试计算缺失的值。

        Args:
            total_granted: 总额度
            total_used: 已用额度
            total_available: 可用余额
            currency: 货币单位
            extra: 额外信息

        Returns:
            BalanceInfo 对象
        """
        # 如果只有部分数据，尝试计算
        if total_available is None and total_granted is not None and total_used is not None:
            total_available = total_granted - total_used
        if total_used is None and total_granted is not None and total_available is not None:
            total_used = total_granted - total_available
        if total_granted is None and total_used is not None and total_available is not None:
            total_granted = total_used + total_available

        return BalanceInfo(
            total_granted=total_granted,
            total_used=total_used,
            total_available=total_available,
            currency=currency,
            extra=extra if extra is not None else {},
        )

    def _to_float(self, value: Any) -> float | None:
        """转换为浮点数"""
        if value is None:
            return None
        try:
            return float(value)
        except (TypeError, ValueError):
            return None

    @classmethod
    def get_config_schema(cls) -> dict[str, Any]:
        """获取操作配置 schema（子类可重写）"""
        return {
            "type": "object",
            "properties": {
                "currency": {
                    "type": "string",
                    "title": "货币单位",
                    "default": "USD",
                },
            },
            "required": [],
        }
