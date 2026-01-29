"""
Provider 架构抽象基类
"""

from abc import ABC, abstractmethod
from contextlib import asynccontextmanager
from dataclasses import dataclass
from datetime import datetime, timezone
from typing import Any
from collections.abc import AsyncIterator

import httpx

from src.services.provider_ops.actions.base import ProviderAction
from src.utils.ssl_utils import get_ssl_context
from src.services.provider_ops.types import (
    ConnectorAuthType,
    ConnectorState,
    ConnectorStatus,
    ProviderActionType,
)


# ==================== 连接器基类 ====================


class ProviderConnector(ABC):
    """
    提供商连接器基类

    负责建立与提供商的认证连接，管理凭据状态。
    每个架构应在自己的文件中实现对应的连接器子类。
    """

    # 子类需要定义的类属性
    auth_type: ConnectorAuthType = ConnectorAuthType.NONE
    display_name: str = "Base Connector"

    def __init__(self, base_url: str, config: dict[str, Any] | None = None):
        """
        初始化连接器

        Args:
            base_url: 提供商 API 基础 URL
            config: 连接器配置
        """
        self.base_url = base_url.rstrip("/")
        self.config = config or {}
        self._status = ConnectorStatus.DISCONNECTED
        self._connected_at: datetime | None = None
        self._expires_at: datetime | None = None
        self._last_error: str | None = None

        # 代理配置
        self._proxy: str | None = self.config.get("proxy")

        # HTTP 客户端配置
        self._timeout = self.config.get("timeout", 30)
        self._headers: dict[str, str] = {}

    @abstractmethod
    async def connect(self, credentials: dict[str, Any]) -> bool:
        """
        建立认证连接

        Args:
            credentials: 凭据信息（如用户名密码、API Key 等）

        Returns:
            是否连接成功
        """
        pass

    @abstractmethod
    async def disconnect(self) -> None:
        """断开连接，清理状态"""
        pass

    @abstractmethod
    async def is_authenticated(self) -> bool:
        """检查当前是否已认证"""
        pass

    @abstractmethod
    def _apply_auth(self, request: httpx.Request) -> httpx.Request:
        """
        为请求应用认证信息

        Args:
            request: 原始请求

        Returns:
            添加认证信息后的请求
        """
        pass

    async def refresh_auth(self, credentials: dict[str, Any]) -> bool:
        """
        刷新认证（如 Token 过期）

        默认实现：重新连接

        Args:
            credentials: 凭据信息

        Returns:
            是否刷新成功
        """
        return await self.connect(credentials)

    @asynccontextmanager
    async def get_client(self) -> AsyncIterator[httpx.AsyncClient]:
        """
        获取已认证的 HTTP 客户端

        使用 context manager 确保资源正确释放

        Yields:
            已配置认证信息的 AsyncClient
        """
        transport = None
        if self._proxy:
            transport = httpx.AsyncHTTPTransport(proxy=self._proxy)

        async with httpx.AsyncClient(
            base_url=self.base_url,
            timeout=self._timeout,
            transport=transport,
            event_hooks={"request": [self._auth_hook]},
            verify=get_ssl_context(),
        ) as client:
            yield client

    async def _auth_hook(self, request: httpx.Request) -> None:
        """请求钩子：应用认证信息"""
        self._apply_auth(request)

    def get_state(self) -> ConnectorState:
        """获取连接器当前状态"""
        return ConnectorState(
            status=self._status,
            auth_type=self.auth_type,
            connected_at=self._connected_at,
            expires_at=self._expires_at,
            last_error=self._last_error,
        )

    def _set_connected(self, expires_at: datetime | None = None) -> None:
        """设置为已连接状态"""
        self._status = ConnectorStatus.CONNECTED
        self._connected_at = datetime.now(timezone.utc)
        self._expires_at = expires_at
        self._last_error = None

    def _set_error(self, error: str) -> None:
        """设置错误状态"""
        self._status = ConnectorStatus.ERROR
        self._last_error = error

    def _set_disconnected(self) -> None:
        """设置为断开状态"""
        self._status = ConnectorStatus.DISCONNECTED
        self._connected_at = None
        self._expires_at = None

    @classmethod
    def get_credentials_schema(cls) -> dict[str, Any]:
        """
        获取凭据配置 JSON Schema（用于前端表单生成）

        子类应重写此方法
        """
        return {"type": "object", "properties": {}, "required": []}


# ==================== 验证结果 ====================


@dataclass
class VerifyResult:
    """认证验证结果"""

    success: bool
    message: str | None = None
    username: str | None = None
    display_name: str | None = None
    email: str | None = None
    quota: float | None = None
    used_quota: float | None = None
    request_count: int | None = None
    extra: dict[str, Any] | None = None

    def to_dict(self) -> dict[str, Any]:
        """转换为字典"""
        if not self.success:
            return {"success": False, "message": self.message}

        return {
            "success": True,
            "data": {
                "username": self.username,
                "display_name": self.display_name or self.username,
                "email": self.email,
                "quota": self.quota,
                "used_quota": self.used_quota,
                "request_count": self.request_count,
                "extra": self.extra or {},
            },
        }


# ==================== 架构基类 ====================


class ProviderArchitecture(ABC):
    """
    提供商架构抽象基类

    架构 = Connector（鉴权方式） + Actions（支持的操作）

    一个架构可以被多个 Provider 复用。
    例如：generic_api 架构可用于各种中转站。

    ## 添加新认证模板的步骤

    1. 在 architectures/ 目录创建新文件
    2. 继承 ProviderArchitecture 和 ProviderConnector
    3. 定义类属性：architecture_id, display_name, description
    4. 实现连接器子类和架构类
    5. 实现认证相关的抽象方法：
       - get_credentials_schema(): 返回凭据字段定义
       - get_verify_endpoint(): 返回验证端点
       - build_verify_headers(): 构建验证请求 headers
       - parse_verify_response(): 解析验证响应
    6. 在 registry.py 的 _register_builtin_architectures() 中注册
    """

    # 子类需要定义的类属性
    architecture_id: str = ""
    display_name: str = ""
    description: str = ""

    # 支持的 Connector 类型列表（按优先级排序）
    supported_connectors: list[type[ProviderConnector]] = []

    # 支持的 Action 类型列表
    supported_actions: list[type[ProviderAction]] = []

    # 默认操作配置
    default_action_configs: dict[ProviderActionType, dict[str, Any]] = {}

    def __init__(self, config: dict[str, Any] | None = None):
        """
        初始化架构

        Args:
            config: 架构配置
        """
        self.config = config or {}

    # ==================== 认证验证相关方法（子类必须实现） ====================

    @abstractmethod
    def get_credentials_schema(self) -> dict[str, Any]:
        """
        获取凭据字段定义（JSON Schema 格式）

        子类必须实现此方法定义需要的凭据字段。
        这个 schema 可用于：
        1. 前端表单生成（如果需要动态渲染）
        2. 凭据验证
        3. 文档生成

        Returns:
            JSON Schema 格式的字段定义

        Example:
            {
                "type": "object",
                "properties": {
                    "api_key": {
                        "type": "string",
                        "title": "API Key",
                        "description": "访问令牌",
                    },
                },
                "required": ["api_key"],
            }
        """
        pass

    @abstractmethod
    def get_verify_endpoint(self) -> str:
        """
        获取认证验证端点

        子类必须实现此方法返回验证端点。

        Returns:
            验证端点路径（如 /api/user/self, /api/v1/auth/profile）
        """
        pass

    @abstractmethod
    def build_verify_headers(
        self,
        config: dict[str, Any],
        credentials: dict[str, Any],
    ) -> dict[str, str]:
        """
        构建认证验证请求的 Headers

        子类必须实现此方法构建认证 Headers。

        Args:
            config: 连接器配置（可能包含 prepare_verify_config 返回的额外配置）
            credentials: 凭据信息

        Returns:
            Headers 字典
        """
        pass

    @abstractmethod
    def parse_verify_response(
        self,
        status_code: int,
        data: dict[str, Any],
    ) -> VerifyResult:
        """
        解析认证验证响应

        子类必须实现此方法解析响应。

        Args:
            status_code: HTTP 状态码
            data: 响应 JSON 数据

        Returns:
            验证结果
        """
        pass

    # ==================== 可选的钩子方法 ====================

    async def prepare_verify_config(
        self,
        base_url: str,
        config: dict[str, Any],
        credentials: dict[str, Any],
    ) -> dict[str, Any]:
        """
        验证前的异步预处理（可选）

        子类可重写以执行异步操作（如获取动态 Cookie）。
        返回的配置会传递给 build_verify_headers。

        Args:
            base_url: API 基础地址
            config: 连接器配置
            credentials: 凭据信息

        Returns:
            处理后的配置（会与原 config 合并）
        """
        return {}

    # ==================== 连接器和操作相关方法 ====================

    def get_connector(
        self,
        base_url: str,
        auth_type: ConnectorAuthType | None = None,
        config: dict[str, Any] | None = None,
    ) -> ProviderConnector:
        """
        获取连接器实例

        Args:
            base_url: 提供商 API 基础 URL
            auth_type: 指定的认证类型，None 则使用默认
            config: 连接器配置

        Returns:
            连接器实例

        Raises:
            ValueError: 不支持的认证类型
        """
        if not self.supported_connectors:
            raise ValueError(f"架构 {self.architecture_id} 未配置支持的连接器")

        # 查找匹配的连接器
        connector_cls: type[ProviderConnector] | None = None

        if auth_type:
            for cls in self.supported_connectors:
                if cls.auth_type == auth_type:
                    connector_cls = cls
                    break

            if not connector_cls:
                supported = [c.auth_type.value for c in self.supported_connectors]
                raise ValueError(
                    f"架构 {self.architecture_id} 不支持 {auth_type.value} 认证，"
                    f"支持的类型: {supported}"
                )
        else:
            # 使用第一个（默认）连接器
            connector_cls = self.supported_connectors[0]

        return connector_cls(base_url, config)

    def get_action(
        self,
        action_type: ProviderActionType,
        config: dict[str, Any] | None = None,
    ) -> ProviderAction:
        """
        获取操作实例

        Args:
            action_type: 操作类型
            config: 操作配置（会与默认配置合并）

        Returns:
            操作实例

        Raises:
            ValueError: 不支持的操作类型
        """
        action_cls: type[ProviderAction] | None = None

        for cls in self.supported_actions:
            if cls.action_type == action_type:
                action_cls = cls
                break

        if not action_cls:
            supported = [a.action_type.value for a in self.supported_actions]
            raise ValueError(
                f"架构 {self.architecture_id} 不支持 {action_type.value} 操作，"
                f"支持的操作: {supported}"
            )

        # 合并默认配置和用户配置
        merged_config = dict(self.default_action_configs.get(action_type, {}))
        if config:
            merged_config.update(config)

        return action_cls(merged_config)

    def supports_action(self, action_type: ProviderActionType) -> bool:
        """检查是否支持指定操作"""
        return any(a.action_type == action_type for a in self.supported_actions)

    def supports_auth_type(self, auth_type: ConnectorAuthType) -> bool:
        """检查是否支持指定认证类型"""
        return any(c.auth_type == auth_type for c in self.supported_connectors)

    def get_supported_auth_types(self) -> list[ConnectorAuthType]:
        """获取支持的认证类型列表"""
        return [c.auth_type for c in self.supported_connectors]

    def get_supported_action_types(self) -> list[ProviderActionType]:
        """获取支持的操作类型列表"""
        return [a.action_type for a in self.supported_actions]

    def to_dict(self) -> dict[str, Any]:
        """转换为字典（用于 API 响应）"""
        return {
            "architecture_id": self.architecture_id,
            "display_name": self.display_name,
            "description": self.description,
            "credentials_schema": self.get_credentials_schema(),
            "verify_endpoint": self.get_verify_endpoint(),
            "supported_auth_types": [
                {"type": c.auth_type.value, "display_name": c.display_name}
                for c in self.supported_connectors
            ],
            "supported_actions": [
                {
                    "type": a.action_type.value,
                    "display_name": a.display_name,
                    "description": a.description,
                    "config_schema": a.get_config_schema(),
                }
                for a in self.supported_actions
            ],
            "default_connector": (
                self.supported_connectors[0].auth_type.value
                if self.supported_connectors
                else None
            ),
        }
