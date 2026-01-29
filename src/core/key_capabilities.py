"""
Key 能力系统

能力类型：
1. 互斥能力 (EXCLUSIVE): 需要时选有的，不需要时选没有的（如 cache_1h）
2. 兼容能力 (COMPATIBLE): 需要时选有的，不需要时都可选（如 context_1m）

配置模式：
1. user_configurable: 用户可配置（模型级 + Key级强制）
2. auto_detect: 自动检测（请求失败后升级）
3. request_param: 从请求参数检测
"""

from dataclasses import dataclass, field
from enum import Enum


class CapabilityMatchMode(Enum):
    """能力匹配模式"""

    EXCLUSIVE = "exclusive"  # 互斥：需要时选有的，不需要时选没有的
    COMPATIBLE = "compatible"  # 兼容：需要时选有的，不需要时都可选


class CapabilityConfigMode(Enum):
    """能力配置模式"""

    USER_CONFIGURABLE = "user_configurable"  # 用户可配置（模型级 + Key级强制）
    AUTO_DETECT = "auto_detect"  # 自动检测（请求失败后升级）
    REQUEST_PARAM = "request_param"  # 从请求参数检测


@dataclass
class CapabilityDefinition:
    """能力定义"""

    name: str
    display_name: str
    description: str
    match_mode: CapabilityMatchMode
    config_mode: CapabilityConfigMode
    short_name: str = ""  # 简短展示名称（用于列表等紧凑场景）
    error_patterns: list[str] = field(default_factory=list)  # 错误检测关键词组


# ============ 能力注册表 ============

_capabilities: dict[str, CapabilityDefinition] = {}


def register_capability(
    name: str,
    display_name: str,
    description: str,
    match_mode: CapabilityMatchMode,
    config_mode: CapabilityConfigMode,
    short_name: str = "",
    error_patterns: list[str] | None = None,
) -> CapabilityDefinition:
    """注册能力"""
    cap = CapabilityDefinition(
        name=name,
        display_name=display_name,
        description=description,
        match_mode=match_mode,
        config_mode=config_mode,
        short_name=short_name or display_name,  # 默认使用 display_name
        error_patterns=error_patterns or [],
    )
    _capabilities[name] = cap
    return cap


def get_capability(name: str) -> CapabilityDefinition | None:
    """获取能力定义"""
    return _capabilities.get(name)


def get_all_capabilities() -> list[CapabilityDefinition]:
    """获取所有能力定义"""
    return list(_capabilities.values())


def get_user_configurable_capabilities() -> list[CapabilityDefinition]:
    """获取用户可配置的能力列表"""
    return [c for c in _capabilities.values() if c.config_mode == CapabilityConfigMode.USER_CONFIGURABLE]


# ============ 能力匹配检查 ============


def check_capability_match(
    key_capabilities: dict[str, bool] | None,
    requirements: dict[str, bool] | None,
) -> tuple[bool, str | None]:
    """
    检查 Key 能力是否满足需求

    匹配逻辑：
    1. EXCLUSIVE（互斥）能力：
       - 请求需要且 Key 有 → 通过
       - 请求需要但 Key 没有 → 拒绝
       - 请求不需要但 Key 有 → 拒绝（避免浪费高价资源）
       - 请求不需要且 Key 没有 → 通过
       - 请求未声明但 Key 有 → 拒绝（关键：未声明等同于不需要）

    2. COMPATIBLE（兼容）能力：
       - 请求需要且 Key 有 → 通过
       - 请求需要但 Key 没有 → 拒绝
       - 请求不需要/未声明且 Key 有 → 通过（无额外成本，不浪费）
       - 请求不需要/未声明且 Key 没有 → 通过

    Args:
        key_capabilities: Key 拥有的能力 {"cache_1h": True, ...}
        requirements: 请求需要的能力 {"cache_1h": True, "context_1m": False}

    Returns:
        (is_match, skip_reason) - 是否匹配及跳过原因
    """
    key_caps = key_capabilities or {}
    reqs = requirements or {}

    # 第一步：检查请求声明的需求
    for cap_name, is_required in reqs.items():
        cap_def = _capabilities.get(cap_name)
        if not cap_def:
            continue

        key_has_cap = key_caps.get(cap_name, False)

        if cap_def.match_mode == CapabilityMatchMode.EXCLUSIVE:
            if is_required and not key_has_cap:
                return False, f"需要{cap_def.display_name}但 Key 不支持"
            if not is_required and key_has_cap:
                return False, f"不需要{cap_def.display_name}(避免浪费高价资源)"

        elif cap_def.match_mode == CapabilityMatchMode.COMPATIBLE:
            if is_required and not key_has_cap:
                return False, f"需要{cap_def.display_name}但 Key 不支持"

    # 第二步：检查 Key 拥有的 EXCLUSIVE 能力是否被请求需要
    # 如果 Key 有某个 EXCLUSIVE 能力，但请求没有声明需要，应该跳过这个 Key
    for cap_name, key_has_cap in key_caps.items():
        if not key_has_cap:
            continue

        cap_def = _capabilities.get(cap_name)
        if not cap_def:
            continue

        if cap_def.match_mode == CapabilityMatchMode.EXCLUSIVE:
            # 如果请求没有声明需要这个 EXCLUSIVE 能力，视为不需要
            if cap_name not in reqs:
                return False, f"不需要{cap_def.display_name}(避免浪费高价资源)"

    return True, None


def _match_error_patterns(error_msg: str, patterns: list[str]) -> bool:
    """检查错误信息是否匹配模式（所有关键词都要出现）"""
    if not patterns:
        return False
    msg_lower = error_msg.lower()
    return all(p.lower() in msg_lower for p in patterns)


def detect_capability_upgrade_from_error(
    error_msg: str,
    current_requirements: dict[str, bool] | None = None,
) -> str | None:
    """
    从错误信息检测是否需要升级某能力

    Args:
        error_msg: 错误信息
        current_requirements: 当前已有的能力需求

    Returns:
        需要升级的能力名称，如果不需要升级则返回 None
    """
    current_reqs = current_requirements or {}

    for cap in _capabilities.values():
        if not current_reqs.get(cap.name) and cap.error_patterns:
            if _match_error_patterns(error_msg, cap.error_patterns):
                return cap.name

    return None


# ============ 兼容性别名 ============

# 保留旧 API 兼容
get_capability_definition = get_capability


class _CapabilityDefinitionsProxy:
    """CAPABILITY_DEFINITIONS 代理，提供字典式访问（兼容旧代码）"""

    def get(self, name: str) -> CapabilityDefinition | None:
        return _capabilities.get(name)

    def __getitem__(self, name: str) -> CapabilityDefinition:
        result = _capabilities.get(name)
        if result is None:
            raise KeyError(name)
        return result

    def __contains__(self, name: str) -> bool:
        return name in _capabilities

    def values(self) -> list[CapabilityDefinition]:
        return list(_capabilities.values())

    def items(self) -> list[tuple[str, CapabilityDefinition]]:
        return list(_capabilities.items())


CAPABILITY_DEFINITIONS = _CapabilityDefinitionsProxy()


# ============ 兼容旧的插件基类（逐步废弃） ============

CapabilityPlugin = CapabilityDefinition  # 类型别名，兼容旧代码


# ============ 注册内置能力 ============

register_capability(
    name="cache_1h",
    display_name="1 小时缓存",
    description="使用 1 小时缓存 TTL（价格更高，适合长对话）",
    match_mode=CapabilityMatchMode.EXCLUSIVE,
    config_mode=CapabilityConfigMode.USER_CONFIGURABLE,
    short_name="1h缓存",
)

register_capability(
    name="context_1m",
    display_name="CLI 1M 上下文",
    description="支持 1M tokens 上下文窗口",
    match_mode=CapabilityMatchMode.COMPATIBLE,
    config_mode=CapabilityConfigMode.REQUEST_PARAM,
    short_name="CLI 1M",
    error_patterns=["context", "token", "length", "exceed"],  # 上下文超限错误
)
