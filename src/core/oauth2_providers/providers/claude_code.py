"""
ClaudeCode OAuth2 Provider

实现 Anthropic Claude Code CLI 的 OAuth2 认证。
参考 done-hub providers/claudecode/ 实现。
"""

from __future__ import annotations

import asyncio
from typing import Any
from urllib.parse import urlencode

import httpx

from src.core.logger import logger
from src.core.oauth2_providers.base import (
    OAuth2AuthError,
    OAuth2AuthProvider,
    OAuth2ProviderConfig,
    OAuth2TokenInfo,
)


# 不可重试的 OAuth 错误类型
# 参考: done-hub/providers/claudecode/type.go:isNonRetryableError
NON_RETRYABLE_ERRORS = frozenset({
    "invalid_grant",
    "invalid_client",
    "unauthorized_client",
    "access_denied",
    "unsupported_grant_type",
    "invalid_scope",
})

# 默认重试次数
DEFAULT_MAX_RETRIES = 3


class ClaudeCodeProvider(OAuth2AuthProvider):
    """
    ClaudeCode (Anthropic CLI) OAuth2 Provider

    使用 Anthropic Console 进行 OAuth2 认证。
    回调方式：手动复制 URL

    参考 done-hub providers/claudecode/type.go 实现，
    使用 application/x-www-form-urlencoded 格式。
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
            "User-Agent": "claude-cli/1.0.81 (external, cli)",
            "Accept": "application/json, text/plain, */*",
            "anthropic-version": "2023-06-01",
        },
    )

    async def _make_token_request(
        self,
        data: dict[str, str],
        headers: dict[str, str] | None = None,
        max_retries: int = DEFAULT_MAX_RETRIES,
    ) -> dict[str, Any]:
        """
        发送 token 请求，带指数退避重试

        参考 done-hub providers/claudecode/type.go: Refresh()
        使用 Content-Type: application/x-www-form-urlencoded 格式
        """
        request_headers = {
            "Content-Type": "application/x-www-form-urlencoded",
            "User-Agent": "claude-cli/1.0.81 (external, cli)",
            "Accept": "application/json, text/plain, */*",
        }
        if headers:
            request_headers.update(headers)

        last_error: Exception | None = None

        for attempt in range(max_retries + 1):
            # 重试时使用指数退避
            if attempt > 0:
                backoff = min(2 ** (attempt - 1), 30)  # 1s, 2s, 4s, ... 最大 30s
                logger.warning(
                    f"[OAuth2:{self.config.provider_id}] Token refresh retry "
                    f"{attempt}/{max_retries} after {backoff}s"
                )
                await asyncio.sleep(backoff)

            try:
                async with httpx.AsyncClient(timeout=self.HTTP_TIMEOUT) as client:
                    resp = await client.post(
                        self.config.token_url,
                        content=urlencode(data),
                        headers=request_headers,
                    )

                    # 解析响应
                    try:
                        resp_data = resp.json()
                    except Exception:
                        resp_data = {}

                    # 非 200 响应
                    if resp.status_code != 200:
                        error_code = resp_data.get("error", "")
                        error_desc = resp_data.get("error_description", "")

                        # 不可重试的错误，立即抛出
                        if error_code in NON_RETRYABLE_ERRORS:
                            raise OAuth2AuthError(
                                f"Token refresh failed (non-retryable): {error_code} - {error_desc}"
                            )

                        # 可重试的错误，记录后继续
                        last_error = OAuth2AuthError(
                            f"Token request failed: HTTP {resp.status_code}: "
                            f"{error_code} - {error_desc}"
                        )
                        continue

                    return resp_data

            except OAuth2AuthError:
                # 不可重试错误直接抛出
                raise
            except httpx.HTTPStatusError as e:
                error_body = e.response.text[:500] if e.response.text else "(empty)"
                last_error = OAuth2AuthError(
                    f"Token request failed: HTTP {e.response.status_code}: {error_body}"
                )
            except Exception as e:
                last_error = OAuth2AuthError(f"Token request error: {e}")

        # 所有重试都失败
        logger.error(
            f"[OAuth2:{self.config.provider_id}] Token refresh failed after "
            f"{max_retries} retries: {last_error}"
        )
        raise last_error or OAuth2AuthError("Token request failed after retries")

    async def exchange_refresh_token(self, refresh_token: str) -> OAuth2TokenInfo:
        """
        使用 refresh_token 获取新的 access_token

        参考: done-hub/providers/claudecode/type.go: Refresh()
        """
        data = {
            "grant_type": "refresh_token",
            "client_id": self.config.client_id,
            "refresh_token": refresh_token,
        }

        resp_data = await self._make_token_request(data)

        # 检查错误（_make_token_request 已处理不可重试错误，这里做兜底）
        if "error" in resp_data:
            error_code = resp_data.get("error", "")
            error_desc = resp_data.get("error_description", "")
            if error_code in NON_RETRYABLE_ERRORS:
                raise OAuth2AuthError(
                    f"Token refresh failed (non-retryable): {error_code} - {error_desc}"
                )

        return self._parse_token_response(resp_data, original_refresh_token=refresh_token)

    async def exchange_authorization_code(
        self,
        code: str,
        code_verifier: str | None = None,
        redirect_uri: str | None = None,
    ) -> OAuth2TokenInfo:
        """
        使用授权码交换 Token

        参考: done-hub 实现
        """
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
