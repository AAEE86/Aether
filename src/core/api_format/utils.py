"""
API 格式工具函数

提供格式判断、规范化等工具函数，供整个项目使用。
"""


from typing import TYPE_CHECKING

if TYPE_CHECKING:
    from src.core.api_format.enums import APIFormat


def is_cli_format(format_id: str | APIFormat | None) -> bool:
    """
    判断是否为 CLI 透传格式

    CLI 格式以 _CLI 结尾，表示该入口更偏向“CLI 兼容层”（鉴权/UA/路径差异等）。
    是否参与格式转换由转换层决定；当前项目已支持 CLI 格式参与转换。

    Args:
        format_id: 格式标识符（字符串或 APIFormat 枚举）

    Returns:
        True 如果是 CLI 格式

    Examples:
        >>> is_cli_format("CLAUDE_CLI")
        True
        >>> is_cli_format("CLAUDE")
        False
        >>> is_cli_format(APIFormat.OPENAI_CLI)
        True
    """
    if format_id is None:
        return False
    if hasattr(format_id, "value"):
        format_id = format_id.value
    return str(format_id).upper().endswith("_CLI")


def get_base_format(format_id: str | APIFormat | None) -> str | None:
    """
    获取基础格式（去除 _CLI 后缀）

    Args:
        format_id: 格式标识符

    Returns:
        基础格式字符串，或 None

    Examples:
        >>> get_base_format("CLAUDE_CLI")
        "CLAUDE"
        >>> get_base_format("OPENAI")
        "OPENAI"
    """
    if format_id is None:
        return None
    if hasattr(format_id, "value"):
        format_id = format_id.value
    format_str = str(format_id).upper()
    if format_str.endswith("_CLI"):
        return format_str[:-4]
    return format_str


def normalize_format(format_id: str | APIFormat | None) -> str | None:
    """
    规范化格式标识符

    Args:
        format_id: 格式标识符（可能是字符串、枚举或 None）

    Returns:
        大写的格式字符串，或 None
    """
    if format_id is None:
        return None
    if hasattr(format_id, "value"):
        return str(format_id.value).upper()
    return str(format_id).upper()


def is_same_format(
    format1: str | APIFormat | None,
    format2: str | APIFormat | None,
) -> bool:
    """
    判断两个格式是否相同

    忽略大小写和枚举/字符串差异。
    """
    return normalize_format(format1) == normalize_format(format2)


def is_convertible_format(format_id: str | APIFormat | None) -> bool:
    """
    判断是否为可转换格式

    .. deprecated::
        此函数语义已退化（对非 None 输入总返回 True）。
        真正的可转换性应通过 format_conversion_registry.can_convert_*() 查询。
        保留此函数仅为向后兼容，不建议新代码使用。
    """
    if format_id is None:
        return False
    return True
