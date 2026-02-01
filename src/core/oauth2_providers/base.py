"""
OAuth2 Provider 基类和配置定义

定义了 OAuth2AuthProvider 抽象基类，所有具体的 Provider 实现都需要继承此类。
"""

from __future__ import annotations

import asyncio
import hashlib
import secrets
import time
import base64
from abc import ABC, abstractmethod
from dataclasses import dataclass, field
from typing import Any

import httpx

from src.core.logger import logger


@dataclass
class OAuth2TokenInfo:
    """
    OAuth2 Token 信息

    存储从 OAuth2 Provider 获取的 Token 及其元数据。
    """
    access_token: str
    refresh_token: str | None
    expires_at: float  # Unix 时间戳
    token_type: str = "Bearer"
    scope: str | None = None
    raw: dict[str, Any] | None = None  # 原始响应数据


@dataclass
class OAuth2ProviderConfig:
    """
    OAuth2 Provider 静态配置

    定义了每个 Provider 的基本参数，包括 OAuth2 端点、客户端凭证、API 配置等。
    """
    # 基本标识
    provider_id: str  # 唯一标识: codex, claude_code, gemini_cli, antigravity
    display_name: str  # 显示名称

    # OAuth2 配置
    token_url: str  # Token 端点
    authorize_url: str  # 授权端点（如果支持授权码流程）
    client_id: str

    # API 配置
    base_url: str  # API 基础 URL
    default_api_path: str  # 默认 API 路径
    api_format: str  # 对应的 API 格式: OPENAI_CLI, CLAUDE_CLI, GEMINI_CLI

    # OAuth2 流程配置
    client_secret: str | None = None  # 部分 Provider 是公开客户端，不需要 secret
    scopes: list[str] = field(default_factory=list)  # OAuth2 scopes
    pkce_required: bool = False  # 是否需要 PKCE
    device_flow_supported: bool = False  # 是否支持设备授权流程

    # 回调配置
    redirect_uri: str | None = None  # 固定的回调 URI（如果有的话）
    callback_mode: str = "auto"  # 回调方式: auto（自动回调）, manual（手动复制 URL）

    # 额外配置
    extra_token_params: dict[str, str] = field(default_factory=dict)  # Token 请求的额外参数
    extra_headers: dict[str, str] = field(default_factory=dict)  # 请求的额外头部


class OAuth2AuthError(Exception):
    """OAuth2 认证错误"""
    pass


class OAuth2AuthProvider(ABC):
    """
    OAuth2 认证 Provider 基类

    每个具体的 Provider (Codex, ClaudeCode, GeminiCli, Antigravity) 都需要继承此类。

    子类必须实现：
    - config: 静态配置属性
    - exchange_refresh_token: 使用 refresh_token 获取新 access_token
    - exchange_authorization_code: 使用授权码获取 token（如果支持授权码流程）

    子类可选覆盖：
    - build_auth_header: 自定义认证头构建
    - get_api_url: 自定义 API URL 构建
    """

    # 子类必须定义
    config: OAuth2ProviderConfig

    # HTTP 客户端超时
    HTTP_TIMEOUT = 30

    # 重试配置（参考 done-hub）
    MAX_RETRIES = 3
    MAX_BACKOFF_SECONDS = 30

    @abstractmethod
    async def exchange_refresh_token(self, refresh_token: str) -> OAuth2TokenInfo:
        """
        使用 refresh_token 获取新的 access_token

        Args:
            refresh_token: 刷新令牌

        Returns:
            新的 Token 信息

        Raises:
            OAuth2AuthError: 刷新失败
        """
        pass

    @abstractmethod
    async def exchange_authorization_code(
        self,
        code: str,
        code_verifier: str | None = None,
        redirect_uri: str | None = None,
    ) -> OAuth2TokenInfo:
        """
        使用授权码交换 Token

        Args:
            code: 授权码
            code_verifier: PKCE code_verifier（如果使用 PKCE）
            redirect_uri: 回调 URI

        Returns:
            Token 信息

        Raises:
            OAuth2AuthError: 交换失败
        """
        pass

    def get_authorization_url(
        self,
        state: str,
        redirect_uri: str,
        code_challenge: str | None = None,
    ) -> str:
        """
        构建授权 URL

        Args:
            state: 随机状态字符串（用于 CSRF 防护）
            redirect_uri: 回调 URI
            code_challenge: PKCE code_challenge（如果使用 PKCE）

        Returns:
            完整的授权 URL
        """
        params = {
            "client_id": self.config.client_id,
            "redirect_uri": redirect_uri,
            "response_type": "code",
            "state": state,
        }

        if self.config.scopes:
            params["scope"] = " ".join(self.config.scopes)

        if code_challenge and self.config.pkce_required:
            params["code_challenge"] = code_challenge
            params["code_challenge_method"] = "S256"

        query = "&".join(f"{k}={httpx.QueryParams({k: v})[k]}" for k, v in params.items())
        return f"{self.config.authorize_url}?{query}"

    def build_auth_header(self, access_token: str) -> tuple[str, str]:
        """
        构建认证请求头

        默认返回 Bearer token 格式。子类可以覆盖此方法以使用不同的认证方式。

        Args:
            access_token: 访问令牌

        Returns:
            (header_name, header_value) 元组
        """
        return ("Authorization", f"Bearer {access_token}")

    def get_api_url(self, model: str | None = None) -> str:
        """
        获取 API URL

        默认返回 base_url + default_api_path。子类可以覆盖以实现动态 URL。

        Args:
            model: 模型名称（部分 API 需要在 URL 中包含模型）

        Returns:
            完整的 API URL
        """
        return f"{self.config.base_url}{self.config.default_api_path}"

    def get_extra_headers(self) -> dict[str, str]:
        """
        获取额外的请求头

        Returns:
            额外的请求头字典
        """
        return self.config.extra_headers.copy()

    async def _make_token_request(
        self,
        data: dict[str, str],
        headers: dict[str, str] | None = None,
        proxy_url: str | None = None,
    ) -> dict[str, Any]:
        """
        发送 Token 请求的通用方法（带重试和代理支持）

        参考 done-hub providers/antigravity/type.go Refresh() 实现。

        Args:
            data: POST 数据
            headers: 额外的请求头
            proxy_url: 代理 URL（可选）

        Returns:
            响应 JSON

        Raises:
            OAuth2AuthError: 请求失败（重试后仍然失败）
        """
        request_headers = {
            "Content-Type": "application/x-www-form-urlencoded",
        }
        if headers:
            request_headers.update(headers)

        last_error: Exception | None = None

        for attempt in range(self.MAX_RETRIES + 1):
            if attempt > 0:
                # 指数退避，最大 MAX_BACKOFF_SECONDS 秒
                backoff = min(2 ** (attempt - 1), self.MAX_BACKOFF_SECONDS)
                logger.info(
                    f"[OAuth2:{self.config.provider_id}] Token request retry {attempt}/{self.MAX_RETRIES} after {backoff}s"
                )
                await asyncio.sleep(backoff)

            try:
                # 配置 HTTP 客户端（支持代理）
                client_kwargs: dict[str, Any] = {"timeout": self.HTTP_TIMEOUT}
                if proxy_url:
                    client_kwargs["proxies"] = {"all://": proxy_url}

                async with httpx.AsyncClient(**client_kwargs) as client:
                    resp = await client.post(
                        self.config.token_url,
                        data=data,
                        headers=request_headers,
                    )

                    # 尝试解析响应
                    try:
                        resp_data = resp.json()
                    except Exception:
                        resp_data = {}

                    # 检查是否是不可重试的错误
                    if "error" in resp_data:
                        error_code = resp_data.get("error", "")
                        if self._is_non_retryable_error(error_code):
                            # 不可重试的错误，直接返回让调用方处理
                            return resp_data

                    # HTTP 错误，但可能可以重试
                    if resp.status_code >= 400:
                        error_body = resp.text[:500] if resp.text else "(empty)"
                        last_error = OAuth2AuthError(
                            f"Token request failed: HTTP {resp.status_code}: {error_body}"
                        )
                        logger.warning(
                            f"[OAuth2:{self.config.provider_id}] Token request failed (attempt {attempt + 1}): "
                            f"HTTP {resp.status_code}"
                        )
                        continue

                    return resp_data

            except httpx.TimeoutException as e:
                last_error = OAuth2AuthError(f"Token request timeout: {e}")
                logger.warning(
                    f"[OAuth2:{self.config.provider_id}] Token request timeout (attempt {attempt + 1})"
                )
            except httpx.RequestError as e:
                last_error = OAuth2AuthError(f"Token request error: {e}")
                logger.warning(
                    f"[OAuth2:{self.config.provider_id}] Token request error (attempt {attempt + 1}): {e}"
                )
            except Exception as e:
                last_error = OAuth2AuthError(f"Token request error: {e}")
                logger.error(f"[OAuth2:{self.config.provider_id}] Token request error: {e}")
                # 未知错误不重试
                break

        # 所有重试都失败
        logger.error(
            f"[OAuth2:{self.config.provider_id}] Token request failed after {self.MAX_RETRIES} retries"
        )
        raise last_error or OAuth2AuthError("Token request failed after retries")

    def _is_non_retryable_error(self, error_code: str) -> bool:
        """
        判断是否是不可重试的 OAuth2 错误

        参考 done-hub providers/antigravity/type.go isNonRetryableError()

        Args:
            error_code: OAuth2 错误码

        Returns:
            True 表示不应该重试
        """
        non_retryable_errors = {
            "invalid_grant",
            "invalid_client",
            "unauthorized_client",
            "access_denied",
            "unsupported_grant_type",
            "invalid_scope",
        }
        return error_code in non_retryable_errors

    @staticmethod
    def generate_code_verifier() -> str:
        """
        生成 PKCE code_verifier

        Returns:
            随机的 code_verifier 字符串
        """
        return secrets.token_urlsafe(64)[:128]

    @staticmethod
    def generate_code_challenge(code_verifier: str) -> str:
        """
        从 code_verifier 生成 code_challenge

        Args:
            code_verifier: code_verifier 字符串

        Returns:
            code_challenge（Base64 URL 编码的 SHA256 哈希）
        """
        digest = hashlib.sha256(code_verifier.encode("utf-8")).digest()
        return base64.urlsafe_b64encode(digest).rstrip(b"=").decode("utf-8")

    @staticmethod
    def generate_state() -> str:
        """
        生成随机 state 字符串

        Returns:
            随机的 state 字符串
        """
        return secrets.token_urlsafe(32)

    def _parse_token_response(
        self,
        data: dict[str, Any],
        original_refresh_token: str | None = None,
    ) -> OAuth2TokenInfo:
        """
        解析 Token 响应

        Args:
            data: 响应 JSON
            original_refresh_token: 原始 refresh_token（如果响应中没有返回）

        Returns:
            OAuth2TokenInfo 实例
        """
        access_token = data.get("access_token")
        if not access_token:
            raise OAuth2AuthError("Token response missing access_token")

        # 计算过期时间
        expires_in = data.get("expires_in", 3600)
        expires_at = time.time() + expires_in

        # refresh_token 可能在响应中，也可能复用原来的
        refresh_token = data.get("refresh_token", original_refresh_token)

        return OAuth2TokenInfo(
            access_token=access_token,
            refresh_token=refresh_token,
            expires_at=expires_at,
            token_type=data.get("token_type", "Bearer"),
            scope=data.get("scope"),
            raw=data,
        )
