"""
Cubence 架构

针对 Cubence 中转站的预设配置。
"""

from typing import Any

import httpx

from src.services.provider_ops.actions import ProviderAction
from src.services.provider_ops.actions.cubence_balance import CubenceBalanceAction
from src.services.provider_ops.architectures.base import (
    ProviderArchitecture,
    ProviderConnector,
    VerifyResult,
)
from src.services.provider_ops.types import ConnectorAuthType, ProviderActionType


def _extract_token_from_cookie(cookie_string: str) -> str:
    """
    从完整的 Cookie 字符串中提取 token 值

    支持两种输入格式：
    1. 完整 Cookie: "token=xxx; other=yyy; ..."
    2. 仅 token 值: "eyJhbGciOiJI..."

    Args:
        cookie_string: Cookie 字符串或 token 值

    Returns:
        token cookie 的值
    """
    # 如果包含 "token="，说明是完整 Cookie 字符串
    if "token=" in cookie_string:
        # 解析 Cookie 字符串
        for part in cookie_string.split(";"):
            part = part.strip()
            if part.startswith("token="):
                return part[6:]  # 去掉 "token=" 前缀
    # 否则认为直接是 token 值
    return cookie_string.strip()


class CubenceConnector(ProviderConnector):
    """
    Cubence 专用连接器

    特点：
    - 使用 Cookie 认证（token JWT）
    """

    auth_type = ConnectorAuthType.COOKIE
    display_name = "Cubence Cookie"

    def __init__(self, base_url: str, config: dict[str, Any] | None = None):
        super().__init__(base_url, config)
        self._token_cookie: str | None = None

    async def connect(self, credentials: dict[str, Any]) -> bool:
        """建立连接"""
        token_cookie = credentials.get("token_cookie")
        if not token_cookie:
            self._set_error("Token Cookie 不能为空")
            return False

        # 提取纯 token 值
        self._token_cookie = _extract_token_from_cookie(token_cookie)

        self._set_connected()
        return True

    async def disconnect(self) -> None:
        """断开连接"""
        self._token_cookie = None
        self._set_disconnected()

    async def is_authenticated(self) -> bool:
        """检查是否已认证"""
        return self._token_cookie is not None

    def _apply_auth(self, request: httpx.Request) -> httpx.Request:
        """为请求应用认证信息"""
        if self._token_cookie:
            request.headers["Cookie"] = f"token={self._token_cookie}"

        return request

    @classmethod
    def get_credentials_schema(cls) -> dict[str, Any]:
        """获取凭据配置 schema"""
        return {
            "type": "object",
            "properties": {
                "token_cookie": {
                    "type": "string",
                    "title": "Token Cookie",
                    "description": "从浏览器复制的 token Cookie 值（JWT 格式）",
                },
            },
            "required": ["token_cookie"],
        }


class CubenceArchitecture(ProviderArchitecture):
    """
    Cubence 架构预设

    针对 Cubence 中转站优化的预设配置。

    特点：
    - 使用 Cookie 认证（token JWT）
    - 验证端点: /api/v1/dashboard/overview
    - 余额单位直接是美元
    - 支持窗口限额（5小时/每周）
    """

    architecture_id = "cubence"
    display_name = "Cubence"
    description = "Cubence 中转站预设配置，使用 Cookie 认证"

    supported_connectors: list[type[ProviderConnector]] = [
        CubenceConnector,
    ]

    supported_actions: list[type[ProviderAction]] = [
        CubenceBalanceAction,
    ]

    default_action_configs: dict[ProviderActionType, dict[str, Any]] = {
        ProviderActionType.QUERY_BALANCE: {
            "endpoint": "/api/v1/dashboard/overview",
            "method": "GET",
            "response_mapping": {
                "total_available": "data.balance.total_balance_dollar",
            },
        },
    }

    def get_credentials_schema(self) -> dict[str, Any]:
        """Cubence 使用 token_cookie 认证"""
        return CubenceConnector.get_credentials_schema()

    def get_verify_endpoint(self) -> str:
        """验证端点"""
        return "/api/v1/dashboard/overview"

    def build_verify_headers(
        self,
        config: dict[str, Any],
        credentials: dict[str, Any],
    ) -> dict[str, str]:
        """
        构建 Cubence 的验证请求 Headers

        使用 Cookie 认证，不使用 Authorization。
        """
        headers: dict[str, str] = {}

        # 添加 token Cookie
        cookie_input = credentials.get("token_cookie")
        if cookie_input:
            token_value = _extract_token_from_cookie(cookie_input)
            headers["Cookie"] = f"token={token_value}"

        return headers

    def parse_verify_response(
        self,
        status_code: int,
        data: dict[str, Any],
    ) -> VerifyResult:
        """解析 Cubence 验证响应"""
        if status_code == 401:
            return VerifyResult(success=False, message="Cookie 已失效，请重新配置")
        if status_code == 403:
            return VerifyResult(success=False, message="Cookie 已失效或无权限")
        if status_code != 200:
            return VerifyResult(success=False, message=f"验证失败：HTTP {status_code}")

        # Cubence 响应格式: {"success": true, "data": {...}}
        if not data.get("success"):
            message = data.get("message", "验证失败")
            return VerifyResult(success=False, message=message)

        user_data = data.get("data", {})
        user_info = user_data.get("user", {})
        balance_info = user_data.get("balance", {})
        subscription_limits = user_data.get("subscription_limits", {})

        # 构建 extra 信息，包含窗口限额
        extra: dict[str, Any] = {
            "role": user_info.get("role"),
            "invite_code": user_info.get("invite_code"),
        }

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

        return VerifyResult(
            success=True,
            username=user_info.get("username"),
            display_name=user_info.get("username"),
            quota=balance_info.get("total_balance_dollar"),
            extra=extra,
        )
