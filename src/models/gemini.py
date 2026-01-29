"""
Google Gemini API 请求/响应模型

支持 Gemini API 的请求/响应格式
作为 API 网关，采用宽松类型定义以支持 API 新特性透传
"""

from typing import Any

from pydantic import BaseModel, ConfigDict, Field


class BaseModelWithExtras(BaseModel):
    """允许额外字段的基础模型"""

    model_config = ConfigDict(extra="allow")


# ---------------------------------------------------------------------------
# 内容定义 - 使用宽松类型以支持透传
# ---------------------------------------------------------------------------


class GeminiContent(BaseModelWithExtras):
    """
    Gemini 消息内容

    使用宽松类型定义，parts 接受任意字典列表以支持 API 新特性
    """

    role: str | None = None
    parts: list[dict[str, Any]]


# ---------------------------------------------------------------------------
# 请求模型 - 只定义网关需要的字段，其余透传
# ---------------------------------------------------------------------------


class GeminiRequest(BaseModelWithExtras):
    """
    Gemini 统一请求模型

    内部使用，统一处理 generateContent 和 streamGenerateContent

    注意: Gemini API 通过 URL 端点区分流式/非流式请求:
    - generateContent - 非流式
    - streamGenerateContent - 流式
    请求体中不应包含 stream 字段

    采用宽松类型定义，除必要字段外全部透传
    """

    model: str | None = Field(default=None, description="模型名称，从 URL 路径提取（内部使用）")
    contents: list[GeminiContent]
    # 以下字段全部使用 Dict[str, Any] 透传，不做结构验证
    system_instruction: dict[str, Any] | None = Field(default=None, alias="systemInstruction")
    tools: list[dict[str, Any]] | None = None
    tool_config: dict[str, Any] | None = Field(default=None, alias="toolConfig")
    safety_settings: list[dict[str, Any]] | None = Field(default=None, alias="safetySettings")
    generation_config: dict[str, Any] | None = Field(default=None, alias="generationConfig")


# ---------------------------------------------------------------------------
# 响应模型 - 用于解析上游响应提取必要信息（如 usage）
# ---------------------------------------------------------------------------


class GeminiUsageMetadata(BaseModelWithExtras):
    """Token 使用量 - 用于计费统计"""

    prompt_token_count: int = Field(default=0, alias="promptTokenCount")
    candidates_token_count: int = Field(default=0, alias="candidatesTokenCount")
    total_token_count: int = Field(default=0, alias="totalTokenCount")


# ---------------------------------------------------------------------------
# Thought Signature 常量
# ---------------------------------------------------------------------------

# 用于从其他模型迁移对话时绕过签名验证
DUMMY_THOUGHT_SIGNATURE = "context_engineering_is_the_way_to_go"
