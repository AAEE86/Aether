"""
API 格式元数据定义

集中维护 API 格式的元数据，避免新增格式时到处修改常量。

使用方式：
    # 解析格式别名
    from src.core.api_format import resolve_api_format
    api_format = resolve_api_format("claude")  # -> APIFormat.CLAUDE

    # 获取格式定义
    from src.core.api_format import get_api_format_definition
    definition = get_api_format_definition(APIFormat.CLAUDE)
"""


import re
from dataclasses import dataclass, field
from functools import lru_cache
from types import MappingProxyType
from collections.abc import Iterable, Mapping, MutableMapping, Sequence

from .enums import APIFormat


@dataclass(frozen=True)
class ApiFormatDefinition:
    """
    描述一个 API 格式的所有通用信息。

    - aliases: 用于 detect_api_format 的 provider 别名或快捷名称
    - default_path: 上游默认请求路径（如 /v1/messages），可通过 Endpoint.custom_path 覆盖
    - path_prefix: 本站路径前缀（如 /claude, /openai），为空表示无前缀
    - auth_header: 认证头名称 (如 "x-api-key", "x-goog-api-key")
    - auth_type: 认证类型 ("header" 直接放值, "bearer" 加 Bearer 前缀)
    - extra_headers: 该格式必须携带的额外头部（如 anthropic-version）
    - protected_keys: 不应被 extra_headers 覆盖的头部（小写）
    - model_in_body: 是否需要在请求体中包含 model 字段（Gemini 等格式通过 URL 传递模型名）
    - stream_in_body: 是否需要在请求体中包含 stream 字段（Gemini 等格式通过 URL 端点区分流式）
    - data_format_id: 数据格式标识，相同 ID 的格式数据结构相同可以透传，不同则需要转换
                      例如：CLAUDE/CLAUDE_CLI 都是 "claude"，可以透传
                           OPENAI 是 "openai_chat"，OPENAI_CLI 是 "openai_responses"，需要转换
    """

    api_format: APIFormat
    aliases: Sequence[str] = field(default_factory=tuple)
    default_path: str = "/"  # 上游默认请求路径
    path_prefix: str = ""  # 本站路径前缀，为空表示无前缀
    auth_header: str = "Authorization"
    auth_type: str = "bearer"  # "bearer" or "header"
    extra_headers: Mapping[str, str] = field(default_factory=dict)  # 格式必须的额外头部
    protected_keys: frozenset[str] = field(default_factory=frozenset)  # 受保护的头部 key（小写）
    model_in_body: bool = True  # 是否需要在请求体中包含 model 字段
    stream_in_body: bool = True  # 是否需要在请求体中包含 stream 字段
    data_format_id: str = ""  # 数据格式标识，相同 ID 可透传，不同需转换

    def iter_aliases(self) -> Iterable[str]:
        """返回大小写统一后的别名集合，包含枚举名本身。"""
        yield normalize_alias_value(self.api_format.value)
        for alias in self.aliases:
            normalized = normalize_alias_value(alias)
            if normalized:
                yield normalized


_DEFINITIONS: dict[APIFormat, ApiFormatDefinition] = {
    APIFormat.CLAUDE: ApiFormatDefinition(
        api_format=APIFormat.CLAUDE,
        aliases=("claude", "anthropic", "claude_compatible"),
        default_path="/v1/messages",
        path_prefix="",  # 通过请求头区分格式，不使用路径前缀
        auth_header="x-api-key",
        auth_type="header",
        extra_headers={"anthropic-version": "2023-06-01"},
        protected_keys=frozenset({"x-api-key", "content-type", "anthropic-version"}),
        data_format_id="claude",  # CLAUDE/CLAUDE_CLI 数据格式相同
    ),
    APIFormat.CLAUDE_CLI: ApiFormatDefinition(
        api_format=APIFormat.CLAUDE_CLI,
        aliases=("claude_cli", "claude-cli"),
        default_path="/v1/messages",
        path_prefix="",  # 与 CLAUDE 共享入口，通过 header 区分
        auth_header="Authorization",
        auth_type="bearer",
        protected_keys=frozenset({"authorization", "content-type"}),
        data_format_id="claude",  # CLAUDE/CLAUDE_CLI 数据格式相同
    ),
    APIFormat.OPENAI: ApiFormatDefinition(
        api_format=APIFormat.OPENAI,
        aliases=(
            "openai",
            "deepseek",
            "grok",
            "moonshot",
            "zhipu",
            "qwen",
            "baichuan",
            "minimax",
            "openai_compatible",
        ),
        default_path="/v1/chat/completions",
        path_prefix="",  # 默认格式
        auth_header="Authorization",
        auth_type="bearer",
        protected_keys=frozenset({"authorization", "content-type"}),
        data_format_id="openai_chat",  # Chat Completions API 格式
    ),
    APIFormat.OPENAI_CLI: ApiFormatDefinition(
        api_format=APIFormat.OPENAI_CLI,
        aliases=("openai_cli", "responses"),
        default_path="/responses",
        path_prefix="",  # 与 OPENAI 共享入口
        auth_header="Authorization",
        auth_type="bearer",
        protected_keys=frozenset({"authorization", "content-type"}),
        data_format_id="openai_responses",  # Responses API 格式，与 OPENAI 不同需转换
    ),
    APIFormat.GEMINI: ApiFormatDefinition(
        api_format=APIFormat.GEMINI,
        aliases=("gemini", "google", "vertex"),
        default_path="/v1beta/models/{model}:{action}",
        path_prefix="",  # 通过请求头区分格式
        auth_header="x-goog-api-key",
        auth_type="header",
        protected_keys=frozenset({"x-goog-api-key", "content-type"}),
        model_in_body=False,  # Gemini 通过 URL 路径传递模型名
        stream_in_body=False,  # Gemini 通过 URL 端点区分流式（streamGenerateContent vs generateContent）
        data_format_id="gemini",  # GEMINI/GEMINI_CLI 数据格式相同
    ),
    APIFormat.GEMINI_CLI: ApiFormatDefinition(
        api_format=APIFormat.GEMINI_CLI,
        aliases=("gemini_cli", "gemini-cli"),
        default_path="/v1beta/models/{model}:{action}",
        path_prefix="",  # 与 GEMINI 共享入口
        auth_header="x-goog-api-key",
        auth_type="header",
        protected_keys=frozenset({"x-goog-api-key", "content-type"}),
        model_in_body=False,  # Gemini 通过 URL 路径传递模型名
        stream_in_body=False,  # Gemini 通过 URL 端点区分流式
        data_format_id="gemini",  # GEMINI/GEMINI_CLI 数据格式相同
    ),
}

# 对外只暴露只读视图，避免被随意修改
API_FORMAT_DEFINITIONS: Mapping[APIFormat, ApiFormatDefinition] = MappingProxyType(_DEFINITIONS)


def get_api_format_definition(api_format: APIFormat) -> ApiFormatDefinition:
    """获取指定格式的定义，不存在时抛出 KeyError。"""
    return API_FORMAT_DEFINITIONS[api_format]


def list_api_format_definitions() -> list[ApiFormatDefinition]:
    """返回所有定义的浅拷贝列表，供遍历使用。"""
    return list(API_FORMAT_DEFINITIONS.values())


def build_alias_lookup() -> dict[str, APIFormat]:
    """
    构建 alias -> APIFormat 的查找表。
    每次调用都会返回新的 dict，避免可变全局引发并发问题。
    """
    lookup: MutableMapping[str, APIFormat] = {}
    for definition in API_FORMAT_DEFINITIONS.values():
        for alias in definition.iter_aliases():
            lookup.setdefault(alias, definition.api_format)
    return dict(lookup)


def get_default_path(api_format: APIFormat) -> str:
    """
    获取该格式的上游默认请求路径。

    可通过 Endpoint.custom_path 覆盖。
    """
    definition = API_FORMAT_DEFINITIONS.get(api_format)
    return definition.default_path if definition else "/"


def get_local_path(api_format: APIFormat) -> str:
    """
    获取该格式的本站入口路径。

    本站入口路径 = path_prefix + default_path
    例如：path_prefix="/openai" + default_path="/v1/chat/completions" -> "/openai/v1/chat/completions"
    """
    definition = API_FORMAT_DEFINITIONS.get(api_format)
    if definition:
        prefix = definition.path_prefix or ""
        return prefix + definition.default_path
    return "/"


def get_auth_config(api_format: APIFormat) -> tuple[str, str]:
    """
    获取该格式的认证配置。

    Returns:
        (auth_header, auth_type) 元组
        - auth_header: 认证头名称
        - auth_type: "bearer" 或 "header"
    """
    definition = API_FORMAT_DEFINITIONS.get(api_format)
    if definition:
        return definition.auth_header, definition.auth_type
    return "Authorization", "bearer"


def get_extra_headers(api_format: APIFormat) -> Mapping[str, str]:
    """
    获取该格式必须携带的额外头部。

    例如 Claude 需要 anthropic-version 头部。

    Returns:
        额外头部字典（只读）
    """
    definition = API_FORMAT_DEFINITIONS.get(api_format)
    if definition:
        return definition.extra_headers
    return {}


def get_protected_keys(api_format: APIFormat) -> frozenset[str]:
    """
    获取该格式的受保护头部 key（小写）。

    这些头部不应被 extra_headers 覆盖。

    Returns:
        受保护的头部 key 集合
    """
    definition = API_FORMAT_DEFINITIONS.get(api_format)
    if definition:
        return definition.protected_keys
    return frozenset({"authorization", "content-type"})


def get_data_format_id(api_format: str | APIFormat) -> str:
    """
    获取格式的数据格式标识。

    相同 data_format_id 的格式数据结构相同，可以透传；不同则需要转换。

    Args:
        api_format: API 格式（字符串或枚举）

    Returns:
        数据格式标识，未找到时返回格式名称本身（小写）
    """
    # 统一转换为 APIFormat 枚举
    if isinstance(api_format, str):
        resolved = resolve_api_format(api_format)
        if resolved is None:
            # 未知格式：返回小写，与已定义格式的 data_format_id 风格一致
            return api_format.lower()
        api_format = resolved

    definition = API_FORMAT_DEFINITIONS.get(api_format)
    if definition and definition.data_format_id:
        return definition.data_format_id
    # 兜底：返回格式名称本身（小写）
    return api_format.value.lower()


def can_passthrough(client_format: str | APIFormat, endpoint_format: str | APIFormat) -> bool:
    """
    判断两个格式之间是否可以透传（不需要数据转换）。

    透传条件：
    1. 格式完全相同
    2. data_format_id 相同（如 CLAUDE 和 CLAUDE_CLI 都是 "claude"）

    Args:
        client_format: 客户端请求格式
        endpoint_format: 端点 API 格式

    Returns:
        True 表示可以透传，False 表示需要转换
    """
    # 统一转换为字符串比较
    client_str = client_format.value if isinstance(client_format, APIFormat) else str(client_format).upper()
    endpoint_str = endpoint_format.value if isinstance(endpoint_format, APIFormat) else str(endpoint_format).upper()

    # 完全相同
    if client_str == endpoint_str:
        return True

    # 检查 data_format_id
    client_data_id = get_data_format_id(client_format)
    endpoint_data_id = get_data_format_id(endpoint_format)
    return client_data_id == endpoint_data_id


@lru_cache(maxsize=1)
def _alias_lookup_cache() -> dict[str, APIFormat]:
    """缓存 alias -> APIFormat 查找表，减少重复构建。"""
    return build_alias_lookup()


def resolve_api_format_alias(value: str) -> APIFormat | None:
    """根据别名查找 APIFormat，找不到时返回 None。"""
    if not value:
        return None
    normalized = normalize_alias_value(value)
    if not normalized:
        return None
    return _alias_lookup_cache().get(normalized)


def resolve_api_format(
    value: str | APIFormat | None,
    default: APIFormat | None = None,
) -> APIFormat | None:
    """
    将任意字符串/枚举值解析为 APIFormat。

    Args:
        value: 可以是 APIFormat 或任意字符串/别名
        default: 未解析成功时返回的默认值
    """
    if isinstance(value, APIFormat):
        return value
    if isinstance(value, str):
        stripped = value.strip()
        if not stripped:
            return default
        upper = stripped.upper()
        if upper in APIFormat.__members__:
            return APIFormat[upper]
        alias = resolve_api_format_alias(stripped)
        if alias:
            return alias
    return default


def register_api_format_definition(definition: ApiFormatDefinition, *, override: bool = False) -> None:
    """
    注册或覆盖 API 格式定义，允许运行时扩展。

    Args:
        definition: 要注册的定义
        override: 若目标枚举已存在，是否允许覆盖
    """
    existing = _DEFINITIONS.get(definition.api_format)
    if existing and not override:
        raise ValueError(f"{definition.api_format.value} 已存在，如需覆盖请设置 override=True")
    _DEFINITIONS[definition.api_format] = definition
    _refresh_metadata_cache()


def _refresh_metadata_cache() -> None:
    """更新别名缓存，供注册函数调用。"""
    _alias_lookup_cache.cache_clear()


def normalize_alias_value(value: str) -> str:
    """统一别名格式：去空白、转小写，并将非字母数字转为单个下划线。"""
    if value is None:
        return ""
    text = value.strip().lower()
    # 将所有非字母数字字符替换为下划线，并折叠连续的下划线
    text = re.sub(r"[^a-z0-9]+", "_", text)
    return text.strip("_")


# is_cli_format 和 is_cli_api_format 已移至 utils.py
# 为保持兼容性，从 utils 重新导出
from src.core.api_format.utils import is_cli_format  # noqa: E402

# is_cli_api_format 是 is_cli_format 的别名（接受 APIFormat 枚举）
is_cli_api_format = is_cli_format
