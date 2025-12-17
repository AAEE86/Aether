"""
认证相关API端点
"""

from typing import Optional

from fastapi import APIRouter, Depends, HTTPException, Request, status
from fastapi.security import HTTPAuthorizationCredentials, HTTPBearer
from pydantic import ValidationError
from sqlalchemy.orm import Session

from src.api.base.adapter import ApiAdapter, ApiMode
from src.api.base.authenticated_adapter import AuthenticatedApiAdapter
from src.api.base.pipeline import ApiRequestPipeline
from src.core.exceptions import InvalidRequestException
from src.core.logger import logger
from src.database import get_db
from src.models.api import (
    LoginRequest,
    LoginResponse,
    LogoutResponse,
    RefreshTokenRequest,
    RefreshTokenResponse,
    RegisterRequest,
    RegisterResponse,
)
from src.models.database import AuditEventType, User, UserRole
from src.services.auth.service import AuthService
from src.services.rate_limit.ip_limiter import IPRateLimiter
from src.services.system.audit import AuditService
from src.services.user.service import UserService
from src.utils.request_utils import get_client_ip, get_user_agent


router = APIRouter(prefix="/api/auth", tags=["Authentication"])
security = HTTPBearer()
pipeline = ApiRequestPipeline()


# API端点
@router.post("/login", response_model=LoginResponse)
async def login(request: Request, db: Session = Depends(get_db)):
    adapter = AuthLoginAdapter()
    return await pipeline.run(adapter=adapter, http_request=request, db=db, mode=adapter.mode)


@router.post("/refresh", response_model=RefreshTokenResponse)
async def refresh_token(request: Request, db: Session = Depends(get_db)):
    adapter = AuthRefreshAdapter()
    return await pipeline.run(adapter=adapter, http_request=request, db=db, mode=adapter.mode)


@router.post("/register", response_model=RegisterResponse)
async def register(request: Request, db: Session = Depends(get_db)):
    adapter = AuthRegisterAdapter()
    return await pipeline.run(adapter=adapter, http_request=request, db=db, mode=adapter.mode)


@router.get("/me")
async def get_current_user_info(request: Request, db: Session = Depends(get_db)):
    adapter = AuthCurrentUserAdapter()
    return await pipeline.run(adapter=adapter, http_request=request, db=db, mode=adapter.mode)


@router.patch("/password")
async def change_password(request: Request, db: Session = Depends(get_db)):
    """Change current user's password"""
    adapter = AuthChangePasswordAdapter()
    return await pipeline.run(adapter=adapter, http_request=request, db=db, mode=adapter.mode)


@router.post("/logout", response_model=LogoutResponse)
async def logout(request: Request, db: Session = Depends(get_db)):
    adapter = AuthLogoutAdapter()
    return await pipeline.run(adapter=adapter, http_request=request, db=db, mode=adapter.mode)


# ============== 适配器实现 ==============


class AuthPublicAdapter(ApiAdapter):
    mode = ApiMode.PUBLIC

    def authorize(self, context):  # type: ignore[override]
        return None


class AuthLoginAdapter(AuthPublicAdapter):
    async def handle(self, context):  # type: ignore[override]
        db = context.db
        payload = context.ensure_json_body()

        try:
            login_request = LoginRequest.model_validate(payload)
        except ValidationError as exc:
            errors = []
            for error in exc.errors():
                field = " -> ".join(str(x) for x in error["loc"])
                errors.append(f"{field}: {error['msg']}")
            raise InvalidRequestException("输入验证失败: " + "; ".join(errors))

        client_ip = get_client_ip(context.request)
        user_agent = get_user_agent(context.request)

        # IP 速率限制检查（登录接口：5次/分钟）
        allowed, remaining, reset_after = await IPRateLimiter.check_limit(client_ip, "login")
        if not allowed:
            logger.warning(f"登录请求超过速率限制: IP={client_ip}, 剩余={remaining}")
            raise HTTPException(
                status_code=status.HTTP_429_TOO_MANY_REQUESTS,
                detail=f"登录请求过于频繁，请在 {reset_after} 秒后重试",
            )

        user = await AuthService.authenticate_user(db, login_request.email, login_request.password)
        if not user:
            AuditService.log_login_attempt(
                db=db,
                email=login_request.email,
                success=False,
                ip_address=client_ip,
                user_agent=user_agent,
                error_reason="邮箱或密码错误",
            )
            db.commit()
            raise HTTPException(status_code=status.HTTP_401_UNAUTHORIZED, detail="邮箱或密码错误")

        AuditService.log_login_attempt(
            db=db,
            email=login_request.email,
            success=True,
            ip_address=client_ip,
            user_agent=user_agent,
            user_id=user.id,
        )
        db.commit()

        access_token = AuthService.create_access_token(
            data={
                "user_id": user.id,
                "email": user.email,
                "role": user.role.value,
                "created_at": user.created_at.isoformat() if user.created_at else None,
            }
        )
        refresh_token = AuthService.create_refresh_token(
            data={"user_id": user.id, "email": user.email}
        )
        response = LoginResponse(
            access_token=access_token,
            refresh_token=refresh_token,
            token_type="bearer",
            expires_in=86400,
            user_id=user.id,
            email=user.email,
            username=user.username,
            role=user.role.value,
        )
        return response.model_dump()


class AuthRefreshAdapter(AuthPublicAdapter):
    async def handle(self, context):  # type: ignore[override]
        db = context.db
        payload = context.ensure_json_body()
        refresh_request = RefreshTokenRequest.model_validate(payload)
        client_ip = get_client_ip(context.request)
        user_agent = get_user_agent(context.request)

        try:
            token_payload = await AuthService.verify_token(
                refresh_request.refresh_token, token_type="refresh"
            )
            user_id = token_payload.get("user_id")
            if not user_id:
                raise HTTPException(
                    status_code=status.HTTP_401_UNAUTHORIZED, detail="无效的刷新令牌"
                )

            user = db.query(User).filter(User.id == user_id).first()
            if not user:
                raise HTTPException(
                    status_code=status.HTTP_401_UNAUTHORIZED, detail="无效的刷新令牌"
                )
            if not user.is_active:
                raise HTTPException(status_code=status.HTTP_403_FORBIDDEN, detail="用户已禁用")

            new_access_token = AuthService.create_access_token(
                data={
                    "user_id": user.id,
                    "email": user.email,
                    "role": user.role.value,
                    "created_at": user.created_at.isoformat() if user.created_at else None,
                }
            )
            new_refresh_token = AuthService.create_refresh_token(
                data={"user_id": user.id, "email": user.email}
            )
            logger.info(f"令牌刷新成功: {user.email}")
            return RefreshTokenResponse(
                access_token=new_access_token,
                refresh_token=new_refresh_token,
                token_type="bearer",
                expires_in=86400,
            ).model_dump()
        except HTTPException:
            raise
        except Exception as exc:
            logger.error(f"刷新令牌失败: {exc}")
            raise HTTPException(status_code=status.HTTP_401_UNAUTHORIZED, detail="刷新令牌失败")


class AuthRegisterAdapter(AuthPublicAdapter):
    async def handle(self, context):  # type: ignore[override]
        from src.models.database import SystemConfig

        db = context.db
        payload = context.ensure_json_body()
        register_request = RegisterRequest.model_validate(payload)
        client_ip = get_client_ip(context.request)
        user_agent = get_user_agent(context.request)

        # IP 速率限制检查（注册接口：3次/分钟）
        allowed, remaining, reset_after = await IPRateLimiter.check_limit(client_ip, "register")
        if not allowed:
            logger.warning(f"注册请求超过速率限制: IP={client_ip}, 剩余={remaining}")
            raise HTTPException(
                status_code=status.HTTP_429_TOO_MANY_REQUESTS,
                detail=f"注册请求过于频繁，请在 {reset_after} 秒后重试",
            )

        allow_registration = db.query(SystemConfig).filter_by(key="enable_registration").first()
        if allow_registration and not allow_registration.value:
            AuditService.log_event(
                db=db,
                event_type=AuditEventType.UNAUTHORIZED_ACCESS,
                description=f"Registration attempt rejected - registration disabled: {register_request.email}",
                ip_address=client_ip,
                user_agent=user_agent,
                metadata={"email": register_request.email, "reason": "registration_disabled"},
            )
            db.commit()
            raise HTTPException(status_code=status.HTTP_403_FORBIDDEN, detail="系统暂不开放注册")

        try:
            user = UserService.create_user(
                db=db,
                email=register_request.email,
                username=register_request.username,
                password=register_request.password,
                role=UserRole.USER,
            )
            AuditService.log_event(
                db=db,
                event_type=AuditEventType.USER_CREATED,
                description=f"User registered: {user.email}",
                user_id=user.id,
                ip_address=client_ip,
                user_agent=user_agent,
                metadata={"email": user.email, "username": user.username, "role": user.role.value},
            )
            db.commit()
            return RegisterResponse(
                user_id=user.id,
                email=user.email,
                username=user.username,
                message="注册成功",
            ).model_dump()
        except ValueError as exc:
            AuditService.log_event(
                db=db,
                event_type=AuditEventType.UNAUTHORIZED_ACCESS,
                description=f"Registration failed: {register_request.email} - {exc}",
                ip_address=client_ip,
                user_agent=user_agent,
                metadata={"email": register_request.email, "error": str(exc)},
            )
            db.commit()
            raise HTTPException(status_code=status.HTTP_400_BAD_REQUEST, detail=str(exc))


class AuthCurrentUserAdapter(AuthenticatedApiAdapter):
    async def handle(self, context):  # type: ignore[override]
        user = context.user
        return {
            "id": user.id,
            "email": user.email,
            "username": user.username,
            "role": user.role.value,
            "is_active": user.is_active,
            "quota_usd": user.quota_usd,
            "used_usd": user.used_usd,
            "total_usd": user.total_usd,
            "allowed_providers": user.allowed_providers,
            "allowed_endpoints": user.allowed_endpoints,
            "allowed_models": user.allowed_models,
            "created_at": user.created_at.isoformat(),
            "last_login_at": user.last_login_at.isoformat() if user.last_login_at else None,
        }


class AuthChangePasswordAdapter(AuthenticatedApiAdapter):
    async def handle(self, context):  # type: ignore[override]
        payload = context.ensure_json_body()
        old_password = payload.get("old_password")
        new_password = payload.get("new_password")
        if not old_password or not new_password:
            raise HTTPException(status_code=400, detail="必须提供旧密码和新密码")
        user = context.user
        if not user.verify_password(old_password):
            raise HTTPException(status_code=status.HTTP_400_BAD_REQUEST, detail="旧密码错误")
        if len(new_password) < 8:
            raise InvalidRequestException("密码长度至少8位")
        user.set_password(new_password)
        context.db.commit()
        logger.info(f"用户修改密码: {user.email}")
        return {"message": "密码修改成功"}


class AuthLogoutAdapter(AuthenticatedApiAdapter):
    async def handle(self, context):  # type: ignore[override]
        """用户登出，将 Token 加入黑名单"""
        user = context.user
        client_ip = get_client_ip(context.request)

        # 从 Authorization header 获取 Token
        auth_header = context.request.headers.get("Authorization")
        if not auth_header or not auth_header.startswith("Bearer "):
            raise HTTPException(status_code=status.HTTP_401_UNAUTHORIZED, detail="缺少认证令牌")

        token = auth_header.replace("Bearer ", "")

        # 将 Token 加入黑名单
        success = await AuthService.logout(token)

        if success:
            # 记录审计日志
            AuditService.log_event(
                db=context.db,
                event_type=AuditEventType.LOGOUT,
                description=f"User logged out: {user.email}",
                user_id=user.id,
                ip_address=client_ip,
                user_agent=get_user_agent(context.request),
                metadata={"user_id": user.id, "email": user.email},
            )
            context.db.commit()

            logger.info(f"用户登出成功: {user.email}")

            return LogoutResponse(message="登出成功", success=True).model_dump()
        else:
            logger.warning(f"用户登出失败（Redis不可用）: {user.email}")
            return LogoutResponse(message="登出成功（降级模式）", success=False).model_dump()
