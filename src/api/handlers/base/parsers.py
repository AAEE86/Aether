"""
响应解析器工厂

直接根据格式 ID 创建对应的 ResponseParser 实现，
不再经过 Protocol 抽象层。
"""

from typing import Any, Dict, Optional, Tuple

from src.api.handlers.base.response_parser import (
    ParsedChunk,
    ParsedResponse,
    ResponseParser,
    StreamStats,
)


def _check_nested_error(response: Dict[str, Any]) -> Tuple[bool, Optional[Dict[str, Any]]]:
    """
    检查响应中是否存在嵌套错误（某些代理服务返回 HTTP 200 但在响应体中包含错误）

    检测格式:
    1. 顶层 error: {"error": {...}}
    2. 顶层 type=error: {"type": "error", ...}
    3. chunks 内嵌套 error: {"chunks": [{"error": {...}}]}

    Args:
        response: 响应字典

    Returns:
        (is_error, error_dict): 是否为错误，以及提取的错误信息
    """
    # 顶层 error
    if "error" in response:
        error = response["error"]
        if isinstance(error, dict):
            return True, error
        return True, {"message": str(error)}

    # 顶层 type=error
    if response.get("type") == "error":
        return True, response

    # chunks 内嵌套 error (某些代理返回这种格式)
    chunks = response.get("chunks", [])
    if chunks and isinstance(chunks, list):
        for chunk in chunks:
            if isinstance(chunk, dict):
                if "error" in chunk:
                    error = chunk["error"]
                    if isinstance(error, dict):
                        return True, error
                    return True, {"message": str(error)}
                if chunk.get("type") == "error":
                    return True, chunk

    return False, None


class OpenAIResponseParser(ResponseParser):
    """OpenAI 格式响应解析器"""

    def __init__(self):
        from src.api.handlers.openai.stream_parser import OpenAIStreamParser

        self._parser = OpenAIStreamParser()
        self.name = "OPENAI"
        self.api_format = "OPENAI"

    def parse_sse_line(self, line: str, stats: StreamStats) -> Optional[ParsedChunk]:
        if not line or not line.strip():
            return None

        if line.startswith("data: "):
            data_str = line[6:]
        else:
            data_str = line

        parsed = self._parser.parse_line(data_str)
        if parsed is None:
            return None

        chunk = ParsedChunk(
            raw_line=line,
            event_type=None,
            data=parsed,
        )

        # 提取文本增量
        text_delta = self._parser.extract_text_delta(parsed)
        if text_delta:
            chunk.text_delta = text_delta
            stats.collected_text += text_delta

        # 检查是否结束
        if self._parser.is_done_chunk(parsed):
            chunk.is_done = True
            stats.has_completion = True

        stats.chunk_count += 1
        stats.data_count += 1

        return chunk

    def parse_response(self, response: Dict[str, Any], status_code: int) -> ParsedResponse:
        result = ParsedResponse(
            raw_response=response,
            status_code=status_code,
        )

        # 提取文本内容
        choices = response.get("choices", [])
        if choices:
            message = choices[0].get("message", {})
            content = message.get("content")
            if content:
                result.text_content = content

        result.response_id = response.get("id")

        # 提取 usage
        usage = response.get("usage", {})
        result.input_tokens = usage.get("prompt_tokens", 0)
        result.output_tokens = usage.get("completion_tokens", 0)

        # 检查错误（支持嵌套错误格式）
        is_error, error_info = _check_nested_error(response)
        if is_error and error_info:
            result.is_error = True
            result.error_type = error_info.get("type")
            result.error_message = error_info.get("message")

        return result

    def extract_usage_from_response(self, response: Dict[str, Any]) -> Dict[str, int]:
        usage = response.get("usage", {})
        return {
            "input_tokens": usage.get("prompt_tokens", 0),
            "output_tokens": usage.get("completion_tokens", 0),
            "cache_creation_tokens": 0,
            "cache_read_tokens": 0,
        }

    def extract_text_content(self, response: Dict[str, Any]) -> str:
        choices = response.get("choices", [])
        if choices:
            message = choices[0].get("message", {})
            content = message.get("content")
            if content:
                return content
        return ""

    def is_error_response(self, response: Dict[str, Any]) -> bool:
        is_error, _ = _check_nested_error(response)
        return is_error


class OpenAICliResponseParser(OpenAIResponseParser):
    """OpenAI CLI 格式响应解析器"""

    def __init__(self):
        super().__init__()
        self.name = "OPENAI_CLI"
        self.api_format = "OPENAI_CLI"


class ClaudeResponseParser(ResponseParser):
    """Claude 格式响应解析器"""

    def __init__(self):
        from src.api.handlers.claude.stream_parser import ClaudeStreamParser

        self._parser = ClaudeStreamParser()
        self.name = "CLAUDE"
        self.api_format = "CLAUDE"

    def parse_sse_line(self, line: str, stats: StreamStats) -> Optional[ParsedChunk]:
        if not line or not line.strip():
            return None

        if line.startswith("data: "):
            data_str = line[6:]
        else:
            data_str = line

        parsed = self._parser.parse_line(data_str)
        if parsed is None:
            return None

        chunk = ParsedChunk(
            raw_line=line,
            event_type=self._parser.get_event_type(parsed),
            data=parsed,
        )

        # 提取文本增量
        text_delta = self._parser.extract_text_delta(parsed)
        if text_delta:
            chunk.text_delta = text_delta
            stats.collected_text += text_delta

        # 检查是否结束
        if self._parser.is_done_event(parsed):
            chunk.is_done = True
            stats.has_completion = True

        # 提取 usage
        usage = self._parser.extract_usage(parsed)
        if usage:
            chunk.input_tokens = usage.get("input_tokens", 0)
            chunk.output_tokens = usage.get("output_tokens", 0)
            chunk.cache_creation_tokens = usage.get("cache_creation_tokens", 0)
            chunk.cache_read_tokens = usage.get("cache_read_tokens", 0)

            stats.input_tokens = chunk.input_tokens
            stats.output_tokens = chunk.output_tokens
            stats.cache_creation_tokens = chunk.cache_creation_tokens
            stats.cache_read_tokens = chunk.cache_read_tokens

        # 检查错误
        if self._parser.is_error_event(parsed):
            chunk.is_error = True
            error = parsed.get("error", {})
            if isinstance(error, dict):
                chunk.error_message = error.get("message", str(error))
            else:
                chunk.error_message = str(error)

        stats.chunk_count += 1
        stats.data_count += 1

        return chunk

    def parse_response(self, response: Dict[str, Any], status_code: int) -> ParsedResponse:
        result = ParsedResponse(
            raw_response=response,
            status_code=status_code,
        )

        # 提取文本内容
        content = response.get("content", [])
        if isinstance(content, list):
            text_parts = []
            for block in content:
                if isinstance(block, dict) and block.get("type") == "text":
                    text_parts.append(block.get("text", ""))
            result.text_content = "".join(text_parts)

        result.response_id = response.get("id")

        # 提取 usage
        usage = response.get("usage", {})
        result.input_tokens = usage.get("input_tokens", 0)
        result.output_tokens = usage.get("output_tokens", 0)
        result.cache_creation_tokens = usage.get("cache_creation_input_tokens", 0)
        result.cache_read_tokens = usage.get("cache_read_input_tokens", 0)

        # 检查错误（支持嵌套错误格式）
        is_error, error_info = _check_nested_error(response)
        if is_error and error_info:
            result.is_error = True
            result.error_type = error_info.get("type")
            result.error_message = error_info.get("message")

        return result

    def extract_usage_from_response(self, response: Dict[str, Any]) -> Dict[str, int]:
        usage = response.get("usage", {})
        return {
            "input_tokens": usage.get("input_tokens", 0),
            "output_tokens": usage.get("output_tokens", 0),
            "cache_creation_tokens": usage.get("cache_creation_input_tokens", 0),
            "cache_read_tokens": usage.get("cache_read_input_tokens", 0),
        }

    def extract_text_content(self, response: Dict[str, Any]) -> str:
        content = response.get("content", [])
        if isinstance(content, list):
            text_parts = []
            for block in content:
                if isinstance(block, dict) and block.get("type") == "text":
                    text_parts.append(block.get("text", ""))
            return "".join(text_parts)
        return ""

    def is_error_response(self, response: Dict[str, Any]) -> bool:
        is_error, _ = _check_nested_error(response)
        return is_error


class ClaudeCliResponseParser(ClaudeResponseParser):
    """Claude CLI 格式响应解析器"""

    def __init__(self):
        super().__init__()
        self.name = "CLAUDE_CLI"
        self.api_format = "CLAUDE_CLI"


class GeminiResponseParser(ResponseParser):
    """Gemini 格式响应解析器"""

    def __init__(self):
        from src.api.handlers.gemini.stream_parser import GeminiStreamParser

        self._parser = GeminiStreamParser()
        self.name = "GEMINI"
        self.api_format = "GEMINI"

    def parse_sse_line(self, line: str, stats: StreamStats) -> Optional[ParsedChunk]:
        """
        解析 Gemini SSE 行

        Gemini 的流式响应使用 SSE 格式 (data: {...})
        """
        if not line or not line.strip():
            return None

        # Gemini SSE 格式: data: {...}
        if line.startswith("data: "):
            data_str = line[6:]
        else:
            data_str = line

        parsed = self._parser.parse_line(data_str)
        if parsed is None:
            return None

        chunk = ParsedChunk(
            raw_line=line,
            event_type="content",
            data=parsed,
        )

        # 提取文本增量
        text_delta = self._parser.extract_text_delta(parsed)
        if text_delta:
            chunk.text_delta = text_delta
            stats.collected_text += text_delta

        # 检查是否结束
        if self._parser.is_done_event(parsed):
            chunk.is_done = True
            stats.has_completion = True

        # 提取 usage
        usage = self._parser.extract_usage(parsed)
        if usage:
            chunk.input_tokens = usage.get("input_tokens", 0)
            chunk.output_tokens = usage.get("output_tokens", 0)
            chunk.cache_read_tokens = usage.get("cached_tokens", 0)

            stats.input_tokens = chunk.input_tokens
            stats.output_tokens = chunk.output_tokens
            stats.cache_read_tokens = chunk.cache_read_tokens

        # 检查错误
        if self._parser.is_error_event(parsed):
            chunk.is_error = True
            error = parsed.get("error", {})
            if isinstance(error, dict):
                chunk.error_message = error.get("message", str(error))
            else:
                chunk.error_message = str(error)

        stats.chunk_count += 1
        stats.data_count += 1

        return chunk

    def parse_response(self, response: Dict[str, Any], status_code: int) -> ParsedResponse:
        result = ParsedResponse(
            raw_response=response,
            status_code=status_code,
        )

        # 提取文本内容
        candidates = response.get("candidates", [])
        if candidates:
            content = candidates[0].get("content", {})
            parts = content.get("parts", [])
            text_parts = []
            for part in parts:
                if "text" in part:
                    text_parts.append(part["text"])
            result.text_content = "".join(text_parts)

        result.response_id = response.get("modelVersion")

        # 提取 usage（调用 GeminiStreamParser.extract_usage 作为单一实现源）
        usage = self._parser.extract_usage(response)
        if usage:
            result.input_tokens = usage.get("input_tokens", 0)
            result.output_tokens = usage.get("output_tokens", 0)
            result.cache_read_tokens = usage.get("cached_tokens", 0)

        # 检查错误（使用增强的错误检测）
        error_info = self._parser.extract_error_info(response)
        if error_info:
            result.is_error = True
            result.error_type = error_info.get("status")
            result.error_message = error_info.get("message")

        return result

    def extract_usage_from_response(self, response: Dict[str, Any]) -> Dict[str, int]:
        """
        从 Gemini 响应中提取 token 使用量

        调用 GeminiStreamParser.extract_usage 作为单一实现源
        """
        usage = self._parser.extract_usage(response)
        if not usage:
            return {
                "input_tokens": 0,
                "output_tokens": 0,
                "cache_creation_tokens": 0,
                "cache_read_tokens": 0,
            }

        return {
            "input_tokens": usage.get("input_tokens", 0),
            "output_tokens": usage.get("output_tokens", 0),
            "cache_creation_tokens": 0,
            "cache_read_tokens": usage.get("cached_tokens", 0),
        }

    def extract_text_content(self, response: Dict[str, Any]) -> str:
        candidates = response.get("candidates", [])
        if candidates:
            content = candidates[0].get("content", {})
            parts = content.get("parts", [])
            text_parts = []
            for part in parts:
                if "text" in part:
                    text_parts.append(part["text"])
            return "".join(text_parts)
        return ""

    def is_error_response(self, response: Dict[str, Any]) -> bool:
        """
        判断响应是否为错误响应

        使用增强的错误检测逻辑，支持嵌套在 chunks 中的错误
        """
        return self._parser.is_error_event(response)


class GeminiCliResponseParser(GeminiResponseParser):
    """Gemini CLI 格式响应解析器"""

    def __init__(self):
        super().__init__()
        self.name = "GEMINI_CLI"
        self.api_format = "GEMINI_CLI"


# 解析器注册表
_PARSERS = {
    "CLAUDE": ClaudeResponseParser,
    "CLAUDE_CLI": ClaudeCliResponseParser,
    "OPENAI": OpenAIResponseParser,
    "OPENAI_CLI": OpenAICliResponseParser,
    "GEMINI": GeminiResponseParser,
    "GEMINI_CLI": GeminiCliResponseParser,
}


def get_parser_for_format(format_id: str) -> ResponseParser:
    """
    根据格式 ID 获取 ResponseParser

    Args:
        format_id: 格式 ID，如 "CLAUDE", "OPENAI", "CLAUDE_CLI", "OPENAI_CLI"

    Returns:
        ResponseParser 实例

    Raises:
        KeyError: 格式不存在
    """
    format_id = format_id.upper()
    if format_id not in _PARSERS:
        raise KeyError(f"Unknown format: {format_id}")
    return _PARSERS[format_id]()


def is_cli_format(format_id: str) -> bool:
    """判断是否为 CLI 格式"""
    return format_id.upper().endswith("_CLI")


__all__ = [
    "OpenAIResponseParser",
    "OpenAICliResponseParser",
    "ClaudeResponseParser",
    "ClaudeCliResponseParser",
    "GeminiResponseParser",
    "GeminiCliResponseParser",
    "get_parser_for_format",
    "get_parser_from_protocol",
    "is_cli_format",
]
