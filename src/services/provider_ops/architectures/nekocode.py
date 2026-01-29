"""
NekoCode 架构

针对 NekoCode 中转站的预设配置，使用 Cookie 认证。
"""

from typing import Any

import httpx

from src.core.logger import logger
from src.services.provider_ops.actions import ProviderAction
from src.services.provider_ops.actions.nekocode_balance import NekoCodeBalanceAction
from src.services.provider_ops.architectures.base import (
    ProviderArchitecture,
    ProviderConnector,
    VerifyResult,
)
from src.services.provider_ops.types import ConnectorAuthType, ProviderActionType
from src.utils.ssl_utils import get_ssl_context


def _extract_session_from_cookie(cookie_string: str) -> str:
    """
    从完整的 Cookie 字符串中提取 session 值

    支持两种输入格式：
    1. 完整 Cookie: "session=xxx; other=xxx; ..."
    2. 仅 session 值: "MTc2OTYx..."

    Args:
        cookie_string: Cookie 字符串或 session 值

    Returns:
        session cookie 的值
    """
    # 如果包含 "session="，说明是完整 Cookie 字符串
    if "session=" in cookie_string:
        # 解析 Cookie 字符串
        for part in cookie_string.split(";"):
            part = part.strip()
            if part.startswith("session="):
                return part[8:]  # 去掉 "session=" 前缀
    # 否则认为直接是 session 值
    return cookie_string.strip()


class NekoCodeConnector(ProviderConnector):
    """
    NekoCode 专用连接器

    特点：
    - 使用 Cookie 认证（session）
    """

    auth_type = ConnectorAuthType.COOKIE
    display_name = "NekoCode Cookie"

    def __init__(self, base_url: str, config: dict[str, Any] | None = None):
        super().__init__(base_url, config)
        self._session_cookie: str | None = None

    async def connect(self, credentials: dict[str, Any]) -> bool:
        """建立连接"""
        session_cookie = credentials.get("session_cookie")
        if not session_cookie:
            self._set_error("Session Cookie 不能为空")
            return False

        # 提取纯 session 值（支持完整 Cookie 字符串或仅 session 值）
        self._session_cookie = _extract_session_from_cookie(session_cookie)

        self._set_connected()
        return True

    async def disconnect(self) -> None:
        """断开连接"""
        self._session_cookie = None
        self._set_disconnected()

    async def is_authenticated(self) -> bool:
        """检查是否已认证"""
        return self._session_cookie is not None

    def _apply_auth(self, request: httpx.Request) -> httpx.Request:
        """为请求应用认证信息"""
        if self._session_cookie:
            request.headers["Cookie"] = f"session={self._session_cookie}"

        return request

    @classmethod
    def get_credentials_schema(cls) -> dict[str, Any]:
        """获取凭据配置 schema"""
        return {
            "type": "object",
            "properties": {
                "session_cookie": {
                    "type": "string",
                    "title": "Session Cookie",
                    "description": "从浏览器复制的 session Cookie 值",
                },
            },
            "required": ["session_cookie"],
        }


class NekoCodeArchitecture(ProviderArchitecture):
    """
    NekoCode 架构预设

    针对 NekoCode 中转站优化的预设配置。

    特点：
    - 使用 Cookie 认证（session）
    - 验证端点: /api/usage/summary
    - 显示余额、每日配额、订阅状态
    """

    architecture_id = "nekocode"
    display_name = "NekoCode"
    description = "NekoCode 中转站预设配置，使用 Cookie 认证"

    supported_connectors: list[type[ProviderConnector]] = [
        NekoCodeConnector,
    ]

    supported_actions: list[type[ProviderAction]] = [NekoCodeBalanceAction]

    default_action_configs: dict[ProviderActionType, dict[str, Any]] = {
        ProviderActionType.QUERY_BALANCE: {
            "endpoint": "/api/usage/summary",
            "method": "GET",
        },
    }

    def get_credentials_schema(self) -> dict[str, Any]:
        """NekoCode 使用 session_cookie 认证"""
        return NekoCodeConnector.get_credentials_schema()

    def get_verify_endpoint(self) -> str:
        """验证端点"""
        return "/api/user/self"

    async def prepare_verify_config(
        self,
        base_url: str,
        config: dict[str, Any],
        credentials: dict[str, Any],
    ) -> dict[str, Any]:
        """
        验证前获取 /api/usage/summary 数据（用于显示天卡信息）

        Args:
            base_url: API 基础地址
            config: 连接器配置
            credentials: 凭据信息

        Returns:
            包含 _usage_summary 的配置（会被合并到验证响应中）
        """
        try:
            # 构建请求头
            headers: dict[str, str] = {
                "User-Agent": (
                    "Mozilla/5.0 (Windows NT 10.0; Win64; x64) "
                    "AppleWebKit/537.36 (KHTML, like Gecko) "
                    "Chrome/120.0.0.0 Safari/537.36"
                ),
            }

            # 添加 Cookie
            cookie_input = credentials.get("session_cookie")
            if cookie_input:
                session_value = _extract_session_from_cookie(cookie_input)
                headers["Cookie"] = f"session={session_value}"

            # 构建 client 参数
            client_kwargs: dict[str, Any] = {
                "timeout": 10,
                "verify": get_ssl_context(),
            }
            proxy = config.get("proxy")
            if proxy:
                client_kwargs["proxy"] = proxy

            async with httpx.AsyncClient(**client_kwargs) as client:
                resp = await client.get(
                    f"{base_url.rstrip('/')}/api/usage/summary",
                    headers=headers,
                )

                if resp.status_code == 200:
                    data = resp.json()
                    if data.get("success"):
                        return {"_usage_summary": data.get("data", {})}

        except Exception as e:
            logger.debug(f"获取 NekoCode usage summary 失败: {e}")

        return {}

    def build_verify_headers(
        self,
        config: dict[str, Any],
        credentials: dict[str, Any],
    ) -> dict[str, str]:
        """
        构建 NekoCode 的验证请求 Headers

        使用 Cookie 认证，不使用 Authorization。
        """
        headers: dict[str, str] = {}

        # 添加 session Cookie
        cookie_input = credentials.get("session_cookie")
        if cookie_input:
            # 提取 session 值（支持完整 Cookie 字符串或仅 session 值）
            session_value = _extract_session_from_cookie(cookie_input)
            headers["Cookie"] = f"session={session_value}"

        return headers

    def parse_verify_response(
        self,
        status_code: int,
        data: dict[str, Any],
    ) -> VerifyResult:
        """解析 NekoCode 验证响应（/api/user/self + _usage_summary）"""
        if status_code == 401:
            return VerifyResult(success=False, message="Cookie 已失效，请重新配置")
        if status_code == 403:
            return VerifyResult(success=False, message="Cookie 已失效或无权限")
        if status_code != 200:
            return VerifyResult(success=False, message=f"验证失败：HTTP {status_code}")

        # NekoCode 响应格式: {"success": true, "data": {...}}
        if not data.get("success"):
            message = data.get("message", "验证失败")
            return VerifyResult(success=False, message=message)

        user_data = data.get("data", {})

        # 转换余额字符串为数字
        balance = user_data.get("balance")
        try:
            quota = float(balance) if balance else None
        except (TypeError, ValueError):
            quota = None

        # 从 prepare_verify_config 获取的 _usage_summary 数据（天卡信息）
        extra: dict[str, Any] = {}
        usage_summary = data.get("_usage_summary", {})
        subscription = usage_summary.get("subscription", {})

        if subscription:
            # 转换字符串为数字
            daily_limit = subscription.get("daily_quota_limit")
            daily_remaining = subscription.get("daily_remaining_quota")
            try:
                extra["daily_quota_limit"] = float(daily_limit) if daily_limit else None
            except (TypeError, ValueError):
                pass
            try:
                extra["daily_remaining_quota"] = float(daily_remaining) if daily_remaining else None
            except (TypeError, ValueError):
                pass

            extra["plan_name"] = subscription.get("plan_name")
            extra["subscription_status"] = subscription.get("status")
            extra["effective_start_date"] = subscription.get("effective_start_date")
            extra["effective_end_date"] = subscription.get("effective_end_date")

        return VerifyResult(
            success=True,
            username=user_data.get("username"),
            display_name=user_data.get("display_name") or user_data.get("username"),
            email=user_data.get("email"),
            quota=quota,
            extra=extra if extra else None,
        )
