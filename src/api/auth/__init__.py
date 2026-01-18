"""Authentication route group."""

from fastapi import APIRouter

from .routes import router as auth_router

router = APIRouter()
router.include_router(auth_router)

# 通用 OAuth 路由（模块可用时自动注册）
try:
    from .oauth import router as oauth_router

    router.include_router(oauth_router)
except ImportError:
    pass

__all__ = ["router"]
