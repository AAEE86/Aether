"""Admin API routers."""

from fastapi import APIRouter

from .adaptive import router as adaptive_router
from .api_keys import router as api_keys_router
from .billing import router as billing_router
from .endpoints import router as endpoints_router
from .management_tokens import router as management_tokens_router
from .models import router as models_router
from .modules import router as modules_router
from .monitoring import router as monitoring_router
from .oauth2_callback import router as oauth2_callback_router
from .provider_ops import router as provider_ops_router
from .provider_query import router as provider_query_router
from .provider_strategy import router as provider_strategy_router
from .providers import router as providers_router
from .security import router as security_router
from .system import router as system_router
from .usage import router as usage_router
from .users import router as users_router
from .video_tasks import router as video_tasks_router

router = APIRouter()
router.include_router(system_router)
router.include_router(users_router)
router.include_router(providers_router)
router.include_router(api_keys_router)
router.include_router(billing_router)
router.include_router(usage_router)
router.include_router(monitoring_router)
router.include_router(endpoints_router)
router.include_router(provider_strategy_router)
router.include_router(adaptive_router)
router.include_router(models_router)
router.include_router(security_router)
router.include_router(provider_query_router)
router.include_router(management_tokens_router)
router.include_router(modules_router)
router.include_router(provider_ops_router)
router.include_router(video_tasks_router)
router.include_router(oauth2_callback_router)

# 注意：ldap_router 已迁移到模块系统，由 ModuleRegistry 动态注册
# 当 LDAP_AVAILABLE=true 时才会注册路由

__all__ = ["router"]
