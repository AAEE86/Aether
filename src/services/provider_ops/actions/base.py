"""
Provider 操作抽象基类
"""

from abc import ABC, abstractmethod
from typing import Any

import httpx

from src.services.provider_ops.types import (
    ActionResult,
    ActionStatus,
    ProviderActionType,
)


class ProviderAction(ABC):
    """
    提供商操作基类

    定义具体的操作逻辑（如查询余额、签到等）。
    """

    # 子类需要定义的类属性
    action_type: ProviderActionType = ProviderActionType.CUSTOM
    display_name: str = "Base Action"
    description: str = ""

    # 默认缓存时间（秒）
    default_cache_ttl: int = 300

    def __init__(self, config: dict[str, Any] | None = None):
        """
        初始化操作

        Args:
            config: 操作配置
        """
        self.config = config or {}

    @abstractmethod
    async def execute(self, client: httpx.AsyncClient) -> ActionResult:
        """
        执行操作

        Args:
            client: 已认证的 HTTP 客户端

        Returns:
            操作结果
        """
        pass

    def _extract_field(self, data: Any, path: str | None) -> Any:
        """
        从响应数据中提取字段

        支持点号分隔的路径，如 "data.user.balance"

        Args:
            data: 响应数据
            path: 字段路径

        Returns:
            提取的值，如果路径无效则返回 None
        """
        if not path:
            return None

        current = data
        for key in path.split("."):
            if isinstance(current, dict):
                current = current.get(key)
            elif isinstance(current, list) and key.isdigit():
                index = int(key)
                current = current[index] if 0 <= index < len(current) else None
            else:
                return None

            if current is None:
                return None

        return current

    def _make_success_result(
        self,
        data: Any = None,
        message: str | None = None,
        response_time_ms: int | None = None,
        raw_response: dict[str, Any] | None = None,
    ) -> ActionResult:
        """创建成功结果"""
        return ActionResult(
            status=ActionStatus.SUCCESS,
            action_type=self.action_type,
            data=data,
            message=message,
            response_time_ms=response_time_ms,
            raw_response=raw_response,
            cache_ttl_seconds=self.default_cache_ttl,
        )

    def _make_error_result(
        self,
        status: ActionStatus,
        message: str | None = None,
        retry_after_seconds: int | None = None,
        raw_response: dict[str, Any] | None = None,
    ) -> ActionResult:
        """创建错误结果"""
        return ActionResult(
            status=status,
            action_type=self.action_type,
            message=message,
            retry_after_seconds=retry_after_seconds,
            raw_response=raw_response,
            cache_ttl_seconds=0,  # 错误不缓存
        )

    def _handle_http_error(
        self, response: httpx.Response, raw_data: dict[str, Any] | None = None
    ) -> ActionResult:
        """处理 HTTP 错误响应"""
        status_code = response.status_code

        if status_code == 401:
            return self._make_error_result(
                ActionStatus.AUTH_FAILED, "认证失败", raw_response=raw_data
            )
        elif status_code == 403:
            return self._make_error_result(
                ActionStatus.AUTH_FAILED, "无权限访问", raw_response=raw_data
            )
        elif status_code == 404:
            # 404 表示接口不存在，通常意味着该功能未开放
            return self._make_error_result(
                ActionStatus.NOT_SUPPORTED, "功能未开放", raw_response=raw_data
            )
        elif status_code == 429:
            retry_after = response.headers.get("Retry-After")
            return self._make_error_result(
                ActionStatus.RATE_LIMITED,
                "请求频率限制",
                retry_after_seconds=int(retry_after) if retry_after else 60,
                raw_response=raw_data,
            )
        else:
            return self._make_error_result(
                ActionStatus.UNKNOWN_ERROR,
                f"HTTP {status_code}: {response.reason_phrase}",
                raw_response=raw_data,
            )

    @classmethod
    def get_config_schema(cls) -> dict[str, Any]:
        """
        获取操作配置 JSON Schema（用于前端表单生成）

        子类应重写此方法
        """
        return {"type": "object", "properties": {}, "required": []}
