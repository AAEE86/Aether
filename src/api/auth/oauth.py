"""通用 OAuth 公开认证路由。

支持多个 OAuth 提供商的统一认证流程。
"""

import time
import uuid
from datetime import datetime, timezone
from typing import Any, Dict, Optional
from urllib.parse import urlencode

from fastapi import APIRouter, Depends, HTTPException, Query, Request, status
from fastapi.responses import RedirectResponse
from sqlalchemy.exc import IntegrityError
from sqlalchemy.orm import Session

from src.core.enums import AuthSource
from src.core.logger import logger
from src.database import get_db
from src.models.database import AuditEventType, User, UserRole
from src.services.auth.oauth import OAuthService
from src.services.auth.service import AuthService
from src.clients.redis_client import get_redis_client
from src.services.rate_limit.ip_limiter import IPRateLimiter
from src.services.system.audit import AuditService
from src.services.system.config import SystemConfigService
from src.utils.request_utils import get_client_ip, get_user_agent

router = APIRouter(prefix="/api/auth/oauth", tags=["Authentication - OAuth"])

# State 参数有效期（秒）
STATE_EXPIRE_SECONDS = 600  # 10 分钟


# ========== Helper Functions ==========


async def _store_oauth_state(provider_id: str, state: str) -> bool:
    """存储 OAuth state 到 Redis"""
    try:
        redis = await get_redis_client()
        if redis:
            # 存储 provider_id 以便回调时验证
            await redis.setex(f"oauth_state:{state}", STATE_EXPIRE_SECONDS, provider_id)
            return True
    except Exception as e:
        logger.error(f"存储 OAuth state 失败: {e}")
    return False


async def _verify_and_consume_state(state: str) -> Optional[str]:
    """验证并消费 OAuth state（一次性使用），返回 provider_id"""
    try:
        redis = await get_redis_client()
        if redis:
            provider_id = await redis.get(f"oauth_state:{state}")
            if provider_id:
                await redis.delete(f"oauth_state:{state}")
                return provider_id.decode() if isinstance(provider_id, bytes) else provider_id
    except Exception as e:
        logger.error(f"验证 OAuth state 失败: {e}")
    return None


@router.get("/{provider_id}/authorize")
async def authorize(
    request: Request, provider_id: str, db: Session = Depends(get_db)
):
    """
    发起 OAuth 授权（公开端点）

    重定向用户到 OAuth 提供商授权页面。

    **路径参数**:
    - `provider_id`: OAuth 提供商标识（如 linuxdo, github）

    **权限**: 公开访问，无需认证

    **流程**:
    1. 检查 OAuth 提供商是否启用
    2. 生成 state 参数（防 CSRF）
    3. 重定向到 OAuth 提供商授权页面

    **速率限制**: 10次/分钟/IP
    """
    client_ip = get_client_ip(request)

    # 速率限制
    allowed, remaining, reset_after = await IPRateLimiter.check_limit(
        client_ip, f"oauth_{provider_id}_authorize", limit=10
    )
    if not allowed:
        raise HTTPException(
            status_code=status.HTTP_429_TOO_MANY_REQUESTS,
            detail=f"请求过于频繁，请在 {reset_after} 秒后重试",
        )

    # 获取配置
    config_data = OAuthService.get_provider_config_data(db, provider_id)
    if not config_data:
        raise HTTPException(
            status_code=status.HTTP_503_SERVICE_UNAVAILABLE,
            detail=f"OAuth 提供商 '{provider_id}' 未启用",
        )

    # 生成 state
    state = OAuthService.generate_state()
    stored = await _store_oauth_state(provider_id, state)
    if not stored:
        raise HTTPException(
            status_code=status.HTTP_503_SERVICE_UNAVAILABLE,
            detail="服务暂时不可用，请稍后重试",
        )

    # 生成授权 URL 并重定向
    auth_url = OAuthService.get_authorization_url(config_data, state)
    return RedirectResponse(url=auth_url, status_code=status.HTTP_302_FOUND)


@router.get("/{provider_id}/callback")
async def callback(
    request: Request,
    provider_id: str,
    code: str = Query(..., description="授权码"),
    state: str = Query(..., description="State 参数"),
    db: Session = Depends(get_db),
):
    """
    OAuth 回调处理（公开端点）

    处理 OAuth 提供商授权回调，完成用户登录。

    **路径参数**:
    - `provider_id`: OAuth 提供商标识

    **查询参数**:
    - `code`: 授权码
    - `state`: State 参数（防 CSRF）

    **权限**: 公开访问，无需认证

    **流程**:
    1. 验证 state 参数（防 CSRF）
    2. 用授权码换取访问令牌
    3. 获取 OAuth 用户信息
    4. 创建或更新本地用户
    5. 生成 JWT token
    6. 重定向到前端页面

    **速率限制**: 10次/分钟/IP
    """
    client_ip = get_client_ip(request)
    user_agent = get_user_agent(request)

    # 速率限制
    allowed, remaining, reset_after = await IPRateLimiter.check_limit(
        client_ip, f"oauth_{provider_id}_callback", limit=10
    )
    if not allowed:
        raise HTTPException(
            status_code=status.HTTP_429_TOO_MANY_REQUESTS,
            detail=f"请求过于频繁，请在 {reset_after} 秒后重试",
        )

    # 验证 state
    state_provider_id = await _verify_and_consume_state(state)
    if not state_provider_id or state_provider_id != provider_id:
        logger.warning(
            f"OAuth state 验证失败: state={state}, provider_id={provider_id}, "
            f"expected={state_provider_id}, ip={client_ip}"
        )
        raise HTTPException(
            status_code=status.HTTP_400_BAD_REQUEST,
            detail="无效的请求，请重新登录",
        )

    # 获取配置
    config_data = OAuthService.get_provider_config_data(db, provider_id)
    if not config_data:
        raise HTTPException(
            status_code=status.HTTP_503_SERVICE_UNAVAILABLE,
            detail=f"OAuth 提供商 '{provider_id}' 未启用",
        )

    # 用授权码换取访问令牌
    success, token_data, error = await OAuthService.exchange_code_for_token(
        config_data, code
    )
    if not success:
        logger.error(f"OAuth [{provider_id}] token 交换失败: {error}")
        AuditService.log_login_attempt(
            db=db,
            email=f"[oauth:{provider_id}]",
            success=False,
            ip_address=client_ip,
            user_agent=user_agent,
            error_reason=f"Token 交换失败: {error}",
        )
        db.commit()
        raise HTTPException(
            status_code=status.HTTP_401_UNAUTHORIZED,
            detail="登录失败，请重试",
        )

    access_token = token_data["access_token"]

    # 获取用户信息
    success, user_info, error = await OAuthService.get_user_info(config_data, access_token)
    if not success:
        logger.error(f"OAuth [{provider_id}] 获取用户信息失败: {error}")
        AuditService.log_login_attempt(
            db=db,
            email=f"[oauth:{provider_id}]",
            success=False,
            ip_address=client_ip,
            user_agent=user_agent,
            error_reason=f"获取用户信息失败: {error}",
        )
        db.commit()
        raise HTTPException(
            status_code=status.HTTP_401_UNAUTHORIZED,
            detail="登录失败，请重试",
        )

    # 创建或更新本地用户
    user = await _get_or_create_oauth_user(db, provider_id, config_data, user_info)
    if not user:
        AuditService.log_login_attempt(
            db=db,
            email=user_info.get("email", f"[oauth:{provider_id}]"),
            success=False,
            ip_address=client_ip,
            user_agent=user_agent,
            error_reason="用户创建/更新失败",
        )
        db.commit()
        raise HTTPException(
            status_code=status.HTTP_401_UNAUTHORIZED,
            detail="登录失败，请重试",
        )

    if not user.is_active:
        logger.warning(f"OAuth [{provider_id}] 登录失败 - 用户已禁用: {user.email}")
        AuditService.log_login_attempt(
            db=db,
            email=user.email,
            success=False,
            ip_address=client_ip,
            user_agent=user_agent,
            error_reason="用户已禁用",
        )
        db.commit()
        raise HTTPException(
            status_code=status.HTTP_403_FORBIDDEN,
            detail="账户已被禁用",
        )

    # 记录登录成功
    AuditService.log_login_attempt(
        db=db,
        email=user.email,
        success=True,
        ip_address=client_ip,
        user_agent=user_agent,
        user_id=user.id,
    )
    db.commit()

    # 生成 JWT tokens
    jwt_access_token = AuthService.create_access_token(
        data={
            "user_id": user.id,
            "email": user.email,
            "role": user.role.value,
            "created_at": user.created_at.isoformat() if user.created_at else None,
        }
    )
    jwt_refresh_token = AuthService.create_refresh_token(
        data={"user_id": user.id, "email": user.email}
    )

    logger.info(f"OAuth [{provider_id}] 登录成功: {user.email} (ID: {user.id})")

    # 重定向到前端（使用 fragment 避免 token 泄露到服务器日志）
    frontend_callback_url = config_data.get("frontend_callback_url")
    if frontend_callback_url:
        params = {
            "access_token": jwt_access_token,
            "refresh_token": jwt_refresh_token,
            "token_type": "bearer",
            "expires_in": 86400,
        }
        redirect_url = f"{frontend_callback_url}#{urlencode(params)}"
        return RedirectResponse(url=redirect_url, status_code=status.HTTP_302_FOUND)

    # 如果没有配置前端回调，返回 JSON
    return {
        "access_token": jwt_access_token,
        "refresh_token": jwt_refresh_token,
        "token_type": "bearer",
        "expires_in": 86400,
        "user_id": user.id,
        "email": user.email,
        "username": user.username,
        "role": user.role.value,
    }


async def _get_or_create_oauth_user(
    db: Session,
    provider_id: str,
    config_data: Dict[str, Any],
    oauth_user: Dict[str, Any],
) -> Optional[User]:
    """
    获取或创建 OAuth 用户

    Args:
        db: 数据库会话
        provider_id: OAuth 提供商标识
        config_data: OAuth 配置数据
        oauth_user: OAuth 用户信息（已经过字段映射）

    Returns:
        User 对象，失败返回 None
    """
    # 提取用户信息（使用标准化的字段名）
    oauth_user_id = str(oauth_user.get("id") or oauth_user.get("user_id") or "")
    oauth_username = oauth_user.get("username") or oauth_user.get("name") or ""
    email = oauth_user.get("email") or ""

    if not oauth_user_id:
        logger.error(f"OAuth [{provider_id}] 用户信息缺少 id")
        return None

    # 如果没有邮箱，生成一个占位邮箱
    if not email:
        if oauth_username:
            email = f"{oauth_username}@{provider_id}.oauth.local"
        else:
            email = f"{provider_id}_{oauth_user_id}@oauth.local"

    # 优先按 oauth_provider_id + oauth_user_id 查找
    user = (
        db.query(User)
        .filter(
            User.auth_source == AuthSource.OAUTH,
            User.oauth_provider_id == provider_id,
            User.oauth_user_id == oauth_user_id,
        )
        .with_for_update()
        .first()
    )

    if not user:
        # 按 email 查找（检查是否有同邮箱的账号）
        user = db.query(User).filter(User.email == email).with_for_update().first()

    if user:
        if user.auth_source != AuthSource.OAUTH or user.oauth_provider_id != provider_id:
            # 避免覆盖已有本地账户或其他 OAuth 提供商账户
            logger.warning(
                f"OAuth [{provider_id}] 登录拒绝 - 账户来源不匹配 "
                f"(现有: {user.auth_source}/{user.oauth_provider_id}, 请求: OAUTH/{provider_id}): {email}"
            )
            return None

        # 安全考虑：不自动同步邮箱，防止账户劫持
        if user.email != email:
            logger.warning(
                f"OAuth [{provider_id}] 用户邮箱变更检测 (用户ID: {user.id}): "
                f"原邮箱={user.email}, 新邮箱={email}。出于安全考虑，不自动同步邮箱。"
            )
            AuditService.log_event(
                db=db,
                event_type=AuditEventType.USER_UPDATED,
                description=f"OAuth [{provider_id}] 用户邮箱变更检测（未同步）",
                user_id=str(user.id),
                metadata={
                    "old_email": user.email,
                    "new_email": email,
                    "oauth_provider_id": provider_id,
                    "oauth_user_id": oauth_user_id,
                    "reason": "security_policy_email_sync_disabled",
                },
            )

        # 同步标识
        if oauth_user_id and user.oauth_user_id != oauth_user_id:
            user.oauth_user_id = oauth_user_id
        if oauth_username and user.oauth_username != oauth_username:
            user.oauth_username = oauth_username

        user.last_login_at = datetime.now(timezone.utc)
        db.commit()
        logger.info(f"OAuth [{provider_id}] 用户登录成功: {user.email} (ID: {user.id})")
        return user

    # 创建新用户前，检查邮箱是否已被其他认证源占用
    existing_email_user = db.query(User).filter(User.email == email).first()
    if existing_email_user:
        logger.error(
            f"OAuth [{provider_id}] 用户创建失败 - 邮箱已被占用: {email} "
            f"(现有账户来源: {existing_email_user.auth_source})"
        )
        return None

    # 创建新用户
    base_username = oauth_username or f"{provider_id}_{oauth_user_id}"
    username = base_username
    max_retries = 3

    for attempt in range(max_retries):
        # 检查用户名是否已存在
        existing_user = db.query(User).filter(User.username == username).first()
        if existing_user:
            username = f"{base_username}_{provider_id}_{int(time.time())}{uuid.uuid4().hex[:4]}"
            logger.info(f"OAuth [{provider_id}] 用户名冲突，使用新用户名: {base_username} -> {username}")
            continue

        # 读取系统配置的默认配额
        default_quota = SystemConfigService.get_config(db, "default_user_quota_usd", default=10.0)

        user = User(
            email=email,
            username=username,
            password_hash="",  # OAuth 用户无本地密码
            auth_source=AuthSource.OAUTH,
            oauth_provider_id=provider_id,
            oauth_user_id=oauth_user_id,
            oauth_username=oauth_username,
            role=UserRole.USER,
            is_active=True,
            last_login_at=datetime.now(timezone.utc),
            quota_usd=default_quota,
        )

        try:
            db.add(user)
            db.commit()
            db.refresh(user)
            logger.info(f"OAuth [{provider_id}] 用户创建成功: {email} (ID: {user.id})")
            return user
        except IntegrityError as e:
            db.rollback()
            error_str = str(e.orig).lower() if e.orig else str(e).lower()

            if "email" in error_str or "ix_users_email" in error_str:
                logger.error(f"OAuth [{provider_id}] 用户创建失败 - 邮箱并发冲突: {email}")
                return None
            elif "username" in error_str or "ix_users_username" in error_str:
                if attempt == max_retries - 1:
                    logger.error(
                        f"OAuth [{provider_id}] 用户创建失败（用户名冲突重试耗尽）: {username}"
                    )
                    return None
                username = f"{base_username}_{provider_id}_{int(time.time())}{uuid.uuid4().hex[:4]}"
                logger.warning(
                    f"OAuth [{provider_id}] 用户创建用户名冲突，重试 ({attempt + 1}/{max_retries}): {username}"
                )
            else:
                logger.error(f"OAuth [{provider_id}] 用户创建失败 - 未知数据库约束冲突: {e}")
                return None

    return None
