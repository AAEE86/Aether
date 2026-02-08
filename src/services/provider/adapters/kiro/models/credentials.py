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


def _parse_iso_to_epoch_seconds(value: object) -> int | None:
    if not isinstance(value, str) or not value.strip():
        return None
    text = value.strip()
    # Support RFC3339 with Z suffix.
    if text.endswith('Z'):
        text = text[:-1] + '+00:00'
    try:
        dt = datetime.fromisoformat(text)
    except Exception:
        return None
    if dt.tzinfo is None:
        dt = dt.replace(tzinfo=timezone.utc)
    return int(dt.timestamp())


@dataclass(slots=True)
class KiroAuthConfig:
    provider_type: str = 'kiro'

    auth_method: str = 'social'  # social | idc
    refresh_token: str = ''
    expires_at: int = 0

    profile_arn: str | None = None
    region: str | None = None

    client_id: str | None = None
    client_secret: str | None = None

    machine_id: str | None = None
    kiro_version: str | None = None
    system_version: str | None = None
    node_version: str | None = None

    email: str | None = None  # 账号邮箱

    # 缓存的 access_token（可选，用于避免频繁刷新）
    access_token: str | None = None

    @staticmethod
    def infer_auth_method(raw: dict[str, Any]) -> str:
        """
        根据凭据字段自动推断认证类型。

        规则：
        - 包含 clientId + clientSecret -> IdC
        - 仅含 refreshToken -> Social
        """
        client_id = raw.get('client_id') or raw.get('clientId')
        client_secret = raw.get('client_secret') or raw.get('clientSecret')

        if client_id and client_secret:
            return 'idc'
        return 'social'

    @staticmethod
    def validate_required_fields(raw: dict[str, Any]) -> tuple[bool, str]:
        """
        验证凭据是否包含必需字段。

        返回: (is_valid, error_message)
        """
        refresh_token = raw.get('refresh_token') or raw.get('refreshToken') or ''
        refresh_token = str(refresh_token).strip()

        if not refresh_token:
            return False, 'refreshToken 为必填字段'

        # refreshToken 不能含有 ...（表示被截断）
        if '...' in refresh_token:
            return False, 'refreshToken 不完整（含有 ...），请导出完整的 Token'

        # IdC 类型需要 clientId 和 clientSecret
        auth_method = KiroAuthConfig.infer_auth_method(raw)
        if auth_method == 'idc':
            client_id = raw.get('client_id') or raw.get('clientId')
            client_secret = raw.get('client_secret') or raw.get('clientSecret')
            if not client_id:
                return False, 'IdC 类型需要 clientId'
            if not client_secret:
                return False, 'IdC 类型需要 clientSecret'

        return True, ''

    @classmethod
    def from_dict(cls, raw: dict[str, Any]) -> 'KiroAuthConfig':
        if not isinstance(raw, dict):
            raw = {}

        provider_type = str(raw.get('provider_type') or raw.get('providerType') or 'kiro').strip()

        # 自动推断 auth_method（如果未显式指定）
        explicit_auth_method = raw.get('auth_method') or raw.get('authMethod')
        if explicit_auth_method:
            auth_method = str(explicit_auth_method).strip().lower()
        else:
            auth_method = cls.infer_auth_method(raw)

        refresh_token = raw.get('refresh_token') or raw.get('refreshToken') or ''
        refresh_token = str(refresh_token or '').strip()

        profile_arn = raw.get('profile_arn') or raw.get('profileArn')
        profile_arn = str(profile_arn).strip() if isinstance(profile_arn, str) and profile_arn else None

        region = raw.get('region')
        region = str(region).strip() if isinstance(region, str) and region else None

        client_id = raw.get('client_id') or raw.get('clientId')
        client_id = str(client_id).strip() if isinstance(client_id, str) and client_id else None

        client_secret = raw.get('client_secret') or raw.get('clientSecret')
        client_secret = (
            str(client_secret).strip() if isinstance(client_secret, str) and client_secret else None
        )

        machine_id = raw.get('machine_id') or raw.get('machineId')
        machine_id = str(machine_id).strip() if isinstance(machine_id, str) and machine_id else None

        kiro_version = raw.get('kiro_version') or raw.get('kiroVersion')
        kiro_version = (
            str(kiro_version).strip() if isinstance(kiro_version, str) and kiro_version else None
        )

        system_version = raw.get('system_version') or raw.get('systemVersion')
        system_version = (
            str(system_version).strip() if isinstance(system_version, str) and system_version else None
        )

        node_version = raw.get('node_version') or raw.get('nodeVersion')
        node_version = (
            str(node_version).strip() if isinstance(node_version, str) and node_version else None
        )

        email = raw.get('email')
        email = str(email).strip() if isinstance(email, str) and email else None

        access_token = raw.get('access_token') or raw.get('accessToken')
        access_token = str(access_token).strip() if isinstance(access_token, str) and access_token else None

        expires_at = _parse_epoch_seconds(raw.get('expires_at'))
        if expires_at is None:
            expires_at = _parse_iso_to_epoch_seconds(raw.get('expiresAt'))
        if expires_at is None:
            expires_at = 0

        cfg = cls(
            provider_type=provider_type or 'kiro',
            auth_method=(auth_method or 'social').lower(),
            refresh_token=refresh_token,
            expires_at=int(expires_at),
            profile_arn=profile_arn,
            region=region,
            client_id=client_id,
            client_secret=client_secret,
            machine_id=machine_id,
            kiro_version=kiro_version,
            system_version=system_version,
            node_version=node_version,
            email=email,
            access_token=access_token,
        )

        # Normalize auth_method aliases.
        if cfg.auth_method in {'builder-id', 'builder_id', 'iam'}:
            cfg.auth_method = 'idc'

        return cfg

    def to_dict(self) -> dict[str, Any]:
        return {
            'provider_type': self.provider_type,
            'auth_method': self.auth_method,
            'refresh_token': self.refresh_token,
            'expires_at': self.expires_at,
            'profile_arn': self.profile_arn,
            'region': self.region,
            'client_id': self.client_id,
            'client_secret': self.client_secret,
            'machine_id': self.machine_id,
            'kiro_version': self.kiro_version,
            'system_version': self.system_version,
            'node_version': self.node_version,
            'email': self.email,
            'access_token': self.access_token,
        }


__all__ = ["KiroAuthConfig"]
