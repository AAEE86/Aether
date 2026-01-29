"""
通用 API 架构

支持各种中转站的可配置架构。

## 添加新认证模板示例

如需添加新的中转站模板（如 MyApi），参考以下步骤：

1. 在 architectures/ 目录创建新文件，如 my_api.py：

    from src.services.provider_ops.architectures.base import ProviderArchitecture
    from src.services.provider_ops.connectors.base import ProviderConnector

    class MyApiConnector(ProviderConnector):
        # 实现自己的连接器
        pass

    class MyApiArchitecture(ProviderArchitecture):
        architecture_id = "my_api"
        display_name = "My API"
        description = "My API 风格中转站"

        supported_connectors = [MyApiConnector]
        supported_actions = [BalanceAction]

        # 如果需要特殊的认证 headers，重写此方法
        def build_verify_headers(self, config, credentials):
            headers = super().build_verify_headers(config, credentials)
            if "custom_field" in credentials:
                headers["X-Custom-Header"] = credentials["custom_field"]
            return headers

2. 在 registry.py 的 _register_builtin_architectures() 中注册：

    from .my_api import MyApiArchitecture
    builtin = [..., MyApiArchitecture]

3. 在前端 auth-templates/ 添加对应的模板定义
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


class GenericApiKeyConnector(ProviderConnector):
    """
    通用 API Key 连接器

    支持多种 API Key 传递方式：
    - Bearer Token (Authorization: Bearer xxx)
    - Custom Header (X-API-Key: xxx)
    """

    auth_type = ConnectorAuthType.API_KEY
    display_name = "API Key"

    def __init__(self, base_url: str, config: dict[str, Any] | None = None):
        super().__init__(base_url, config)
        self._api_key: str | None = None
        # 支持配置认证方式
        self._auth_method = self.config.get("auth_method", "bearer")
        self._header_name = self.config.get("header_name", "Authorization")

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
        if not self._api_key:
            return request

        if self._auth_method == "bearer":
            request.headers["Authorization"] = f"Bearer {self._api_key}"
        elif self._auth_method == "header":
            request.headers[self._header_name] = self._api_key

        return request

    @classmethod
    def get_credentials_schema(cls) -> dict[str, Any]:
        """获取凭据配置 schema"""
        return {
            "type": "object",
            "properties": {
                "api_key": {
                    "type": "string",
                    "title": "API Key",
                    "description": "提供商的 API Key",
                },
            },
            "required": ["api_key"],
        }


class GenericApiArchitecture(ProviderArchitecture):
    """
    通用 API 架构

    适用于各种中转站，支持所有认证方式和操作类型。
    用户可以完全自定义 endpoint 和响应映射。

    这是"自定义"模板对应的后端架构。
    """

    architecture_id = "generic_api"
    display_name = "通用 API"
    description = "可配置的通用 API 架构，适用于各种中转站"

    supported_connectors: list[type[ProviderConnector]] = [
        GenericApiKeyConnector,
    ]

    supported_actions: list[type[ProviderAction]] = [NewApiBalanceAction]

    # 默认操作配置（可被用户配置覆盖）
    default_action_configs: dict[ProviderActionType, dict[str, Any]] = {
        ProviderActionType.QUERY_BALANCE: {
            "endpoint": "/api/user/balance",
            "method": "GET",
        },
        ProviderActionType.CHECKIN: {
            "endpoint": "/api/user/checkin",
            "method": "POST",
        },
    }

    def get_credentials_schema(self) -> dict[str, Any]:
        """通用架构只需要 api_key"""
        return GenericApiKeyConnector.get_credentials_schema()

    def get_verify_endpoint(self) -> str:
        """通用架构验证端点"""
        return "/api/user/self"

    def build_verify_headers(
        self,
        config: dict[str, Any],
        credentials: dict[str, Any],
    ) -> dict[str, str]:
        """构建通用 API 的验证请求 Headers"""
        headers: dict[str, str] = {}

        api_key = credentials.get("api_key", "")
        if api_key:
            auth_method = config.get("auth_method", "bearer")
            if auth_method == "bearer":
                headers["Authorization"] = f"Bearer {api_key}"
            elif auth_method == "header":
                header_name = config.get("header_name", "X-API-Key")
                headers[header_name] = api_key

        return headers

    def parse_verify_response(
        self,
        status_code: int,
        data: dict[str, Any],
    ) -> VerifyResult:
        """解析通用 API 验证响应"""
        if status_code == 401:
            return VerifyResult(success=False, message="认证失败：无效的凭据")
        if status_code == 403:
            return VerifyResult(success=False, message="认证失败：权限不足")
        if status_code != 200:
            return VerifyResult(success=False, message=f"验证失败：HTTP {status_code}")

        # 尝试解析通用响应格式
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
