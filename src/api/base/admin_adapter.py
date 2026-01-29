from fastapi import HTTPException

from src.models.database import UserRole

from .adapter import ApiAdapter, ApiMode
from .context import ApiRequestContext


class AdminApiAdapter(ApiAdapter):
    """管理员端点适配器基类，提供统一的权限校验。"""

    mode = ApiMode.ADMIN
    required_roles: tuple[UserRole, ...] = (UserRole.ADMIN,)

    def authorize(self, context: ApiRequestContext) -> None:
        user = context.user
        if not user:
            raise HTTPException(status_code=401, detail="未登录")

        # 检查是否使用独立余额Key访问管理接口
        if context.api_key and context.api_key.is_standalone:
            raise HTTPException(
                status_code=403, detail="独立余额Key不允许访问管理接口，仅可用于代理请求"
            )

        if not any(user.role == role for role in self.required_roles):
            raise HTTPException(status_code=403, detail="需要管理员权限")
