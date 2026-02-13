"""
Sub2API 架构

针对 Sub2API 风格的中转站优化的预设配置。
"""

from typing import Any

import httpx

from src.services.provider_ops.actions import ProviderAction
from src.services.provider_ops.actions.sub2api_balance import Sub2ApiBalanceAction
from src.services.provider_ops.architectures.base import (
    ProviderArchitecture,
    ProviderConnector,
    VerifyResult,
)
from src.services.provider_ops.types import ConnectorAuthType, ProviderActionType


class Sub2ApiConnector(ProviderConnector):
    """
    Sub2API 连接器

    使用 Bearer Token 认证。
    """

    auth_type = ConnectorAuthType.API_KEY
    display_name = "Sub2API Key"

    def __init__(self, base_url: str, config: dict[str, Any] | None = None):
        super().__init__(base_url, config)
        self._api_key: str | None = None

    @staticmethod
    def _strip_bearer(value: str) -> str:
        """去掉用户可能粘贴的 Bearer 前缀"""
        stripped = value.strip()
        if stripped.lower().startswith("bearer "):
            stripped = stripped[7:].strip()
        return stripped

    async def connect(self, credentials: dict[str, Any]) -> bool:
        api_key = credentials.get("api_key")
        if not api_key:
            self._set_error("JWT Token 不能为空")
            return False
        self._api_key = self._strip_bearer(api_key)
        self._set_connected()
        return True

    async def disconnect(self) -> None:
        self._api_key = None
        self._set_disconnected()

    async def is_authenticated(self) -> bool:
        return self._api_key is not None

    def _apply_auth(self, request: httpx.Request) -> httpx.Request:
        if self._api_key:
            request.headers["Authorization"] = f"Bearer {self._api_key}"
        return request

    @classmethod
    def get_credentials_schema(cls) -> dict[str, Any]:
        return {
            "type": "object",
            "properties": {
                "base_url": {
                    "type": "string",
                    "title": "站点地址",
                    "description": "API 基础地址",
                },
                "api_key": {
                    "type": "string",
                    "title": "JWT Token",
                    "description": "Sub2API 的访问令牌",
                    "x-sensitive": True,
                    "x-input-type": "password",
                },
            },
            "required": ["api_key"],
            "x-field-groups": [
                {"fields": ["base_url"]},
                {"fields": ["api_key"]},
            ],
            "x-auth-type": "api_key",
            "x-auth-method": "bearer",
            "x-validation": [
                {
                    "type": "required",
                    "fields": ["api_key"],
                    "message": "请填写 JWT Token",
                },
            ],
        }


class Sub2ApiArchitecture(ProviderArchitecture):
    """
    Sub2API 架构

    特点：
    - 使用 Bearer Token 认证
    - 验证端点: /api/v1/auth/me
    - balance 为充值余额，points 为赠送余额
    """

    architecture_id = "sub2api"
    display_name = "Sub2API"
    description = "Sub2API 风格中转站的预设配置"

    supported_connectors: list[type[ProviderConnector]] = [Sub2ApiConnector]

    supported_actions: list[type[ProviderAction]] = [Sub2ApiBalanceAction]

    default_action_configs: dict[ProviderActionType, dict[str, Any]] = {
        ProviderActionType.QUERY_BALANCE: {
            "endpoint": "/api/v1/auth/me?timezone=Asia/Shanghai",
            "method": "GET",
        },
    }

    def get_credentials_schema(self) -> dict[str, Any]:
        return Sub2ApiConnector.get_credentials_schema()

    def get_verify_endpoint(self) -> str:
        return "/api/v1/auth/me?timezone=Asia/Shanghai"

    def build_verify_headers(
        self,
        config: dict[str, Any],
        credentials: dict[str, Any],
    ) -> dict[str, str]:
        headers: dict[str, str] = {}
        api_key = credentials.get("api_key", "")
        if api_key:
            headers["Authorization"] = f"Bearer {Sub2ApiConnector._strip_bearer(api_key)}"
        return headers

    def parse_verify_response(
        self,
        status_code: int,
        data: dict[str, Any],
    ) -> VerifyResult:
        """
        解析 Sub2API 验证响应

        Sub2API 使用 {"code": 0, "message": "success", "data": {...}} 格式。
        """
        if status_code == 401:
            return VerifyResult(success=False, message=self._auth_fail_message(401))
        if status_code == 403:
            return VerifyResult(success=False, message=self._auth_fail_message(403))
        if status_code != 200:
            return VerifyResult(success=False, message=f"验证失败：HTTP {status_code}")

        code = data.get("code")
        if code != 0:
            message = data.get("message", "验证失败")
            return VerifyResult(success=False, message=message)

        user_data = data.get("data", {})
        return self._build_verify_result(user_data, data)

    def _build_verify_result(
        self, user_data: dict[str, Any], raw_data: dict[str, Any] | None = None
    ) -> VerifyResult:
        # or 0 防御上游返回 None / "" 等 falsy 值
        balance = float(user_data.get("balance") or 0)
        points = float(user_data.get("points") or 0)

        return VerifyResult(
            success=True,
            username=user_data.get("username") or user_data.get("email"),
            display_name=user_data.get("username") or user_data.get("email"),
            email=user_data.get("email"),
            quota=balance + points,
            extra={
                "balance": balance,
                "points": points,
                "status": user_data.get("status"),
                "concurrency": user_data.get("concurrency"),
            },
        )
