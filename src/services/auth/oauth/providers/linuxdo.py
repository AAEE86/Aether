from typing import TYPE_CHECKING, Any

from src.core.logger import logger
from src.services.auth.oauth.base import OAuthProviderBase
from src.services.auth.oauth.models import OAuthFlowError, OAuthToken, OAuthUserInfo

if TYPE_CHECKING:
    from src.models.database import OAuthProvider


class LinuxDoOAuthProvider(OAuthProviderBase):
    """
    LinuxDo OAuth Provider。

    基于论坛信任等级（trust_level 0-4）的 OAuth2 认证，
    用于通过用户等级进行额度配给和频率限制。

    参考：https://linux.do/t/topic/329408

    返回的用户信息示例：
    {
        "id": 1,
        "username": "neo",
        "name": "Neo",
        "active": true,
        "trust_level": 4,
        "email": "u1@linux.do",
        "avatar_url": "https://linux.do/xxxx",
        "silenced": false
    }
    """

    provider_type = "linuxdo"
    display_name = "Linux Do"

    allowed_domains = ("linux.do", "connect.linux.do", "connect.linuxdo.org")

    # 默认端点
    authorization_url = "https://connect.linux.do/oauth2/authorize"
    token_url = "https://connect.linux.do/oauth2/token"
    userinfo_url = "https://connect.linux.do/api/user"

    # LinuxDo 不需要 scope
    default_scopes = ()

    async def exchange_code(self, config: OAuthProvider, code: str) -> OAuthToken:
        url = self.get_effective_token_url(config)
        client_secret = config.get_client_secret()
        if not client_secret:
            raise OAuthFlowError("provider_unavailable", "client_secret 未配置")

        redirect_uri = config.redirect_uri
        client_id = config.client_id
        if not redirect_uri or not client_id:
            raise OAuthFlowError("provider_unavailable", "redirect_uri/client_id 未配置")

        resp = await self._http_post_form(
            url,
            data={
                "grant_type": "authorization_code",
                "code": code,
                "redirect_uri": redirect_uri,
                "client_id": client_id,
                "client_secret": client_secret,
            },
        )

        if resp.status_code >= 400:
            logger.warning("LinuxDo token 兑换失败: status={}", resp.status_code)
            raise OAuthFlowError("token_exchange_failed", f"status={resp.status_code}")

        data = resp.json()
        access_token = data.get("access_token")
        if not access_token:
            raise OAuthFlowError("token_exchange_failed", "missing access_token")

        return OAuthToken(
            access_token=str(access_token),
            token_type=str(data.get("token_type") or "bearer"),
            refresh_token=(str(data["refresh_token"]) if data.get("refresh_token") else None),
            expires_in=(int(data["expires_in"]) if data.get("expires_in") is not None else None),
            id_token=(str(data["id_token"]) if data.get("id_token") else None),
            scope=(str(data["scope"]) if data.get("scope") else None),
            raw=data,
        )

    async def get_user_info(self, config: OAuthProvider, access_token: str) -> OAuthUserInfo:
        url = self.get_effective_userinfo_url(config)
        resp = await self._http_get(url, headers={"Authorization": f"Bearer {access_token}"})

        if resp.status_code >= 400:
            logger.warning("LinuxDo userinfo 获取失败: status={}", resp.status_code)
            raise OAuthFlowError("userinfo_fetch_failed", f"status={resp.status_code}")

        data: dict[str, Any] = resp.json()

        # LinuxDo 返回的 id 是数字类型
        provider_user_id = data.get("id")
        if provider_user_id is None:
            raise OAuthFlowError("userinfo_fetch_failed", "missing user id")

        return OAuthUserInfo(
            id=str(provider_user_id),
            username=data.get("username"),
            email=str(data["email"]).lower() if data.get("email") else None,
            email_verified=None,  # LinuxDo 不返回此字段
            raw=data,  # 包含 trust_level, active, silenced, avatar_url, name 等
        )
