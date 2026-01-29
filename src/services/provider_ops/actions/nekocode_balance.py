"""
NekoCode 余额查询操作
"""

import time
from datetime import datetime
from typing import Any

import httpx

from src.core.logger import logger
from src.services.provider_ops.actions.balance import BalanceAction
from src.services.provider_ops.types import ActionResult, ActionStatus, BalanceInfo


class NekoCodeBalanceAction(BalanceAction):
    """
    NekoCode 余额查询

    特点：
    - 查询余额和订阅信息
    - 显示每日配额限制和剩余
    - 显示订阅状态和有效期
    - 余额单位为积分
    """

    display_name = "查询余额"
    description = "查询 NekoCode 账户余额和订阅信息"

    async def _do_query_balance(self, client: httpx.AsyncClient) -> ActionResult:
        """执行余额查询"""
        endpoint = self.config.get("endpoint", "/api/usage/summary")
        method = self.config.get("method", "GET")

        start_time = time.time()

        try:
            response = await client.request(method, endpoint)
            response_time_ms = int((time.time() - start_time) * 1000)

            try:
                data = response.json()
            except Exception:
                return self._make_error_result(
                    ActionStatus.PARSE_ERROR,
                    "响应不是有效的 JSON",
                )

            if response.status_code != 200:
                return self._handle_http_error(response, data)

            if data.get("success") is False:
                message = data.get("message", "业务状态码表示失败")
                return self._make_error_result(
                    ActionStatus.UNKNOWN_ERROR,
                    message,
                    raw_response=data,
                )

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

    def _parse_balance(self, data: Any) -> BalanceInfo:
        """解析余额信息"""
        response_data = data.get("data", {}) if isinstance(data, dict) else {}
        subscription = response_data.get("subscription", {})

        # 解析余额（积分）
        balance = self._to_float(response_data.get("balance"))

        # 解析每日配额
        daily_quota_limit = self._to_float(subscription.get("daily_quota_limit"))
        daily_remaining_quota = self._to_float(subscription.get("daily_remaining_quota"))

        # 计算每日已用配额
        daily_used = None
        if daily_quota_limit is not None and daily_remaining_quota is not None:
            daily_used = daily_quota_limit - daily_remaining_quota

        # 解析订阅信息
        plan_name = subscription.get("plan_name")
        status = subscription.get("status")
        effective_start_date = subscription.get("effective_start_date")
        effective_end_date = subscription.get("effective_end_date")

        # 解析日期
        expires_at = None
        refresh_at = None
        if effective_end_date:
            try:
                expires_at = datetime.fromisoformat(effective_end_date)
            except ValueError as e:
                logger.debug(f"解析 effective_end_date 失败: {e}")

        if effective_start_date:
            try:
                refresh_at = datetime.fromisoformat(effective_start_date)
            except ValueError as e:
                logger.debug(f"解析 effective_start_date 失败: {e}")

        # 构建 extra 信息
        extra: dict[str, Any] = {
            "plan_name": plan_name,
            "subscription_status": status,
            "daily_quota_limit": daily_quota_limit,
            "daily_remaining_quota": daily_remaining_quota,
            "daily_used_quota": daily_used,
            "effective_start_date": effective_start_date,
            "effective_end_date": effective_end_date,
        }

        # 添加刷新时间信息
        if refresh_at:
            extra["refresh_at"] = refresh_at.isoformat()
            extra["refresh_at_display"] = refresh_at.strftime("%Y-%m-%d %H:%M:%S")

        # 添加月度统计
        month_data = response_data.get("month", {})
        if month_data:
            extra["month_stats"] = {
                "total_input_tokens": month_data.get("total_input_tokens"),
                "total_output_tokens": month_data.get("total_output_tokens"),
                "total_quota": month_data.get("total_quota"),
                "total_requests": month_data.get("total_requests"),
            }

        # 添加今日统计
        today_data = response_data.get("today", {})
        if today_data:
            extra["today_stats"] = today_data.get("stats", [])

        return self._create_balance_info(
            total_available=balance,
            total_granted=daily_quota_limit,  # 每日配额作为总额度
            total_used=daily_used,  # 每日已用
            currency="USD",  # NekoCode 使用美元单位
            extra=extra,
        )

    def _handle_http_error(
        self, response: httpx.Response, raw_data: dict[str, Any] | None = None
    ) -> ActionResult:
        """处理 HTTP 错误响应"""
        status_code = response.status_code

        if status_code == 401:
            return self._make_error_result(
                ActionStatus.AUTH_FAILED, "Cookie 已失效，请重新配置", raw_response=raw_data
            )
        elif status_code == 403:
            return self._make_error_result(
                ActionStatus.AUTH_FAILED, "Cookie 已失效或无权限", raw_response=raw_data
            )

        return super()._handle_http_error(response, raw_data)

    @classmethod
    def get_config_schema(cls) -> dict[str, Any]:
        """获取操作配置 schema"""
        return {
            "type": "object",
            "properties": {
                "endpoint": {
                    "type": "string",
                    "title": "API 端点",
                    "default": "/api/usage/summary",
                },
            },
            "required": [],
        }
