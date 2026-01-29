"""
New API 架构

针对 New API 风格的中转站优化的预设配置。
"""

from typing import Any

import httpx

from src.services.provider_ops.actions import (
    NewApiBalanceAction,
    ProviderAction,
)
from src.services.provider_ops.architectures.base import (
    ProviderArchitecture,
    ProviderConnector,
    VerifyResult,
)
from src.services.provider_ops.types import ConnectorAuthType, ProviderActionType


class NewApiConnector(ProviderConnector):
    """
    New API 专用连接器

    特点：
    - 使用 Bearer Token 认证
    - 需要 New-Api-User Header 传递用户 ID
    """

    auth_type = ConnectorAuthType.API_KEY
    display_name = "New API Key"

    def __init__(self, base_url: str, config: dict[str, Any] | None = None):
        super().__init__(base_url, config)
        self._api_key: str | None = None
        self._user_id: str | None = None
        self._cookie: str | None = None

    async def connect(self, credentials: dict[str, Any]) -> bool:
        """建立连接"""
        api_key = credentials.get("api_key")
        cookie = credentials.get("cookie")
        user_id = credentials.get("user_id")

        # api_key 和 cookie 至少需要一个
        if not api_key and not cookie:
            self._set_error("访问令牌和 Cookie 至少需要填写一个")
            return False

        # 使用 api_key 时必须提供 user_id，使用 cookie 时 user_id 可选
        if api_key and not cookie and not user_id:
            self._set_error("使用访问令牌时，用户 ID 不能为空")
            return False

        self._api_key = api_key
        self._user_id = str(user_id) if user_id else None
        self._cookie = cookie
        self._set_connected()
        return True

    async def disconnect(self) -> None:
        """断开连接"""
        self._api_key = None
        self._user_id = None
        self._cookie = None
        self._set_disconnected()

    async def is_authenticated(self) -> bool:
        """检查是否已认证"""
        # 有 cookie 就行，或者有 api_key + user_id
        if self._cookie:
            return True
        return self._api_key is not None and self._user_id is not None

    def _apply_auth(self, request: httpx.Request) -> httpx.Request:
        """为请求应用认证信息"""
        if self._api_key:
            request.headers["Authorization"] = f"Bearer {self._api_key}"
        if self._user_id:
            request.headers["New-Api-User"] = self._user_id
        if self._cookie:
            request.headers["Cookie"] = self._cookie
        return request

    @classmethod
    def get_credentials_schema(cls) -> dict[str, Any]:
        """获取凭据配置 schema"""
        return {
            "type": "object",
            "properties": {
                "api_key": {
                    "type": "string",
                    "title": "访问令牌 (API Key)",
                    "description": "New API 的访问令牌，与 Cookie 二选一",
                },
                "user_id": {
                    "type": "string",
                    "title": "用户 ID",
                    "description": "使用访问令牌时必填，使用 Cookie 时可选",
                },
                "cookie": {
                    "type": "string",
                    "title": "Cookie",
                    "description": "用于 Cookie 认证，与访问令牌二选一",
                },
            },
            "required": [],
        }


class NewApiArchitecture(ProviderArchitecture):
    """
    New API 架构预设

    针对 New API 风格的中转站优化的预设配置。

    特点：
    - 使用 Bearer Token 认证
    - 需要 New-Api-User Header 传递用户 ID
    - 验证端点: /api/user/self
    - quota 单位通常是 1/500000 美元
    """

    architecture_id = "new_api"
    display_name = "New API"
    description = "New API 风格中转站的预设配置"

    supported_connectors: list[type[ProviderConnector]] = [
        NewApiConnector,
    ]

    supported_actions: list[type[ProviderAction]] = [
        NewApiBalanceAction,
    ]

    default_action_configs: dict[ProviderActionType, dict[str, Any]] = {
        ProviderActionType.QUERY_BALANCE: {
            "endpoint": "/api/user/self",
            "method": "GET",
            "quota_divisor": 500000,  # New API 的 quota 单位是 1/500000 美元
            "checkin_endpoint": "/api/user/checkin",  # 签到端点
        },
    }

    def get_credentials_schema(self) -> dict[str, Any]:
        """New API 需要 api_key 和 user_id"""
        return NewApiConnector.get_credentials_schema()

    def get_verify_endpoint(self) -> str:
        """New API 验证端点"""
        return "/api/user/self"

    def build_verify_headers(
        self,
        config: dict[str, Any],
        credentials: dict[str, Any],
    ) -> dict[str, str]:
        """
        构建 New API 的验证请求 Headers

        New API 特有：需要 New-Api-User Header 传递用户 ID
        """
        headers: dict[str, str] = {}

        # Bearer Token 认证
        api_key = credentials.get("api_key", "")
        if api_key:
            headers["Authorization"] = f"Bearer {api_key}"

        # New API 特有的 header
        user_id = credentials.get("user_id", "")
        if user_id:
            headers["New-Api-User"] = str(user_id)

        # 可选的 Cookie
        cookie = credentials.get("cookie", "")
        if cookie:
            headers["Cookie"] = cookie

        return headers

    def parse_verify_response(
        self,
        status_code: int,
        data: dict[str, Any],
    ) -> VerifyResult:
        """解析 New API 验证响应"""
        if status_code == 401:
            return VerifyResult(success=False, message="认证失败：无效的凭据")
        if status_code == 403:
            return VerifyResult(success=False, message="认证失败：权限不足")
        if status_code != 200:
            return VerifyResult(success=False, message=f"验证失败：HTTP {status_code}")

        # New API 响应格式: {"success": true, "data": {...}}
        if data.get("success") is True and "data" in data:
            user_data = data["data"]
        elif data.get("success") is False:
            message = data.get("message", "验证失败")
            return VerifyResult(success=False, message=message)
        else:
            user_data = data

        return VerifyResult(
            success=True,
            username=user_data.get("username"),
            display_name=user_data.get("display_name") or user_data.get("username"),
            email=user_data.get("email"),
            quota=user_data.get("quota"),
            used_quota=user_data.get("used_quota"),
            request_count=user_data.get("request_count"),
            extra={
                k: v
                for k, v in user_data.items()
                if k
                not in (
                    "username",
                    "display_name",
                    "email",
                    "quota",
                    "used_quota",
                    "request_count",
                )
            },
        )
