"""Endpoint management API routers."""

from fastapi import APIRouter

from .concurrency import router as concurrency_router
from .health import router as health_router
from ..keys import endpoint_keys_router as keys_router
from .routes import router as routes_router

router = APIRouter(prefix="/api/admin/endpoints", tags=["Admin - Endpoints"])

# Endpoint CRUD
router.include_router(routes_router)

# Endpoint Keys management
router.include_router(keys_router)

# Health monitoring
router.include_router(health_router)

# Concurrency control
router.include_router(concurrency_router)

__all__ = ["router"]
