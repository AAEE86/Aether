"""
API 格式辅助函数，确保在调度/编排链路中使用统一的枚举值。
"""



from src.core.api_format import APIFormat, resolve_api_format


def normalize_api_format(
    value: str | APIFormat | None, default: APIFormat = APIFormat.CLAUDE
) -> APIFormat:
    """
    将任意字符串/枚举值归一化为 APIFormat。
    未识别的值回退到默认枚举（默认 CLAUDE）。
    """
    resolved = resolve_api_format(value)
    return resolved or default
