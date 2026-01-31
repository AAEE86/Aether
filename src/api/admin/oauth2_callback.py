"""
OAuth2 授权回调 API 端点

提供管理员 OAuth2 授权流程支持：
- 生成授权 URL
- 处理授权回调
- 获取 Provider 信息
"""

from __future__ import annotations

import json
import secrets
import time
from typing import Any

from fastapi import APIRouter, Depends, HTTPException, Query
from fastapi.responses import HTMLResponse
from pydantic import BaseModel, Field

from src.core.cache_service import CacheService
from src.core.logger import logger
from src.core.oauth2_providers import OAuth2ProviderRegistry
from src.core.oauth2_providers.base import OAuth2AuthError, OAuth2AuthProvider
from src.core.oauth2_providers.models import (
    OAuth2AuthorizeResponse,
    OAuth2CallbackResponse,
    OAuth2ProviderInfo,
    OAuth2TokenData,
)
from src.utils.auth_utils import require_admin

router = APIRouter(prefix="/api/admin/oauth2", tags=["Admin - OAuth2 Providers"])


# ==============================================================================
# 常量
# ==============================================================================

# OAuth2 state 缓存前缀和过期时间
OAUTH2_STATE_CACHE_PREFIX = "oauth2_state:"
OAUTH2_STATE_EXPIRE_SECONDS = 30 * 60  # 30 分钟

# OAuth2 结果缓存前缀和过期时间
OAUTH2_RESULT_CACHE_PREFIX = "oauth2_result:"
OAUTH2_RESULT_EXPIRE_SECONDS = 10 * 60  # 10 分钟


# ==============================================================================
# 请求/响应模型
# ==============================================================================


class OAuth2ProvidersResponse(BaseModel):
    """OAuth2 Providers 列表响应"""
    providers: list[OAuth2ProviderInfo] = Field(..., description="可用的 OAuth2 Provider 列表")


class OAuth2ResultRequest(BaseModel):
    """查询 OAuth2 授权结果请求"""
    state: str = Field(..., description="授权 state 参数")


# ==============================================================================
# API 端点
# ==============================================================================


@router.get("/providers", response_model=OAuth2ProvidersResponse)
async def list_oauth2_providers(
    _: Any = Depends(require_admin),
) -> OAuth2ProvidersResponse:
    """
    获取所有可用的 OAuth2 Provider 列表

    返回每个 Provider 的配置信息，包括 ID、显示名称、API 格式等。
    """
    providers = OAuth2ProviderRegistry.get_all_providers()
    provider_infos = [
        OAuth2ProviderInfo(
            provider_id=p.config.provider_id,
            display_name=p.config.display_name,
            api_format=p.config.api_format,
            pkce_required=p.config.pkce_required,
            device_flow_supported=p.config.device_flow_supported,
            callback_mode=p.config.callback_mode,
            redirect_uri=p.config.redirect_uri,
        )
        for p in providers
    ]
    return OAuth2ProvidersResponse(providers=provider_infos)


@router.post("/authorize/{provider_id}", response_model=OAuth2AuthorizeResponse)
async def start_oauth2_authorization(
    provider_id: str,
    _: Any = Depends(require_admin),
) -> OAuth2AuthorizeResponse:
    """
    启动 OAuth2 授权流程

    生成授权 URL 和相关参数，供前端打开授权页面。
    所有 Provider 均使用固定的 redirect_uri。

    Args:
        provider_id: Provider ID (codex, claude_code, gemini_cli, antigravity)

    Returns:
        授权 URL 和相关参数
    """
    provider = OAuth2ProviderRegistry.get_provider(provider_id)
    if not provider:
        raise HTTPException(status_code=404, detail=f"未知的 OAuth2 Provider: {provider_id}")

    if not provider.config.redirect_uri:
        raise HTTPException(status_code=400, detail=f"Provider {provider_id} 未配置 redirect_uri")

    # 生成 state
    state = secrets.token_urlsafe(32)

    # 使用 Provider 配置的固定 redirect_uri
    redirect_uri = provider.config.redirect_uri

    # 生成 PKCE code_verifier（如果需要）
    code_verifier = None
    code_challenge = None
    if provider.config.pkce_required:
        code_verifier = OAuth2AuthProvider.generate_code_verifier()
        code_challenge = OAuth2AuthProvider.generate_code_challenge(code_verifier)

    # 生成授权 URL
    authorization_url = provider.get_authorization_url(
        state=state,
        redirect_uri=redirect_uri,
        code_challenge=code_challenge,
    )

    # 缓存 state 和相关信息
    cache_data = {
        "provider_id": provider_id,
        "redirect_uri": redirect_uri,
        "code_verifier": code_verifier,
        "callback_mode": provider.config.callback_mode,  # 记录回调方式
        "created_at": time.time(),
    }
    cache_key = f"{OAUTH2_STATE_CACHE_PREFIX}{state}"
    await CacheService.set(cache_key, json.dumps(cache_data), ttl_seconds=OAUTH2_STATE_EXPIRE_SECONDS)

    logger.info(
        f"[OAuth2] Started authorization for provider: {provider_id}, "
        f"mode: {provider.config.callback_mode}, state: {state[:8]}..."
    )

    return OAuth2AuthorizeResponse(
        authorization_url=authorization_url,
        state=state,
        code_verifier=code_verifier,
        provider_id=provider_id,
    )


@router.get("/callback/{provider_id}", response_class=HTMLResponse)
async def oauth2_callback(
    provider_id: str,
    code: str = Query(default="", description="授权码"),
    state: str = Query(default="", description="State 参数"),
    error: str | None = Query(default=None, description="错误代码"),
    error_description: str | None = Query(default=None, description="错误描述"),
) -> HTMLResponse:
    """
    OAuth2 授权回调端点

    处理 OAuth2 Provider 的回调，交换授权码获取 Token。
    回调结果会被缓存，供前端轮询获取。

    注意：此端点不需要认证，因为是 OAuth2 Provider 回调。
    安全性通过 state 参数验证。
    """
    # 处理错误回调
    if error:
        logger.warning(f"[OAuth2] Callback error for {provider_id}: {error} - {error_description}")
        # 缓存错误结果
        result_data = {
            "success": False,
            "error": error_description or error,
            "provider_id": provider_id,
        }
        result_key = f"{OAUTH2_RESULT_CACHE_PREFIX}{state}"
        await CacheService.set(result_key, json.dumps(result_data), ttl_seconds=OAUTH2_RESULT_EXPIRE_SECONDS)

        # 返回 HTML 页面，通知用户关闭窗口
        return _build_callback_html(success=False, error=error_description or error)

    # 校验必要参数
    if not code:
        logger.warning(f"[OAuth2] Callback missing code for {provider_id}")
        return _build_callback_html(success=False, error="缺少授权码参数")

    if not state:
        logger.warning(f"[OAuth2] Callback missing state for {provider_id}")
        return _build_callback_html(success=False, error="缺少 state 参数")

    # 验证 state
    cache_key = f"{OAUTH2_STATE_CACHE_PREFIX}{state}"
    cached = await CacheService.get(cache_key)
    if not cached:
        logger.warning(f"[OAuth2] Invalid or expired state: {state[:8]}...")
        return _build_callback_html(success=False, error="授权已过期，请重新授权")

    # CacheService.get 已经自动反序列化 JSON，直接使用
    if isinstance(cached, dict):
        state_data = cached
    else:
        try:
            state_data = json.loads(cached)
        except (json.JSONDecodeError, TypeError):
            return _build_callback_html(success=False, error="内部错误：无法解析状态数据")

    # 验证 provider_id 匹配
    if state_data.get("provider_id") != provider_id:
        return _build_callback_html(success=False, error="Provider 不匹配")

    # 获取 Provider
    provider = OAuth2ProviderRegistry.get_provider(provider_id)
    if not provider:
        return _build_callback_html(success=False, error=f"未知的 Provider: {provider_id}")

    # 交换授权码
    try:
        token_info = await provider.exchange_authorization_code(
            code=code,
            code_verifier=state_data.get("code_verifier"),
            redirect_uri=state_data.get("redirect_uri"),
        )
    except OAuth2AuthError as e:
        logger.error(f"[OAuth2] Token exchange failed for {provider_id}: {e}")
        result_data = {
            "success": False,
            "error": str(e),
            "provider_id": provider_id,
        }
        result_key = f"{OAUTH2_RESULT_CACHE_PREFIX}{state}"
        await CacheService.set(result_key, json.dumps(result_data), ttl_seconds=OAUTH2_RESULT_EXPIRE_SECONDS)
        return _build_callback_html(success=False, error=str(e))

    # 缓存成功结果
    result_data = {
        "success": True,
        "provider_id": provider_id,
        "token_data": {
            "access_token": token_info.access_token,
            "refresh_token": token_info.refresh_token,
            "expires_at": token_info.expires_at,
            "token_type": token_info.token_type,
            "scope": token_info.scope,
            "obtained_at": time.time(),
        },
    }
    result_key = f"{OAUTH2_RESULT_CACHE_PREFIX}{state}"
    await CacheService.set(result_key, json.dumps(result_data), ttl_seconds=OAUTH2_RESULT_EXPIRE_SECONDS)

    # 删除 state 缓存
    await CacheService.delete(cache_key)

    logger.info(f"[OAuth2] Authorization successful for provider: {provider_id}")

    return _build_callback_html(success=True, error=None)


@router.post("/result", response_model=OAuth2CallbackResponse)
async def get_oauth2_result(
    body: OAuth2ResultRequest,
    _: Any = Depends(require_admin),
) -> OAuth2CallbackResponse:
    """
    获取 OAuth2 授权结果

    前端在授权窗口打开后轮询此端点获取授权结果。

    Args:
        body: 包含 state 参数的请求体

    Returns:
        授权结果，包含 Token 数据（如果成功）
    """
    result_key = f"{OAUTH2_RESULT_CACHE_PREFIX}{body.state}"
    cached = await CacheService.get(result_key)

    if not cached:
        # 结果尚未到达，返回等待状态
        return OAuth2CallbackResponse(
            success=False,
            error="pending",
            token_data=None,
            provider_id="",
        )

    # CacheService.get 已经自动反序列化 JSON，直接使用
    if isinstance(cached, dict):
        result_data = cached
    else:
        try:
            result_data = json.loads(cached)
        except (json.JSONDecodeError, TypeError):
            return OAuth2CallbackResponse(
                success=False,
                error="内部错误：无法解析结果数据",
                token_data=None,
                provider_id="",
            )

    # 解析 token_data
    token_data = None
    if result_data.get("success") and result_data.get("token_data"):
        token_data = OAuth2TokenData(**result_data["token_data"])

    # 成功获取后删除缓存
    if result_data.get("success"):
        await CacheService.delete(result_key)

    return OAuth2CallbackResponse(
        success=result_data.get("success", False),
        error=result_data.get("error"),
        token_data=token_data,
        provider_id=result_data.get("provider_id", ""),
    )


class OAuth2ManualCallbackRequest(BaseModel):
    """手动复制流程的回调请求"""
    callback_url: str = Field(..., description="用户复制的完整回调 URL")
    state: str = Field(..., description="授权时返回的 state 参数")


@router.post("/manual-callback", response_model=OAuth2CallbackResponse)
async def process_manual_callback(
    body: OAuth2ManualCallbackRequest,
    _: Any = Depends(require_admin),
) -> OAuth2CallbackResponse:
    """
    处理手动复制流程的回调

    对于所有 callback_mode="manual" 的 Provider（ClaudeCode、Codex、Antigravity、GeminiCli），
    前端在用户粘贴回调 URL 后调用此端点提取授权码并交换 Token。

    Args:
        body: 包含 callback_url 和 state 的请求体

    Returns:
        授权结果，包含 Token 数据（如果成功）
    """
    from urllib.parse import parse_qs, urlparse

    # 验证 state
    cache_key = f"{OAUTH2_STATE_CACHE_PREFIX}{body.state}"
    cached = await CacheService.get(cache_key)
    if not cached:
        return OAuth2CallbackResponse(
            success=False,
            error="授权已过期，请重新授权",
            token_data=None,
            provider_id="",
        )

    # CacheService.get 已经自动反序列化 JSON，直接使用
    if isinstance(cached, dict):
        state_data = cached
    else:
        try:
            state_data = json.loads(cached)
        except (json.JSONDecodeError, TypeError):
            return OAuth2CallbackResponse(
                success=False,
                error="内部错误：无法解析状态数据",
                token_data=None,
                provider_id="",
            )

    provider_id = state_data.get("provider_id")
    if not provider_id:
        return OAuth2CallbackResponse(
            success=False,
            error="内部错误：缺少 provider_id",
            token_data=None,
            provider_id="",
        )

    # 解析回调 URL 中的授权码
    try:
        parsed = urlparse(body.callback_url)
        query_params = parse_qs(parsed.query)

        # 检查是否有错误
        if "error" in query_params:
            error = query_params.get("error", ["unknown"])[0]
            error_desc = query_params.get("error_description", [error])[0]
            return OAuth2CallbackResponse(
                success=False,
                error=f"授权失败: {error_desc}",
                token_data=None,
                provider_id=provider_id,
            )

        # 获取授权码
        code_list = query_params.get("code", [])
        if not code_list:
            return OAuth2CallbackResponse(
                success=False,
                error="回调 URL 中未找到授权码 (code 参数)",
                token_data=None,
                provider_id=provider_id,
            )
        code = code_list[0]

    except Exception as e:
        return OAuth2CallbackResponse(
            success=False,
            error=f"解析回调 URL 失败: {str(e)}",
            token_data=None,
            provider_id=provider_id,
        )

    # 获取 Provider
    provider = OAuth2ProviderRegistry.get_provider(provider_id)
    if not provider:
        return OAuth2CallbackResponse(
            success=False,
            error=f"未知的 Provider: {provider_id}",
            token_data=None,
            provider_id=provider_id,
        )

    # 交换授权码
    try:
        token_info = await provider.exchange_authorization_code(
            code=code,
            code_verifier=state_data.get("code_verifier"),
            redirect_uri=state_data.get("redirect_uri"),
        )
    except OAuth2AuthError as e:
        logger.error(f"[OAuth2] Manual callback token exchange failed for {provider_id}: {e}")
        return OAuth2CallbackResponse(
            success=False,
            error=str(e),
            token_data=None,
            provider_id=provider_id,
        )
    except Exception as e:
        logger.error(f"[OAuth2] Manual callback unexpected error for {provider_id}: {e}")
        return OAuth2CallbackResponse(
            success=False,
            error=f"交换授权码失败: {str(e)}",
            token_data=None,
            provider_id=provider_id,
        )

    # 删除 state 缓存
    await CacheService.delete(cache_key)

    logger.info(f"[OAuth2] Manual callback authorization successful for provider: {provider_id}")

    # 构造成功响应
    try:
        return OAuth2CallbackResponse(
            success=True,
            error=None,
            token_data=OAuth2TokenData(
                access_token=token_info.access_token,
                refresh_token=token_info.refresh_token or "",
                expires_at=token_info.expires_at,
                token_type=token_info.token_type,
                scope=token_info.scope,
                obtained_at=time.time(),
            ),
            provider_id=provider_id,
        )
    except Exception as e:
        logger.error(f"[OAuth2] Manual callback response construction failed for {provider_id}: {e}")
        return OAuth2CallbackResponse(
            success=False,
            error=f"构造响应失败: {str(e)}",
            token_data=None,
            provider_id=provider_id,
        )


# ==============================================================================
# 辅助函数
# ==============================================================================


def _build_callback_html(success: bool, error: str | None) -> HTMLResponse:
    """
    构建回调 HTML 响应

    返回一个简单的 HTML 页面，通知用户授权结果并自动关闭窗口。
    """
    if success:
        title = "授权成功"
        message = "授权已完成，您可以关闭此窗口。"
        color = "#22c55e"  # green
    else:
        title = "授权失败"
        message = f"授权失败：{error or '未知错误'}"
        color = "#ef4444"  # red

    html = f"""
    <!DOCTYPE html>
    <html>
    <head>
        <meta charset="UTF-8">
        <title>{title}</title>
        <style>
            body {{
                font-family: -apple-system, BlinkMacSystemFont, 'Segoe UI', Roboto, sans-serif;
                display: flex;
                justify-content: center;
                align-items: center;
                height: 100vh;
                margin: 0;
                background: #f9fafb;
            }}
            .container {{
                text-align: center;
                padding: 40px;
                background: white;
                border-radius: 12px;
                box-shadow: 0 4px 6px rgba(0,0,0,0.1);
            }}
            .icon {{
                font-size: 48px;
                margin-bottom: 16px;
            }}
            .title {{
                color: {color};
                margin-bottom: 8px;
            }}
            .message {{
                color: #6b7280;
            }}
        </style>
    </head>
    <body>
        <div class="container">
            <div class="icon">{'✓' if success else '✗'}</div>
            <h2 class="title">{title}</h2>
            <p class="message">{message}</p>
        </div>
        <script>
            // 通知父窗口授权完成
            if (window.opener) {{
                window.opener.postMessage({{
                    type: 'oauth2_callback',
                    success: {str(success).lower()},
                    error: {json.dumps(error)}
                }}, '*');
            }}
            // 3 秒后自动关闭
            setTimeout(function() {{
                window.close();
            }}, 3000);
        </script>
    </body>
    </html>
    """

    return HTMLResponse(content=html)
