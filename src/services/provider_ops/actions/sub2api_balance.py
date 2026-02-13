"""
Sub2API 余额查询操作
"""

from typing import Any

from src.services.provider_ops.actions.balance import BalanceAction
from src.services.provider_ops.types import BalanceInfo


class Sub2ApiBalanceAction(BalanceAction):
    """
    Sub2API 余额查询

    特点：
    - 使用 /api/v1/auth/me 端点
    - balance 为充值余额，points 为赠送余额，均以美元为单位
    - 响应格式: {"code": 0, "message": "success", "data": {...}}
    """

    display_name = "查询余额"
    description = "查询 Sub2API 账户余额信息"

    def _parse_balance(self, data: Any) -> BalanceInfo:
        """解析 Sub2API 余额信息"""
        # Sub2API 使用 {"code": 0, ...} 表示成功，非 0 表示业务错误
        if isinstance(data, dict) and data.get("code") is not None and data.get("code") != 0:
            message = data.get("message", "查询失败")
            raise ValueError(f"Sub2API 业务错误: {message}")

        user_data = data.get("data", {}) if isinstance(data, dict) else {}

        balance = self._to_float(user_data.get("balance")) or 0.0
        points = self._to_float(user_data.get("points")) or 0.0

        total_available = balance + points

        return self._create_balance_info(
            total_available=total_available,
            currency="USD",
            extra={
                "balance": balance,
                "points": points,
            },
        )
