"""
Codex OAuth2 Provider

实现 OpenAI Codex CLI 的 OAuth2 认证。
参考 done-hub providers/codex/ 及 controller/codex_oauth.go 实现。
"""

from __future__ import annotations

import time

import httpx

from src.core.logger import logger
from src.core.oauth2_providers.base import (
    OAuth2AuthError,
    OAuth2AuthProvider,
    OAuth2ProviderConfig,
    OAuth2TokenInfo,
)


class CodexProvider(OAuth2AuthProvider):
    """
    Codex (OpenAI CLI) OAuth2 Provider

    使用 Auth0 进行 OAuth2 认证，API 为 ChatGPT 后端。
    回调方式：手动复制 URL
    需要 PKCE (S256) 支持。
    """

    config = OAuth2ProviderConfig(
        provider_id="codex",
        display_name="Codex (OpenAI CLI)",
        token_url="https://auth.openai.com/oauth/token",
        authorize_url="https://auth.openai.com/oauth/authorize",
        client_id="app_EMoamEEZ73f0CkXaXp7hrann",
        client_secret=None,  # 公开客户端
        base_url="https://chatgpt.com",
        default_api_path="/backend-api/codex/responses",
        api_format="OPENAI_CLI",
        scopes=["openid", "profile", "email", "offline_access"],
        pkce_required=True,  # done-hub 使用 PKCE S256
        # 固定回调 URI，用户需要手动复制授权完成后的 URL
        redirect_uri="http://localhost:1455/auth/callback",
        callback_mode="manual",  # 手动复制 URL
        extra_headers={
            "User-Agent": "codex_cli_rs/0.38.0 (Ubuntu 22.4.0; x86_64) WindowsTerminal",
            "Accept": "application/json, text/plain, */*",
        },
    )

    def get_authorization_url(
        self,
        state: str,
        redirect_uri: str,
        code_challenge: str | None = None,
    ) -> str:
        """
        构建 Codex OAuth2 授权 URL

        覆盖基类方法以添加 Codex 特有的参数：
        - id_token_add_organizations=true: 在 token 中包含组织信息
        - codex_cli_simplified_flow=true: 简化的 CLI 授权流程

        参考 done-hub controller/codex_oauth.go: StartCodexOAuth
        """
        params = {
            "response_type": "code",
            "client_id": self.config.client_id,
            "redirect_uri": redirect_uri,
            "scope": " ".join(self.config.scopes),
            "state": state,
            "id_token_add_organizations": "true",
            "codex_cli_simplified_flow": "true",
        }

        if code_challenge and self.config.pkce_required:
            params["code_challenge"] = code_challenge
            params["code_challenge_method"] = "S256"

        query = "&".join(f"{k}={httpx.QueryParams({k: v})[k]}" for k, v in params.items())
        return f"{self.config.authorize_url}?{query}"

    async def exchange_refresh_token(self, refresh_token: str) -> OAuth2TokenInfo:
        """使用 refresh_token 获取新的 access_token"""
        data = {
            "grant_type": "refresh_token",
            "client_id": self.config.client_id,
            "refresh_token": refresh_token,
        }

        resp_data = await self._make_token_request(data)

        # 检查不可重试的错误
        if "error" in resp_data:
            error_code = resp_data.get("error", "")
            if error_code in ("invalid_grant", "invalid_client", "unauthorized_client", "access_denied"):
                raise OAuth2AuthError(
                    f"Non-retryable error from Codex: {error_code}: {resp_data.get('error_description', '')}"
                )

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
            "code": code,
        }

        if redirect_uri:
            data["redirect_uri"] = redirect_uri
        if code_verifier:
            data["code_verifier"] = code_verifier

        resp_data = await self._make_token_request(data)
        return self._parse_token_response(resp_data)

    def get_extra_headers(self) -> dict[str, str]:
        """
        获取 Codex 特有的额外请求头

        Codex 可能需要 chatgpt-account-id 头部，
        这个值从 JWT access_token 中提取。
        """
        return self.config.extra_headers.copy()

    @staticmethod
    def extract_account_id(access_token: str) -> str | None:
        """
        从 JWT access_token 中提取 account_id

        Args:
            access_token: JWT access token

        Returns:
            account_id 或 None
        """
        try:
            import base64
            import json

            # JWT 由三部分组成，用点号分隔
            parts = access_token.split(".")
            if len(parts) < 2:
                return None

            # 解码 payload（第二部分）
            payload = parts[1]
            # 添加 padding
            padding = 4 - len(payload) % 4
            if padding != 4:
                payload += "=" * padding

            decoded = base64.urlsafe_b64decode(payload)
            claims = json.loads(decoded)

            return claims.get("https://api.openai.com/auth", {}).get("chatgpt_account_id")
        except Exception:
            return None
