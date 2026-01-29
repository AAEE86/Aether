"""
Usage 字段映射器

将不同 API 格式的原始 usage 数据映射为标准化格式。

支持的格式：
- OPENAI / OPENAI_CLI: OpenAI Chat Completions API
- CLAUDE / CLAUDE_CLI: Anthropic Messages API
- GEMINI / GEMINI_CLI: Google Gemini API
"""

from typing import Any

from src.services.billing.models import StandardizedUsage


class UsageMapper:
    """
    Usage 字段映射器

    将不同 API 格式的 usage 统一映射为 StandardizedUsage。

    示例:
        # OpenAI 格式
        raw_usage = {
            "prompt_tokens": 100,
            "completion_tokens": 50,
            "prompt_tokens_details": {"cached_tokens": 20},
            "completion_tokens_details": {"reasoning_tokens": 10}
        }
        usage = UsageMapper.map(raw_usage, "OPENAI")

        # Claude 格式
        raw_usage = {
            "input_tokens": 100,
            "output_tokens": 50,
            "cache_creation_input_tokens": 30,
            "cache_read_input_tokens": 20
        }
        usage = UsageMapper.map(raw_usage, "CLAUDE")
    """

    # =========================================================================
    # 字段映射配置
    # 格式: "source_path" -> "target_field"
    # source_path 支持点号分隔的嵌套路径
    # =========================================================================

    # OpenAI 格式字段映射
    OPENAI_MAPPING: dict[str, str] = {
        "prompt_tokens": "input_tokens",
        "completion_tokens": "output_tokens",
        "prompt_tokens_details.cached_tokens": "cache_read_tokens",
        "completion_tokens_details.reasoning_tokens": "reasoning_tokens",
    }

    # Claude 格式字段映射
    CLAUDE_MAPPING: dict[str, str] = {
        "input_tokens": "input_tokens",
        "output_tokens": "output_tokens",
        "cache_creation_input_tokens": "cache_creation_tokens",
        "cache_read_input_tokens": "cache_read_tokens",
    }

    # Gemini 格式字段映射
    GEMINI_MAPPING: dict[str, str] = {
        "promptTokenCount": "input_tokens",
        "candidatesTokenCount": "output_tokens",
        "cachedContentTokenCount": "cache_read_tokens",
        # Gemini 的 usageMetadata 格式
        "usageMetadata.promptTokenCount": "input_tokens",
        "usageMetadata.candidatesTokenCount": "output_tokens",
        "usageMetadata.cachedContentTokenCount": "cache_read_tokens",
    }

    # 格式名称到映射的对应关系
    FORMAT_MAPPINGS: dict[str, dict[str, str]] = {
        "OPENAI": OPENAI_MAPPING,
        "OPENAI_CLI": OPENAI_MAPPING,
        "CLAUDE": CLAUDE_MAPPING,
        "CLAUDE_CLI": CLAUDE_MAPPING,
        "GEMINI": GEMINI_MAPPING,
        "GEMINI_CLI": GEMINI_MAPPING,
    }

    @classmethod
    def map(
        cls,
        raw_usage: dict[str, Any],
        api_format: str,
        extra_mapping: dict[str, str] | None = None,
    ) -> StandardizedUsage:
        """
        将原始 usage 映射为标准化格式

        Args:
            raw_usage: 原始 usage 字典
            api_format: API 格式 ("OPENAI", "CLAUDE", "GEMINI" 等)
            extra_mapping: 额外的字段映射（用于自定义扩展）

        Returns:
            标准化的 usage 对象
        """
        if not raw_usage:
            return StandardizedUsage()

        # 获取对应格式的字段映射
        mapping = cls._get_mapping(api_format)

        # 合并额外映射
        if extra_mapping:
            mapping = {**mapping, **extra_mapping}

        result = StandardizedUsage()

        # 执行映射
        for source_path, target_field in mapping.items():
            value = cls._get_nested_value(raw_usage, source_path)
            if value is not None:
                result.set(target_field, value)

        return result

    @classmethod
    def map_from_response(
        cls,
        response: dict[str, Any],
        api_format: str,
    ) -> StandardizedUsage:
        """
        从完整响应中提取并映射 usage

        不同 API 格式的 usage 位置可能不同：
        - OpenAI: response["usage"]
        - Claude: response["usage"] 或 message_delta 中
        - Gemini: response["usageMetadata"]

        Args:
            response: 完整的 API 响应
            api_format: API 格式

        Returns:
            标准化的 usage 对象
        """
        format_upper = api_format.upper() if api_format else ""

        # 提取 usage 部分
        usage_data: dict[str, Any] = {}

        if format_upper.startswith("GEMINI"):
            # Gemini: usageMetadata
            usage_data = response.get("usageMetadata", {})
            if not usage_data:
                # 尝试从 candidates 中获取
                candidates = response.get("candidates", [])
                if candidates:
                    usage_data = candidates[0].get("usageMetadata", {})
        else:
            # OpenAI/Claude: usage
            usage_data = response.get("usage", {})

        return cls.map(usage_data, api_format)

    @classmethod
    def _get_mapping(cls, api_format: str) -> dict[str, str]:
        """获取对应格式的字段映射"""
        if not api_format:
            return cls.CLAUDE_MAPPING

        format_upper = api_format.upper()

        # 精确匹配
        if format_upper in cls.FORMAT_MAPPINGS:
            return cls.FORMAT_MAPPINGS[format_upper]

        # 前缀匹配
        for key, mapping in cls.FORMAT_MAPPINGS.items():
            if format_upper.startswith(key.split("_")[0]):
                return mapping

        # 默认使用 Claude 映射
        return cls.CLAUDE_MAPPING

    @classmethod
    def _get_nested_value(cls, data: dict[str, Any], path: str) -> Any:
        """
        获取嵌套字段值

        支持点号分隔的路径，如 "prompt_tokens_details.cached_tokens"

        Args:
            data: 数据字典
            path: 字段路径

        Returns:
            字段值，不存在则返回 None
        """
        if not data or not path:
            return None

        keys = path.split(".")
        value: Any = data

        for key in keys:
            if isinstance(value, dict):
                value = value.get(key)
                if value is None:
                    return None
            else:
                return None

        return value

    @classmethod
    def register_format(cls, format_name: str, mapping: dict[str, str]) -> None:
        """
        注册新的格式映射

        Args:
            format_name: 格式名称（会自动转为大写）
            mapping: 字段映射
        """
        cls.FORMAT_MAPPINGS[format_name.upper()] = mapping

    @classmethod
    def get_supported_formats(cls) -> list:
        """获取所有支持的格式"""
        return list(cls.FORMAT_MAPPINGS.keys())


# =========================================================================
# 便捷函数
# =========================================================================


def map_usage(
    raw_usage: dict[str, Any],
    api_format: str,
) -> StandardizedUsage:
    """
    便捷函数：将原始 usage 映射为标准化格式

    Args:
        raw_usage: 原始 usage 字典
        api_format: API 格式

    Returns:
        StandardizedUsage 对象
    """
    return UsageMapper.map(raw_usage, api_format)


def map_usage_from_response(
    response: dict[str, Any],
    api_format: str,
) -> StandardizedUsage:
    """
    便捷函数：从响应中提取并映射 usage

    Args:
        response: API 响应
        api_format: API 格式

    Returns:
        StandardizedUsage 对象
    """
    return UsageMapper.map_from_response(response, api_format)
