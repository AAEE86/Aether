"""
数据库模型扩展 - 新增的提供商策略相关表
"""

import uuid
from datetime import datetime, timezone

from sqlalchemy import (
    Boolean,
    Column,
    DateTime,
    Float,
    ForeignKey,
    Index,
    Integer,
    String,
    UniqueConstraint,
)
from sqlalchemy.orm import relationship

from .database import Base


class ApiKeyProviderMapping(Base):
    """
    API Key 和 Provider 的关联映射表

    用途：管理员为特定的 API Key 指定提供商
    - 如果存在映射：该 API Key 只能使用指定的提供商（无负载均衡和故障转移）
    - 如果不存在映射：该 API Key 使用所有可用提供商（系统默认优先级，有负载均衡和故障转移）

    注意：priority_adjustment 和 weight_multiplier 字段保留但在当前版本不使用
    """

    __tablename__ = "api_key_provider_mappings"

    id = Column(String(36), primary_key=True, default=lambda: str(uuid.uuid4()), index=True)
    api_key_id = Column(
        String(36), ForeignKey("api_keys.id", ondelete="CASCADE"), nullable=False, index=True
    )
    provider_id = Column(
        String(36), ForeignKey("providers.id", ondelete="CASCADE"), nullable=False, index=True
    )

    # 管理员设置的优先级调整（非用户自己设置）
    priority_adjustment = Column(Integer, default=0)  # 优先级调整值（可正可负）
    weight_multiplier = Column(Float, default=1.0)  # 权重乘数（>0）

    # 是否启用
    is_enabled = Column(Boolean, default=True, nullable=False)

    # 时间戳
    created_at = Column(
        DateTime(timezone=True), default=lambda: datetime.now(timezone.utc), nullable=False
    )
    updated_at = Column(
        DateTime(timezone=True),
        default=lambda: datetime.now(timezone.utc),
        onupdate=lambda: datetime.now(timezone.utc),
        nullable=False,
    )

    # 关系
    api_key = relationship("ApiKey", back_populates="provider_mappings")
    provider = relationship("Provider", back_populates="api_key_mappings")

    # 唯一约束
    __table_args__ = (
        UniqueConstraint("api_key_id", "provider_id", name="uq_apikey_provider"),
        Index("idx_apikey_provider_enabled", "api_key_id", "is_enabled"),
    )


class ProviderUsageTracking(Base):
    """提供商使用追踪 (用于RPM限流和健康检测)"""

    __tablename__ = "provider_usage_tracking"

    id = Column(String(36), primary_key=True, default=lambda: str(uuid.uuid4()), index=True)
    provider_id = Column(
        String(36), ForeignKey("providers.id", ondelete="CASCADE"), nullable=False, index=True
    )

    # 时间窗口
    window_start = Column(DateTime(timezone=True), nullable=False, index=True)
    window_end = Column(DateTime(timezone=True), nullable=False)

    # 统计数据
    total_requests = Column(Integer, default=0)
    successful_requests = Column(Integer, default=0)
    failed_requests = Column(Integer, default=0)

    # 性能数据
    avg_response_time_ms = Column(Float, default=0.0)
    total_response_time_ms = Column(Float, default=0.0)  # 用于计算平均值

    # 成本数据
    total_cost_usd = Column(Float, default=0.0)

    # 时间戳
    created_at = Column(
        DateTime(timezone=True), default=lambda: datetime.now(timezone.utc), nullable=False
    )
    updated_at = Column(
        DateTime(timezone=True),
        default=lambda: datetime.now(timezone.utc),
        onupdate=lambda: datetime.now(timezone.utc),
        nullable=False,
    )

    # 关系
    provider = relationship("Provider", back_populates="usage_tracking")

    # 索引
    __table_args__ = (
        Index("idx_provider_window", "provider_id", "window_start"),
        Index("idx_window_time", "window_start", "window_end"),
    )
