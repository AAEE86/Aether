"""
统一的 API Keys 管理模块

该模块合并了原本分离的 Provider 共享 Keys 和 Endpoint 专用 Keys 管理功能,
通过统一的 Adapters 减少代码重复,同时保持清晰的路由结构。

模块结构:
- adapters.py: 统一的业务逻辑处理层
- provider_routes.py: Provider 共享 Keys 的路由
- endpoint_routes.py: Endpoint 专用 Keys 的路由
"""

from .provider_routes import router as provider_keys_router
from .endpoint_routes import router as endpoint_keys_router

__all__ = [
    "provider_keys_router",
    "endpoint_keys_router",
]