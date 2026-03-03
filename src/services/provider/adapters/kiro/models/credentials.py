"""Internal Kiro credential schema (stored in ProviderAPIKey.auth_config)."""

from __future__ import annotations

from dataclasses import dataclass
from datetime import datetime, timezone
from typing import Any


def _parse_epoch_seconds(value: object) -> int | None:
    if value is None:
        return None
    try:
        if isinstance(value, (int, float)):
            return int(value)
        if isinstance(value, str) and value.strip().isdigit():
            return int(value.strip())
    except Exception:
        return None
    return None


def _get_str(raw: dict[str, Any], *keys: str) -> str | None:
    """Return the first non-empty stripped string for *keys*, or ``None``."""
    for k in keys:
        v = raw.get(k)
        if isinstance(v, str) and v.strip():
            return v.strip()
    return None


def _nonempty(s: str | None) -> str | None:
    """Return *s* if it's a non-empty stripped string, else ``None``."""
    if isinstance(s, str) and s.strip():
        return s.strip()
    return None


def _normalize_auth_method(value: str | None) -> str:
    method = (value or "").strip().lower()
    if not method:
        return "social"
    # 历史/别名兼容：统一映射到 idc
    if method in {
        "idc",
        "builder-id",
        "builder_id",
        "builderid",
        "identity-center",
        "identity_center",
        "identitycenter",
        "iam",
        "device",
        "device_authorization",
        "device-auth",
    }:
        return "idc"
    return method


def _parse_iso_to_epoch_seconds(value: object) -> int | None:
    if not isinstance(value, str) or not value.strip():
        return None
    text = value.strip()
    # Support RFC3339 with Z suffix.
    if text.endswith("Z"):
        text = text[:-1] + "+00:00"
    try:
        dt = datetime.fromisoformat(text)
    except Exception:
        return None
    if dt.tzinfo is None:
        dt = dt.replace(tzinfo=timezone.utc)
    return int(dt.timestamp())


@dataclass(slots=True)
class KiroAuthConfig:
    provider_type: str = "kiro"

    auth_method: str = "social"  # social | idc
    refresh_token: str = ""
    expires_at: int = 0

    profile_arn: str | None = None
    region: str | None = None  # OIDC region（IdC token 刷新用）

    # 独立的 auth / api region（与 kiro.rs 对齐）
    # auth_region: token 刷新端点，未设置时回退到 region
    # api_region: q.{region} 服务端点，未设置时回退到 DEFAULT_REGION
    auth_region: str | None = None
    api_region: str | None = None

    client_id: str | None = None
    client_secret: str | None = None

    machine_id: str | None = None
    kiro_version: str | None = None
    system_version: str | None = None
    node_version: str | None = None

    email: str | None = None  # 账号邮箱

    # 缓存的 access_token（可选，用于避免频繁刷新）
    access_token: str | None = None

    def effective_auth_region(self) -> str:
        """Token 刷新用的 region。

        优先级: auth_region > region > DEFAULT_REGION
        """
        from src.services.provider.adapters.kiro.constants import DEFAULT_REGION

        return _nonempty(self.auth_region) or _nonempty(self.region) or DEFAULT_REGION

    def effective_api_region(self) -> str:
        """API 服务端点（q.{region}）用的 region。

        优先级: api_region > DEFAULT_REGION
        注意: 不从 region 继承，因为 region 通常是 OIDC region（如 eu-north-1），
        而 q.{region} 端点目前仅 us-east-1 可用。
        """
        from src.services.provider.adapters.kiro.constants import DEFAULT_REGION

        return _nonempty(self.api_region) or DEFAULT_REGION

    @staticmethod
    def infer_auth_method(raw: dict[str, Any]) -> str:
        """
        根据凭据字段自动推断认证类型。

        规则：
        - 包含 clientId + clientSecret -> IdC
        - 仅含 refreshToken -> Social
        """
        explicit_method = _get_str(raw, "auth_method", "authMethod", "auth_type", "authType")
        normalized_explicit = _normalize_auth_method(explicit_method)
        if normalized_explicit != "social":
            return normalized_explicit

        client_id = raw.get("client_id") or raw.get("clientId")
        client_secret = raw.get("client_secret") or raw.get("clientSecret")

        if client_id and client_secret:
            return "idc"
        return "social"

    @staticmethod
    def validate_required_fields(raw: dict[str, Any]) -> tuple[bool, str]:
        """
        验证凭据是否包含必需字段。

        返回: (is_valid, error_message)
        """
        refresh_token = raw.get("refresh_token") or raw.get("refreshToken") or ""
        refresh_token = str(refresh_token).strip()

        if not refresh_token:
            return False, "refreshToken 为必填字段"

        # refreshToken 不能含有 ...（表示被截断）
        if "..." in refresh_token:
            return False, "refreshToken 不完整（含有 ...），请导出完整的 Token"

        # IdC 类型需要 clientId 和 clientSecret
        explicit_method = _get_str(raw, "auth_method", "authMethod", "auth_type", "authType")
        auth_method = (
            _normalize_auth_method(explicit_method)
            if explicit_method
            else KiroAuthConfig.infer_auth_method(raw)
        )
        if auth_method == "idc":
            client_id = raw.get("client_id") or raw.get("clientId")
            client_secret = raw.get("client_secret") or raw.get("clientSecret")
            if not client_id:
                return False, "IdC 类型需要 clientId"
            if not client_secret:
                return False, "IdC 类型需要 clientSecret"

        return True, ""

    @classmethod
    def from_dict(cls, raw: dict[str, Any]) -> "KiroAuthConfig":
        if not isinstance(raw, dict):
            raw = {}

        provider_type = _get_str(raw, "provider_type", "providerType") or "kiro"

        # 自动推断 auth_method（如果未显式指定）
        explicit_method = _get_str(raw, "auth_method", "authMethod", "auth_type", "authType")
        auth_method = (
            _normalize_auth_method(explicit_method)
            if explicit_method
            else cls.infer_auth_method(raw)
        )

        refresh_token = (_get_str(raw, "refresh_token", "refreshToken") or "").strip()

        expires_at = _parse_epoch_seconds(raw.get("expires_at"))
        if expires_at is None:
            expires_at = _parse_iso_to_epoch_seconds(raw.get("expiresAt"))
        if expires_at is None:
            expires_at = 0

        cfg = cls(
            provider_type=provider_type,
            auth_method=_normalize_auth_method(auth_method),
            refresh_token=refresh_token,
            expires_at=int(expires_at),
            profile_arn=_get_str(raw, "profile_arn", "profileArn"),
            region=_get_str(raw, "region"),
            auth_region=_get_str(raw, "auth_region", "authRegion"),
            api_region=_get_str(raw, "api_region", "apiRegion"),
            client_id=_get_str(raw, "client_id", "clientId"),
            client_secret=_get_str(raw, "client_secret", "clientSecret"),
            machine_id=_get_str(raw, "machine_id", "machineId"),
            kiro_version=_get_str(raw, "kiro_version", "kiroVersion"),
            system_version=_get_str(raw, "system_version", "systemVersion"),
            node_version=_get_str(raw, "node_version", "nodeVersion"),
            email=_get_str(raw, "email"),
            access_token=_get_str(raw, "access_token", "accessToken"),
        )

        return cfg

    def to_dict(self) -> dict[str, Any]:
        return {
            "provider_type": self.provider_type,
            "auth_method": self.auth_method,
            "refresh_token": self.refresh_token,
            "expires_at": self.expires_at,
            "profile_arn": self.profile_arn,
            "region": self.region,
            "auth_region": self.auth_region,
            "api_region": self.api_region,
            "client_id": self.client_id,
            "client_secret": self.client_secret,
            "machine_id": self.machine_id,
            "kiro_version": self.kiro_version,
            "system_version": self.system_version,
            "node_version": self.node_version,
            "email": self.email,
            "access_token": self.access_token,
        }


__all__ = ["KiroAuthConfig"]
