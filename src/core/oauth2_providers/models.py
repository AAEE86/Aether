"""
OAuth2 Token 数据模型

定义用于 auth_config 存储的 Token 数据结构。
"""

from __future__ import annotations

from typing import Any, Literal

from pydantic import BaseModel, Field


# OAuth2 认证类型的 Literal 类型定义
OAuth2AuthType = Literal["codex", "claude_code", "gemini_cli", "antigravity"]

# 所有支持的认证类型
AllAuthType = Literal["api_key", "vertex_ai", "codex", "claude_code", "gemini_cli", "antigravity"]


class OAuth2TokenData(BaseModel):
    """
    OAuth2 Token 数据

    存储在 ProviderAPIKey.auth_config 中的 token_data 部分。
    """
    access_token: str = Field(..., description="访问令牌")
    refresh_token: str = Field(..., description="刷新令牌")
    expires_at: float = Field(..., description="过期时间（Unix 时间戳）")
    token_type: str = Field(default="Bearer", description="Token 类型")
    scope: str | None = Field(default=None, description="授权范围")
    obtained_at: float | None = Field(default=None, description="获取时间（Unix 时间戳）")

    def is_expired(self, threshold_seconds: int = 60) -> bool:
        """
        检查 Token 是否过期或即将过期

        Args:
            threshold_seconds: 提前多少秒认为已过期（默认 60 秒）

        Returns:
            如果已过期或即将过期则返回 True
        """
        import time
        return time.time() >= self.expires_at - threshold_seconds


class OAuth2AuthConfig(BaseModel):
    """
    OAuth2 认证配置

    存储在 ProviderAPIKey.auth_config 字段中的完整结构。
    """
    token_data: OAuth2TokenData = Field(..., description="Token 数据")
    custom_base_url: str | None = Field(default=None, description="自定义 API 基础 URL（覆盖默认）")
    extra_config: dict[str, Any] | None = Field(default=None, description="Provider 特定的额外配置")

    class Config:
        # 允许额外字段以保持向后兼容性
        extra = "allow"


class OAuth2AuthorizeRequest(BaseModel):
    """
    OAuth2 授权请求参数

    用于启动 OAuth2 授权流程的 API 请求。
    """
    provider_id: OAuth2AuthType = Field(..., description="Provider ID")
    redirect_uri: str | None = Field(default=None, description="自定义回调 URI")


class OAuth2AuthorizeResponse(BaseModel):
    """
    OAuth2 授权响应

    返回授权 URL 和相关参数。
    """
    authorization_url: str = Field(..., description="授权 URL")
    state: str = Field(..., description="State 参数（用于回调验证）")
    code_verifier: str | None = Field(default=None, description="PKCE code_verifier（需要保存用于回调）")
    provider_id: str = Field(..., description="Provider ID")


class OAuth2CallbackRequest(BaseModel):
    """
    OAuth2 回调请求参数

    处理 OAuth2 Provider 回调时的请求参数。
    """
    code: str = Field(..., description="授权码")
    state: str = Field(..., description="State 参数")
    code_verifier: str | None = Field(default=None, description="PKCE code_verifier")


class OAuth2CallbackResponse(BaseModel):
    """
    OAuth2 回调响应

    返回获取到的 Token 数据，供前端保存到 Key 的 auth_config。
    """
    success: bool = Field(..., description="是否成功")
    error: str | None = Field(default=None, description="错误信息")
    token_data: OAuth2TokenData | None = Field(default=None, description="Token 数据")
    provider_id: str = Field(..., description="Provider ID")


class OAuth2ProviderInfo(BaseModel):
    """
    OAuth2 Provider 信息

    用于前端显示的 Provider 配置信息。
    """
    provider_id: str = Field(..., description="Provider ID")
    display_name: str = Field(..., description="显示名称")
    api_format: str = Field(..., description="对应的 API 格式")
    pkce_required: bool = Field(..., description="是否需要 PKCE")
    device_flow_supported: bool = Field(default=False, description="是否支持设备授权流程")
    callback_mode: str = Field(default="auto", description="回调方式: auto（自动回调）, manual（手动复制 URL）")
    redirect_uri: str | None = Field(default=None, description="固定的回调 URI（手动模式需要展示给用户）")
