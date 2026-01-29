"""
功能模块注册

所有可选功能模块在此注册
"""

from src.core.modules.base import ModuleDefinition

# 导入所有模块定义
from src.modules.ldap import ldap_module
from src.modules.oauth import oauth_module

# 所有模块列表
ALL_MODULES: list[ModuleDefinition] = [
    ldap_module,
    oauth_module,
]

__all__ = ["ALL_MODULES"]
