from abc import ABC, abstractmethod
from typing import TYPE_CHECKING
from urllib.parse import urlencode, urlparse, urlunparse

import httpx

from src.services.auth.oauth.models import OAuthToken, OAuthUserInfo
from src.utils.ssl_utils import get_ssl_context

if TYPE_CHECKING:
    from src.models.database import OAuthProvider


class OAuthProviderBase(ABC):
    """
    OAuth Provider 基类（稳定扩展点）。

    v1 收敛点：仅实现 OAuth2 授权码流程所需的最小接口。
    """

    provider_type: str
    display_name: str

    # 允许的 host 白名单（用于端点覆盖校验，支持子域名）
    allowed_domains: tuple[str, ...] = ()

    authorization_url: str
    token_url: str
    userinfo_url: str
    default_scopes: tuple[str, ...] = ()

    def get_effective_authorization_url(self, config: OAuthProvider) -> str:
        return config.authorization_url_override or self.authorization_url

    def get_effective_token_url(self, config: OAuthProvider) -> str:
        return config.token_url_override or self.token_url

    def get_effective_userinfo_url(self, config: OAuthProvider) -> str:
        return config.userinfo_url_override or self.userinfo_url

    def get_effective_scopes(self, config: OAuthProvider) -> str:
        scopes = config.scopes or list(self.default_scopes)
        return " ".join(scopes)

    def get_authorization_url(self, config: OAuthProvider, state: str) -> str:
        """
        构造 provider 授权 URL。

        redirect_uri 必须由服务端控制，不从客户端传入。
        """
        base = self.get_effective_authorization_url(config)
        # 避免覆盖原有 query（若 provider 默认 url 带 query，保留）
        parsed = urlparse(base)
        query: dict[str, str] = {}
        if parsed.query:
            # 保留已有 query 参数
            for kv in parsed.query.split("&"):
                if not kv:
                    continue
                if "=" in kv:
                    k, v = kv.split("=", 1)
                    query[k] = v
                else:
                    query[kv] = ""

        client_id = config.client_id
        redirect_uri = config.redirect_uri
        if not client_id or not redirect_uri:
            raise ValueError("OAuthProvider 配置不完整：client_id/redirect_uri 不能为空")

        query.update(
            {
                "response_type": "code",
                "client_id": client_id,
                "redirect_uri": redirect_uri,
                "scope": self.get_effective_scopes(config),
                "state": state,
            }
        )

        return urlunparse(parsed._replace(query=urlencode(query)))

    @abstractmethod
    async def exchange_code(self, config: OAuthProvider, code: str) -> OAuthToken:
        """使用授权码兑换 token。"""

    @abstractmethod
    async def get_user_info(self, config: OAuthProvider, access_token: str) -> OAuthUserInfo:
        """获取用户信息。"""

    async def _http_post_form(
        self,
        url: str,
        data: dict[str, str],
        *,
        timeout_seconds: float = 5.0,
        headers: dict[str, str] | None = None,
    ) -> httpx.Response:
        async with httpx.AsyncClient(timeout=httpx.Timeout(timeout_seconds), verify=get_ssl_context()) as client:
            return await client.post(url, data=data, headers=headers)

    async def _http_get(
        self,
        url: str,
        *,
        timeout_seconds: float = 5.0,
        headers: dict[str, str] | None = None,
    ) -> httpx.Response:
        async with httpx.AsyncClient(timeout=httpx.Timeout(timeout_seconds), verify=get_ssl_context()) as client:
            return await client.get(url, headers=headers)
