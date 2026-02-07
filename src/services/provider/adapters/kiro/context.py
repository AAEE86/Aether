from __future__ import annotations

import contextvars
from dataclasses import dataclass


@dataclass(frozen=True, slots=True)
class KiroRequestContext:
    """Per-request context for the Kiro adapter.

    This bridges data from `KiroEnvelope.wrap_request()` (which receives the
    decrypted auth_config + original request body) to other layers that only
    expose parameterless hooks (extra_headers) or transport hooks.
    """

    region: str
    machine_id: str
    kiro_version: str | None = None
    system_version: str | None = None
    node_version: str | None = None
    thinking_enabled: bool = False


_kiro_request_context: contextvars.ContextVar[KiroRequestContext | None] = contextvars.ContextVar(
    "kiro_request_context",
    default=None,
)


def set_kiro_request_context(ctx: KiroRequestContext | None) -> None:
    _kiro_request_context.set(ctx)


def get_kiro_request_context() -> KiroRequestContext | None:
    return _kiro_request_context.get()


__all__ = [
    "KiroRequestContext",
    "get_kiro_request_context",
    "set_kiro_request_context",
]
