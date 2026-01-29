import json
from typing import Any
from unittest.mock import AsyncMock

import pytest

from src.api.handlers.base.response_parser import ParsedResponse, ResponseParser, StreamStats
from src.api.handlers.base.stream_context import StreamContext
from src.api.handlers.base.stream_processor import StreamProcessor
from src.core.api_format.conversion import register_default_normalizers


class DummyParser(ResponseParser):
    def parse_sse_line(self, line: str, stats: StreamStats) -> Any | None:  # noqa: ANN401
        return None

    def parse_response(self, response: dict[str, Any], status_code: int) -> ParsedResponse:
        return ParsedResponse(raw_response=response, status_code=status_code)

    def extract_usage_from_response(self, response: dict[str, Any]) -> dict[str, int]:
        return {}

    def extract_text_content(self, response: dict[str, Any]) -> str:
        return ""


async def _empty_async_iter():
    if False:  # pragma: no cover
        yield b""


@pytest.mark.asyncio
async def test_create_response_stream_converts_claude_to_openai() -> None:
    register_default_normalizers()

    ctx = StreamContext(model="test-model", api_format="OPENAI")
    ctx.client_api_format = "OPENAI"
    ctx.provider_api_format = "CLAUDE"
    ctx.needs_conversion = True

    processor = StreamProcessor(request_id="test-request", default_parser=DummyParser())

    response_ctx = AsyncMock()
    response_ctx.__aexit__ = AsyncMock(return_value=None)

    http_client = AsyncMock()
    http_client.aclose = AsyncMock(return_value=None)

    message_start = {
        "type": "message_start",
        "message": {
            "id": "msg_1",
            "type": "message",
            "role": "assistant",
            "model": "claude-test",
            "content": [],
            "stop_reason": None,
            "stop_sequence": None,
        },
    }
    content_delta = {
        "type": "content_block_delta",
        "index": 0,
        "delta": {"type": "text_delta", "text": "Hi"},
    }

    prefetched_chunks = [
        b"event: message_start\n",
        f"data: {json.dumps(message_start)}\n".encode("utf-8"),
        b"\n",
        f"data: {json.dumps(content_delta)}\n".encode("utf-8"),
        b"\n",
    ]

    out = b"".join(
        [
            chunk
            async for chunk in processor.create_response_stream(
                ctx,
                byte_iterator=_empty_async_iter(),
                response_ctx=response_ctx,
                http_client=http_client,
                prefetched_chunks=prefetched_chunks,
            )
        ]
    )

    text = out.decode("utf-8")
    assert "event:" not in text

    events = []
    for line in text.splitlines():
        if line.startswith("data: "):
            if line == "data: [DONE]":
                continue
            events.append(json.loads(line[6:]))

    assert len(events) >= 2
    assert any(e.get("object") == "chat.completion.chunk" for e in events)
    assert any(
        e.get("choices", [{}])[0].get("delta", {}).get("content") == "Hi"
        for e in events
        if isinstance(e, dict)
    )
