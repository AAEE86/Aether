"""
Gemini Chat Handler

处理 Gemini API 格式的请求
"""

from __future__ import annotations

from typing import Any

from src.api.handlers.base.chat_handler_base import ChatHandlerBase


class GeminiChatHandler(ChatHandlerBase):
    """
    Gemini Chat Handler - 处理 Google Gemini API 格式的请求

    格式特点:
    - 使用 promptTokenCount / candidatesTokenCount
    - 支持 cachedContentTokenCount
    - 请求格式: GeminiRequest
    - 响应格式: JSON 数组流（非 SSE）
    """

    FORMAT_ID = "GEMINI"

    async def _resolve_preferred_key_ids(
        self,
        model_name: str,  # noqa: ARG002 - 仅做文件绑定
        request_body: dict[str, Any] | None = None,
    ) -> list[str] | None:
        """
        从 files/xxx 绑定关系中解析优先 Key ID 列表。

        Gemini 文件与上传它的 API Key 绑定，必须使用同一 Key 访问。
        此方法从缓存中查找文件→Key 映射，优先使用正确的 Key。

        注意事项：
        - 如果映射缺失（缓存过期/重启），会记录警告，请求可能失败
        - 如果多个文件属于不同 Key，只能使用其中一个，其他文件可能无法访问
        """
        from src.core.logger import logger
        from src.services.gemini_files_mapping import (
            extract_file_names_from_request,
            get_file_key_mapping,
        )

        file_names = extract_file_names_from_request(request_body or {})
        if not file_names:
            return None

        preferred_key_ids: list[str] = []
        unmapped_files: list[str] = []  # 记录找不到映射的文件

        for file_name in file_names:
            key_id = await get_file_key_mapping(file_name)
            if key_id:
                if key_id not in preferred_key_ids:
                    preferred_key_ids.append(key_id)
            else:
                unmapped_files.append(file_name)

        # 警告：映射缺失
        if unmapped_files:
            logger.warning(
                f"[{self.request_id}] Gemini 文件→Key 映射缺失: {unmapped_files}，"
                "请求可能失败（文件属于其他 Key 或映射已过期）"
            )

        # 警告：多个文件属于不同 Key
        if len(preferred_key_ids) > 1:
            logger.warning(
                f"[{self.request_id}] 请求使用了多个文件，但它们属于不同的 Key: "
                f"{preferred_key_ids}，只能使用第一个 Key，其他文件可能无法访问"
            )

        return preferred_key_ids or None

    def extract_model_from_request(
        self,
        request_body: dict[str, Any],
        path_params: dict[str, Any] | None = None,
    ) -> str:
        """
        从请求中提取模型名 - Gemini Chat 格式实现

        Gemini Chat 模式下，model 在请求体中（经过转换后的 GeminiRequest）。
        与 Gemini CLI 不同，CLI 模式的 model 在 URL 路径中。

        Args:
            request_body: 请求体
            path_params: URL 路径参数（Chat 模式通常不使用）

        Returns:
            模型名
        """
        # 优先从请求体获取，其次从 path_params
        model = request_body.get("model")
        if model:
            return str(model)
        if path_params and "model" in path_params:
            return str(path_params["model"])
        return "unknown"

    async def _convert_request(self, request):
        """
        将请求转换为 Gemini 格式的 Pydantic 对象

        注意：此方法只做类型转换（dict → Pydantic），不做跨格式转换。
        跨格式转换由 FallbackOrchestrator 在选中候选后、发送请求前执行，
        并受全局开关和端点配置控制。

        Args:
            request: 原始请求对象（应已是 Gemini 格式）

        Returns:
            GeminiRequest 对象
        """
        from src.models.gemini import GeminiRequest

        # 如果已经是 Gemini 格式 Pydantic 对象，直接返回
        if isinstance(request, GeminiRequest):
            return request

        # 如果是字典，转换为 Pydantic 对象（假设已是 Gemini 格式）
        if isinstance(request, dict):
            return GeminiRequest(**request)

        return request

    def _extract_usage(self, response: dict) -> dict[str, int]:
        """
        从 Gemini 响应中提取 token 使用情况

        调用 GeminiStreamParser.extract_usage 作为单一实现源
        """
        from src.api.handlers.gemini.stream_parser import GeminiStreamParser

        usage = GeminiStreamParser().extract_usage(response)

        if not usage:
            return {
                "input_tokens": 0,
                "output_tokens": 0,
                "cache_creation_input_tokens": 0,
                "cache_read_input_tokens": 0,
            }

        return {
            "input_tokens": usage.get("input_tokens", 0),
            "output_tokens": usage.get("output_tokens", 0),
            "cache_creation_input_tokens": 0,  # Gemini 不区分缓存创建
            "cache_read_input_tokens": usage.get("cached_tokens", 0),
        }

    def _normalize_response(self, response: dict) -> dict:
        """
        规范化 Gemini 响应

        Args:
            response: 原始响应

        Returns:
            规范化后的响应
        """
        # 作为中转站，直接透传响应，不做标准化处理
        return response
