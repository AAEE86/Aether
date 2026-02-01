"""
Codex (OpenAI CLI) 适配器

处理 Codex CLI 的请求/响应变换：
- 模型名规范化: gpt-5-* → gpt-5
- 设置 store: false（必需）
- 处理 temperature/top_p 冲突
- 适配 Codex CLI 格式（注入系统提示词）
- 从 JWT 提取 chatgpt-account-id
- 注入必需请求头
- 强制流式请求
- URL 路径: /backend-api/codex/responses

参考: done-hub/providers/codex/chat.go, responses.go, instructions.go
"""

from __future__ import annotations

from typing import Any
from urllib.parse import urlparse, urlunparse

from .base import AdapterContext, CliProviderAdapter, TransformedRequest
from .codex_instructions import CODEX_CLI_INSTRUCTIONS
from .utils import extract_chatgpt_account_id, set_header_if_absent


class CodexAdapter(CliProviderAdapter):
    """
    Codex (OpenAI CLI) 适配器

    请求变换:
    - 模型名规范化: gpt-5-* → gpt-5（除 gpt-5-codex 外）
    - 设置 store: false（Codex API 必需）
    - 处理 temperature/top_p 冲突（保留 temperature，删除 top_p）
    - 强制 stream: true（Codex API 要求）
    - 适配 Codex CLI 格式（非原生请求时注入系统提示词）
    - 从 JWT access_token 提取 chatgpt_account_id
    - 注入必需请求头: User-Agent, chatgpt-account-id, Host
    - URL 覆盖: /backend-api/codex/responses

    响应变换:
    - 无需额外变换（Aether 格式转换系统已处理 Responses → Chat 转换）
    """

    def transform_request(
        self,
        ctx: AdapterContext,
        payload: dict[str, Any],
        headers: dict[str, str],
        url: str,
    ) -> TransformedRequest:
        """变换 Codex 请求"""
        new_payload = dict(payload)
        new_headers = dict(headers)

        # 1. 模型名规范化
        model = new_payload.get("model", ctx.model)
        normalized_model = self.normalize_model(model)
        new_payload["model"] = normalized_model

        # 2. 设置 store: false（Codex API 必需）
        new_payload["store"] = False

        # 3. 处理 temperature/top_p 冲突
        # 当两者都存在时，优先保留 temperature，删除 top_p
        if "temperature" in new_payload and "top_p" in new_payload:
            del new_payload["top_p"]

        # 4. 强制 stream: true（Codex API 要求）
        new_payload["stream"] = True

        # 5. 适配 Codex CLI 格式（注入系统提示词）
        self._adapt_codex_cli(new_payload)

        # 6. 从 JWT 提取 chatgpt-account-id
        if ctx.access_token:
            account_id = extract_chatgpt_account_id(ctx.access_token)
            if account_id:
                new_headers["chatgpt-account-id"] = account_id

        # 7. 注入默认请求头
        self._apply_default_headers(new_headers)

        # 8. 构建 URL
        new_url = self._build_codex_url(url)

        return TransformedRequest(
            payload=new_payload,
            headers=new_headers,
            url_override=new_url,
        )

    def normalize_model(self, model: str) -> str:
        """
        规范化模型名

        gpt-5-* 系列统一为 gpt-5（除 gpt-5-codex 外）
        """
        if not model:
            return model
        if model.startswith("gpt-5-") and model != "gpt-5-codex":
            return "gpt-5"
        return model

    def _apply_default_headers(self, headers: dict[str, str]) -> None:
        """应用 Codex 默认请求头"""
        # Host 头
        set_header_if_absent(headers, "Host", "chatgpt.com")

        # User-Agent
        set_header_if_absent(
            headers,
            "User-Agent",
            "codex_cli_rs/0.38.0 (Ubuntu 22.4.0; x86_64) WindowsTerminal",
        )

        # Accept
        set_header_if_absent(headers, "Accept", "text/event-stream")

    def _build_codex_url(self, original_url: str) -> str:
        """
        构建 Codex URL

        使用 /backend-api/codex/responses 路径，保留原始 URL 的 host
        （支持用户配置的代理地址）
        """
        parsed = urlparse(original_url)

        # 保留原始 URL 的 host（支持代理配置）
        netloc = parsed.netloc or "chatgpt.com"

        # 使用 Codex responses 端点
        new_path = "/backend-api/codex/responses"

        return urlunparse((
            parsed.scheme or "https",
            netloc,
            new_path,
            "",  # params
            "",  # query
            "",  # fragment
        ))

    def _adapt_codex_cli(self, payload: dict[str, Any]) -> None:
        """
        适配 Codex CLI 格式

        检测请求是否已经包含 Codex CLI 的 instructions。
        如果不是 Codex CLI 原生请求，则：
        - 移除不兼容的参数（temperature, top_p, max_output_tokens）
        - 注入标准 Codex CLI 系统提示词

        参考: done-hub/providers/codex/responses.go: adaptCodexCLI
        """
        instructions = payload.get("instructions", "")

        # 检测是否为 Codex CLI 原生请求
        is_codex_cli = (
            isinstance(instructions, str)
            and len(instructions) > 50
            and (
                instructions.startswith("You are a coding agent running in the Codex CLI")
                or instructions.startswith("You are Codex")
            )
        )

        if not is_codex_cli:
            # 非 Codex CLI 请求，移除不兼容的字段并注入提示词
            payload.pop("temperature", None)
            payload.pop("top_p", None)
            payload.pop("max_output_tokens", None)
            payload["instructions"] = CODEX_CLI_INSTRUCTIONS
