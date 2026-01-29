"""
New API 余额查询操作
"""

import time
from typing import Any

import httpx

from src.core.logger import logger
from src.services.provider_ops.actions.balance import BalanceAction
from src.services.provider_ops.types import ActionResult, ActionStatus, BalanceInfo


class NewApiBalanceAction(BalanceAction):
    """
    New API 风格的余额查询

    特点：
    - 使用 /api/user/self 端点
    - quota 单位是 1/500000 美元
    - 支持查询前自动签到（通过基类的模板方法）
    """

    display_name = "查询余额"
    description = "查询 New API 账户余额信息"

    async def _do_query_balance(self, client: httpx.AsyncClient) -> ActionResult:
        """执行余额查询（实现抽象方法）"""
        endpoint = self.config.get("endpoint", "/api/user/self")
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

    def _parse_balance(
        self,
        data: Any,
    ) -> BalanceInfo:
        """解析 New API 余额信息"""
        # New API 响应格式: {"success": true, "data": {...}}
        user_data = data.get("data", {}) if isinstance(data, dict) else {}

        # 获取 quota 除数（默认 500000，New API 的标准）
        quota_divisor = self.config.get("quota_divisor", 500000)

        # 提取原始值
        # 注意：New API 中 quota 是剩余额度（total_available），不是总额度
        raw_quota = self._to_float(user_data.get("quota"))
        raw_used = self._to_float(user_data.get("used_quota"))

        # 转换为美元
        total_available = raw_quota / quota_divisor if raw_quota is not None else None
        total_used = raw_used / quota_divisor if raw_used is not None else None

        return self._create_balance_info(
            total_available=total_available,
            total_used=total_used,
            currency=self.config.get("currency", "USD"),
        )

    async def _do_checkin(self, client: httpx.AsyncClient) -> dict[str, Any] | None:
        """
        执行签到（静默，不抛出异常）

        New API 签到需要 Cookie 认证。没有 Cookie 时跳过签到。
        失败时仅记录日志，不影响余额查询。

        Returns:
            签到结果字典，包含 success 和 message 字段；
            如果功能未开放返回 None；
            如果 Cookie 失效返回 {"cookie_expired": True}
        """
        site = client.base_url.host or str(client.base_url)
        checkin_endpoint = self.config.get("checkin_endpoint", "/api/user/checkin")

        # 检查是否配置了 Cookie（通过 service 层注入的 _has_cookie 标志）
        has_cookie = self.config.get("_has_cookie", False)
        if not has_cookie:
            logger.debug(f"[{site}] 未配置 Cookie，跳过签到")
            return None

        try:
            response = await client.post(checkin_endpoint)

            # 404 表示签到功能未开放
            if response.status_code == 404:
                logger.debug(f"[{site}] 签到功能未开放")
                return None

            # 401/403 表示 Cookie 已失效
            if response.status_code in (401, 403):
                logger.warning(f"[{site}] Cookie 已失效（签到返回 {response.status_code}）")
                return {"cookie_expired": True, "message": "Cookie 已失效"}

            try:
                data = response.json()
                message = data.get("message", "")
                success = data.get("success", False)

                if success:
                    logger.debug(f"[{site}] 签到成功: {message}")
                    return {"success": True, "message": message or "签到成功"}
                else:
                    # 检查是否是"已签到"的情况
                    already_indicators = ["already", "已签到", "今日已签", "重复签到"]
                    is_already = any(ind in message.lower() for ind in already_indicators)
                    if is_already:
                        logger.debug(f"[{site}] 今日已签到: {message}")
                        return {"success": None, "message": message or "今日已签到"}

                    # 检查是否是认证失败（未登录、无权限等）- Cookie 已失效
                    auth_fail_indicators = [
                        "未登录", "请登录", "login", "unauthorized", "无权限", "权限不足",
                        "turnstile", "captcha", "验证码",  # 需要人机验证
                    ]
                    is_auth_fail = any(ind in message.lower() for ind in auth_fail_indicators)
                    if is_auth_fail:
                        logger.warning(f"[{site}] Cookie 已失效（签到认证失败）: {message}")
                        return {"cookie_expired": True, "message": message or "Cookie 已失效"}

                    # 其他失败情况
                    logger.debug(f"[{site}] 签到失败: {message}")
                    return {"success": False, "message": message or "签到失败"}
            except Exception as e:
                logger.debug(f"[{site}] 签到响应解析失败: {e}")
                return {"success": False, "message": "响应解析失败"}

        except Exception as e:
            # 签到失败不影响余额查询
            logger.debug(f"[{site}] 签到请求失败（不影响余额查询）: {e}")
            return None

    @classmethod
    def get_config_schema(cls) -> dict[str, Any]:
        """获取操作配置 schema"""
        return {
            "type": "object",
            "properties": {
                "endpoint": {
                    "type": "string",
                    "title": "API 路径",
                    "description": "余额查询 API 路径",
                    "default": "/api/user/self",
                },
                "method": {
                    "type": "string",
                    "title": "请求方法",
                    "enum": ["GET", "POST"],
                    "default": "GET",
                },
                "quota_divisor": {
                    "type": "number",
                    "title": "额度除数",
                    "description": "将原始额度值转换为美元的除数",
                    "default": 500000,
                },
                "currency": {
                    "type": "string",
                    "title": "货币单位",
                    "default": "USD",
                },
            },
            "required": [],
        }
