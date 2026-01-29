"""用户管理 API 端点。"""

from datetime import datetime, timezone

from fastapi import APIRouter, Depends, HTTPException, Query, Request, status
from pydantic import ValidationError
from sqlalchemy.orm import Session

from src.api.base.admin_adapter import AdminApiAdapter
from src.api.base.pipeline import ApiRequestPipeline
from src.core.exceptions import InvalidRequestException, NotFoundException, translate_pydantic_error
from src.core.logger import logger
from src.database import get_db
from src.models.admin_requests import UpdateUserRequest
from src.models.api import CreateApiKeyRequest, CreateUserRequest
from src.models.database import ApiKey, User, UserRole
from src.services.system.config import SystemConfigService
from src.services.user.apikey import ApiKeyService
from src.services.user.service import UserService


router = APIRouter(prefix="/api/admin/users", tags=["Admin - Users"])
pipeline = ApiRequestPipeline()


# 管理员端点
@router.post("")
async def create_user_endpoint(request: Request, db: Session = Depends(get_db)):
    """
    创建用户

    创建新用户账号（管理员专用）。

    **请求体**:
    - `email`: 邮箱地址
    - `username`: 用户名
    - `password`: 密码
    - `role`: 角色（user/admin）
    - `quota_usd`: 配额（USD）
    """
    adapter = AdminCreateUserAdapter()
    return await pipeline.run(adapter=adapter, http_request=request, db=db, mode=adapter.mode)


@router.get("")
async def list_users(
    request: Request,
    skip: int = Query(0, ge=0, description="跳过记录数"),
    limit: int = Query(100, ge=1, le=1000, description="返回记录数"),
    role: str | None = Query(None, description="按角色筛选（user/admin）"),
    is_active: bool | None = Query(None, description="按状态筛选"),
    db: Session = Depends(get_db),
):
    """
    获取用户列表

    分页获取用户列表，支持按角色和状态筛选。

    **返回字段**: id, email, username, role, quota_usd, used_usd, is_active, created_at 等
    """
    adapter = AdminListUsersAdapter(skip=skip, limit=limit, role=role, is_active=is_active)
    return await pipeline.run(adapter=adapter, http_request=request, db=db, mode=adapter.mode)


@router.get("/{user_id}")
async def get_user(user_id: str, request: Request, db: Session = Depends(get_db)):
    """
    获取用户详情

    获取指定用户的详细信息。

    **路径参数**:
    - `user_id`: 用户 ID (UUID)
    """
    adapter = AdminGetUserAdapter(user_id=user_id)
    return await pipeline.run(adapter=adapter, http_request=request, db=db, mode=adapter.mode)


@router.put("/{user_id}")
async def update_user(
    user_id: str,
    request: Request,
    db: Session = Depends(get_db),
):
    """
    更新用户信息

    更新指定用户的信息，包括角色、配额、权限等。

    **路径参数**:
    - `user_id`: 用户 ID (UUID)

    **请求体** (均为可选):
    - `email`: 邮箱地址
    - `username`: 用户名
    - `role`: 角色
    - `quota_usd`: 配额
    - `is_active`: 是否启用
    - `allowed_providers`: 允许的提供商列表
    - `allowed_models`: 允许的模型列表
    """
    adapter = AdminUpdateUserAdapter(user_id=user_id)
    return await pipeline.run(adapter=adapter, http_request=request, db=db, mode=adapter.mode)


@router.delete("/{user_id}")
async def delete_user(user_id: str, request: Request, db: Session = Depends(get_db)):
    """
    删除用户

    永久删除指定用户。不能删除最后一个管理员账户。

    **路径参数**:
    - `user_id`: 用户 ID (UUID)
    """
    adapter = AdminDeleteUserAdapter(user_id=user_id)
    return await pipeline.run(adapter=adapter, http_request=request, db=db, mode=adapter.mode)


@router.patch("/{user_id}/quota")
async def reset_user_quota(user_id: str, request: Request, db: Session = Depends(get_db)):
    """
    重置用户配额

    将用户的已用配额（used_usd）重置为 0。

    **路径参数**:
    - `user_id`: 用户 ID (UUID)
    """
    adapter = AdminResetUserQuotaAdapter(user_id=user_id)
    return await pipeline.run(adapter=adapter, http_request=request, db=db, mode=adapter.mode)


@router.get("/{user_id}/api-keys")
async def get_user_api_keys(
    user_id: str,
    request: Request,
    is_active: bool | None = Query(None, description="按状态筛选"),
    db: Session = Depends(get_db),
):
    """
    获取用户的 API 密钥列表

    获取指定用户的所有 API 密钥（不包括独立密钥）。

    **路径参数**:
    - `user_id`: 用户 ID (UUID)
    """
    adapter = AdminGetUserKeysAdapter(user_id=user_id, is_active=is_active)
    return await pipeline.run(adapter=adapter, http_request=request, db=db, mode=adapter.mode)


@router.post("/{user_id}/api-keys")
async def create_user_api_key(
    user_id: str,
    request: Request,
    db: Session = Depends(get_db),
):
    """
    为用户创建 API 密钥

    为指定用户创建新的 API 密钥。

    **路径参数**:
    - `user_id`: 用户 ID (UUID)

    **请求体**:
    - `name`: 密钥名称
    - `allowed_providers`: 允许的提供商（可选）
    - `allowed_models`: 允许的模型（可选）
    - `rate_limit`: 速率限制（可选）
    - `expire_days`: 过期天数（可选）

    **返回**: 包含完整密钥值的响应（仅此一次显示）
    """
    adapter = AdminCreateUserKeyAdapter(user_id=user_id)
    return await pipeline.run(adapter=adapter, http_request=request, db=db, mode=adapter.mode)


@router.delete("/{user_id}/api-keys/{key_id}")
async def delete_user_api_key(
    user_id: str,
    key_id: str,
    request: Request,
    db: Session = Depends(get_db),
):
    """
    删除用户的 API 密钥

    删除指定用户的指定 API 密钥。

    **路径参数**:
    - `user_id`: 用户 ID (UUID)
    - `key_id`: 密钥 ID
    """
    adapter = AdminDeleteUserKeyAdapter(user_id=user_id, key_id=key_id)
    return await pipeline.run(adapter=adapter, http_request=request, db=db, mode=adapter.mode)


# ============== 管理员适配器实现 ==============


class AdminCreateUserAdapter(AdminApiAdapter):
    async def handle(self, context):  # type: ignore[override]
        db = context.db
        payload = context.ensure_json_body()
        try:
            request = CreateUserRequest.model_validate(payload)
        except ValidationError as e:
            errors = e.errors()
            if errors:
                raise InvalidRequestException(translate_pydantic_error(errors[0]))
            raise InvalidRequestException("请求数据验证失败")
        try:
            role = (
                request.role if hasattr(request.role, "value") else UserRole[request.role.upper()]
            )
        except (KeyError, AttributeError):
            raise InvalidRequestException("角色参数不合法")

        # 确定配额：unlimited 优先，其次是指定值，最后是系统默认
        if request.unlimited:
            quota_usd = None  # None 表示无限制
        elif request.quota_usd is not None:
            quota_usd = request.quota_usd
        else:
            quota_usd = SystemConfigService.get_config(db, "default_user_quota_usd", default=10.0)

        # 处理访问权限字段：空数组转为 None（表示无限制）
        allowed_providers = request.allowed_providers if request.allowed_providers else None
        allowed_api_formats = request.allowed_api_formats if request.allowed_api_formats else None
        allowed_models = request.allowed_models if request.allowed_models else None

        try:
            user = UserService.create_user(
                db=db,
                email=request.email,
                username=request.username,
                password=request.password,
                role=role,
                quota_usd=quota_usd,
                allowed_providers=allowed_providers,
                allowed_api_formats=allowed_api_formats,
                allowed_models=allowed_models,
            )
        except ValueError as exc:
            raise InvalidRequestException(str(exc))

        context.add_audit_metadata(
            action="create_user",
            target_user_id=user.id,
            target_email=user.email,
            target_username=user.username,
            target_role=user.role.value,
            quota_usd=user.quota_usd,
            is_active=user.is_active,
        )

        return {
            "id": user.id,
            "email": user.email,
            "username": user.username,
            "role": user.role.value,
            "allowed_providers": user.allowed_providers,
            "allowed_api_formats": user.allowed_api_formats,
            "allowed_models": user.allowed_models,
            "quota_usd": user.quota_usd,
            "used_usd": user.used_usd,
            "total_usd": getattr(user, "total_usd", 0),
            "is_active": user.is_active,
            "created_at": user.created_at.isoformat(),
        }


class AdminListUsersAdapter(AdminApiAdapter):
    def __init__(self, skip: int, limit: int, role: str | None, is_active: bool | None):
        self.skip = skip
        self.limit = limit
        self.role = role
        self.is_active = is_active

    async def handle(self, context):  # type: ignore[override]
        db = context.db
        role_enum = UserRole[self.role.upper()] if self.role else None
        users = UserService.list_users(db, self.skip, self.limit, role_enum, self.is_active)
        return [
            {
                "id": u.id,
                "email": u.email,
                "username": u.username,
                "role": u.role.value,
                "allowed_providers": u.allowed_providers,
                "allowed_api_formats": u.allowed_api_formats,
                "allowed_models": u.allowed_models,
                "quota_usd": u.quota_usd,
                "used_usd": u.used_usd,
                "total_usd": getattr(u, "total_usd", 0),
                "is_active": u.is_active,
                "created_at": u.created_at.isoformat(),
            }
            for u in users
        ]


class AdminGetUserAdapter(AdminApiAdapter):
    def __init__(self, user_id: str):
        self.user_id = user_id

    async def handle(self, context):  # type: ignore[override]
        db = context.db
        user = UserService.get_user(db, self.user_id)
        if not user:
            raise NotFoundException("用户不存在", "user")

        context.add_audit_metadata(
            action="get_user_detail",
            target_user_id=user.id,
            target_role=user.role.value,
            include_history=bool(user.last_login_at),
        )

        return {
            "id": user.id,
            "email": user.email,
            "username": user.username,
            "role": user.role.value,
            "allowed_providers": user.allowed_providers,
            "allowed_api_formats": user.allowed_api_formats,
            "allowed_models": user.allowed_models,
            "quota_usd": user.quota_usd,
            "used_usd": user.used_usd,
            "total_usd": getattr(user, "total_usd", 0),
            "is_active": user.is_active,
            "created_at": user.created_at.isoformat(),
            "updated_at": user.updated_at.isoformat() if user.updated_at else None,
            "last_login_at": user.last_login_at.isoformat() if user.last_login_at else None,
        }


class AdminUpdateUserAdapter(AdminApiAdapter):
    def __init__(self, user_id: str):
        self.user_id = user_id

    async def handle(self, context):  # type: ignore[override]
        db = context.db
        existing_user = UserService.get_user(db, self.user_id)
        if not existing_user:
            raise NotFoundException("用户不存在", "user")

        payload = context.ensure_json_body()
        try:
            request = UpdateUserRequest.model_validate(payload)
        except ValidationError as e:
            errors = e.errors()
            if errors:
                raise InvalidRequestException(translate_pydantic_error(errors[0]))
            raise InvalidRequestException("请求数据验证失败")

        update_data = request.model_dump(exclude_unset=True)
        old_role = existing_user.role
        if "role" in update_data and update_data["role"]:
            if hasattr(update_data["role"], "value"):
                update_data["role"] = update_data["role"]
            else:
                update_data["role"] = UserRole[update_data["role"].upper()]

        user = UserService.update_user(db, self.user_id, **update_data)
        if not user:
            raise NotFoundException("用户不存在", "user")

        # 角色变更时清除热力图缓存（影响 include_actual_cost 权限）
        if "role" in update_data and update_data["role"] != old_role:
            from src.services.usage.service import UsageService

            await UsageService.clear_user_heatmap_cache(self.user_id)

        changed_fields = list(update_data.keys())
        context.add_audit_metadata(
            action="update_user",
            target_user_id=user.id,
            updated_fields=changed_fields,
            role_before=existing_user.role.value if existing_user.role else None,
            role_after=user.role.value,
            quota_usd=user.quota_usd,
            is_active=user.is_active,
        )

        return {
            "id": user.id,
            "email": user.email,
            "username": user.username,
            "role": user.role.value,
            "allowed_providers": user.allowed_providers,
            "allowed_api_formats": user.allowed_api_formats,
            "allowed_models": user.allowed_models,
            "quota_usd": user.quota_usd,
            "used_usd": user.used_usd,
            "total_usd": getattr(user, "total_usd", 0),
            "is_active": user.is_active,
            "created_at": user.created_at.isoformat(),
            "updated_at": user.updated_at.isoformat() if user.updated_at else None,
        }


class AdminDeleteUserAdapter(AdminApiAdapter):
    def __init__(self, user_id: str):
        self.user_id = user_id

    async def handle(self, context):  # type: ignore[override]
        db = context.db
        user = UserService.get_user(db, self.user_id)
        if not user:
            raise HTTPException(status_code=status.HTTP_404_NOT_FOUND, detail="用户不存在")

        if user.role == UserRole.ADMIN:
            admin_count = db.query(User).filter(User.role == UserRole.ADMIN).count()
            if admin_count <= 1:
                raise InvalidRequestException("不能删除最后一个管理员账户")

        success = UserService.delete_user(db, self.user_id)
        if not success:
            raise HTTPException(status_code=status.HTTP_404_NOT_FOUND, detail="用户不存在")

        context.add_audit_metadata(
            action="delete_user",
            target_user_id=user.id,
            target_email=user.email,
            target_role=user.role.value,
        )

        return {"message": "用户删除成功"}


class AdminResetUserQuotaAdapter(AdminApiAdapter):
    def __init__(self, user_id: str):
        self.user_id = user_id

    async def handle(self, context):  # type: ignore[override]
        db = context.db
        user = UserService.get_user(db, self.user_id)
        if not user:
            raise NotFoundException("用户不存在", "user")

        user.used_usd = 0.0
        user.total_usd = getattr(user, "total_usd", 0)
        user.updated_at = datetime.now(timezone.utc)
        db.commit()

        context.add_audit_metadata(
            action="reset_user_quota",
            target_user_id=user.id,
            quota_usd=user.quota_usd,
            used_usd=user.used_usd,
            total_usd=user.total_usd,
        )

        return {
            "message": "配额已重置",
            "user_id": user.id,
            "quota_usd": user.quota_usd,
            "used_usd": user.used_usd,
            "total_usd": user.total_usd,
        }


class AdminGetUserKeysAdapter(AdminApiAdapter):
    """获取用户的API Keys"""

    def __init__(self, user_id: str, is_active: bool | None):
        self.user_id = user_id
        self.is_active = is_active

    async def handle(self, context):  # type: ignore[override]
        db = context.db

        # 验证用户存在
        user = db.query(User).filter(User.id == self.user_id).first()
        if not user:
            raise NotFoundException("用户不存在", "user")

        # 获取用户的Keys（不包括独立Keys）
        api_keys = ApiKeyService.list_user_api_keys(
            db=db, user_id=self.user_id, is_active=self.is_active
        )

        context.add_audit_metadata(
            action="list_user_api_keys",
            target_user_id=self.user_id,
            total=len(api_keys),
        )

        return {
            "api_keys": [
                {
                    "id": key.id,
                    "name": key.name,
                    "key_display": key.get_display_key(),
                    "is_active": key.is_active,
                    "is_locked": key.is_locked,
                    "total_requests": key.total_requests,
                    "total_cost_usd": float(key.total_cost_usd or 0),
                    "rate_limit": key.rate_limit,
                    "expires_at": key.expires_at.isoformat() if key.expires_at else None,
                    "last_used_at": key.last_used_at.isoformat() if key.last_used_at else None,
                    "created_at": key.created_at.isoformat(),
                }
                for key in api_keys
            ],
            "total": len(api_keys),
            "user_email": user.email,
            "username": user.username,
        }


class AdminCreateUserKeyAdapter(AdminApiAdapter):
    """为用户创建API Key"""

    def __init__(self, user_id: str):
        self.user_id = user_id

    async def handle(self, context):  # type: ignore[override]
        db = context.db
        payload = context.ensure_json_body()
        try:
            key_data = CreateApiKeyRequest.model_validate(payload)
        except ValidationError as e:
            errors = e.errors()
            if errors:
                raise InvalidRequestException(translate_pydantic_error(errors[0]))
            raise InvalidRequestException("请求数据验证失败")

        # 验证用户存在
        user = db.query(User).filter(User.id == self.user_id).first()
        if not user:
            raise NotFoundException("用户不存在", "user")

        # 为用户创建Key（不是独立Key）
        api_key, plain_key = ApiKeyService.create_api_key(
            db=db,
            user_id=self.user_id,
            name=key_data.name,
            allowed_providers=key_data.allowed_providers,
            allowed_models=key_data.allowed_models,
            rate_limit=key_data.rate_limit,  # None = 无限制
            expire_days=key_data.expire_days,
            initial_balance_usd=None,  # 普通Key不设置余额限制
            is_standalone=False,  # 不是独立Key
        )

        logger.info(f"管理员为用户创建API Key: 用户 {user.email}, Key ID {api_key.id}")

        context.add_audit_metadata(
            action="create_user_api_key",
            target_user_id=self.user_id,
            key_id=api_key.id,
        )

        return {
            "id": api_key.id,
            "key": plain_key,  # 只在创建时返回
            "name": api_key.name,
            "key_display": api_key.get_display_key(),
            "rate_limit": api_key.rate_limit,
            "expires_at": api_key.expires_at.isoformat() if api_key.expires_at else None,
            "created_at": api_key.created_at.isoformat(),
            "message": "API Key创建成功，请妥善保存完整密钥",
        }


class AdminDeleteUserKeyAdapter(AdminApiAdapter):
    """删除用户的API Key"""

    def __init__(self, user_id: str, key_id: str):
        self.user_id = user_id
        self.key_id = key_id

    async def handle(self, context):  # type: ignore[override]
        db = context.db

        # 验证Key存在且属于该用户
        api_key = (
            db.query(ApiKey)
            .filter(
                ApiKey.id == self.key_id,
                ApiKey.user_id == self.user_id,
                ApiKey.is_standalone == False,  # 只能删除普通Key
            )
            .first()
        )

        if not api_key:
            raise NotFoundException("API Key不存在或不属于该用户", "api_key")

        db.delete(api_key)
        db.commit()

        logger.info(f"管理员删除用户API Key: 用户ID {self.user_id}, Key ID {self.key_id}")

        context.add_audit_metadata(
            action="delete_user_api_key",
            target_user_id=self.user_id,
            key_id=self.key_id,
        )

        return {"message": "API Key已删除"}
