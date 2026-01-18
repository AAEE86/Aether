"""OAuth 提供商配置管理 API 端点。

支持多个 OAuth 提供商的统一管理。
"""

from typing import Any, Dict, List, Optional

from fastapi import APIRouter, Depends, Request
from pydantic import BaseModel, Field, ValidationError
from sqlalchemy.orm import Session

from src.api.base.admin_adapter import AdminApiAdapter
from src.api.base.pipeline import ApiRequestPipeline
from src.core.crypto import crypto_service
from src.core.exceptions import InvalidRequestException, translate_pydantic_error
from src.core.logger import logger
from src.database import get_db
from src.models.database import AuditEventType, OAuthProviderConfig
from src.services.auth.oauth import OAuthService
from src.services.system.audit import AuditService

router = APIRouter(prefix="/api/admin/oauth", tags=["Admin - OAuth"])
pipeline = ApiRequestPipeline()


# ========== Request/Response Models ==========


class OAuthProviderConfigResponse(BaseModel):
    """OAuth 提供商配置响应（不返回 secret）"""

    provider_id: str
    display_name: str
    authorization_url: str
    token_url: str
    userinfo_url: str
    userinfo_mapping: Optional[Dict[str, str]] = None
    client_id: Optional[str] = None
    redirect_uri: Optional[str] = None
    frontend_callback_url: Optional[str] = None
    scope: Optional[str] = None
    has_client_secret: bool = False
    is_enabled: bool = False
    created_at: Optional[str] = None
    updated_at: Optional[str] = None


class OAuthProviderListResponse(BaseModel):
    """OAuth 提供商列表响应"""

    providers: List[OAuthProviderConfigResponse]


class OAuthProviderConfigCreate(BaseModel):
    """创建 OAuth 提供商配置"""

    provider_id: str = Field(..., min_length=1, max_length=64, pattern=r"^[a-z][a-z0-9_]*$")
    display_name: str = Field(..., min_length=1, max_length=100)
    authorization_url: str = Field(..., min_length=1, max_length=500)
    token_url: str = Field(..., min_length=1, max_length=500)
    userinfo_url: str = Field(..., min_length=1, max_length=500)
    userinfo_mapping: Optional[Dict[str, str]] = None
    client_id: str = Field(..., min_length=1, max_length=255)
    client_secret: str = Field(..., min_length=1, max_length=1024)
    redirect_uri: str = Field(..., min_length=1, max_length=500)
    frontend_callback_url: Optional[str] = Field(None, max_length=500)
    scope: Optional[str] = Field(None, max_length=500)
    is_enabled: bool = False


class OAuthProviderConfigUpdate(BaseModel):
    """更新 OAuth 提供商配置"""

    display_name: Optional[str] = Field(None, min_length=1, max_length=100)
    authorization_url: Optional[str] = Field(None, min_length=1, max_length=500)
    token_url: Optional[str] = Field(None, min_length=1, max_length=500)
    userinfo_url: Optional[str] = Field(None, min_length=1, max_length=500)
    userinfo_mapping: Optional[Dict[str, str]] = None
    client_id: Optional[str] = Field(None, min_length=1, max_length=255)
    client_secret: Optional[str] = Field(None, max_length=1024)  # 空字符串表示清除
    redirect_uri: Optional[str] = Field(None, min_length=1, max_length=500)
    frontend_callback_url: Optional[str] = Field(None, max_length=500)
    scope: Optional[str] = Field(None, max_length=500)
    is_enabled: Optional[bool] = None


class OAuthProviderTestRequest(BaseModel):
    """OAuth 提供商连接测试请求"""

    # 可选覆盖字段
    client_id: Optional[str] = Field(None, min_length=1, max_length=255)
    client_secret: Optional[str] = Field(None, min_length=1)
    authorization_url: Optional[str] = Field(None, min_length=1, max_length=500)
    token_url: Optional[str] = Field(None, min_length=1, max_length=500)
    userinfo_url: Optional[str] = Field(None, min_length=1, max_length=500)
    redirect_uri: Optional[str] = Field(None, min_length=1, max_length=500)


class OAuthProviderTestResponse(BaseModel):
    """OAuth 提供商连接测试响应"""

    success: bool
    message: str


# ========== API Endpoints ==========


@router.get("/providers")
async def list_oauth_providers(request: Request, db: Session = Depends(get_db)) -> Any:
    """
    获取所有 OAuth 提供商配置列表

    **返回字段**:
    - `providers`: 提供商配置列表
    """
    adapter = AdminListOAuthProvidersAdapter()
    return await pipeline.run(adapter=adapter, http_request=request, db=db, mode=adapter.mode)


@router.get("/providers/{provider_id}")
async def get_oauth_provider(
    request: Request, provider_id: str, db: Session = Depends(get_db)
) -> Any:
    """
    获取指定 OAuth 提供商配置

    **路径参数**:
    - `provider_id`: 提供商标识（如 linuxdo, github）

    **返回字段**:
    - 提供商配置详情（不含 client_secret）
    """
    adapter = AdminGetOAuthProviderAdapter(provider_id=provider_id)
    return await pipeline.run(adapter=adapter, http_request=request, db=db, mode=adapter.mode)


@router.post("/providers")
async def create_oauth_provider(request: Request, db: Session = Depends(get_db)) -> Any:
    """
    创建新的 OAuth 提供商配置

    **请求体字段**:
    - `provider_id`: 提供商标识（必填，唯一，小写字母开头，只能包含小写字母、数字、下划线）
    - `display_name`: 显示名称（必填）
    - `authorization_url`: 授权 URL（必填）
    - `token_url`: Token 交换 URL（必填）
    - `userinfo_url`: 用户信息 URL（必填）
    - `userinfo_mapping`: 用户信息字段映射（可选）
    - `client_id`: OAuth Client ID（必填）
    - `client_secret`: OAuth Client Secret（必填）
    - `redirect_uri`: OAuth 回调地址（必填）
    - `frontend_callback_url`: 前端回调地址（可选）
    - `scope`: OAuth scope（可选，默认 user）
    - `is_enabled`: 是否启用（可选，默认 false）
    """
    adapter = AdminCreateOAuthProviderAdapter()
    return await pipeline.run(adapter=adapter, http_request=request, db=db, mode=adapter.mode)


@router.put("/providers/{provider_id}")
async def update_oauth_provider(
    request: Request, provider_id: str, db: Session = Depends(get_db)
) -> Any:
    """
    更新 OAuth 提供商配置

    **路径参数**:
    - `provider_id`: 提供商标识

    **请求体字段**（均为可选）:
    - `display_name`: 显示名称
    - `authorization_url`: 授权 URL
    - `token_url`: Token 交换 URL
    - `userinfo_url`: 用户信息 URL
    - `userinfo_mapping`: 用户信息字段映射
    - `client_id`: OAuth Client ID
    - `client_secret`: OAuth Client Secret（空字符串表示清除）
    - `redirect_uri`: OAuth 回调地址
    - `frontend_callback_url`: 前端回调地址
    - `scope`: OAuth scope
    - `is_enabled`: 是否启用
    """
    adapter = AdminUpdateOAuthProviderAdapter(provider_id=provider_id)
    return await pipeline.run(adapter=adapter, http_request=request, db=db, mode=adapter.mode)


@router.delete("/providers/{provider_id}")
async def delete_oauth_provider(
    request: Request, provider_id: str, db: Session = Depends(get_db)
) -> Any:
    """
    删除 OAuth 提供商配置

    **路径参数**:
    - `provider_id`: 提供商标识

    **警告**: 删除后，使用该提供商登录的用户将无法登录
    """
    adapter = AdminDeleteOAuthProviderAdapter(provider_id=provider_id)
    return await pipeline.run(adapter=adapter, http_request=request, db=db, mode=adapter.mode)


@router.post("/providers/{provider_id}/test")
async def test_oauth_provider(
    request: Request, provider_id: str, db: Session = Depends(get_db)
) -> Any:
    """
    测试 OAuth 提供商连接

    **路径参数**:
    - `provider_id`: 提供商标识

    **请求体字段**（均为可选，用于临时覆盖）:
    - `client_id`: 覆盖 Client ID
    - `client_secret`: 覆盖 Client Secret
    - `authorization_url`: 覆盖授权 URL
    - `token_url`: 覆盖 Token URL
    - `userinfo_url`: 覆盖用户信息 URL
    """
    adapter = AdminTestOAuthProviderAdapter(provider_id=provider_id)
    return await pipeline.run(adapter=adapter, http_request=request, db=db, mode=adapter.mode)


# ========== Adapters ==========


def _config_to_response(config: OAuthProviderConfig) -> Dict[str, Any]:
    """将配置模型转换为响应"""
    return OAuthProviderConfigResponse(
        provider_id=config.provider_id,
        display_name=config.display_name,
        authorization_url=config.authorization_url,
        token_url=config.token_url,
        userinfo_url=config.userinfo_url,
        userinfo_mapping=config.userinfo_mapping,
        client_id=config.client_id,
        redirect_uri=config.redirect_uri,
        frontend_callback_url=config.frontend_callback_url,
        scope=config.scope,
        has_client_secret=bool(config.client_secret_encrypted),
        is_enabled=config.is_enabled,
        created_at=config.created_at.isoformat() if config.created_at else None,
        updated_at=config.updated_at.isoformat() if config.updated_at else None,
    ).model_dump()


class AdminListOAuthProvidersAdapter(AdminApiAdapter):
    async def handle(self, context) -> Dict[str, Any]:  # type: ignore[override]
        db = context.db
        configs = db.query(OAuthProviderConfig).order_by(OAuthProviderConfig.provider_id).all()

        return OAuthProviderListResponse(
            providers=[_config_to_response(config) for config in configs]
        ).model_dump()


class AdminGetOAuthProviderAdapter(AdminApiAdapter):
    def __init__(self, provider_id: str):
        super().__init__()
        self.provider_id = provider_id

    async def handle(self, context) -> Dict[str, Any]:  # type: ignore[override]
        db = context.db
        config = (
            db.query(OAuthProviderConfig)
            .filter(OAuthProviderConfig.provider_id == self.provider_id)
            .first()
        )

        if not config:
            raise InvalidRequestException(f"OAuth 提供商 '{self.provider_id}' 不存在")

        return _config_to_response(config)


class AdminCreateOAuthProviderAdapter(AdminApiAdapter):
    async def handle(self, context) -> Dict[str, Any]:  # type: ignore[override]
        db = context.db
        payload = context.ensure_json_body()

        try:
            config_create = OAuthProviderConfigCreate.model_validate(payload)
        except ValidationError as e:
            errors = e.errors()
            if errors:
                raise InvalidRequestException(translate_pydantic_error(errors[0]))
            raise InvalidRequestException("请求数据验证失败")

        # 检查 provider_id 是否已存在
        existing = (
            db.query(OAuthProviderConfig)
            .filter(OAuthProviderConfig.provider_id == config_create.provider_id)
            .first()
        )
        if existing:
            raise InvalidRequestException(f"OAuth 提供商 '{config_create.provider_id}' 已存在")

        # 创建配置
        config = OAuthProviderConfig(
            provider_id=config_create.provider_id,
            display_name=config_create.display_name,
            authorization_url=config_create.authorization_url,
            token_url=config_create.token_url,
            userinfo_url=config_create.userinfo_url,
            userinfo_mapping=config_create.userinfo_mapping,
            client_id=config_create.client_id,
            redirect_uri=config_create.redirect_uri,
            frontend_callback_url=config_create.frontend_callback_url,
            scope=config_create.scope or "user",
            is_enabled=config_create.is_enabled,
        )
        config.set_client_secret(config_create.client_secret)

        db.add(config)
        db.commit()

        # 记录审计日志
        AuditService.log_event(
            db=db,
            event_type=AuditEventType.CONFIG_CHANGED,
            description=f"OAuth 提供商 '{config_create.provider_id}' 已创建",
            user_id=str(context.user.id) if context.user else None,
            metadata={
                "provider_id": config_create.provider_id,
                "display_name": config_create.display_name,
                "is_enabled": config_create.is_enabled,
            },
        )
        db.commit()

        return {"message": f"OAuth 提供商 '{config_create.provider_id}' 创建成功"}


class AdminUpdateOAuthProviderAdapter(AdminApiAdapter):
    def __init__(self, provider_id: str):
        super().__init__()
        self.provider_id = provider_id

    async def handle(self, context) -> Dict[str, Any]:  # type: ignore[override]
        db = context.db
        payload = context.ensure_json_body()

        try:
            config_update = OAuthProviderConfigUpdate.model_validate(payload)
        except ValidationError as e:
            errors = e.errors()
            if errors:
                raise InvalidRequestException(translate_pydantic_error(errors[0]))
            raise InvalidRequestException("请求数据验证失败")

        # 使用行级锁获取配置
        config = (
            db.query(OAuthProviderConfig)
            .filter(OAuthProviderConfig.provider_id == self.provider_id)
            .with_for_update()
            .first()
        )

        if not config:
            raise InvalidRequestException(f"OAuth 提供商 '{self.provider_id}' 不存在")

        # 计算更新后的 secret 状态
        if config_update.client_secret is None:
            will_have_secret = bool(config.client_secret_encrypted)
        elif config_update.client_secret == "":
            will_have_secret = False
        else:
            will_have_secret = True

        # 启用时必须有 secret
        is_enabled = (
            config_update.is_enabled if config_update.is_enabled is not None else config.is_enabled
        )
        if is_enabled and not will_have_secret:
            raise InvalidRequestException("启用 OAuth 认证需要先设置 Client Secret")

        # 更新字段
        if config_update.display_name is not None:
            config.display_name = config_update.display_name
        if config_update.authorization_url is not None:
            config.authorization_url = config_update.authorization_url
        if config_update.token_url is not None:
            config.token_url = config_update.token_url
        if config_update.userinfo_url is not None:
            config.userinfo_url = config_update.userinfo_url
        if config_update.userinfo_mapping is not None:
            config.userinfo_mapping = config_update.userinfo_mapping
        if config_update.client_id is not None:
            config.client_id = config_update.client_id
        if config_update.redirect_uri is not None:
            config.redirect_uri = config_update.redirect_uri
        if config_update.frontend_callback_url is not None:
            config.frontend_callback_url = config_update.frontend_callback_url
        if config_update.scope is not None:
            config.scope = config_update.scope
        if config_update.is_enabled is not None:
            config.is_enabled = config_update.is_enabled

        secret_changed = None
        if config_update.client_secret is not None:
            if config_update.client_secret == "":
                config.client_secret_encrypted = None
                secret_changed = "cleared"
            else:
                config.client_secret_encrypted = crypto_service.encrypt(config_update.client_secret)
                secret_changed = "updated"

        db.commit()

        # 记录审计日志
        AuditService.log_event(
            db=db,
            event_type=AuditEventType.CONFIG_CHANGED,
            description=f"OAuth 提供商 '{self.provider_id}' 配置已更新",
            user_id=str(context.user.id) if context.user else None,
            metadata={
                "provider_id": self.provider_id,
                "is_enabled": config.is_enabled,
                "secret_changed": secret_changed,
            },
        )
        db.commit()

        return {"message": f"OAuth 提供商 '{self.provider_id}' 配置更新成功"}


class AdminDeleteOAuthProviderAdapter(AdminApiAdapter):
    def __init__(self, provider_id: str):
        super().__init__()
        self.provider_id = provider_id

    async def handle(self, context) -> Dict[str, Any]:  # type: ignore[override]
        db = context.db

        config = (
            db.query(OAuthProviderConfig)
            .filter(OAuthProviderConfig.provider_id == self.provider_id)
            .first()
        )

        if not config:
            raise InvalidRequestException(f"OAuth 提供商 '{self.provider_id}' 不存在")

        db.delete(config)
        db.commit()

        # 记录审计日志
        AuditService.log_event(
            db=db,
            event_type=AuditEventType.CONFIG_CHANGED,
            description=f"OAuth 提供商 '{self.provider_id}' 已删除",
            user_id=str(context.user.id) if context.user else None,
            metadata={"provider_id": self.provider_id},
        )
        db.commit()

        return {"message": f"OAuth 提供商 '{self.provider_id}' 删除成功"}


class AdminTestOAuthProviderAdapter(AdminApiAdapter):
    def __init__(self, provider_id: str):
        super().__init__()
        self.provider_id = provider_id

    async def handle(self, context) -> Dict[str, Any]:  # type: ignore[override]
        db = context.db

        if context.json_body is not None:
            payload = context.json_body
        elif not context.raw_body:
            payload = {}
        else:
            payload = context.ensure_json_body()

        saved_config = (
            db.query(OAuthProviderConfig)
            .filter(OAuthProviderConfig.provider_id == self.provider_id)
            .first()
        )

        if not saved_config:
            raise InvalidRequestException(f"OAuth 提供商 '{self.provider_id}' 不存在")

        try:
            overrides = OAuthProviderTestRequest.model_validate(payload)
        except ValidationError as e:
            errors = e.errors()
            if errors:
                raise InvalidRequestException(translate_pydantic_error(errors[0]))
            raise InvalidRequestException("请求数据验证失败")

        # 构建测试配置
        config_data: Dict[str, Any] = {
            "provider_id": saved_config.provider_id,
            "client_id": saved_config.client_id,
            "authorization_url": saved_config.authorization_url,
            "token_url": saved_config.token_url,
            "userinfo_url": saved_config.userinfo_url,
            "redirect_uri": saved_config.redirect_uri,
        }

        # 应用覆盖值
        for field in ["client_id", "authorization_url", "token_url", "userinfo_url", "redirect_uri"]:
            value = getattr(overrides, field)
            if value is not None:
                config_data[field] = value

        # client_secret 优先使用 overrides
        if overrides.client_secret is not None:
            config_data["client_secret"] = overrides.client_secret
        elif saved_config.client_secret_encrypted:
            try:
                config_data["client_secret"] = crypto_service.decrypt(
                    saved_config.client_secret_encrypted
                )
            except Exception as e:
                logger.error(f"Client Secret 解密失败: {type(e).__name__}: {e}")
                return OAuthProviderTestResponse(
                    success=False, message="Client Secret 解密失败，请检查配置或重新设置"
                ).model_dump()

        # 必填字段检查
        required_fields = [
            "client_id",
            "client_secret",
            "authorization_url",
            "token_url",
            "userinfo_url",
            "redirect_uri",
        ]
        missing = [f for f in required_fields if not config_data.get(f)]
        if missing:
            return OAuthProviderTestResponse(
                success=False, message=f"缺少必要字段: {', '.join(missing)}"
            ).model_dump()

        success, message = await OAuthService.test_connection(config_data)
        return OAuthProviderTestResponse(success=success, message=message).model_dump()
