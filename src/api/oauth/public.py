"""OAuth 公开端点（无需登录）。"""


from typing import Any

from fastapi import APIRouter, Depends, Query, status
from sqlalchemy.orm import Session
from starlette.responses import RedirectResponse

from src.database import get_db
from src.services.auth.oauth.service import OAuthService

router = APIRouter(prefix="/api/oauth", tags=["OAuth"])


@router.get("/providers")
async def list_oauth_providers(db: Session = Depends(get_db)) -> dict[str, Any]:
    """
    获取可用 OAuth Providers 列表。

    模块未启用时返回空列表（前端友好）。
    """
    providers = await OAuthService.list_public_providers(db)
    return {"providers": providers}


@router.get("/{provider_type}/authorize")
async def oauth_authorize(provider_type: str, db: Session = Depends(get_db)) -> RedirectResponse:
    """
    发起 OAuth 登录（login flow）。
    """
    url = await OAuthService.build_login_authorize_url(db, provider_type)
    return RedirectResponse(url=url, status_code=status.HTTP_302_FOUND)


@router.get("/{provider_type}/callback")
async def oauth_callback(
    provider_type: str,
    db: Session = Depends(get_db),
    code: str | None = Query(None),
    state: str | None = Query(None),
    error: str | None = Query(None),
    error_description: str | None = Query(None),
) -> RedirectResponse:
    """
    OAuth 回调端点。

    成功/失败都会重定向到前端回调页。
    """
    redirect_url = await OAuthService.handle_callback(
        db=db,
        provider_type=provider_type,
        state=state or "",
        code=code,
        error=error,
        error_description=error_description,
    )
    return RedirectResponse(url=redirect_url, status_code=status.HTTP_302_FOUND)
