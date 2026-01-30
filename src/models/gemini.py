"""
Google Gemini API 请求/响应模型

支持 Gemini API 的请求/响应格式
作为 API 网关，采用宽松类型定义以支持 API 新特性透传
"""

from __future__ import annotations

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
    # 以下字段全部使用 dict[str, Any] 透传，不做结构验证
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
# 文件 API 模型
# ---------------------------------------------------------------------------


class GeminiFileMetadata(BaseModelWithExtras):
    """
    Gemini 文件元数据

    用于上传文件时指定的元数据信息
    """

    display_name: str | None = Field(default=None, alias="displayName")


class GeminiFileUploadRequest(BaseModelWithExtras):
    """
    Gemini 文件上传请求

    用于 media.upload API 的请求体
    """

    file: GeminiFileMetadata | None = None


class GeminiFile(BaseModelWithExtras):
    """
    Gemini 文件资源

    表示已上传到 Gemini API 的文件
    """

    name: str | None = None  # 文件名，格式：files/xxx
    display_name: str | None = Field(default=None, alias="displayName")
    mime_type: str | None = Field(default=None, alias="mimeType")
    size_bytes: str | None = Field(default=None, alias="sizeBytes")
    create_time: str | None = Field(default=None, alias="createTime")
    update_time: str | None = Field(default=None, alias="updateTime")
    expiration_time: str | None = Field(default=None, alias="expirationTime")
    sha256_hash: str | None = Field(default=None, alias="sha256Hash")
    uri: str | None = None  # 文件 URI，用于在请求中引用
    download_uri: str | None = Field(default=None, alias="downloadUri")
    state: str | None = None  # PROCESSING, ACTIVE, FAILED
    error: dict[str, Any] | None = None
    # 视频文件元数据
    video_metadata: dict[str, Any] | None = Field(default=None, alias="videoMetadata")


class GeminiFileListResponse(BaseModelWithExtras):
    """
    Gemini 文件列表响应

    用于 files.list API 的响应体
    """

    files: list["GeminiFile"] | None = None
    next_page_token: str | None = Field(default=None, alias="nextPageToken")


class GeminiFileUploadResponse(BaseModelWithExtras):
    """
    Gemini 文件上传响应

    用于 media.upload API 的响应体
    """

    file: GeminiFile | None = None


class GeminiFilePart(BaseModelWithExtras):
    """
    Gemini 文件引用部分

    用于在请求内容中引用已上传的文件
    使用 file_data 字段引用文件 URI
    """

    file_data: dict[str, Any] | None = Field(default=None, alias="fileData")
    # fileData 格式：{"mimeType": "...", "fileUri": "..."}


# ---------------------------------------------------------------------------
# Thought Signature 常量
# ---------------------------------------------------------------------------

# 用于从其他模型迁移对话时绕过签名验证
DUMMY_THOUGHT_SIGNATURE = "context_engineering_is_the_way_to_go"
