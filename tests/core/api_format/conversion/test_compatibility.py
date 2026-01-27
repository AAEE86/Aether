"""
is_format_compatible 单元测试

覆盖：
- 同格式透传
- CLI 格式允许转换（按 registry 能力）
- 全局开关/端点开关/白黑名单
- 流式转换开关
- 转换器能力校验
"""

from unittest.mock import MagicMock

from src.core.api_format.conversion.compatibility import is_format_compatible


def test_same_format_is_compatible() -> None:
    ok, needs_conv, reason = is_format_compatible(
        "CLAUDE",
        "CLAUDE",
        endpoint_format_acceptance_config=None,
        is_stream=False,
        global_conversion_enabled=False,
        registry=MagicMock(),
    )
    assert ok is True
    assert needs_conv is False
    assert reason is None


def test_cli_format_convertible_when_converter_supports_full() -> None:
    registry = MagicMock()
    registry.can_convert_full.return_value = True

    ok, needs_conv, reason = is_format_compatible(
        "CLAUDE_CLI",
        "OPENAI",
        endpoint_format_acceptance_config={"enabled": True},
        is_stream=False,
        global_conversion_enabled=True,
        registry=registry,
    )
    assert ok is True
    assert needs_conv is True
    assert reason is None


def test_global_switch_disabled_blocks_conversion() -> None:
    ok, needs_conv, reason = is_format_compatible(
        "CLAUDE",
        "OPENAI",
        endpoint_format_acceptance_config={"enabled": True},
        is_stream=False,
        global_conversion_enabled=False,
        registry=MagicMock(),
    )
    assert ok is False
    assert needs_conv is False
    assert reason and "全局" in reason


def test_endpoint_config_none_blocks_conversion() -> None:
    ok, needs_conv, reason = is_format_compatible(
        "CLAUDE",
        "OPENAI",
        endpoint_format_acceptance_config=None,
        is_stream=False,
        global_conversion_enabled=True,
        registry=MagicMock(),
    )
    assert ok is False
    assert needs_conv is False
    assert reason and "未配置" in reason


def test_endpoint_disabled_blocks_conversion() -> None:
    ok, needs_conv, reason = is_format_compatible(
        "CLAUDE",
        "OPENAI",
        endpoint_format_acceptance_config={"enabled": False},
        is_stream=False,
        global_conversion_enabled=True,
        registry=MagicMock(),
    )
    assert ok is False
    assert needs_conv is False
    assert reason and "未启用" in reason


def test_accept_formats_allows_only_whitelist() -> None:
    ok, needs_conv, reason = is_format_compatible(
        "CLAUDE",
        "OPENAI",
        endpoint_format_acceptance_config={"enabled": True, "accept_formats": ["OPENAI"]},
        is_stream=False,
        global_conversion_enabled=True,
        registry=MagicMock(),
    )
    assert ok is False
    assert needs_conv is False
    assert reason and "不接受" in reason


def test_reject_formats_blocks_blacklist() -> None:
    ok, needs_conv, reason = is_format_compatible(
        "CLAUDE",
        "OPENAI",
        endpoint_format_acceptance_config={"enabled": True, "reject_formats": ["CLAUDE"]},
        is_stream=False,
        global_conversion_enabled=True,
        registry=MagicMock(),
    )
    assert ok is False
    assert needs_conv is False
    assert reason and "拒绝" in reason


def test_stream_conversion_disabled_blocks_stream() -> None:
    ok, needs_conv, reason = is_format_compatible(
        "CLAUDE",
        "OPENAI",
        endpoint_format_acceptance_config={"enabled": True, "stream_conversion": False},
        is_stream=True,
        global_conversion_enabled=True,
        registry=MagicMock(),
    )
    assert ok is False
    assert needs_conv is False
    assert reason and "流式" in reason


def test_converter_support_required() -> None:
    registry = MagicMock()
    registry.can_convert_full.return_value = False

    ok, needs_conv, reason = is_format_compatible(
        "CLAUDE",
        "OPENAI",
        endpoint_format_acceptance_config={"enabled": True},
        is_stream=False,
        global_conversion_enabled=True,
        registry=registry,
    )
    assert ok is False
    assert needs_conv is False
    assert reason and "转换器" in reason


def test_conversion_allowed_when_converter_supports_full() -> None:
    registry = MagicMock()
    registry.can_convert_full.return_value = True

    ok, needs_conv, reason = is_format_compatible(
        "CLAUDE",
        "OPENAI",
        endpoint_format_acceptance_config={"enabled": True, "accept_formats": ["CLAUDE"]},
        is_stream=False,
        global_conversion_enabled=True,
        registry=registry,
    )
    assert ok is True
    assert needs_conv is True
    assert reason is None


# ==================== 同族格式测试 ====================


def test_claude_cli_to_claude_no_conversion_needed() -> None:
    """CLAUDE 和 CLAUDE_CLI 格式相同，只是认证不同，可透传"""
    ok, needs_conv, reason = is_format_compatible(
        "CLAUDE_CLI",
        "CLAUDE",
        endpoint_format_acceptance_config=None,
        is_stream=False,
        global_conversion_enabled=False,
        registry=MagicMock(),
    )
    assert ok is True
    assert needs_conv is False
    assert reason is None


def test_claude_to_claude_cli_no_conversion_needed() -> None:
    """CLAUDE 和 CLAUDE_CLI 格式相同，只是认证不同，可透传"""
    ok, needs_conv, reason = is_format_compatible(
        "CLAUDE",
        "CLAUDE_CLI",
        endpoint_format_acceptance_config=None,
        is_stream=False,
        global_conversion_enabled=False,
        registry=MagicMock(),
    )
    assert ok is True
    assert needs_conv is False
    assert reason is None


def test_gemini_cli_to_gemini_no_conversion_needed() -> None:
    """GEMINI 和 GEMINI_CLI 格式相同，只是认证不同，可透传"""
    ok, needs_conv, reason = is_format_compatible(
        "GEMINI_CLI",
        "GEMINI",
        endpoint_format_acceptance_config=None,
        is_stream=False,
        global_conversion_enabled=False,
        registry=MagicMock(),
    )
    assert ok is True
    assert needs_conv is False
    assert reason is None


def test_openai_cli_to_openai_needs_conversion() -> None:
    """OPENAI 和 OPENAI_CLI 格式不同（Chat Completions vs Responses API），需要转换"""
    registry = MagicMock()
    registry.can_convert_full.return_value = True

    ok, needs_conv, reason = is_format_compatible(
        "OPENAI_CLI",
        "OPENAI",
        endpoint_format_acceptance_config=None,  # 同族转换不需要端点配置
        is_stream=False,
        global_conversion_enabled=False,  # 同族转换不需要全局开关
        registry=registry,
    )
    assert ok is True
    assert needs_conv is True  # 需要转换！
    assert reason is None


def test_openai_to_openai_cli_needs_conversion() -> None:
    """OPENAI 和 OPENAI_CLI 格式不同（Chat Completions vs Responses API），需要转换"""
    registry = MagicMock()
    registry.can_convert_full.return_value = True

    ok, needs_conv, reason = is_format_compatible(
        "OPENAI",
        "OPENAI_CLI",
        endpoint_format_acceptance_config=None,  # 同族转换不需要端点配置
        is_stream=False,
        global_conversion_enabled=False,  # 同族转换不需要全局开关
        registry=registry,
    )
    assert ok is True
    assert needs_conv is True  # 需要转换！
    assert reason is None


def test_openai_cli_to_openai_stream_needs_conversion() -> None:
    """OPENAI_CLI 流式请求到 OPENAI 也需要转换"""
    registry = MagicMock()
    registry.can_convert_full.return_value = True

    ok, needs_conv, reason = is_format_compatible(
        "OPENAI_CLI",
        "OPENAI",
        endpoint_format_acceptance_config=None,
        is_stream=True,
        global_conversion_enabled=False,
        registry=registry,
    )
    assert ok is True
    assert needs_conv is True
    assert reason is None


def test_openai_cli_to_openai_fails_without_converter() -> None:
    """如果转换器不支持，同族转换也会失败"""
    registry = MagicMock()
    registry.can_convert_full.return_value = False

    ok, needs_conv, reason = is_format_compatible(
        "OPENAI_CLI",
        "OPENAI",
        endpoint_format_acceptance_config=None,
        is_stream=False,
        global_conversion_enabled=False,
        registry=registry,
    )
    assert ok is False
    assert needs_conv is False
    assert reason and "转换器" in reason
