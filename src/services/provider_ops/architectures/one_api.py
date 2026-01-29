"""
One API 架构

针对 One API 风格的中转站优化的预设配置。
"""

from typing import Any

import httpx

from src.services.provider_ops.actions import NewApiBalanceAction, ProviderAction
from src.services.provider_ops.architectures.base import (
    ProviderArchitecture,
    ProviderConnector,
    VerifyResult,
)
from src.services.provider_ops.types import ConnectorAuthType, ProviderActionType


class OneApiConnector(ProviderConnector):
    """
    One API 专用连接器

    特点：
    - 使用 Bearer Token 认证
    - 不需要额外的 Header
    """

    auth_type = ConnectorAuthType.API_KEY
    display_name = "One API Key"

    def __init__(self, base_url: str, config: dict[str, Any] | None = None):
        super().__init__(base_url, config)
        self._api_key: str | None = None

    async def connect(self, credentials: dict[str, Any]) -> bool:
        """建立连接"""
        api_key = credentials.get("api_key")
        if not api_key:
            self._set_error("API Key 不能为空")
            return False

        self._api_key = api_key
        self._set_connected()
        return True

    async def disconnect(self) -> None:
        """断开连接"""
        self._api_key = None
        self._set_disconnected()

    async def is_authenticated(self) -> bool:
        """检查是否已认证"""
        return self._api_key is not None

    def _apply_auth(self, request: httpx.Request) -> httpx.Request:
        """为请求应用认证信息"""
        if self._api_key:
            request.headers["Authorization"] = f"Bearer {self._api_key}"
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
                    "description": "One API 的访问令牌",
                },
            },
            "required": ["api_key"],
        }


class OneApiArchitecture(ProviderArchitecture):
    """
    One API 架构预设

    针对 One API 风格的中转站优化的预设配置。

    特点：
    - 使用 Bearer Token 认证
    - 验证端点: /api/user/self
    - 不需要额外的 Header
    """

    architecture_id = "one_api"
    display_name = "One API"
    description = "One API 风格中转站的预设配置"

    supported_connectors: list[type[ProviderConnector]] = [
        OneApiConnector,
    ]

    supported_actions: list[type[ProviderAction]] = [
        NewApiBalanceAction,
    ]

    default_action_configs: dict[ProviderActionType, dict[str, Any]] = {
        ProviderActionType.QUERY_BALANCE: {
            "endpoint": "/api/user/self",
            "method": "GET",
            "response_mapping": {
                "total_granted": "data.quota",
                "total_used": "data.used_quota",
            },
        },
    }

    def get_credentials_schema(self) -> dict[str, Any]:
        """One API 只需要 api_key"""
        return OneApiConnector.get_credentials_schema()

    def get_verify_endpoint(self) -> str:
        """One API 验证端点"""
        return "/api/user/self"

    def build_verify_headers(
        self,
        config: dict[str, Any],
        credentials: dict[str, Any],
    ) -> dict[str, str]:
        """构建 One API 的验证请求 Headers"""
        headers: dict[str, str] = {}

        api_key = credentials.get("api_key", "")
        if api_key:
            headers["Authorization"] = f"Bearer {api_key}"

        return headers

    def parse_verify_response(
        self,
        status_code: int,
        data: dict[str, Any],
    ) -> VerifyResult:
        """解析 One API 验证响应"""
        if status_code == 401:
            return VerifyResult(success=False, message="认证失败：无效的凭据")
        if status_code == 403:
            return VerifyResult(success=False, message="认证失败：权限不足")
        if status_code != 200:
            return VerifyResult(success=False, message=f"验证失败：HTTP {status_code}")

        # One API 响应格式: {"success": true, "data": {...}}
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
