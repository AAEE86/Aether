from __future__ import annotations

import json
import time
import uuid
from dataclasses import dataclass, field
from typing import Any, Dict, Optional

from fastapi import HTTPException, Request
from sqlalchemy.orm import Session

from src.core.logger import logger
from src.models.database import ApiKey, ManagementToken, User
from src.utils.request_utils import get_client_ip



@dataclass
class ApiRequestContext:
    """统一的API请求上下文，贯穿Pipeline与格式适配器。"""

    request: Request
    db: Session
    user: Optional[User]
    api_key: Optional[ApiKey]
    request_id: str
    start_time: float
    client_ip: str
    user_agent: str
    original_headers: Dict[str, str]
    query_params: Dict[str, str]
    raw_body: bytes | None = None
    json_body: Optional[Dict[str, Any]] = None
    quota_remaining: Optional[float] = None
    mode: str = "standard"  # standard / proxy
    api_format_hint: Optional[str] = None

    # URL 路径参数（如 Gemini API 的 /v1beta/models/{model}:generateContent）
    path_params: Dict[str, Any] = field(default_factory=dict)

    # Management Token（用于管理 API 认证）
    management_token: Optional[ManagementToken] = None

    # 供适配器扩展的状态存储
    extra: Dict[str, Any] = field(default_factory=dict)
    audit_metadata: Dict[str, Any] = field(default_factory=dict)

    # 高频轮询端点日志抑制标志
    quiet_logging: bool = False

    def ensure_json_body(self) -> Dict[str, Any]:
        """确保请求体已解析为JSON并返回。"""
        if self.json_body is not None:
            return self.json_body

        if not self.raw_body:
            raise HTTPException(status_code=400, detail="请求体不能为空")

        try:
            self.json_body = json.loads(self.raw_body.decode("utf-8"))
        except json.JSONDecodeError as exc:
            logger.warning(f"解析JSON失败: {exc}")
            raise HTTPException(status_code=400, detail="请求体必须是合法的JSON") from exc

        return self.json_body

    def add_audit_metadata(self, **values: Any) -> None:
        """向审计日志附加字段（会自动过滤 None）。"""
        for key, value in values.items():
            if value is not None:
                self.audit_metadata[key] = value

    def extend_audit_metadata(self, data: Dict[str, Any]) -> None:
        """批量附加审计字段。"""
        for key, value in data.items():
            if value is not None:
                self.audit_metadata[key] = value

    @classmethod
    def build(
        cls,
        request: Request,
        db: Session,
        user: Optional[User],
        api_key: Optional[ApiKey],
        raw_body: Optional[bytes] = None,
        mode: str = "standard",
        api_format_hint: Optional[str] = None,
        path_params: Optional[Dict[str, Any]] = None,
    ) -> "ApiRequestContext":
        """创建上下文实例并提前读取必要的元数据。"""
        request_id = getattr(request.state, "request_id", None) or str(uuid.uuid4())[:8]
        setattr(request.state, "request_id", request_id)

        start_time = time.time()
        client_ip = get_client_ip(request)
        user_agent = request.headers.get("user-agent", "unknown")

        context = cls(
            request=request,
            db=db,
            user=user,
            api_key=api_key,
            request_id=request_id,
            start_time=start_time,
            client_ip=client_ip,
            user_agent=user_agent,
            original_headers=dict(request.headers),
            query_params=dict(request.query_params),
            raw_body=raw_body,
            mode=mode,
            api_format_hint=api_format_hint,
            path_params=path_params or {},
        )

        # 便于插件/日志引用
        request.state.request_id = request_id
        if user:
            request.state.user_id = user.id
        if api_key:
            request.state.api_key_id = api_key.id

        return context
