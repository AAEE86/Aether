"""
认证相关API端点
"""

from typing import Optional, Tuple

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
    RegistrationSettingsResponse,
    SendVerificationCodeRequest,
    SendVerificationCodeResponse,
    VerificationStatusRequest,
    VerificationStatusResponse,
    VerifyEmailRequest,
    VerifyEmailResponse,
)
from src.models.database import AuditEventType, User, UserRole
from src.services.auth.service import AuthService
from src.services.auth.ldap import LDAPService
from src.services.rate_limit.ip_limiter import IPRateLimiter
from src.services.system.audit import AuditService
from src.services.system.config import SystemConfigService
from src.services.user.service import UserService
from src.services.email import EmailSenderService, EmailVerificationService
from src.utils.request_utils import get_client_ip, get_user_agent


def validate_email_suffix(db: Session, email: str) -> Tuple[bool, Optional[str]]:
    """
    验证邮箱后缀是否允许注册

    Args:
        db: 数据库会话
        email: 邮箱地址

    Returns:
        (是否允许, 错误信息)
    """
    # 获取邮箱后缀限制配置
    mode = SystemConfigService.get_config(db, "email_suffix_mode", default="none")

    if mode == "none":
        return True, None

    # 获取邮箱后缀列表
    suffix_list = SystemConfigService.get_config(db, "email_suffix_list", default=[])
    if not suffix_list:
        # 没有配置后缀列表时，不限制
        return True, None

    # 确保 suffix_list 是列表类型
    if isinstance(suffix_list, str):
        suffix_list = [s.strip().lower() for s in suffix_list.split(",") if s.strip()]

    # 获取邮箱后缀
    if "@" not in email:
        return False, "邮箱格式无效"

    email_suffix = email.split("@")[1].lower()

    if mode == "whitelist":
        # 白名单模式：只允许列出的后缀
        if email_suffix not in suffix_list:
            return False, f"该邮箱后缀不在允许列表中，仅支持: {', '.join(suffix_list)}"
    elif mode == "blacklist":
        # 黑名单模式：拒绝列出的后缀
        if email_suffix in suffix_list:
            return False, f"该邮箱后缀 ({email_suffix}) 不允许注册"

    return True, None


router = APIRouter(prefix="/api/auth", tags=["Authentication"])
security = HTTPBearer()
pipeline = ApiRequestPipeline()


# API端点
@router.get("/registration-settings", response_model=RegistrationSettingsResponse)
async def registration_settings(request: Request, db: Session = Depends(get_db)):
    """公开获取注册相关配置"""
    adapter = AuthRegistrationSettingsAdapter()
    return await pipeline.run(adapter=adapter, http_request=request, db=db, mode=adapter.mode)


@router.get("/settings")
async def auth_settings(request: Request, db: Session = Depends(get_db)):
    """公开获取认证设置（用于前端判断显示哪些登录选项）"""
    adapter = AuthSettingsAdapter()
    return await pipeline.run(adapter=adapter, http_request=request, db=db, mode=adapter.mode)


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


@router.post("/send-verification-code", response_model=SendVerificationCodeResponse)
async def send_verification_code(request: Request, db: Session = Depends(get_db)):
    """发送邮箱验证码"""
    adapter = AuthSendVerificationCodeAdapter()
    return await pipeline.run(adapter=adapter, http_request=request, db=db, mode=adapter.mode)


@router.post("/verify-email", response_model=VerifyEmailResponse)
async def verify_email(request: Request, db: Session = Depends(get_db)):
    """验证邮箱验证码"""
    adapter = AuthVerifyEmailAdapter()
    return await pipeline.run(adapter=adapter, http_request=request, db=db, mode=adapter.mode)


@router.post("/verification-status", response_model=VerificationStatusResponse)
async def verification_status(request: Request, db: Session = Depends(get_db)):
    """查询邮箱验证状态"""
    adapter = AuthVerificationStatusAdapter()
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

        user = await AuthService.authenticate_user(
            db, login_request.email, login_request.password, login_request.auth_type
        )
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


class AuthRegistrationSettingsAdapter(AuthPublicAdapter):
    async def handle(self, context):  # type: ignore[override]
        """公开返回注册相关配置"""
        db = context.db

        enable_registration = SystemConfigService.get_config(db, "enable_registration", default=False)
        require_verification = SystemConfigService.get_config(db, "require_email_verification", default=False)

        return RegistrationSettingsResponse(
            enable_registration=bool(enable_registration),
            require_email_verification=bool(require_verification),
        ).model_dump()


class AuthSettingsAdapter(AuthPublicAdapter):
    async def handle(self, context):  # type: ignore[override]
        """公开返回认证设置"""
        db = context.db

        ldap_enabled = LDAPService.is_ldap_enabled(db)
        ldap_exclusive = LDAPService.is_ldap_exclusive(db)

        return {
            "local_enabled": not ldap_exclusive,
            "ldap_enabled": ldap_enabled,
            "ldap_exclusive": ldap_exclusive,
        }


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

        # 仅允许 LDAP 登录时拒绝本地注册
        if LDAPService.is_ldap_exclusive(db):
            raise HTTPException(
                status_code=status.HTTP_403_FORBIDDEN, detail="系统已启用 LDAP 专属登录，禁止本地注册"
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

        # 检查邮箱后缀是否允许
        suffix_allowed, suffix_error = validate_email_suffix(db, register_request.email)
        if not suffix_allowed:
            logger.warning(f"注册失败：邮箱后缀不允许: {register_request.email}")
            AuditService.log_event(
                db=db,
                event_type=AuditEventType.UNAUTHORIZED_ACCESS,
                description=f"Registration attempt rejected - email suffix not allowed: {register_request.email}",
                ip_address=client_ip,
                user_agent=user_agent,
                metadata={"email": register_request.email, "reason": "email_suffix_not_allowed"},
            )
            db.commit()
            raise HTTPException(
                status_code=status.HTTP_400_BAD_REQUEST,
                detail=suffix_error,
            )

        # 检查是否需要邮箱验证
        require_verification = SystemConfigService.get_config(db, "require_email_verification", default=False)

        if require_verification:
            # 检查邮箱是否已验证
            is_verified = await EmailVerificationService.is_email_verified(register_request.email)
            if not is_verified:
                logger.warning(f"注册失败：邮箱未验证: {register_request.email}")
                raise HTTPException(
                    status_code=status.HTTP_400_BAD_REQUEST,
                    detail="请先完成邮箱验证。请发送验证码并验证后再注册。",
                )

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

            # 注册成功后清除验证状态（在 commit 后清理，即使清理失败也不影响注册结果）
            if require_verification:
                try:
                    await EmailVerificationService.clear_verification(register_request.email)
                except Exception as e:
                    logger.warning(f"清理验证状态失败: {e}")

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
        if len(new_password) < 6:
            raise InvalidRequestException("密码长度至少6位")
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


class AuthSendVerificationCodeAdapter(AuthPublicAdapter):
    async def handle(self, context):  # type: ignore[override]
        """发送邮箱验证码"""
        db = context.db
        payload = context.ensure_json_body()

        try:
            send_request = SendVerificationCodeRequest.model_validate(payload)
        except ValidationError as exc:
            errors = []
            for error in exc.errors():
                field = " -> ".join(str(x) for x in error["loc"])
                errors.append(f"{field}: {error['msg']}")
            raise InvalidRequestException("输入验证失败: " + "; ".join(errors))

        client_ip = get_client_ip(context.request)
        email = send_request.email

        # IP 速率限制检查（验证码发送：3次/分钟）
        allowed, remaining, reset_after = await IPRateLimiter.check_limit(
            client_ip, "verification_send"
        )
        if not allowed:
            logger.warning(f"验证码发送请求超过速率限制: IP={client_ip}, 剩余={remaining}")
            raise HTTPException(
                status_code=status.HTTP_429_TOO_MANY_REQUESTS,
                detail=f"请求过于频繁，请在 {reset_after} 秒后重试",
            )

        # 检查邮箱是否已注册
        existing_user = db.query(User).filter(User.email == email).first()
        if existing_user:
            raise HTTPException(
                status_code=status.HTTP_400_BAD_REQUEST,
                detail="该邮箱已被注册，请直接登录或使用其他邮箱",
            )

        # 检查邮箱后缀是否允许
        suffix_allowed, suffix_error = validate_email_suffix(db, email)
        if not suffix_allowed:
            logger.warning(f"邮箱后缀不允许: {email}")
            raise HTTPException(
                status_code=status.HTTP_400_BAD_REQUEST,
                detail=suffix_error,
            )

        # 生成并发送验证码（使用服务中的默认配置）
        success, code_or_error, error_detail = await EmailVerificationService.send_verification_code(
            email
        )

        if not success:
            logger.error(f"发送验证码失败: {email}, 错误: {code_or_error}")
            raise HTTPException(
                status_code=status.HTTP_400_BAD_REQUEST,
                detail=error_detail or code_or_error,
            )

        # 发送邮件
        expire_minutes = EmailVerificationService.DEFAULT_CODE_EXPIRE_MINUTES
        email_success, email_error = await EmailSenderService.send_verification_code(
            db=db, to_email=email, code=code_or_error, expire_minutes=expire_minutes
        )

        if not email_success:
            logger.error(f"发送验证码邮件失败: {email}, 错误: {email_error}")
            # 不向用户暴露 SMTP 详细错误信息，防止信息泄露
            raise HTTPException(
                status_code=status.HTTP_500_INTERNAL_SERVER_ERROR,
                detail="发送验证码失败，请稍后重试",
            )

        logger.info(f"验证码已发送: {email}")

        return SendVerificationCodeResponse(
            message="验证码已发送，请查收邮件",
            success=True,
            expire_minutes=expire_minutes,
        ).model_dump()


class AuthVerifyEmailAdapter(AuthPublicAdapter):
    async def handle(self, context):  # type: ignore[override]
        """验证邮箱验证码"""
        db = context.db
        payload = context.ensure_json_body()

        try:
            verify_request = VerifyEmailRequest.model_validate(payload)
        except ValidationError as exc:
            errors = []
            for error in exc.errors():
                field = " -> ".join(str(x) for x in error["loc"])
                errors.append(f"{field}: {error['msg']}")
            raise InvalidRequestException("输入验证失败: " + "; ".join(errors))

        client_ip = get_client_ip(context.request)
        email = verify_request.email
        code = verify_request.code

        # IP 速率限制检查（验证码验证：10次/分钟）
        allowed, remaining, reset_after = await IPRateLimiter.check_limit(
            client_ip, "verification_verify"
        )
        if not allowed:
            logger.warning(f"验证码验证请求超过速率限制: IP={client_ip}, 剩余={remaining}")
            raise HTTPException(
                status_code=status.HTTP_429_TOO_MANY_REQUESTS,
                detail=f"请求过于频繁，请在 {reset_after} 秒后重试",
            )

        # 验证验证码
        success, message = await EmailVerificationService.verify_code(email, code)

        if not success:
            logger.warning(f"验证码验证失败: {email}, 原因: {message}")
            raise HTTPException(status_code=status.HTTP_400_BAD_REQUEST, detail=message)

        logger.info(f"邮箱验证成功: {email}")

        return VerifyEmailResponse(message="邮箱验证成功", success=True).model_dump()


class AuthVerificationStatusAdapter(AuthPublicAdapter):
    async def handle(self, context):  # type: ignore[override]
        """查询邮箱验证状态"""
        payload = context.ensure_json_body()

        try:
            status_request = VerificationStatusRequest.model_validate(payload)
        except ValidationError as exc:
            errors = []
            for error in exc.errors():
                field = " -> ".join(str(x) for x in error["loc"])
                errors.append(f"{field}: {error['msg']}")
            raise InvalidRequestException("输入验证失败: " + "; ".join(errors))

        client_ip = get_client_ip(context.request)
        email = status_request.email

        # IP 速率限制检查（验证状态查询：20次/分钟）
        allowed, remaining, reset_after = await IPRateLimiter.check_limit(
            client_ip, "verification_status", limit=20
        )
        if not allowed:
            logger.warning(f"验证状态查询请求超过速率限制: IP={client_ip}, 剩余={remaining}")
            raise HTTPException(
                status_code=status.HTTP_429_TOO_MANY_REQUESTS,
                detail=f"请求过于频繁，请在 {reset_after} 秒后重试",
            )

        # 获取验证状态
        status_data = await EmailVerificationService.get_verification_status(email)

        # 计算冷却剩余时间
        cooldown_remaining = None
        if status_data.get("has_pending_code") and status_data.get("created_at"):
            from datetime import datetime, timezone

            created_at = datetime.fromisoformat(status_data["created_at"])
            elapsed = (datetime.now(timezone.utc) - created_at).total_seconds()
            cooldown = EmailVerificationService.SEND_COOLDOWN_SECONDS - int(elapsed)
            if cooldown > 0:
                cooldown_remaining = cooldown

        return VerificationStatusResponse(
            email=email,
            has_pending_code=status_data.get("has_pending_code", False),
            is_verified=status_data.get("is_verified", False),
            cooldown_remaining=cooldown_remaining,
            code_expires_in=status_data.get("code_expires_in"),
        ).model_dump()
