"""
OpenAI Chat Handler - 基于通用 Chat Handler 基类的简化实现

继承 ChatHandlerBase，只需覆盖格式特定的方法。
代码量从原来的 ~1315 行减少到 ~100 行。
"""

from typing import Any

from src.api.handlers.base.chat_handler_base import ChatHandlerBase


class OpenAIChatHandler(ChatHandlerBase):
    """
    OpenAI Chat Handler - 处理 OpenAI Chat Completions API 格式的请求

    格式特点：
    - 使用 prompt_tokens/completion_tokens
    - 不支持 cache tokens
    - 请求格式：OpenAIRequest
    """

    FORMAT_ID = "OPENAI"

    def extract_model_from_request(
        self,
        request_body: dict[str, Any],
        path_params: dict[str, Any] | None = None,  # noqa: ARG002
    ) -> str:
        """
        从请求中提取模型名 - OpenAI 格式实现

        OpenAI API 的 model 在请求体顶级字段。

        Args:
            request_body: 请求体
            path_params: URL 路径参数（OpenAI 不使用）

        Returns:
            模型名
        """
        model = request_body.get("model")
        return str(model) if model else "unknown"

    def apply_mapped_model(
        self,
        request_body: dict[str, Any],
        mapped_model: str,
    ) -> dict[str, Any]:
        """
        将映射后的模型名应用到请求体

        OpenAI API 的 model 在请求体顶级字段。

        Args:
            request_body: 原始请求体
            mapped_model: 映射后的模型名

        Returns:
            更新了 model 字段的请求体
        """
        result = dict(request_body)
        result["model"] = mapped_model
        return result

    async def _convert_request(self, request):
        """
        将请求转换为 OpenAI 格式的 Pydantic 对象

        注意：此方法只做类型转换（dict → Pydantic），不做跨格式转换。
        跨格式转换由 FallbackOrchestrator 在选中候选后、发送请求前执行，
        并受全局开关和端点配置控制。

        Args:
            request: 原始请求对象（应已是 OpenAI 格式）

        Returns:
            OpenAIRequest 对象
        """
        from src.models.openai import OpenAIRequest

        # 如果已经是 OpenAI 格式 Pydantic 对象，直接返回
        if isinstance(request, OpenAIRequest):
            return request

        # 如果是字典，转换为 Pydantic 对象（假设已是 OpenAI 格式）
        if isinstance(request, dict):
            return OpenAIRequest(**request)

        return request

    def _extract_usage(self, response: dict) -> dict[str, int]:
        """
        从 OpenAI 响应中提取 token 使用情况

        OpenAI 格式使用：
        - prompt_tokens / completion_tokens
        - 不支持 cache tokens
        """
        usage = response.get("usage", {})

        return {
            "input_tokens": usage.get("prompt_tokens", 0),
            "output_tokens": usage.get("completion_tokens", 0),
            "cache_creation_input_tokens": 0,
            "cache_read_input_tokens": 0,
        }

    def _normalize_response(self, response: dict) -> dict:
        """
        规范化 OpenAI 响应

        Args:
            response: 原始响应

        Returns:
            规范化后的响应
        """
        # 作为中转站，直接透传响应，不做标准化处理
        return response
