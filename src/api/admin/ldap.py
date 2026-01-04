"""LDAP配置管理API端点。"""

from typing import Any, Dict, Optional

from fastapi import APIRouter, Depends, Request
from pydantic import BaseModel, Field, ValidationError
from sqlalchemy.orm import Session

from src.api.base.admin_adapter import AdminApiAdapter
from src.api.base.pipeline import ApiRequestPipeline
from src.core.crypto import crypto_service
from src.core.exceptions import InvalidRequestException, translate_pydantic_error
from src.database import get_db
from src.models.database import LDAPConfig

router = APIRouter(prefix="/api/admin/ldap", tags=["Admin - LDAP"])
pipeline = ApiRequestPipeline()


# ========== Request/Response Models ==========


class LDAPConfigResponse(BaseModel):
    """LDAP配置响应（不返回密码）"""

    server_url: Optional[str] = None
    bind_dn: Optional[str] = None
    base_dn: Optional[str] = None
    user_search_filter: str
    username_attr: str
    email_attr: str
    display_name_attr: str
    is_enabled: bool
    is_exclusive: bool
    use_starttls: bool


class LDAPConfigUpdate(BaseModel):
    """LDAP配置更新请求"""

    server_url: str = Field(..., min_length=1, max_length=255)
    bind_dn: str = Field(..., min_length=1, max_length=255)
    bind_password: Optional[str] = Field(None, min_length=1)
    base_dn: str = Field(..., min_length=1, max_length=255)
    user_search_filter: str = Field(default="(uid={username})", max_length=500)
    username_attr: str = Field(default="uid", max_length=50)
    email_attr: str = Field(default="mail", max_length=50)
    display_name_attr: str = Field(default="cn", max_length=50)
    is_enabled: bool = False
    is_exclusive: bool = False
    use_starttls: bool = False


class LDAPTestResponse(BaseModel):
    """LDAP连接测试响应"""

    success: bool
    message: str


# ========== API Endpoints ==========


@router.get("/config")
async def get_ldap_config(request: Request, db: Session = Depends(get_db)) -> Any:
    """获取LDAP配置（管理员）"""
    adapter = AdminGetLDAPConfigAdapter()
    return await pipeline.run(adapter=adapter, http_request=request, db=db, mode=adapter.mode)


@router.put("/config")
async def update_ldap_config(request: Request, db: Session = Depends(get_db)) -> Any:
    """更新LDAP配置（管理员）"""
    adapter = AdminUpdateLDAPConfigAdapter()
    return await pipeline.run(adapter=adapter, http_request=request, db=db, mode=adapter.mode)


@router.post("/test")
async def test_ldap_connection(request: Request, db: Session = Depends(get_db)) -> Any:
    """测试LDAP连接（管理员）"""
    adapter = AdminTestLDAPConnectionAdapter()
    return await pipeline.run(adapter=adapter, http_request=request, db=db, mode=adapter.mode)


# ========== Adapters ==========


class AdminGetLDAPConfigAdapter(AdminApiAdapter):
    async def handle(self, context) -> Dict[str, Any]:  # type: ignore[override]
        db = context.db
        config = db.query(LDAPConfig).first()

        if not config:
            return LDAPConfigResponse(
                server_url=None,
                bind_dn=None,
                base_dn=None,
                user_search_filter="(uid={username})",
                username_attr="uid",
                email_attr="mail",
                display_name_attr="cn",
                is_enabled=False,
                is_exclusive=False,
                use_starttls=False,
            ).model_dump()

        return LDAPConfigResponse(
            server_url=config.server_url,
            bind_dn=config.bind_dn,
            base_dn=config.base_dn,
            user_search_filter=config.user_search_filter,
            username_attr=config.username_attr,
            email_attr=config.email_attr,
            display_name_attr=config.display_name_attr,
            is_enabled=config.is_enabled,
            is_exclusive=config.is_exclusive,
            use_starttls=config.use_starttls,
        ).model_dump()


class AdminUpdateLDAPConfigAdapter(AdminApiAdapter):
    async def handle(self, context) -> Dict[str, str]:  # type: ignore[override]
        db = context.db
        payload = context.ensure_json_body()

        try:
            config_update = LDAPConfigUpdate.model_validate(payload)
        except ValidationError as e:
            errors = e.errors()
            if errors:
                raise InvalidRequestException(translate_pydantic_error(errors[0]))
            raise InvalidRequestException("请求数据验证失败")

        config = db.query(LDAPConfig).first()
        is_new_config = config is None

        if is_new_config:
            # 首次创建配置时必须提供密码
            if not config_update.bind_password:
                raise InvalidRequestException("首次配置 LDAP 时必须设置绑定密码")
            config = LDAPConfig()
            db.add(config)

        config.server_url = config_update.server_url
        config.bind_dn = config_update.bind_dn
        config.base_dn = config_update.base_dn
        config.user_search_filter = config_update.user_search_filter
        config.username_attr = config_update.username_attr
        config.email_attr = config_update.email_attr
        config.display_name_attr = config_update.display_name_attr
        config.is_enabled = config_update.is_enabled
        config.is_exclusive = config_update.is_exclusive
        config.use_starttls = config_update.use_starttls

        if config_update.bind_password:
            config.bind_password_encrypted = crypto_service.encrypt(config_update.bind_password)

        db.commit()

        return {"message": "LDAP配置更新成功"}


class AdminTestLDAPConnectionAdapter(AdminApiAdapter):
    async def handle(self, context) -> Dict[str, Any]:  # type: ignore[override]
        db = context.db
        config = db.query(LDAPConfig).first()

        if not config:
            return LDAPTestResponse(success=False, message="LDAP配置不存在").model_dump()

        try:
            import ldap3

            bind_password = crypto_service.decrypt(config.bind_password_encrypted)

            use_ssl = config.server_url.startswith("ldaps://")
            server = ldap3.Server(config.server_url, use_ssl=use_ssl, get_info=ldap3.ALL)
            conn = ldap3.Connection(server, user=config.bind_dn, password=bind_password)

            if config.use_starttls and not use_ssl:
                conn.start_tls()

            if not conn.bind():
                return LDAPTestResponse(
                    success=False, message=f"绑定失败: {conn.result}"
                ).model_dump()

            conn.unbind()
            return LDAPTestResponse(success=True, message="LDAP连接测试成功").model_dump()

        except ImportError:
            return LDAPTestResponse(success=False, message="ldap3库未安装").model_dump()
        except Exception as e:
            return LDAPTestResponse(success=False, message=f"连接失败: {str(e)}").model_dump()
