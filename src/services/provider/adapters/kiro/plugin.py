"""Kiro provider plugin ? unified registration entry.

Kiro upstream looks like Claude CLI (Bearer token) from the outside, but uses a
custom wire protocol:
- Request: Claude Messages API -> Kiro generateAssistantResponse envelope
- Response (stream): AWS Event Stream (binary) -> Claude SSE events

This plugin registers:
- Envelope
- Transport hook (dynamic region base_url)
"""

from __future__ import annotations

from typing import Any
from urllib.parse import urlencode

from src.services.provider.adapters.kiro.constants import KIRO_GENERATE_ASSISTANT_PATH
from src.services.provider.adapters.kiro.context import get_kiro_request_context


def build_kiro_url(
    endpoint: Any,
    *,
    is_stream: bool,
    effective_query_params: dict[str, Any],
) -> str:
    """Build Kiro generateAssistantResponse URL.

    Endpoint base_url may contain a `{region}` placeholder. The actual region is
    resolved from per-request context (set by the envelope).
    """
    _ = is_stream

    base = str(getattr(endpoint, "base_url", "") or "").rstrip("/")

    ctx = get_kiro_request_context()
    region = (ctx.region if ctx else "") or "us-east-1"
    if "{region}" in base:
        base = base.replace("{region}", region)

    path = KIRO_GENERATE_ASSISTANT_PATH
    url = base if base.endswith(path) else f"{base}{path}"

    if effective_query_params:
        query_string = urlencode(effective_query_params, doseq=True)
        if query_string:
            url = f"{url}?{query_string}"

    return url


def register_all() -> None:
    """Register all Kiro hooks into shared registries."""

    from src.services.provider.adapters.kiro.envelope import kiro_envelope
    from src.services.provider.envelope import register_envelope
    from src.services.provider.transport import register_transport_hook

    register_envelope("kiro", "claude:cli", kiro_envelope)
    register_envelope("kiro", "", kiro_envelope)

    register_transport_hook("kiro", "claude:cli", build_kiro_url)


__all__ = ["build_kiro_url", "register_all"]
