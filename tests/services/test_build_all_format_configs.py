"""测试 build_all_format_configs 是否正确使用每个端点自身的 base_url。"""

from __future__ import annotations

from types import SimpleNamespace

from src.services.model.upstream_fetcher import build_all_format_configs


def _make_endpoint(base_url: str, header_rules: list | None = None) -> SimpleNamespace:
    """创建一个最小化的 ProviderEndpoint 替身。"""
    return SimpleNamespace(base_url=base_url, header_rules=header_rules)


def test_three_formats_use_their_own_base_url() -> None:
    """三种格式各有不同的 base_url，结果应各自使用自己的 URL。"""
    format_to_endpoint = {
        "openai:chat": _make_endpoint("https://api.openai.example.com"),
        "claude:chat": _make_endpoint("https://api.claude.example.com"),
        "gemini:chat": _make_endpoint("https://api.gemini.example.com"),
    }

    configs = build_all_format_configs("sk-test-key", format_to_endpoint)  # type: ignore[arg-type]

    assert len(configs) == 3

    by_fmt = {c["api_format"]: c for c in configs}
    assert by_fmt["openai:chat"]["base_url"] == "https://api.openai.example.com"
    assert by_fmt["claude:chat"]["base_url"] == "https://api.claude.example.com"
    assert by_fmt["gemini:chat"]["base_url"] == "https://api.gemini.example.com"

    # 所有配置都应使用同一个 api_key
    for c in configs:
        assert c["api_key"] == "sk-test-key"


def test_only_configured_formats_are_included() -> None:
    """只配置了 claude:chat，不应出现 openai:chat 和 gemini:chat 的请求。"""
    format_to_endpoint = {
        "claude:chat": _make_endpoint("https://betterclau.de/claude/api.freekey.site"),
    }

    configs = build_all_format_configs("sk-test-key", format_to_endpoint)  # type: ignore[arg-type]

    assert len(configs) == 1
    assert configs[0]["api_format"] == "claude:chat"
    assert configs[0]["base_url"] == "https://betterclau.de/claude/api.freekey.site"


def test_cli_format_only_is_skipped() -> None:
    """如果只配置了 CLI 格式（不在 MODEL_FETCH_FORMATS 中），应返回空列表。"""
    format_to_endpoint = {
        "openai:cli": _make_endpoint("https://api.openai.example.com"),
    }

    configs = build_all_format_configs("sk-test-key", format_to_endpoint)  # type: ignore[arg-type]

    assert configs == []


def test_empty_format_to_endpoint() -> None:
    """空映射应返回空列表。"""
    configs = build_all_format_configs("sk-test-key", {})
    assert configs == []


def test_extra_headers_from_each_endpoint() -> None:
    """每个端点的 extra_headers 应独立获取，不应混用。"""
    format_to_endpoint = {
        "openai:chat": _make_endpoint(
            "https://api.openai.example.com",
            header_rules=[{"action": "set", "key": "X-OpenAI", "value": "1"}],
        ),
        "claude:chat": _make_endpoint(
            "https://api.claude.example.com",
            header_rules=[{"action": "set", "key": "X-Claude", "value": "2"}],
        ),
    }

    configs = build_all_format_configs("sk-test-key", format_to_endpoint)  # type: ignore[arg-type]

    by_fmt = {c["api_format"]: c for c in configs}
    assert by_fmt["openai:chat"]["extra_headers"] == {"X-OpenAI": "1"}
    assert by_fmt["claude:chat"]["extra_headers"] == {"X-Claude": "2"}
