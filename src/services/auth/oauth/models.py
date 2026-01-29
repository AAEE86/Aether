from dataclasses import dataclass
from typing import Any


@dataclass(frozen=True)
class OAuthToken:
    access_token: str
    token_type: str = "bearer"
    refresh_token: str | None = None
    expires_in: int | None = None
    id_token: str | None = None
    scope: str | None = None
    raw: dict[str, Any] | None = None


@dataclass(frozen=True)
class OAuthUserInfo:
    id: str
    username: str | None = None
    email: str | None = None
    email_verified: bool | None = None
    raw: dict[str, Any] | None = None


class OAuthFlowError(Exception):
    """用于 OAuth 流程的可控错误（会映射到 error_code）。"""

    def __init__(self, error_code: str, detail: str = ""):
        super().__init__(error_code)
        self.error_code = error_code
        self.detail = detail

