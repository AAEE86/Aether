"""Provider admin routes export."""

from fastapi import APIRouter

from .models import router as models_router
from .routes import router as routes_router
from .summary import router as summary_router
from ..keys import provider_keys_router as keys_router

router = APIRouter(prefix="/api/admin/providers", tags=["Admin - Providers"])

# Provider CRUD
router.include_router(routes_router)

# Provider Shared Keys
router.include_router(keys_router)

# Provider summary & health monitor
router.include_router(summary_router)

# Provider models management
router.include_router(models_router)

__all__ = ["router"]
