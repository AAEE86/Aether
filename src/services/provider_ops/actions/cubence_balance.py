"""
Cubence 余额查询操作
"""

import time
from typing import Any

import httpx

from src.services.provider_ops.actions.balance import BalanceAction
from src.services.provider_ops.types import ActionResult, ActionStatus, BalanceInfo


class CubenceBalanceAction(BalanceAction):
    """
    Cubence 专用余额查询

    特点：
    - 余额单位直接是美元
    - 支持窗口限额查询（5小时/每周）
    - Cookie 失效时返回友好的错误提示
    """

    display_name = "查询余额（含窗口限额）"
    description = "查询账户余额和窗口限额信息"

    async def _do_query_balance(self, client: httpx.AsyncClient) -> ActionResult:
        """执行 Cubence 余额查询（实现抽象方法）"""
        endpoint = self.config.get("endpoint", "/api/v1/dashboard/overview")
        method = self.config.get("method", "GET")

        start_time = time.time()

        try:
            response = await client.request(method, endpoint)
            response_time_ms = int((time.time() - start_time) * 1000)

            # 尝试解析 JSON
            try:
                data = response.json()
            except Exception:
                return self._make_error_result(
                    ActionStatus.PARSE_ERROR,
                    "响应不是有效的 JSON",
                )

            # 检查 HTTP 状态
            if response.status_code != 200:
                return self._handle_http_error(response, data)

            # 检查业务状态码
            if data.get("success") is False:
                message = data.get("message", "业务状态码表示失败")
                return self._make_error_result(
                    ActionStatus.UNKNOWN_ERROR,
                    message,
                    raw_response=data,
                )

            # 解析余额信息
            balance = self._parse_balance(data)

            return self._make_success_result(
                data=balance,
                response_time_ms=response_time_ms,
                raw_response=data,
            )

        except httpx.TimeoutException:
            return self._make_error_result(
                ActionStatus.NETWORK_ERROR,
                "请求超时",
                retry_after_seconds=30,
            )
        except httpx.RequestError as e:
            return self._make_error_result(
                ActionStatus.NETWORK_ERROR,
                f"网络错误: {str(e)}",
                retry_after_seconds=30,
            )
        except Exception as e:
            return self._make_error_result(
                ActionStatus.UNKNOWN_ERROR,
                f"未知错误: {str(e)}",
            )

    def _handle_http_error(
        self, response: httpx.Response, raw_data: dict[str, Any] | None = None
    ) -> ActionResult:
        """处理 HTTP 错误响应（Cubence 专用）"""
        status_code = response.status_code

        # Cubence 使用 Cookie 认证，提供更友好的错误提示
        if status_code == 401:
            return self._make_error_result(
                ActionStatus.AUTH_FAILED, "Cookie 已失效，请重新配置", raw_response=raw_data
            )
        elif status_code == 403:
            return self._make_error_result(
                ActionStatus.AUTH_FAILED, "Cookie 已失效或无权限", raw_response=raw_data
            )

        # 其他错误使用基类处理
        return super()._handle_http_error(response, raw_data)

    def _parse_balance(self, data: Any) -> BalanceInfo:
        """解析 Cubence 余额信息"""
        # Cubence 响应格式：data.balance 和 data.subscription_limits
        response_data = data.get("data", {}) if isinstance(data, dict) else {}
        balance_data = response_data.get("balance", {})
        subscription_limits = response_data.get("subscription_limits", {})

        # 余额信息（单位直接是美元）
        total_available = balance_data.get("total_balance_dollar")
        normal_balance = balance_data.get("normal_balance_dollar")
        subscription_balance = balance_data.get("subscription_balance_dollar")
        charity_balance = balance_data.get("charity_balance_dollar")

        # 窗口限额信息
        extra: dict[str, Any] = {}

        # 5小时窗口限额
        five_hour = subscription_limits.get("five_hour", {})
        if five_hour:
            extra["five_hour_limit"] = {
                "limit": five_hour.get("limit"),
                "used": five_hour.get("used"),
                "remaining": five_hour.get("remaining"),
                "resets_at": five_hour.get("resets_at"),
            }

        # 每周窗口限额
        weekly = subscription_limits.get("weekly", {})
        if weekly:
            extra["weekly_limit"] = {
                "limit": weekly.get("limit"),
                "used": weekly.get("used"),
                "remaining": weekly.get("remaining"),
                "resets_at": weekly.get("resets_at"),
            }

        # 余额组成
        if normal_balance is not None:
            extra["normal_balance"] = normal_balance
        if subscription_balance is not None:
            extra["subscription_balance"] = subscription_balance
        if charity_balance is not None:
            extra["charity_balance"] = charity_balance

        return self._create_balance_info(
            total_available=total_available,
            currency=self.config.get("currency", "USD"),
            extra=extra if extra else None,
        )
