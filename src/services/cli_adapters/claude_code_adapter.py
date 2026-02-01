"""
ClaudeCode CLI 适配器

处理 Claude Code CLI 的请求/响应变换：
- 注入 system 指令: "You are Claude Code, Anthropic's official CLI for Claude."
- 确保 metadata.user_id 存在
- 注入 ClaudeCode 特定请求头

参考: done-hub/providers/claudecode/chat.go
"""

from __future__ import annotations

from typing import Any

from .base import AdapterContext, CliProviderAdapter, TransformedRequest
from .utils import generate_claude_code_user_id, set_header_if_absent


# ClaudeCode 必需的 system 指令文本
CLAUDE_CODE_INSTRUCTION = "You are Claude Code, Anthropic's official CLI for Claude."


class ClaudeCodeAdapter(CliProviderAdapter):
    """
    ClaudeCode CLI 适配器

    请求变换:
    - 确保 system 字段包含 Claude Code 指令（带 cache_control）
    - 确保 metadata.user_id 存在（生成或保留客户端提供的）
    - 注入 ClaudeCode 必需的请求头

    响应变换:
    - 无需变换（Claude 原生格式）
    """

    def transform_request(
        self,
        ctx: AdapterContext,
        payload: dict[str, Any],
        headers: dict[str, str],
        url: str,
    ) -> TransformedRequest:
        """变换 ClaudeCode 请求"""
        # 复制以避免修改原始数据
        new_payload = dict(payload)
        new_headers = dict(headers)

        # 1. 确保 system 指令存在
        self._ensure_system_instruction(new_payload)

        # 2. 从原始请求体提取 metadata（如果有）
        self._extract_metadata_from_original(ctx, new_payload)

        # 3. 确保 metadata.user_id 存在
        self._ensure_metadata_user_id(new_payload)

        # 4. 注入 ClaudeCode 默认请求头
        self._apply_default_headers(new_headers)

        return TransformedRequest(payload=new_payload, headers=new_headers)

    def _ensure_system_instruction(self, payload: dict[str, Any]) -> None:
        """
        确保 system 字段包含 Claude Code 指令

        处理多种 system 格式：
        - None/空: 设置为包含指令的数组
        - 字符串: 转换为数组，在开头添加指令
        - 数组: 检查是否已包含指令，如果没有则在开头添加
        """
        required_item = {
            "type": "text",
            "text": CLAUDE_CODE_INSTRUCTION,
            "cache_control": {"type": "ephemeral"},
        }

        system = payload.get("system")

        # system 为空
        if system is None or system == "":
            payload["system"] = [required_item]
            return

        # system 是字符串
        if isinstance(system, str):
            if system.strip() == "":
                payload["system"] = [required_item]
                return
            # 检查是否已经以指令开头
            if system.strip().startswith(CLAUDE_CODE_INSTRUCTION):
                return
            # 在开头添加指令
            payload["system"] = [
                required_item,
                {"type": "text", "text": system},
            ]
            return

        # system 是数组
        if isinstance(system, list):
            if len(system) == 0:
                payload["system"] = [required_item]
                return

            # 检查第一项是否已经是正确的指令
            first_item = system[0]
            if isinstance(first_item, dict):
                if (
                    first_item.get("type") == "text"
                    and first_item.get("text") == CLAUDE_CODE_INSTRUCTION
                    and isinstance(first_item.get("cache_control"), dict)
                    and first_item.get("cache_control", {}).get("type") == "ephemeral"
                ):
                    return

            # 检查数组中是否已包含指令（带正确的 cache_control）
            if not self._has_instruction_with_cache(system):
                payload["system"] = [required_item] + list(system)
            return

    def _has_instruction_with_cache(self, system_array: list) -> bool:
        """检查 system 数组是否已包含带 ephemeral cache_control 的指令"""
        for item in system_array:
            if not isinstance(item, dict):
                continue
            if item.get("type") != "text":
                continue
            if item.get("text") != CLAUDE_CODE_INSTRUCTION:
                continue
            cache_control = item.get("cache_control")
            if isinstance(cache_control, dict) and cache_control.get("type") == "ephemeral":
                return True
        return False

    def _extract_metadata_from_original(
        self, ctx: AdapterContext, payload: dict[str, Any]
    ) -> None:
        """从原始请求体中提取 metadata（保留客户端提供的 user_id）"""
        original_body = ctx.original_request_body
        if not original_body:
            return

        original_metadata = original_body.get("metadata")
        if not isinstance(original_metadata, dict):
            return

        # 提取 user_id
        user_id = original_metadata.get("user_id")
        if user_id and isinstance(user_id, str):
            if "metadata" not in payload:
                payload["metadata"] = {}
            if isinstance(payload["metadata"], dict):
                payload["metadata"]["user_id"] = user_id

    def _ensure_metadata_user_id(self, payload: dict[str, Any]) -> None:
        """确保 metadata.user_id 存在（如果没有则生成）"""
        if "metadata" not in payload:
            payload["metadata"] = {}

        metadata = payload["metadata"]
        if not isinstance(metadata, dict):
            payload["metadata"] = {"user_id": generate_claude_code_user_id()}
            return

        if not metadata.get("user_id"):
            metadata["user_id"] = generate_claude_code_user_id()

    def _apply_default_headers(self, headers: dict[str, str]) -> None:
        """应用 ClaudeCode 默认请求头"""
        # anthropic-beta 头
        set_header_if_absent(
            headers,
            "anthropic-beta",
            "claude-code-20250219,oauth-2025-04-20,interleaved-thinking-2025-05-14,fine-grained-tool-streaming-2025-05-14",
        )

        # user-agent
        set_header_if_absent(headers, "user-agent", "claude-cli/1.0.81 (external, cli)")

        # x-stainless-* 头部
        set_header_if_absent(headers, "x-stainless-retry-count", "0")
        set_header_if_absent(headers, "x-stainless-timeout", "60")
        set_header_if_absent(headers, "x-stainless-lang", "js")
        set_header_if_absent(headers, "x-stainless-package-version", "0.55.1")
        set_header_if_absent(headers, "x-stainless-os", "Windows")
        set_header_if_absent(headers, "x-stainless-arch", "x64")
        set_header_if_absent(headers, "x-stainless-runtime", "node")
        set_header_if_absent(headers, "x-stainless-runtime-version", "v20.19.2")

        # 其他必需头部
        set_header_if_absent(headers, "x-app", "cli")
        set_header_if_absent(headers, "anthropic-dangerous-direct-browser-access", "true")
        set_header_if_absent(headers, "accept-language", "*")
        set_header_if_absent(headers, "sec-fetch-mode", "cors")
