"""Kiro provider envelope.

Kiro upstream is not Claude wire-compatible:
- Request: wrap Claude Messages body into Kiro `conversationState` request.
- Stream response: handled by StreamProcessor via binary EventStream rewrite.

We use contextvars to pass request-scoped values (region, machine_id, thinking)
from wrap_request() to extra_headers() and transport hook.
"""

from __future__ import annotations

from typing import Any

from src.services.provider.adapters.kiro.context import KiroRequestContext, set_kiro_request_context
from src.services.provider.adapters.kiro.converter import convert_claude_messages_to_conversation_state
from src.services.provider.adapters.kiro.headers import build_generate_assistant_headers
from src.services.provider.adapters.kiro.models.credentials import KiroAuthConfig
from src.services.provider.adapters.kiro.token_manager import generate_machine_id


def _resolve_region(cfg: KiroAuthConfig) -> str:
    region = str(cfg.region or "").strip()
    return region or "us-east-1"


def _is_thinking_enabled(request_body: dict[str, Any]) -> bool:
    thinking = request_body.get("thinking")
    if not isinstance(thinking, dict):
        return False
    ttype = str(thinking.get("type") or "").strip().lower()
    return ttype in {"enabled", "adaptive"}


class KiroEnvelope:
    name = "kiro:generateAssistantResponse"

    def extra_headers(self) -> dict[str, str] | None:
        # Called after wrap_request(); relies on KiroRequestContext.
        from src.services.provider.adapters.kiro.context import get_kiro_request_context

        ctx = get_kiro_request_context()
        if ctx is None:
            return None

        host = f"q.{ctx.region}.amazonaws.com"
        return build_generate_assistant_headers(
            host=host,
            access_token=None,
            machine_id=ctx.machine_id,
            kiro_version=ctx.kiro_version,
            system_version=ctx.system_version,
            node_version=ctx.node_version,
        )

    def wrap_request(
        self,
        request_body: dict[str, Any],
        *,
        model: str,
        url_model: str | None,
        decrypted_auth_config: dict[str, Any] | None,
    ) -> tuple[dict[str, Any], str | None]:
        cfg = KiroAuthConfig.from_dict(decrypted_auth_config or {})

        region = _resolve_region(cfg)
        machine_id = generate_machine_id(cfg)

        thinking_enabled = _is_thinking_enabled(request_body)

        set_kiro_request_context(
            KiroRequestContext(
                region=region,
                machine_id=machine_id,
                kiro_version=cfg.kiro_version,
                system_version=cfg.system_version,
                node_version=cfg.node_version,
                thinking_enabled=thinking_enabled,
            )
        )

        conversation_state = convert_claude_messages_to_conversation_state(
            request_body,
            model=model,
        )

        wrapped: dict[str, Any] = {
            "conversationState": conversation_state,
        }
        if isinstance(cfg.profile_arn, str) and cfg.profile_arn.strip():
            wrapped["profileArn"] = cfg.profile_arn.strip()

        return wrapped, url_model

    def unwrap_response(self, data: Any) -> Any:
        return data

    def postprocess_unwrapped_response(self, *, model: str, data: Any) -> None:  # noqa: ARG002
        return

    def capture_selected_base_url(self) -> str | None:
        return None

    def on_http_status(self, *, base_url: str | None, status_code: int) -> None:  # noqa: ARG002
        return

    def on_connection_error(self, *, base_url: str | None, exc: Exception) -> None:  # noqa: ARG002
        return

    def force_stream_rewrite(self) -> bool:
        # Kiro streaming is binary AWS Event Stream and must be rewritten.
        return True


kiro_envelope = KiroEnvelope()


__all__ = ["KiroEnvelope", "kiro_envelope"]
