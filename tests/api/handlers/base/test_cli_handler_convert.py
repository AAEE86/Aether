"""
CliMessageHandlerBase._convert_sse_line 单元测试

测试覆盖：
1. 基本转换（空行、非 data 行、JSON 解析失败）
2. 一入多出场景
3. 状态追踪
4. 错误处理
"""

import json
from unittest.mock import MagicMock, patch

import pytest

from src.api.handlers.base.stream_context import StreamContext
from src.core.api_format.conversion.stream_state import StreamState


# Mock CliMessageHandlerBase 用于测试
class MockCliHandler:
    """Mock handler for testing _convert_sse_line"""

    def _convert_sse_line(
        self,
        ctx: StreamContext,
        line: str,
        events: list,
    ) -> list[str]:
        """复制自 CliMessageHandlerBase._convert_sse_line"""
        from src.core.api_format.conversion import (
            format_conversion_registry,
            register_default_normalizers,
        )

        register_default_normalizers()

        # 如果是空行或特殊控制行，直接返回
        if not line or line.strip() == "" or line == "data: [DONE]":
            return [line] if line else []

        # 如果不是 data 行，直接透传
        if not line.startswith("data:"):
            return [line]

        # 提取 data 内容
        data_content = line[5:].strip()

        # 尝试解析 JSON
        try:
            data_obj = json.loads(data_content)
        except json.JSONDecodeError:
            return [line]

        # 初始化流式转换状态
        # 使用客户端请求的模型名（ctx.model），而非映射后的模型名（ctx.mapped_model）
        if ctx.stream_conversion_state is None:
            ctx.stream_conversion_state = StreamState(
                model=ctx.model,
                message_id=ctx.response_id or ctx.request_id,
            )

        provider_format = ctx.provider_api_format or ""
        client_format = ctx.client_api_format or ""

        try:
            converted_events = format_conversion_registry.convert_stream_chunk(
                data_obj,
                provider_format,
                client_format,
                state=ctx.stream_conversion_state,
            )

            result = []
            for evt in converted_events:
                result.append(f"data: {json.dumps(evt, ensure_ascii=False)}")
            return result

        except Exception:
            return [line]


class TestConvertSseLineBasic:
    """基本转换测试"""

    def test_empty_line_returns_empty_list(self) -> None:
        handler = MockCliHandler()
        ctx = StreamContext(model="test", api_format="OPENAI")

        result = handler._convert_sse_line(ctx, "", [])

        assert result == []

    def test_whitespace_line_returns_line(self) -> None:
        handler = MockCliHandler()
        ctx = StreamContext(model="test", api_format="OPENAI")

        result = handler._convert_sse_line(ctx, "   ", [])

        assert result == ["   "]

    def test_done_marker_returns_as_is(self) -> None:
        handler = MockCliHandler()
        ctx = StreamContext(model="test", api_format="OPENAI")

        result = handler._convert_sse_line(ctx, "data: [DONE]", [])

        assert result == ["data: [DONE]"]

    def test_non_data_line_passthrough(self) -> None:
        handler = MockCliHandler()
        ctx = StreamContext(model="test", api_format="OPENAI")

        result = handler._convert_sse_line(ctx, "event: message_start", [])

        assert result == ["event: message_start"]

    def test_invalid_json_passthrough(self) -> None:
        handler = MockCliHandler()
        ctx = StreamContext(model="test", api_format="OPENAI")

        result = handler._convert_sse_line(ctx, "data: {invalid json}", [])

        assert result == ["data: {invalid json}"]


class TestConvertSseLineWithMockConverter:
    """使用 Mock 转换器的测试"""

    def test_same_format_returns_original(self) -> None:
        """同格式无需转换"""
        handler = MockCliHandler()
        ctx = StreamContext(model="test", api_format="OPENAI")
        ctx.provider_api_format = "OPENAI"
        ctx.client_api_format = "OPENAI"

        chunk = {"choices": [{"delta": {"content": "hello"}}]}
        line = f"data: {json.dumps(chunk)}"

        result = handler._convert_sse_line(ctx, line, [])

        assert len(result) == 1
        assert json.loads(result[0][6:]) == chunk

    def test_state_initialization(self) -> None:
        """测试状态自动初始化
        
        流式转换状态应使用用户请求的原始模型名（ctx.model），
        而非映射后的模型名（ctx.mapped_model），确保返回给客户端的响应使用原始模型名。
        """
        handler = MockCliHandler()
        ctx = StreamContext(model="gpt-4", api_format="OPENAI")
        ctx.provider_api_format = "OPENAI"
        ctx.client_api_format = "OPENAI"
        ctx.mapped_model = "claude-3-5-sonnet"  # 映射后的模型名（发给上游的）
        ctx.request_id = "req_123"

        chunk = {"choices": [{"delta": {"content": "test"}}]}
        line = f"data: {json.dumps(chunk)}"

        handler._convert_sse_line(ctx, line, [])

        # 验证状态已初始化，使用用户请求的原始模型名
        assert ctx.stream_conversion_state is not None
        assert ctx.stream_conversion_state.model == "gpt-4"  # 应使用原始模型名，非 mapped_model
        assert ctx.stream_conversion_state.message_id == "req_123"


class TestConvertSseLineOneInManyOut:
    """一入多出测试（需要注册转换器）"""

    @pytest.fixture(autouse=True)
    def setup_converters(self):
        """确保 Canonical normalizers 已注册"""
        from src.core.api_format.conversion import register_default_normalizers

        register_default_normalizers()

        yield

    def test_openai_to_claude_conversion(self) -> None:
        """测试 OpenAI -> Claude 流式转换"""
        handler = MockCliHandler()
        ctx = StreamContext(model="gpt-4", api_format="OPENAI")
        ctx.provider_api_format = "OPENAI"
        ctx.client_api_format = "CLAUDE"
        ctx.mapped_model = "claude-3-5-sonnet"
        ctx.request_id = "req_test"

        # 第一个 chunk：带 role
        chunk1 = {
            "id": "chatcmpl-123",
            "object": "chat.completion.chunk",
            "choices": [{"index": 0, "delta": {"role": "assistant"}, "finish_reason": None}],
        }
        line1 = f"data: {json.dumps(chunk1)}"

        result1 = handler._convert_sse_line(ctx, line1, [])

        # 应返回 message_start 事件
        assert len(result1) >= 1
        first_event = json.loads(result1[0][6:])
        assert first_event.get("type") == "message_start"

    def test_claude_to_openai_conversion(self) -> None:
        """测试 Claude -> OpenAI 流式转换"""
        handler = MockCliHandler()
        ctx = StreamContext(model="claude-3-5-sonnet", api_format="CLAUDE")
        ctx.provider_api_format = "CLAUDE"
        ctx.client_api_format = "OPENAI"
        ctx.mapped_model = "gpt-4"
        ctx.request_id = "msg_test"

        # Claude message_start 事件
        event = {"type": "message_start", "message": {"id": "msg_123", "role": "assistant"}}
        line = f"data: {json.dumps(event)}"

        result = handler._convert_sse_line(ctx, line, [])

        # 应返回 OpenAI 格式的 chunk
        assert len(result) >= 1
        chunk = json.loads(result[0][6:])
        assert "choices" in chunk

    def test_multiple_chunks_state_persistence(self) -> None:
        """测试多个 chunk 之间状态持久化"""
        handler = MockCliHandler()
        ctx = StreamContext(model="gpt-4", api_format="OPENAI")
        ctx.provider_api_format = "OPENAI"
        ctx.client_api_format = "CLAUDE"
        ctx.mapped_model = "claude-3-5-sonnet"

        # 第一个 chunk
        chunk1 = {"choices": [{"delta": {"role": "assistant"}}]}
        handler._convert_sse_line(ctx, f"data: {json.dumps(chunk1)}", [])

        state_after_first = ctx.stream_conversion_state

        # 第二个 chunk
        chunk2 = {"choices": [{"delta": {"content": "hello"}}]}
        handler._convert_sse_line(ctx, f"data: {json.dumps(chunk2)}", [])

        # 状态应该是同一个对象
        assert ctx.stream_conversion_state is state_after_first


class TestStreamContextIntegration:
    """StreamContext 集成测试"""

    def test_stream_conversion_state_reset_on_retry(self) -> None:
        """测试重试时重置流式转换状态"""
        ctx = StreamContext(model="test", api_format="OPENAI")
        ctx.stream_conversion_state = StreamState(model="test", message_id="123")

        ctx.reset_for_retry()

        assert ctx.stream_conversion_state is None

    def test_stream_conversion_state_field_exists(self) -> None:
        """测试 StreamContext 有 stream_conversion_state 字段"""
        ctx = StreamContext(model="test", api_format="OPENAI")

        assert hasattr(ctx, "stream_conversion_state")
        assert ctx.stream_conversion_state is None
