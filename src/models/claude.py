from typing import Any, Literal

from pydantic import BaseModel, ConfigDict


# 配置允许额外字段，以支持API的新特性
class BaseModelWithExtras(BaseModel):
    model_config = ConfigDict(extra="allow")


class ClaudeContentBlockText(BaseModelWithExtras):
    type: Literal["text"]
    text: str


class ClaudeContentBlockImage(BaseModelWithExtras):
    type: Literal["image"]
    source: dict[str, Any]


class ClaudeContentBlockToolUse(BaseModelWithExtras):
    type: Literal["tool_use"]
    id: str
    name: str
    input: dict[str, Any]


class ClaudeContentBlockToolResult(BaseModelWithExtras):
    type: Literal["tool_result"]
    tool_use_id: str
    content: str | list[dict[str, Any]] | dict[str, Any]


class ClaudeContentBlockThinking(BaseModelWithExtras):
    type: Literal["thinking"]
    thinking: str


class ClaudeSystemContent(BaseModelWithExtras):
    type: Literal["text"]
    text: str


class ClaudeMessage(BaseModelWithExtras):
    role: Literal["user", "assistant"]
    # 宽松的内容类型定义 - 接受字符串或任意字典列表
    # 作为转发代理,不应该严格限制内容块类型,以支持API的新特性
    content: str | list[dict[str, Any]]


class ClaudeTool(BaseModelWithExtras):
    name: str
    description: str | None = None
    input_schema: dict[str, Any]


class ClaudeThinkingConfig(BaseModelWithExtras):
    enabled: bool = True


class ClaudeMessagesRequest(BaseModelWithExtras):
    model: str
    max_tokens: int
    messages: list[ClaudeMessage]
    # 宽松的system类型 - 接受字符串、字典列表或任意字典
    system: str | list[dict[str, Any]] | dict[str, Any] | None = None
    stop_sequences: list[str] | None = None
    stream: bool | None = False
    temperature: float | None = 1.0
    top_p: float | None = None
    top_k: int | None = None
    metadata: dict[str, Any] | None = None
    tools: list[dict[str, Any]] | None = None  # 改为更宽松的类型
    tool_choice: dict[str, Any] | None = None
    thinking: dict[str, Any] | None = None  # 改为更宽松的类型


class ClaudeTokenCountRequest(BaseModelWithExtras):
    model: str
    messages: list[ClaudeMessage]
    # 宽松的类型定义以支持API新特性
    system: str | list[dict[str, Any]] | dict[str, Any] | None = None
    tools: list[dict[str, Any]] | None = None
    thinking: dict[str, Any] | None = None
    tool_choice: dict[str, Any] | None = None


# ---------------------------------------------------------------------------
# 响应模型
# ---------------------------------------------------------------------------


class ClaudeResponseUsage(BaseModelWithExtras):
    """Claude 响应 token 使用量"""

    input_tokens: int = 0
    output_tokens: int = 0
    cache_creation_input_tokens: int | None = None
    cache_read_input_tokens: int | None = None


class ClaudeResponse(BaseModelWithExtras):
    """
    Claude Messages API 响应模型

    对应 POST /v1/messages 端点的响应体。
    """

    id: str
    model: str
    type: Literal["message"] = "message"
    role: Literal["assistant"] = "assistant"
    content: list[dict[str, Any]]
    stop_reason: str | None = None
    stop_sequence: str | None = None
    usage: ClaudeResponseUsage | None = None
    context_management: dict[str, Any] | None = None
    container: dict[str, Any] | None = None
