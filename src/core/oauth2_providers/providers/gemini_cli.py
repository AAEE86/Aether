"""
GeminiCli OAuth2 Provider

实现 Google Gemini CLI 的 OAuth2 认证。
参考 done-hub providers/geminicli/ 实现。
"""

from __future__ import annotations

from src.core.logger import logger
from src.core.oauth2_providers.base import (
    OAuth2AuthError,
    OAuth2AuthProvider,
    OAuth2ProviderConfig,
    OAuth2TokenInfo,
)


class GeminiCliProvider(OAuth2AuthProvider):
    """
    GeminiCli (Google Cloud) OAuth2 Provider

    使用 Google OAuth2 进行认证，API 为 Google Cloud Code 内部端点。
    回调方式：自动处理
    """

    config = OAuth2ProviderConfig(
        provider_id="gemini_cli",
        display_name="Gemini CLI",
        token_url="https://oauth2.googleapis.com/token",
        authorize_url="https://accounts.google.com/o/oauth2/auth",
        client_id="681255809395-oo8ft2oprdrnp9e3aqf6av3hmdib135j.apps.googleusercontent.com",
        client_secret="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",
        base_url="https://cloudcode-pa.googleapis.com",
        default_api_path="/v1internal/chat/completions",
        api_format="GEMINI_CLI",
        scopes=[
            "https://www.googleapis.com/auth/cloud-platform",
            "https://www.googleapis.com/auth/userinfo.email",
            "https://www.googleapis.com/auth/userinfo.profile",
        ],
        pkce_required=False,
        # 固定回调 URI（与 done-hub 一致）
        redirect_uri="http://localhost:8080/api/geminicli/oauth/callback",
        callback_mode="manual",  # 用户需要手动复制授权完成后浏览器地址栏的完整 URL 并粘贴
    )

    async def exchange_refresh_token(self, refresh_token: str) -> OAuth2TokenInfo:
        """使用 refresh_token 获取新的 access_token"""
        data = {
            "grant_type": "refresh_token",
            "client_id": self.config.client_id,
            "client_secret": self.config.client_secret,
            "refresh_token": refresh_token,
        }

        resp_data = await self._make_token_request(data)

        # 检查错误
        if "error" in resp_data:
            error_code = resp_data.get("error", "")
            error_desc = resp_data.get("error_description", "")
            if error_code in ("invalid_grant", "invalid_client", "unauthorized_client", "access_denied"):
                raise OAuth2AuthError(
                    f"Non-retryable error from GeminiCli: {error_code}: {error_desc}"
                )
            raise OAuth2AuthError(f"GeminiCli token refresh failed: {error_code}: {error_desc}")

        return self._parse_token_response(resp_data, original_refresh_token=refresh_token)

    async def exchange_authorization_code(
        self,
        code: str,
        code_verifier: str | None = None,
        redirect_uri: str | None = None,
    ) -> OAuth2TokenInfo:
        """使用授权码交换 Token"""
        data = {
            "grant_type": "authorization_code",
            "client_id": self.config.client_id,
            "client_secret": self.config.client_secret,
            "code": code,
        }

        if redirect_uri:
            data["redirect_uri"] = redirect_uri
        if code_verifier:
            data["code_verifier"] = code_verifier

        resp_data = await self._make_token_request(data)
        return self._parse_token_response(resp_data)

    def get_api_url(self, model: str | None = None) -> str:
        """
        获取 API URL

        GeminiCli 使用固定的 cloudcode-pa.googleapis.com 端点。
        """
        return f"{self.config.base_url}{self.config.default_api_path}"

    def get_authorization_url(
        self,
        state: str,
        redirect_uri: str,
        code_challenge: str | None = None,
    ) -> str:
        """
        构建 Google OAuth2 授权 URL

        覆盖基类方法以添加 Google 特有的参数：
        - access_type=offline: 获取 refresh_token
        - prompt=consent: 强制显示同意页面
        - include_granted_scopes=true: 包含已授权的 scopes
        """
        import httpx

        params = {
            "client_id": self.config.client_id,
            "redirect_uri": redirect_uri,
            "response_type": "code",
            "state": state,
            "access_type": "offline",
            "prompt": "consent",
            "include_granted_scopes": "true",
        }

        if self.config.scopes:
            params["scope"] = " ".join(self.config.scopes)

        if code_challenge and self.config.pkce_required:
            params["code_challenge"] = code_challenge
            params["code_challenge_method"] = "S256"

        query = "&".join(f"{k}={httpx.QueryParams({k: v})[k]}" for k, v in params.items())
        return f"{self.config.authorize_url}?{query}"
