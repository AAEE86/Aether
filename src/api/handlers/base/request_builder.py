"""
请求构建器 - 透传模式

透传模式 (Passthrough): CLI 和 Chat 等场景，原样转发请求体和头部
- 清理敏感头部：authorization, x-api-key, host, content-length 等
- 保留所有其他头部和请求体字段
- 适用于：Claude CLI、OpenAI CLI、Chat API 等场景

使用方式：
    builder = PassthroughRequestBuilder()
    payload, headers = builder.build(original_body, original_headers, endpoint, key)
"""

from __future__ import annotations

import copy
import json
import time
from abc import ABC, abstractmethod
from dataclasses import dataclass
from typing import TYPE_CHECKING, Any

import httpx
from sqlalchemy.orm import object_session

from src.clients.redis_client import get_redis_client
from src.core.api_format import (
    UPSTREAM_DROP_HEADERS,
    HeaderBuilder,
    get_auth_config_for_endpoint,
    make_signature_key,
)
from src.core.crypto import crypto_service
from src.core.logger import logger
from src.core.provider_oauth_utils import enrich_auth_config, post_oauth_token

if TYPE_CHECKING:
    from src.models.database import ProviderAPIKey, ProviderEndpoint


# ==============================================================================
# Service Account 认证结果类型
# ==============================================================================


@dataclass
class ProviderAuthInfo:
    """Provider 认证信息（用于 Service Account 等异步认证场景）"""

    auth_header: str
    auth_value: str
    # 解密后的认证配置（用于 URL 构建等场景，避免重复解密）
    decrypted_auth_config: dict[str, Any] | None = None

    def as_tuple(self) -> tuple[str, str]:
        """返回 (auth_header, auth_value) 元组"""
        return (self.auth_header, self.auth_value)


# ==============================================================================
# 统一的头部配置常量
# ==============================================================================

# 兼容别名：历史代码使用 SENSITIVE_HEADERS 命名
SENSITIVE_HEADERS: frozenset[str] = UPSTREAM_DROP_HEADERS

# 请求体中受保护的字段（不能被 body_rules 修改）
PROTECTED_BODY_FIELDS: frozenset[str] = frozenset(
    {
        "model",  # 模型名由系统管理
        "stream",  # 流式标志由系统管理
    }
)


# ==============================================================================
# 测试请求常量与辅助函数
# ==============================================================================

# 标准测试请求体（OpenAI 格式）
# 用于 check_endpoint 等测试场景，使用简单安全的消息内容避免触发安全过滤
DEFAULT_TEST_REQUEST: dict[str, Any] = {
    "messages": [{"role": "user", "content": "Hi"}],
    "max_tokens": 5,
    "temperature": 0,
}


def get_test_request_data(request_data: dict[str, Any] | None = None) -> dict[str, Any]:
    """获取测试请求数据

    如果传入 request_data，则合并到默认测试请求中；
    否则使用默认测试请求。

    Args:
        request_data: 用户提供的请求数据（会覆盖默认值）

    Returns:
        合并后的测试请求数据（OpenAI 格式）
    """
    if request_data:
        merged = DEFAULT_TEST_REQUEST.copy()
        merged.update(request_data)
        return merged
    return DEFAULT_TEST_REQUEST.copy()


def build_test_request_body(
    format_id: str,
    request_data: dict[str, Any] | None = None,
    *,
    target_variant: str | None = None,
) -> dict[str, Any]:
    """构建测试请求体，自动处理格式转换

    使用格式转换注册表将 OpenAI 格式的测试请求转换为目标格式。

    Args:
        format_id: 目标 endpoint signature（如 "claude:chat", "gemini:chat", "openai:cli"）
        request_data: 可选的请求数据，会与默认测试请求合并
        target_variant: 目标变体（如 "codex"），用于同格式但有细微差异的上游

    Returns:
        转换为目标 API 格式的请求体
    """
    from src.core.api_format.conversion import (
        format_conversion_registry,
        register_default_normalizers,
    )

    register_default_normalizers()

    # 获取测试请求数据（OpenAI 格式）
    source_data = get_test_request_data(request_data)

    # 直接使用目标格式进行转换，不再转换为基础格式
    # 这样 openai:cli 会正确转换为 Responses API 格式
    return format_conversion_registry.convert_request(
        source_data,
        make_signature_key("openai", "chat"),
        format_id,
        target_variant=target_variant,
    )


# ==============================================================================
# 请求体规则应用
# ==============================================================================


def _parse_path(path: str) -> list[str]:
    """
    解析点号路径，支持转义（用 \\.
    表示字面量点号）。

    Examples:
        "metadata.user.name" -> ["metadata", "user", "name"]
        "config\\.v1.enabled" -> ["config.v1", "enabled"]

    约束：
        - 不允许空段（例如：".a" / "a." / "a..b"），遇到则返回空列表表示无效路径。
        - 仅对 "\\." 做特殊处理；其他反斜杠组合按字面量保留。
    """
    raw = (path or "").strip()
    if not raw:
        return []

    parts: list[str] = []
    current: list[str] = []

    i = 0
    while i < len(raw):
        ch = raw[i]
        if ch == "\\" and i + 1 < len(raw) and raw[i + 1] == ".":
            current.append(".")
            i += 2
            continue

        if ch == ".":
            if not current:
                return []
            parts.append("".join(current))
            current = []
            i += 1
            continue

        current.append(ch)
        i += 1

    if not current:
        return []

    parts.append("".join(current))
    return parts


def _get_nested_value(obj: dict[str, Any], path: str) -> tuple[bool, Any]:
    """
    获取嵌套值

    Returns:
        (found, value) - found 为 True 时 value 有效
    """
    parts = _parse_path(path)
    if not parts:
        return False, None

    current: Any = obj
    for key in parts:
        if isinstance(current, dict) and key in current:
            current = current[key]
        else:
            return False, None
    return True, current


def _set_nested_value(obj: dict[str, Any], path: str, value: Any) -> bool:
    """
    设置嵌套值，自动创建中间层级。

    当中间层存在但不是 dict 时，会覆盖为 dict 后继续写入（覆写语义）。

    Returns:
        True: 写入成功
        False: 路径无效（空/含空段等）
    """
    parts = _parse_path(path)
    if not parts:
        return False

    current: dict[str, Any] = obj
    for key in parts[:-1]:
        next_val = current.get(key)
        if not isinstance(next_val, dict):
            next_val = {}
            current[key] = next_val
        current = next_val

    current[parts[-1]] = value
    return True


def _delete_nested_value(obj: dict[str, Any], path: str) -> bool:
    """
    删除嵌套值

    Returns:
        True: 删除成功
        False: 路径不存在或无效
    """
    parts = _parse_path(path)
    if not parts:
        return False

    current: Any = obj
    for key in parts[:-1]:
        if isinstance(current, dict) and key in current:
            current = current[key]
        else:
            return False
        if not isinstance(current, dict):
            return False

    if isinstance(current, dict) and parts[-1] in current:
        del current[parts[-1]]
        return True
    return False


def _rename_nested_value(obj: dict[str, Any], from_path: str, to_path: str) -> bool:
    """
    重命名嵌套值（移动到新路径）

    Returns:
        True: 重命名成功
        False: 源路径不存在或路径无效
    """
    src = (from_path or "").strip()
    dst = (to_path or "").strip()
    if not src or not dst:
        return False
    if src == dst:
        found, _ = _get_nested_value(obj, src)
        return found

    found, value = _get_nested_value(obj, src)
    if not found:
        return False

    _delete_nested_value(obj, src)
    _set_nested_value(obj, dst, value)
    return True


def apply_body_rules(
    body: dict[str, Any],
    rules: list[dict[str, Any]],
    protected_keys: frozenset[str] | None = None,
) -> dict[str, Any]:
    """
    应用请求体规则

    路径语法：
    - 使用点号分隔层级：metadata.user.name
    - 转义字面量点号：config\\.v1.enabled -> key "config.v1" 下的 "enabled"

    支持的规则类型：
    - set: 设置/覆盖字段 {"action": "set", "path": "metadata.user_id", "value": 123}
    - drop: 删除字段 {"action": "drop", "path": "unwanted_field"}
    - rename: 重命名字段 {"action": "rename", "from": "old.key", "to": "new.key"}

    Args:
        body: 原始请求体
        rules: 规则列表
        protected_keys: 受保护的字段（不能被 set/drop/rename 修改）

    Returns:
        应用规则后的请求体
    """
    if not rules:
        return body

    # 深拷贝，避免修改原始数据（尤其是嵌套 dict）
    result = copy.deepcopy(body)
    protected = protected_keys or PROTECTED_BODY_FIELDS
    protected_lower = frozenset(str(k).lower() for k in protected)

    for rule in rules:
        if not isinstance(rule, dict):
            continue

        action = rule.get("action")
        if not isinstance(action, str):
            continue
        action = action.strip().lower()

        if action == "set":
            raw_path = rule.get("path", "")
            if not isinstance(raw_path, str):
                continue
            path = raw_path.strip()
            value = rule.get("value")
            parts = _parse_path(path)
            if not parts:
                continue
            if parts[0].lower() in protected_lower:
                continue
            _set_nested_value(result, path, value)

        elif action == "drop":
            raw_path = rule.get("path", "")
            if not isinstance(raw_path, str):
                continue
            path = raw_path.strip()
            parts = _parse_path(path)
            if not parts:
                continue
            if parts[0].lower() in protected_lower:
                continue
            _delete_nested_value(result, path)

        elif action == "rename":
            raw_from = rule.get("from", "")
            raw_to = rule.get("to", "")
            if not isinstance(raw_from, str) or not isinstance(raw_to, str):
                continue
            from_path = raw_from.strip()
            to_path = raw_to.strip()
            if not from_path or not to_path:
                continue
            from_parts = _parse_path(from_path)
            to_parts = _parse_path(to_path)
            if not from_parts or not to_parts:
                continue

            # 受保护字段只检查顶层 key
            if from_parts[0].lower() in protected_lower or to_parts[0].lower() in protected_lower:
                continue

            _rename_nested_value(result, from_path, to_path)

    return result


# ==============================================================================
# 请求构建器
# ==============================================================================


class RequestBuilder(ABC):
    """请求构建器抽象基类"""

    @abstractmethod
    def build_payload(
        self,
        original_body: dict[str, Any],
        *,
        mapped_model: str | None = None,
        is_stream: bool = False,
    ) -> dict[str, Any]:
        """构建请求体"""
        pass

    @abstractmethod
    def build_headers(
        self,
        original_headers: dict[str, str],
        endpoint: Any,
        key: Any,
        *,
        extra_headers: dict[str, str] | None = None,
        pre_computed_auth: tuple[str, str] | None = None,
    ) -> dict[str, str]:
        """构建请求头"""
        pass

    def build(
        self,
        original_body: dict[str, Any],
        original_headers: dict[str, str],
        endpoint: Any,
        key: Any,
        *,
        mapped_model: str | None = None,
        is_stream: bool = False,
        extra_headers: dict[str, str] | None = None,
        pre_computed_auth: tuple[str, str] | None = None,
    ) -> tuple[dict[str, Any], dict[str, str]]:
        """
        构建完整的请求（请求体 + 请求头）

        Args:
            original_body: 原始请求体
            original_headers: 原始请求头
            endpoint: 端点配置
            key: Provider API Key
            mapped_model: 映射后的模型名
            is_stream: 是否为流式请求
            extra_headers: 额外请求头
            pre_computed_auth: 预先计算的认证信息 (auth_header, auth_value)

        Returns:
            Tuple[payload, headers]
        """
        payload = self.build_payload(
            original_body,
            mapped_model=mapped_model,
            is_stream=is_stream,
        )

        # 应用请求体规则（如果 endpoint 配置了 body_rules）
        body_rules = getattr(endpoint, "body_rules", None)
        if body_rules:
            payload = apply_body_rules(payload, body_rules)

        headers = self.build_headers(
            original_headers,
            endpoint,
            key,
            extra_headers=extra_headers,
            pre_computed_auth=pre_computed_auth,
        )
        return payload, headers


class PassthroughRequestBuilder(RequestBuilder):
    """
    透传模式请求构建器

    适用于 CLI 等场景，尽量保持请求原样：
    - 请求体：直接复制，只修改必要字段（model, stream）
    - 请求头：清理敏感头部（黑名单），透传其他所有头部
    """

    def build_payload(
        self,
        original_body: dict[str, Any],
        *,
        mapped_model: str | None = None,  # noqa: ARG002 - 由 apply_mapped_model 处理
        is_stream: bool = False,  # noqa: ARG002 - 保留原始值，不自动添加
    ) -> dict[str, Any]:
        """
        透传请求体 - 原样复制，不做任何修改

        透传模式下：
        - model: 由各 handler 的 apply_mapped_model 方法处理
        - stream: 保留客户端原始值（不同 API 处理方式不同）
        """
        return dict(original_body)

    def build_headers(
        self,
        original_headers: dict[str, str],
        endpoint: Any,
        key: Any,
        *,
        extra_headers: dict[str, str] | None = None,
        pre_computed_auth: tuple[str, str] | None = None,
    ) -> dict[str, str]:
        """
        透传请求头 - 清理敏感头部（黑名单），透传其他所有头部

        Args:
            original_headers: 原始请求头
            endpoint: 端点配置
            key: Provider API Key
            extra_headers: 额外请求头
            pre_computed_auth: 预先计算的认证信息 (auth_header, auth_value)，
                               用于 Service Account 等异步获取 token 的场景
        """
        # 1. 根据 API 格式自动设置认证头
        if pre_computed_auth:
            # 使用预先计算的认证信息（Service Account 等场景）
            auth_header, auth_value = pre_computed_auth
        else:
            # 标准 API Key 认证
            decrypted_key = crypto_service.decrypt(key.api_key)
            raw_family = getattr(endpoint, "api_family", None)
            raw_kind = getattr(endpoint, "endpoint_kind", None)
            endpoint_sig: str | None = None
            if (
                isinstance(raw_family, str)
                and isinstance(raw_kind, str)
                and raw_family
                and raw_kind
            ):
                endpoint_sig = make_signature_key(raw_family, raw_kind)
            else:
                # 兜底：允许 endpoint.api_format 已经是 signature key 的情况
                raw_format = getattr(endpoint, "api_format", None)
                if isinstance(raw_format, str) and ":" in raw_format:
                    endpoint_sig = raw_format

            auth_header, auth_type = get_auth_config_for_endpoint(endpoint_sig or "openai:chat")
            auth_value = f"Bearer {decrypted_key}" if auth_type == "bearer" else decrypted_key
        # 认证头始终受保护，防止 header_rules 覆盖
        protected_keys = {auth_header.lower(), "content-type"}

        builder = HeaderBuilder()

        # 2. 透传原始头部（排除默认敏感头部）
        if original_headers:
            for name, value in original_headers.items():
                if name.lower() in SENSITIVE_HEADERS:
                    continue
                builder.add(name, value)

        # 3. 应用 endpoint 的请求头规则（认证头受保护，无法通过 rules 设置）
        header_rules = getattr(endpoint, "header_rules", None)
        if header_rules:
            builder.apply_rules(header_rules, protected_keys)

        # 4. 添加额外头部
        if extra_headers:
            builder.add_many(extra_headers)

        # 5. 设置认证头（最高优先级，上游始终使用 header 认证）
        builder.add(auth_header, auth_value)

        # 6. 确保有 Content-Type
        headers = builder.build()
        if not any(k.lower() == "content-type" for k in headers):
            headers["Content-Type"] = "application/json"

        return headers


# ==============================================================================
# 便捷函数
# ==============================================================================


def build_passthrough_request(
    original_body: dict[str, Any],
    original_headers: dict[str, str],
    endpoint: Any,
    key: Any,
) -> tuple[dict[str, Any], dict[str, str]]:
    """
    构建透传模式的请求

    纯透传：原样复制请求体，只处理请求头（认证等）。
    model mapping 和 stream 由调用方自行处理（不同 API 格式处理方式不同）。
    """
    builder = PassthroughRequestBuilder()
    return builder.build(
        original_body,
        original_headers,
        endpoint,
        key,
    )


# ==============================================================================
# Service Account 认证支持
# ==============================================================================


async def get_provider_auth(
    endpoint: "ProviderEndpoint",
    key: "ProviderAPIKey",
    *,
    force_refresh: bool = False,
) -> ProviderAuthInfo | None:
    """
    获取 Provider 的认证信息

    对于标准 API Key，返回 None（由 build_headers 自动处理）。
    对于 Service Account，异步获取 Access Token 并返回认证信息。

    Args:
        endpoint: 端点配置
        key: Provider API Key

    Returns:
        Service Account 场景: ProviderAuthInfo 对象（包含认证信息和解密后的配置）
        API Key 场景: None（由 build_headers 处理）

    Raises:
        InvalidRequestException: 认证配置无效或认证失败
    """
    from src.core.exceptions import InvalidRequestException

    auth_type = getattr(key, "auth_type", "api_key")

    if auth_type == "oauth":
        # OAuth token 保存在 key.api_key（加密），refresh_token/expires_at 等在 auth_config（加密 JSON）中。
        # 在请求前做一次懒刷新：接近过期时刷新 access_token，并用 Redis lock 避免并发风暴。
        encrypted_auth_config = getattr(key, "auth_config", None)
        if encrypted_auth_config:
            try:
                decrypted_config = crypto_service.decrypt(encrypted_auth_config)
                token_meta = json.loads(decrypted_config)
            except Exception:
                token_meta = {}
        else:
            token_meta = {}

        expires_at = token_meta.get("expires_at")
        refresh_token = token_meta.get("refresh_token")
        provider_type = str(token_meta.get("provider_type") or "")

        # 120s skew (or force refresh when upstream returns 401)
        should_refresh = False
        try:
            if expires_at is not None:
                should_refresh = int(time.time()) >= int(expires_at) - 120
        except Exception:
            should_refresh = False

        if force_refresh:
            should_refresh = True

        if should_refresh and refresh_token and provider_type:
            try:
                from src.core.provider_templates.fixed_providers import FIXED_PROVIDERS
                from src.core.provider_templates.types import ProviderType

                try:
                    template = FIXED_PROVIDERS.get(ProviderType(provider_type))
                except Exception:
                    template = None
                if provider_type == ProviderType.KIRO.value:
                    from src.services.provider.adapters.kiro.models.credentials import KiroAuthConfig
                    from src.services.provider.adapters.kiro.token_manager import (
                        refresh_access_token,
                        validate_refresh_token,
                    )

                    cfg = KiroAuthConfig.from_dict(token_meta or {})
                    if not (cfg.refresh_token or "").strip():
                        raise InvalidRequestException(
                            "Kiro auth_config missing refresh_token; please re-import credentials."
                        )

                    redis = await get_redis_client(require_redis=False)
                    lock_key = f"provider_oauth_refresh_lock:{key.id}"
                    got_lock = False
                    if redis is not None:
                        try:
                            got_lock = bool(await redis.set(lock_key, "1", ex=30, nx=True))
                        except Exception:
                            got_lock = False

                    if got_lock or redis is None:
                        try:
                            proxy_config = None
                            try:
                                provider = getattr(key, "provider", None) or getattr(endpoint, "provider", None)
                                proxy_config = getattr(provider, "proxy", None)
                            except Exception:
                                proxy_config = None

                            validate_refresh_token(cfg.refresh_token)
                            access_token, new_cfg = await refresh_access_token(
                                cfg,
                                proxy_config=proxy_config,
                            )
                            token_meta = new_cfg.to_dict()
                            token_meta["updated_at"] = int(time.time())

                            key.api_key = crypto_service.encrypt(access_token)
                            key.auth_config = crypto_service.encrypt(json.dumps(token_meta))

                            sess = object_session(key)
                            if sess is not None:
                                sess.add(key)
                                sess.commit()
                            else:
                                logger.warning(
                                    "[OAUTH_REFRESH] key {} refreshed but cannot persist (no session); "
                                    "next request will refresh again",
                                    key.id,
                                )
                        finally:
                            if got_lock and redis is not None:
                                try:
                                    await redis.delete(lock_key)
                                except Exception:
                                    pass
                elif template:
                    redis = await get_redis_client(require_redis=False)
                    lock_key = f"provider_oauth_refresh_lock:{key.id}"
                    got_lock = False
                    if redis is not None:
                        try:
                            got_lock = bool(await redis.set(lock_key, "1", ex=30, nx=True))
                        except Exception:
                            got_lock = False

                    if got_lock or redis is None:
                        try:
                            token_url = template.oauth.token_url
                            is_json = "anthropic.com" in token_url

                            if is_json:
                                body: dict[str, Any] = {
                                    "grant_type": "refresh_token",
                                    "client_id": template.oauth.client_id,
                                    "refresh_token": str(refresh_token),
                                }
                                headers = {
                                    "Content-Type": "application/json",
                                    "Accept": "application/json",
                                }
                                data = None
                                json_body = body
                            else:
                                form: dict[str, str] = {
                                    "grant_type": "refresh_token",
                                    "client_id": template.oauth.client_id,
                                    "refresh_token": str(refresh_token),
                                }
                                if template.oauth.client_secret:
                                    form["client_secret"] = template.oauth.client_secret
                                headers = {
                                    "Content-Type": "application/x-www-form-urlencoded",
                                    "Accept": "application/json",
                                }
                                data = form
                                json_body = None

                            proxy_config = None
                            try:
                                provider = getattr(key, "provider", None)
                                proxy_config = getattr(provider, "proxy", None)
                            except Exception:
                                proxy_config = None

                            resp = await post_oauth_token(
                                provider_type=provider_type,
                                token_url=token_url,
                                headers=headers,
                                data=data,
                                json_body=json_body,
                                proxy_config=proxy_config,
                                timeout_seconds=30.0,
                            )

                            if 200 <= resp.status_code < 300:
                                token = resp.json()
                                access_token = str(token.get("access_token") or "")
                                new_refresh_token = str(token.get("refresh_token") or "")
                                expires_in = token.get("expires_in")
                                new_expires_at: int | None = None
                                try:
                                    if expires_in is not None:
                                        new_expires_at = int(time.time()) + int(expires_in)
                                except Exception:
                                    new_expires_at = None

                                if access_token:
                                    token_meta["token_type"] = token.get("token_type")
                                    if new_refresh_token:
                                        token_meta["refresh_token"] = new_refresh_token
                                    token_meta["expires_at"] = new_expires_at
                                    token_meta["scope"] = token.get("scope")
                                    token_meta["updated_at"] = int(time.time())

                                    token_meta = await enrich_auth_config(
                                        provider_type=provider_type,
                                        auth_config=token_meta,
                                        token_response=token,
                                        access_token=access_token,
                                        proxy_config=proxy_config,
                                    )

                                    key.api_key = crypto_service.encrypt(access_token)
                                    key.auth_config = crypto_service.encrypt(json.dumps(token_meta))

                                    # 持久化：key 实体来自 DB session 时，尝试直接提交更新。
                                    sess = object_session(key)
                                    if sess is not None:
                                        sess.add(key)
                                        sess.commit()
                                    else:
                                        logger.warning(
                                            "[OAUTH_REFRESH] key {} 刷新成功但无法持久化（无绑定 session），"
                                            "下次请求将重新刷新",
                                            key.id,
                                        )
                        finally:
                            if got_lock and redis is not None:
                                try:
                                    await redis.delete(lock_key)
                                except Exception:
                                    pass
            except Exception:
                # 刷新失败不阻断请求；后续由上游返回 401 再触发管理端处理
                pass

        decrypted_key = crypto_service.decrypt(key.api_key)

        decrypted_auth_config: dict[str, Any] | None = None
        if isinstance(token_meta, dict) and token_meta:
            decrypted_auth_config = token_meta

        return ProviderAuthInfo(
            auth_header="Authorization",
            auth_value=f"Bearer {decrypted_key}",
            decrypted_auth_config=decrypted_auth_config,
        )
    if auth_type == "vertex_ai":
        from src.core.vertex_auth import VertexAuthError, VertexAuthService

        try:
            # 优先从 auth_config 读取，兼容从 api_key 读取（过渡期）
            encrypted_auth_config = getattr(key, "auth_config", None)
            if encrypted_auth_config:
                # auth_config 可能是加密字符串或未加密的 dict
                if isinstance(encrypted_auth_config, dict):
                    # 已经是 dict，直接使用（兼容未加密存储的情况）
                    sa_json = encrypted_auth_config
                else:
                    # 是加密字符串，需要解密
                    decrypted_config = crypto_service.decrypt(encrypted_auth_config)
                    sa_json = json.loads(decrypted_config)
            else:
                # 兼容旧数据：从 api_key 读取
                decrypted_key = crypto_service.decrypt(key.api_key)
                # 检查是否是占位符（表示 auth_config 丢失）
                if decrypted_key == "__placeholder__":
                    raise InvalidRequestException("认证配置丢失，请重新添加该密钥。")
                sa_json = json.loads(decrypted_key)

            if not isinstance(sa_json, dict):
                raise InvalidRequestException("Service Account JSON 无效，请重新添加该密钥。")

            # 获取 Access Token
            service = VertexAuthService(sa_json)
            access_token = await service.get_access_token()

            # Vertex AI 使用 Bearer token
            return ProviderAuthInfo(
                auth_header="Authorization",
                auth_value=f"Bearer {access_token}",
                decrypted_auth_config=sa_json,
            )
        except InvalidRequestException:
            raise
        except VertexAuthError as e:
            raise InvalidRequestException(f"Vertex AI 认证失败：{e}")
        except json.JSONDecodeError:
            raise InvalidRequestException("Service Account JSON 格式无效，请重新添加该密钥。")
        except Exception:
            raise InvalidRequestException("Vertex AI 认证失败，请检查 Key 的 auth_config")

    # 其他认证类型可在此扩展
    # elif auth_type == "oauth2":
    #     ...

    # 标准 API Key：返回 None，由 build_headers 处理
    return None
