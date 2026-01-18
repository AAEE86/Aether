"""
通用 OAuth 认证服务

支持可配置的 OAuth 提供商（如 Linux Do、GitHub、Google 等）
"""

import secrets
from typing import Any, Dict, Optional, Tuple
from urllib.parse import urlencode

import httpx
from sqlalchemy.orm import Session

from src.core.logger import logger
from src.models.database import OAuthProviderConfig

# HTTP 请求超时（秒）
HTTP_TIMEOUT = 30

# 默认的用户信息字段映射
DEFAULT_USERINFO_MAPPING = {
    "user_id": "id",
    "username": "username",
    "email": "email",
}


class OAuthService:
    """通用 OAuth 认证服务"""

    @staticmethod
    def get_provider_config(db: Session, provider_id: str) -> Optional[OAuthProviderConfig]:
        """获取指定 OAuth 提供商配置"""
        return (
            db.query(OAuthProviderConfig)
            .filter(OAuthProviderConfig.provider_id == provider_id)
            .first()
        )

    @staticmethod
    def get_enabled_providers(db: Session) -> list[OAuthProviderConfig]:
        """获取所有已启用的 OAuth 提供商"""
        return (
            db.query(OAuthProviderConfig)
            .filter(OAuthProviderConfig.is_enabled == True)
            .all()
        )

    @staticmethod
    def is_provider_enabled(db: Session, provider_id: str) -> bool:
        """检查指定 OAuth 提供商是否启用"""
        config_data = OAuthService.get_provider_config_data(db, provider_id)
        return config_data is not None

    @staticmethod
    def get_provider_config_data(db: Session, provider_id: str) -> Optional[Dict[str, Any]]:
        """
        获取并解密指定提供商的配置数据，供 OAuth 流程使用

        Args:
            db: 数据库会话
            provider_id: 提供商标识

        Returns:
            配置字典，如果未启用或配置无效则返回 None
        """
        # 检查模块是否激活
        from src.core.modules import get_module_registry

        registry = get_module_registry()
        if not registry.is_active("oauth", db):
            return None

        config = OAuthService.get_provider_config(db, provider_id)
        if not config or not config.is_enabled:
            return None

        try:
            client_secret = config.get_client_secret()
        except Exception as e:
            logger.error(f"OAuth provider {provider_id} client_secret 解密失败: {e}")
            return None

        if not client_secret:
            return None

        # 获取用户信息字段映射
        userinfo_mapping = config.userinfo_mapping or DEFAULT_USERINFO_MAPPING

        return {
            "provider_id": config.provider_id,
            "display_name": config.display_name,
            "authorization_url": config.authorization_url,
            "token_url": config.token_url,
            "userinfo_url": config.userinfo_url,
            "userinfo_mapping": userinfo_mapping,
            "client_id": config.client_id,
            "client_secret": client_secret,
            "redirect_uri": config.redirect_uri,
            "frontend_callback_url": config.frontend_callback_url,
            "scope": config.scope or "user",
        }

    @staticmethod
    def generate_state() -> str:
        """生成 OAuth state 参数（防 CSRF）"""
        return secrets.token_urlsafe(32)

    @staticmethod
    def get_authorization_url(config_data: Dict[str, Any], state: str) -> str:
        """
        生成 OAuth 授权 URL

        Args:
            config_data: 配置数据
            state: CSRF 防护 state 参数

        Returns:
            授权 URL
        """
        params = {
            "client_id": config_data["client_id"],
            "redirect_uri": config_data["redirect_uri"],
            "response_type": "code",
            "scope": config_data.get("scope", "user"),
            "state": state,
        }
        return f"{config_data['authorization_url']}?{urlencode(params)}"

    @staticmethod
    async def exchange_code_for_token(
        config_data: Dict[str, Any], code: str
    ) -> Tuple[bool, Optional[Dict[str, Any]], Optional[str]]:
        """
        用授权码换取访问令牌

        Args:
            config_data: 配置数据
            code: 授权码

        Returns:
            (成功, token_data, 错误信息)
        """
        provider_id = config_data.get("provider_id", "unknown")
        try:
            async with httpx.AsyncClient(timeout=HTTP_TIMEOUT) as client:
                response = await client.post(
                    config_data["token_url"],
                    data={
                        "client_id": config_data["client_id"],
                        "client_secret": config_data["client_secret"],
                        "code": code,
                        "redirect_uri": config_data["redirect_uri"],
                        "grant_type": "authorization_code",
                    },
                    headers={
                        "Content-Type": "application/x-www-form-urlencoded",
                        "Accept": "application/json",
                    },
                )

                if response.status_code != 200:
                    logger.error(
                        f"OAuth [{provider_id}] token 请求失败: status={response.status_code}, body={response.text}"
                    )
                    return False, None, f"获取访问令牌失败: HTTP {response.status_code}"

                token_data = response.json()
                if "access_token" not in token_data:
                    logger.error(f"OAuth [{provider_id}] token 响应缺少 access_token: {token_data}")
                    return False, None, "获取访问令牌失败: 响应格式错误"

                return True, token_data, None

        except httpx.TimeoutException:
            logger.error(f"OAuth [{provider_id}] token 请求超时")
            return False, None, "获取访问令牌超时"
        except Exception as e:
            logger.error(f"OAuth [{provider_id}] token 请求异常: {e}")
            return False, None, f"获取访问令牌失败: {str(e)}"

    @staticmethod
    async def get_user_info(
        config_data: Dict[str, Any], access_token: str
    ) -> Tuple[bool, Optional[Dict[str, Any]], Optional[str]]:
        """
        获取 OAuth 用户信息

        Args:
            config_data: 配置数据
            access_token: 访问令牌

        Returns:
            (成功, user_info, 错误信息)
        """
        provider_id = config_data.get("provider_id", "unknown")
        try:
            async with httpx.AsyncClient(timeout=HTTP_TIMEOUT) as client:
                response = await client.get(
                    config_data["userinfo_url"],
                    headers={
                        "Authorization": f"Bearer {access_token}",
                        "Accept": "application/json",
                    },
                )

                if response.status_code != 200:
                    logger.error(
                        f"OAuth [{provider_id}] 用户信息请求失败: status={response.status_code}, body={response.text}"
                    )
                    return False, None, f"获取用户信息失败: HTTP {response.status_code}"

                raw_user_info = response.json()

                # 根据映射提取标准化的用户信息
                mapping = config_data.get("userinfo_mapping", DEFAULT_USERINFO_MAPPING)
                user_info = OAuthService._extract_user_info(raw_user_info, mapping)
                user_info["_raw"] = raw_user_info  # 保留原始数据以便调试

                return True, user_info, None

        except httpx.TimeoutException:
            logger.error(f"OAuth [{provider_id}] 用户信息请求超时")
            return False, None, "获取用户信息超时"
        except Exception as e:
            logger.error(f"OAuth [{provider_id}] 用户信息请求异常: {e}")
            return False, None, f"获取用户信息失败: {str(e)}"

    @staticmethod
    def _extract_user_info(raw_data: Dict[str, Any], mapping: Dict[str, str]) -> Dict[str, Any]:
        """
        根据映射从原始数据中提取标准化的用户信息

        Args:
            raw_data: OAuth 提供商返回的原始用户信息
            mapping: 字段映射配置

        Returns:
            标准化的用户信息字典
        """
        result = {}

        for standard_field, source_field in mapping.items():
            # 支持嵌套字段访问，如 "user.id" -> raw_data["user"]["id"]
            value = raw_data
            try:
                for key in source_field.split("."):
                    if isinstance(value, dict):
                        value = value.get(key)
                    else:
                        value = None
                        break
                result[standard_field] = value
            except (KeyError, TypeError):
                result[standard_field] = None

        return result

    @staticmethod
    async def test_connection(config_data: Dict[str, Any]) -> Tuple[bool, str]:
        """
        测试 OAuth 配置连接

        Args:
            config_data: 配置数据

        Returns:
            (成功, 消息)
        """
        provider_id = config_data.get("provider_id", "unknown")

        # 验证配置完整性
        if not config_data.get("client_id"):
            return False, "Client ID 未配置"
        if not config_data.get("client_secret"):
            return False, "Client Secret 未配置"
        if not config_data.get("redirect_uri"):
            return False, "回调地址未配置"
        if not config_data.get("authorization_url"):
            return False, "授权 URL 未配置"
        if not config_data.get("token_url"):
            return False, "Token URL 未配置"
        if not config_data.get("userinfo_url"):
            return False, "用户信息 URL 未配置"

        try:
            # 测试能否访问 OAuth 服务（尝试访问授权端点）
            from urllib.parse import urlparse

            auth_url = config_data["authorization_url"]
            parsed = urlparse(auth_url)
            base_url = f"{parsed.scheme}://{parsed.netloc}"

            async with httpx.AsyncClient(timeout=HTTP_TIMEOUT) as client:
                response = await client.get(base_url, follow_redirects=True)
                # 只要能连接上就算成功（不管返回什么状态码）
                if response.status_code >= 500:
                    return False, f"OAuth 服务异常: HTTP {response.status_code}"

            return True, f"配置验证成功，{provider_id} OAuth 服务可访问"

        except httpx.TimeoutException:
            return False, f"连接 OAuth 服务超时"
        except httpx.ConnectError:
            return False, f"无法连接到 OAuth 服务，请检查网络"
        except Exception as e:
            return False, f"连接测试失败: {str(e)}"
