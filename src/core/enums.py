"""
统一的枚举定义
避免重复定义造成的不一致
"""

from enum import Enum


class APIFormat(Enum):
    """API格式枚举 - 决定请求/响应的处理方式"""

    CLAUDE = "CLAUDE"  # Claude API 格式
    CLAUDE_CLI = "CLAUDE_CLI"  # Claude CLI API 格式（使用 authorization: Bearer）
    OPENAI = "OPENAI"  # OpenAI API 格式
    OPENAI_CLI = "OPENAI_CLI"  # OpenAI CLI/Responses API 格式（用于 Claude Code 等客户端）
    GEMINI = "GEMINI"  # Google Gemini API 格式
    GEMINI_CLI = "GEMINI_CLI"  # Gemini CLI API 格式


class UserRole(Enum):
    """用户角色枚举"""

    ADMIN = "admin"
    USER = "user"


class ProviderBillingType(Enum):
    """提供商计费类型"""

    MONTHLY_QUOTA = "monthly_quota"  # 月卡额度
    PAY_AS_YOU_GO = "pay_as_you_go"  # 按量付费
    FREE_TIER = "free_tier"  # 免费额度


class AuthSource(str, Enum):
    """认证来源枚举"""

    LOCAL = "local"  # 本地认证
    LDAP = "ldap"  # LDAP 认证
