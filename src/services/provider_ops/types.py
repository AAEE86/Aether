"""
Provider 操作模块类型定义
"""

from dataclasses import dataclass, field
from datetime import datetime, timezone
from enum import Enum
from typing import Any, Dict, List, Optional


class ConnectorAuthType(str, Enum):
    """连接器认证类型"""

    API_KEY = "api_key"  # API Key 直接认证
    SESSION_LOGIN = "session_login"  # 用户名密码登录获取 Session
    OAUTH = "oauth"  # OAuth 流程
    COOKIE = "cookie"  # 直接使用 Cookie
    NONE = "none"  # 无需认证


class ProviderActionType(str, Enum):
    """提供商操作类型"""

    QUERY_BALANCE = "query_balance"  # 查询余额
    CHECKIN = "checkin"  # 签到
    CLAIM_QUOTA = "claim_quota"  # 领取额度
    REFRESH_TOKEN = "refresh_token"  # 刷新 Token
    GET_USAGE = "get_usage"  # 获取使用记录
    GET_MODELS = "get_models"  # 获取可用模型列表
    CUSTOM = "custom"  # 自定义操作


class ActionStatus(str, Enum):
    """操作执行状态"""

    SUCCESS = "success"  # 成功
    PENDING = "pending"  # 处理中（异步任务已触发，尚未完成）
    AUTH_FAILED = "auth_failed"  # 认证失败
    AUTH_EXPIRED = "auth_expired"  # 认证过期
    RATE_LIMITED = "rate_limited"  # 频率限制
    NETWORK_ERROR = "network_error"  # 网络错误
    PARSE_ERROR = "parse_error"  # 响应解析错误
    NOT_CONFIGURED = "not_configured"  # 未配置
    NOT_SUPPORTED = "not_supported"  # 不支持
    ALREADY_DONE = "already_done"  # 已完成（如今日已签到）
    UNKNOWN_ERROR = "unknown_error"  # 未知错误


class ConnectorStatus(str, Enum):
    """连接器状态"""

    DISCONNECTED = "disconnected"  # 未连接
    CONNECTING = "connecting"  # 连接中
    CONNECTED = "connected"  # 已连接
    EXPIRED = "expired"  # 已过期
    ERROR = "error"  # 错误


@dataclass
class BalanceInfo:
    """余额信息"""

    total_granted: Optional[float] = None  # 总授予额度
    total_used: Optional[float] = None  # 已使用额度
    total_available: Optional[float] = None  # 可用余额
    expires_at: Optional[datetime] = None  # 过期时间
    currency: str = "USD"  # 货币单位
    extra: Dict[str, Any] = field(default_factory=dict)  # 额外信息


@dataclass
class CheckinInfo:
    """签到信息"""

    reward: Optional[float] = None  # 奖励额度
    streak_days: Optional[int] = None  # 连续签到天数
    next_reward: Optional[float] = None  # 下次奖励
    message: Optional[str] = None  # 签到消息
    extra: Dict[str, Any] = field(default_factory=dict)


@dataclass
class ActionResult:
    """操作执行结果"""

    status: ActionStatus
    action_type: ProviderActionType
    data: Optional[Any] = None  # 操作返回的数据（如 BalanceInfo, CheckinInfo）
    message: Optional[str] = None  # 消息
    executed_at: datetime = field(default_factory=lambda: datetime.now(timezone.utc))
    response_time_ms: Optional[int] = None  # 响应时间（毫秒）
    raw_response: Optional[Dict[str, Any]] = None  # 原始响应（调试用）
    cache_ttl_seconds: int = 300  # 建议缓存时间
    retry_after_seconds: Optional[int] = None  # 失败后重试间隔

    @property
    def is_success(self) -> bool:
        return self.status == ActionStatus.SUCCESS


@dataclass
class ConnectorState:
    """连接器状态信息"""

    status: ConnectorStatus
    auth_type: ConnectorAuthType
    connected_at: Optional[datetime] = None
    expires_at: Optional[datetime] = None
    last_error: Optional[str] = None
    extra: Dict[str, Any] = field(default_factory=dict)


@dataclass
class ProviderOpsConfig:
    """Provider 操作配置（存储在 Provider.config['provider_ops'] 中）"""

    architecture_id: str = "generic_api"
    base_url: Optional[str] = None  # API 基础地址

    # 连接器配置
    connector_auth_type: ConnectorAuthType = ConnectorAuthType.API_KEY
    connector_config: Dict[str, Any] = field(default_factory=dict)
    connector_credentials: Dict[str, Any] = field(default_factory=dict)  # 加密存储

    # 操作配置
    actions: Dict[str, Dict[str, Any]] = field(default_factory=dict)

    # 定时任务配置
    schedule: Dict[str, str] = field(default_factory=dict)  # {action_type: cron_expression}

    @classmethod
    def from_dict(cls, data: Optional[Dict[str, Any]]) -> "ProviderOpsConfig":
        """从字典创建配置"""
        if not data:
            return cls()

        return cls(
            architecture_id=data.get("architecture_id", "generic_api"),
            base_url=data.get("base_url"),
            connector_auth_type=ConnectorAuthType(
                data.get("connector", {}).get("auth_type", "api_key")
            ),
            connector_config=data.get("connector", {}).get("config", {}),
            connector_credentials=data.get("connector", {}).get("credentials", {}),
            actions=data.get("actions", {}),
            schedule=data.get("schedule", {}),
        )

    def to_dict(self) -> Dict[str, Any]:
        """转换为字典（用于存储）"""
        return {
            "architecture_id": self.architecture_id,
            "base_url": self.base_url,
            "connector": {
                "auth_type": self.connector_auth_type.value,
                "config": self.connector_config,
                "credentials": self.connector_credentials,
            },
            "actions": self.actions,
            "schedule": self.schedule,
        }
