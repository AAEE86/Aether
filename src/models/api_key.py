"""
Provider API Key相关的API模型
"""

from datetime import datetime

from pydantic import BaseModel, ConfigDict, Field


class ProviderAPIKeyBase(BaseModel):
    """Provider API Key基础模型"""

    name: str | None = Field(None, description="密钥名称/备注")
    api_key: str = Field(..., description="API密钥")
    rpm_limit: int | None = Field(None, description="RPM限制（每分钟请求数），NULL=自适应模式")
    priority: int = Field(0, description="优先级（越高越优先使用）")
    is_active: bool = Field(True, description="是否启用")
    expires_at: datetime | None = Field(None, description="过期时间")


class ProviderAPIKeyCreate(ProviderAPIKeyBase):
    """创建Provider API Key请求"""

    pass


class ProviderAPIKeyUpdate(BaseModel):
    """更新Provider API Key请求"""

    name: str | None = None
    api_key: str | None = None
    rpm_limit: int | None = None
    priority: int | None = None
    is_active: bool | None = None
    expires_at: datetime | None = None


class ProviderAPIKeyResponse(ProviderAPIKeyBase):
    """Provider API Key响应"""

    id: str
    provider_id: str
    request_count: int | None = Field(0, description="请求次数")
    error_count: int | None = Field(0, description="错误次数")
    last_used_at: datetime | None = Field(None, description="最后使用时间")
    last_error_at: datetime | None = Field(None, description="最后错误时间")
    last_error_msg: str | None = Field(None, description="最后错误信息")
    created_at: datetime
    updated_at: datetime

    model_config = ConfigDict(from_attributes=True)


class ProviderAPIKeyStats(BaseModel):
    """Provider API Key统计信息"""

    id: str
    name: str | None
    request_count: int
    error_count: int
    success_rate: float
    last_used_at: datetime | None
    is_active: bool
    is_expired: bool
