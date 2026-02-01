"""
CLI Provider Adapter 共享工具函数

提供各适配器共用的工具函数。
"""

from __future__ import annotations

import base64
import hashlib
import json
import uuid
from typing import Any


def generate_claude_code_user_id() -> str:
    """
    生成 ClaudeCode 格式的 user_id

    格式: user_{sha256_hex}_account__session_{uuid}

    参考: done-hub/providers/claudecode/chat.go:generateClaudeCodeUserId
    """
    # 生成随机 session UUID 并计算其 SHA256 哈希
    session_uuid = str(uuid.uuid4())
    user_hash = hashlib.sha256(session_uuid.encode()).hexdigest()

    # 生成另一个 session ID
    session_id = str(uuid.uuid4())

    return f"user_{user_hash}_account__session_{session_id}"


def extract_jwt_claim(token: str, claim: str) -> Any | None:
    """
    从 JWT token 中提取指定的 claim

    仅解析 payload 部分，不验证签名。

    Args:
        token: JWT token 字符串
        claim: 要提取的 claim 名称

    Returns:
        claim 的值，如果不存在或解析失败返回 None
    """
    try:
        parts = token.split(".")
        if len(parts) != 3:
            return None

        # 解码 payload（第二部分）
        payload_b64 = parts[1]
        # 补齐 padding
        padding = 4 - len(payload_b64) % 4
        if padding != 4:
            payload_b64 += "=" * padding

        payload_json = base64.urlsafe_b64decode(payload_b64)
        payload = json.loads(payload_json)

        return payload.get(claim)
    except Exception:
        return None


def extract_chatgpt_account_id(access_token: str) -> str | None:
    """
    从 Codex/OpenAI access token 中提取 chatgpt_account_id

    JWT claim 路径: https://api.openai.com/auth.chatgpt_account_id

    参考: done-hub/providers/codex/base.go
    """
    return extract_jwt_claim(access_token, "https://api.openai.com/auth.chatgpt_account_id")


def extract_bearer_token(auth_value: str | None) -> str | None:
    """
    从 Authorization header 值中提取 Bearer token

    Args:
        auth_value: Authorization header 的值（如 "Bearer xxx"）

    Returns:
        token 字符串，或 None
    """
    if not auth_value:
        return None
    if auth_value.startswith("Bearer "):
        return auth_value[7:]
    return auth_value


def generate_request_id() -> str:
    """
    生成请求 ID

    格式: agent-{uuid}
    """
    return f"agent-{uuid.uuid4()}"


def set_header_if_absent(headers: dict[str, str], name: str, value: str) -> None:
    """
    如果请求头不存在则设置

    Args:
        headers: 请求头字典
        name: 请求头名称
        value: 请求头值
    """
    if name.lower() not in {k.lower() for k in headers}:
        headers[name] = value
