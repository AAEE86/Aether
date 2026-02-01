"""
CLI Provider Adapter 基类

为 CLI 提供商（claude_code, codex, gemini_cli, antigravity）提供请求/响应变换能力。
这些提供商需要特殊处理：请求包装、响应解包、请求头注入、请求体字段变换等。

设计目标：
1. 插入点明确 - 在 PassthroughRequestBuilder.build() 之后、HTTP 请求之前
2. 最小侵入 - 不修改现有格式转换逻辑，只做额外变换
3. 可扩展 - 新 CLI 提供商只需实现适配器类
"""

from __future__ import annotations

from abc import ABC, abstractmethod
from dataclasses import dataclass, field
from typing import Any


@dataclass
class AdapterContext:
    """
    传递给适配器方法的上下文信息

    包含请求处理过程中的关键信息，供适配器做决策。
    """

    # 认证类型：claude_code, codex, gemini_cli, antigravity
    auth_type: str

    # 客户端请求的模型名
    model: str

    # 映射后的模型名（如果有映射）
    mapped_model: str | None = None

    # 是否为流式请求
    is_stream: bool = True

    # 原始请求体（用于提取 metadata 等）
    original_request_body: dict[str, Any] = field(default_factory=dict)

    # 解密后的 auth_config（含 project_id、refresh_token 等）
    auth_config: dict[str, Any] | None = None

    # OAuth2 access token（用于 JWT 解析等）
    access_token: str | None = None

    # 原始请求头
    original_headers: dict[str, str] = field(default_factory=dict)


@dataclass
class TransformedRequest:
    """
    请求变换结果

    适配器的 transform_request 方法返回此对象。
    """

    # 变换后的请求体
    payload: dict[str, Any]

    # 变换后的请求头
    headers: dict[str, str]

    # 可选的 URL 覆盖（如果需要修改请求 URL）
    url_override: str | None = None


class CliProviderAdapter(ABC):
    """
    CLI 提供商适配器抽象基类

    每个需要特殊处理的 CLI 提供商实现此接口：
    - ClaudeCode: 请求头注入、system 指令、metadata.user_id
    - Codex: 模型名规范化、store=false、chatgpt-account-id 头
    - GeminiCli: 请求包装、响应解包
    - Antigravity: tool 变换、请求包装、响应解包
    """

    @abstractmethod
    def transform_request(
        self,
        ctx: AdapterContext,
        payload: dict[str, Any],
        headers: dict[str, str],
        url: str,
    ) -> TransformedRequest:
        """
        变换发送给上游提供商的请求

        在 PassthroughRequestBuilder.build() 和 build_provider_url() 之后调用，
        HTTP 请求发送之前。

        Args:
            ctx: 适配器上下文（含模型名、认证信息等）
            payload: 当前请求体（已经过格式转换和模型映射）
            headers: 当前请求头（已包含认证头）
            url: 当前请求 URL

        Returns:
            TransformedRequest 包含变换后的 payload、headers 和可选的 url_override
        """
        ...

    def transform_response_line(
        self,
        ctx: AdapterContext,
        raw_line: bytes,
    ) -> bytes | None:
        """
        变换从上游提供商收到的 SSE 响应行

        用于响应解包（如从 {"response": {...}} 中提取内层数据）。

        Args:
            ctx: 适配器上下文
            raw_line: 原始 SSE 行（bytes，含换行符）

        Returns:
            变换后的行（bytes），或 None 表示跳过此行。
            默认实现原样返回。
        """
        return raw_line

    def transform_prefetch_data(
        self,
        ctx: AdapterContext,
        data: dict[str, Any],
    ) -> dict[str, Any] | None:
        """
        变换预读阶段解析的 JSON 数据

        在错误检测之前调用，用于从包装中提取内层数据以便正确检测嵌套错误。

        Args:
            ctx: 适配器上下文
            data: 解析出的 JSON 数据

        Returns:
            变换后的数据，或 None 表示保持原样。
            默认实现返回 None（保持原样）。
        """
        return None

    def normalize_model(self, model: str) -> str:
        """
        规范化模型名

        某些提供商需要模型名转换（如 Codex: gpt-5-* → gpt-5）。

        Args:
            model: 原始模型名

        Returns:
            规范化后的模型名。默认实现原样返回。
        """
        return model
