"""
ClaudeCode OAuth2 Provider

实现 Anthropic Claude Code CLI 的 OAuth2 认证。
参考 done-hub providers/claudecode/ 实现。
"""

from __future__ import annotations

from typing import Any

import httpx

from src.core.logger import logger
from src.core.oauth2_providers.base import (
    OAuth2AuthError,
    OAuth2AuthProvider,
    OAuth2ProviderConfig,
    OAuth2TokenInfo,
)


class ClaudeCodeProvider(OAuth2AuthProvider):
    """
    ClaudeCode (Anthropic CLI) OAuth2 Provider

    使用 Anthropic Console 进行 OAuth2 认证。
    回调方式：手动复制 URL

    注意：Claude Code 的 token 端点要求 JSON 格式请求体，
    而非标准的 application/x-www-form-urlencoded。
    """

    config = OAuth2ProviderConfig(
        provider_id="claude_code",
        display_name="Claude Code",
        token_url="https://console.anthropic.com/v1/oauth/token",
        authorize_url="https://claude.ai/oauth/authorize",  # 授权页面在 claude.ai
        client_id="9d1c250a-e61b-44d9-88ed-5944d1962f5e",
        client_secret=None,  # 公开客户端
        base_url="https://api.anthropic.com",
        default_api_path="/v1/messages",
        api_format="CLAUDE_CLI",
        scopes=["user:inference", "user:profile"],
        pkce_required=False,
        # 固定回调 URI（Anthropic 控制台），用户需要手动复制授权完成后的 URL
        redirect_uri="https://console.anthropic.com/oauth/code/callback",
        callback_mode="manual",  # 手动复制 URL
        extra_headers={
            "User-Agent": "claude-cli/1.0.56 (external, cli)",
            "Accept": "application/json, text/plain, */*",
            "anthropic-version": "2023-06-01",
        },
    )

    async def _make_token_request(
        self,
        data: dict[str, str],
        headers: dict[str, str] | None = None,
    ) -> dict[str, Any]:
        """
        覆盖基类方法：Claude Code token 端点要求 JSON 格式请求体

        参考 done-hub claudecode_oauth.go: exchangeClaudeCodeForToken
        使用 Content-Type: application/json 而非 application/x-www-form-urlencoded
        """
        request_headers = {
            "Content-Type": "application/json",
            "User-Agent": "claude-cli/1.0.56 (external, cli)",
            "Accept": "application/json, text/plain, */*",
            "Accept-Language": "en-US,en;q=0.9",
            "Referer": "https://claude.ai/",
            "Origin": "https://claude.ai",
        }
        if headers:
            request_headers.update(headers)

        try:
            async with httpx.AsyncClient(timeout=self.HTTP_TIMEOUT) as client:
                resp = await client.post(
                    self.config.token_url,
                    json=data,
                    headers=request_headers,
                )
                resp.raise_for_status()
                return resp.json()
        except httpx.HTTPStatusError as e:
            error_body = e.response.text[:500] if e.response.text else "(empty)"
            logger.error(
                f"[OAuth2:{self.config.provider_id}] Token request failed: "
                f"HTTP {e.response.status_code}: {error_body}"
            )
            raise OAuth2AuthError(
                f"Token request failed: HTTP {e.response.status_code}: {error_body}"
            )
        except Exception as e:
            logger.error(f"[OAuth2:{self.config.provider_id}] Token request error: {e}")
            raise OAuth2AuthError(f"Token request error: {e}")

    async def exchange_refresh_token(self, refresh_token: str) -> OAuth2TokenInfo:
        """使用 refresh_token 获取新的 access_token"""
        data = {
            "grant_type": "refresh_token",
            "client_id": self.config.client_id,
            "refresh_token": refresh_token,
        }

        resp_data = await self._make_token_request(data)

        # 检查错误
        if "error" in resp_data:
            error_code = resp_data.get("error", "")
            if error_code in ("invalid_grant", "invalid_client", "unauthorized_client", "access_denied"):
                raise OAuth2AuthError(
                    f"Non-retryable error from ClaudeCode: {error_code}: {resp_data.get('error_description', '')}"
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
        获取 ClaudeCode 特有的额外请求头

        包含 anthropic-version 头部。
        """
        return self.config.extra_headers.copy()
