"""
GeminiCli 适配器

处理 Gemini CLI 的请求/响应变换：
- 请求包装: {"model", "project", "request": ...}
- 响应解包: 从 {"response": ...} 中提取内层数据
- URL 路径: /v1internal:generateContent 或 /v1internal:streamGenerateContent

参考: done-hub/providers/geminicli/chat.go
"""

from __future__ import annotations

import json
import uuid
from typing import Any
from urllib.parse import urlparse, urlunparse

from .base import AdapterContext, CliProviderAdapter, TransformedRequest


class GeminiCliAdapter(CliProviderAdapter):
    """
    GeminiCli 适配器

    请求变换:
    - 从 auth_config 提取 project_id
    - 包装请求体为 {"model": ..., "project": ..., "request": ...}
    - 清理 function call/response 中的 id 字段（relay 模式）
    - 修改 URL 路径为 /v1internal:generateContent 或 /v1internal:streamGenerateContent

    响应变换:
    - SSE 行解包: 从 {"response": {...}} 中提取内层数据
    """

    def transform_request(
        self,
        ctx: AdapterContext,
        payload: dict[str, Any],
        headers: dict[str, str],
        url: str,
    ) -> TransformedRequest:
        """变换 GeminiCli 请求"""
        # 获取 project_id
        project_id = self._get_project_id(ctx)

        # 获取模型名
        model = ctx.mapped_model or ctx.model

        # 清理 function call/response 中的 id 字段
        cleaned_payload = self._clean_function_ids(payload)

        # 包装请求体
        wrapped_payload = {
            "model": model,
            "project": project_id,
            "request": cleaned_payload,
        }

        # 修改 URL 为 v1internal 格式
        new_url = self._build_internal_url(url, ctx.is_stream)

        return TransformedRequest(
            payload=wrapped_payload,
            headers=headers,
            url_override=new_url,
        )

    def transform_response_line(
        self,
        ctx: AdapterContext,
        raw_line: bytes,
    ) -> bytes | None:
        """
        解包 GeminiCli 响应行

        将 data: {"response": {...}} 转换为 data: {...}
        """
        try:
            line_str = raw_line.decode("utf-8")
        except UnicodeDecodeError:
            return raw_line

        # 跳过非 data 行
        if not line_str.startswith("data: "):
            return raw_line

        # 提取 JSON 数据
        data_str = line_str[6:].strip()
        if not data_str or data_str == "[DONE]":
            return raw_line

        try:
            data = json.loads(data_str)
        except json.JSONDecodeError:
            return raw_line

        # 解包 response 字段
        if isinstance(data, dict) and "response" in data:
            inner_response = data["response"]
            if inner_response is not None:
                unwrapped_json = json.dumps(inner_response, ensure_ascii=False)
                # 保持原始格式（包含换行符）
                if raw_line.endswith(b"\n"):
                    return f"data: {unwrapped_json}\n".encode("utf-8")
                return f"data: {unwrapped_json}".encode("utf-8")

        return raw_line

    def transform_prefetch_data(
        self,
        ctx: AdapterContext,
        data: dict[str, Any],
    ) -> dict[str, Any] | None:
        """解包预读数据中的 response 字段"""
        if "response" in data:
            inner = data.get("response")
            if isinstance(inner, dict):
                return inner
        return None

    def _get_project_id(self, ctx: AdapterContext) -> str:
        """从 auth_config 获取 project_id，如果为空则生成随机值"""
        if ctx.auth_config:
            # 尝试直接获取 project_id
            project_id = ctx.auth_config.get("project_id")
            if project_id:
                return str(project_id)

            # 尝试从 token_data 获取
            token_data = ctx.auth_config.get("token_data", {})
            if isinstance(token_data, dict):
                project_id = token_data.get("project_id")
                if project_id:
                    return str(project_id)

        # 无 project_id 时生成随机值（与 done-hub 保持一致）
        return self._generate_random_project_id()

    def _generate_random_project_id(self) -> str:
        """生成随机 project_id"""
        adjectives = ["useful", "bright", "swift", "calm", "bold"]
        nouns = ["fuze", "wave", "spark", "flow", "core"]
        uid = uuid.uuid4()
        adj = adjectives[uid.int % len(adjectives)]
        noun = nouns[uid.int % len(nouns)]
        random_part = str(uid)[:5].lower()
        return f"{adj}-{noun}-{random_part}"

    def _clean_function_ids(self, payload: dict[str, Any]) -> dict[str, Any]:
        """
        清理 contents 中 function_call/function_response 的 id 字段

        GeminiCli 的 relay 模式不需要这些 id 字段。
        """
        if "contents" not in payload:
            return payload

        # 深拷贝以避免修改原始数据
        result = dict(payload)
        contents = payload.get("contents")
        if not isinstance(contents, list):
            return result

        new_contents = []
        for content in contents:
            if not isinstance(content, dict):
                new_contents.append(content)
                continue

            new_content = dict(content)

            # 确保有 role 字段
            if "role" not in new_content:
                new_content["role"] = "user"

            # 清理 parts 中的 id 字段
            parts = new_content.get("parts")
            if isinstance(parts, list):
                new_parts = []
                for part in parts:
                    if not isinstance(part, dict):
                        new_parts.append(part)
                        continue

                    new_part = dict(part)

                    # 清理 functionCall 和 function_call
                    for fn_key in ("functionCall", "function_call"):
                        if fn_key in new_part and isinstance(new_part[fn_key], dict):
                            fn_call = dict(new_part[fn_key])
                            fn_call.pop("id", None)
                            new_part[fn_key] = fn_call

                    # 清理 functionResponse 和 function_response
                    for fn_key in ("functionResponse", "function_response"):
                        if fn_key in new_part and isinstance(new_part[fn_key], dict):
                            fn_resp = dict(new_part[fn_key])
                            fn_resp.pop("id", None)
                            new_part[fn_key] = fn_resp

                    new_parts.append(new_part)

                new_content["parts"] = new_parts

            new_contents.append(new_content)

        result["contents"] = new_contents
        return result

    def _build_internal_url(self, original_url: str, is_stream: bool) -> str:
        """
        构建 v1internal URL

        格式: {base_url}/v1internal:generateContent 或 /v1internal:streamGenerateContent
        流式请求添加 ?alt=sse 参数
        """
        parsed = urlparse(original_url)

        # 确定 action
        action = "streamGenerateContent" if is_stream else "generateContent"

        # 构建新路径
        new_path = f"/v1internal:{action}"

        # 构建查询参数
        if is_stream:
            new_query = "alt=sse"
        else:
            new_query = ""

        # 重建 URL
        new_url = urlunparse((
            parsed.scheme,
            parsed.netloc,
            new_path,
            "",  # params
            new_query,
            "",  # fragment
        ))

        return new_url
