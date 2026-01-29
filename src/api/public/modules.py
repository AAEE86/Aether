"""公开模块状态 API（供登录页等使用）"""


from fastapi import APIRouter, Depends
from pydantic import BaseModel
from sqlalchemy.orm import Session

from src.core.modules import get_module_registry
from src.database import get_db

router = APIRouter(prefix="/api/modules", tags=["Modules"])


class AuthModuleInfo(BaseModel):
    """认证模块简要信息"""

    name: str
    display_name: str
    active: bool


@router.get("/auth-status", response_model=list[AuthModuleInfo])
async def get_auth_modules_status(db: Session = Depends(get_db)):
    """
    获取认证模块状态（公开接口）

    供登录页使用，返回所有可用的认证模块及其激活状态。
    不需要认证即可访问。

    **返回字段**:
    - `name`: 模块名称
    - `display_name`: 显示名称
    - `active`: 是否激活
    """
    registry = get_module_registry()
    auth_modules = registry.get_auth_modules_status(db)

    return [
        AuthModuleInfo(
            name=status.name,
            display_name=status.display_name,
            active=status.active,
        )
        for status in auth_modules
    ]
