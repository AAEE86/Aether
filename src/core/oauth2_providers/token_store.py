"""
OAuth2 Token 存储和刷新服务

管理 OAuth2 Provider Key 的 Token 缓存和自动刷新。
参考 VertexAuthService 的 LRU 缓存模式实现。
"""

from __future__ import annotations

import asyncio
import json
import time
from collections import OrderedDict
from typing import TYPE_CHECKING, Any

from src.core.logger import logger
from src.core.oauth2_providers.base import OAuth2AuthError

if TYPE_CHECKING:
    from src.core.oauth2_providers.base import OAuth2TokenInfo
    from src.models.database import ProviderAPIKey


def _mask_token(token: str) -> str:
    """脱敏 Token，只显示前 8 个字符"""
    if len(token) <= 8:
        return "***"
    return f"{token[:8]}***"


class OAuth2TokenStore:
    """
    OAuth2 Token 存储和刷新服务

    特性：
    - LRU 缓存：最多缓存 200 个 Key 的 Token
    - 自动刷新：Token 过期前 60 秒自动刷新
    - 并发安全：使用 asyncio.Lock 防止并发刷新同一个 Key
    - 异步持久化：刷新后异步更新数据库，不阻塞 API 响应
    """

    # Token 缓存: key_id -> (access_token, expires_at)
    _token_cache: OrderedDict[str, tuple[str, float]] = OrderedDict()
    _cache_max_size: int = 200

    # 每个 Key 的刷新锁，防止并发刷新
    _refresh_locks: dict[str, asyncio.Lock] = {}

    # 提前刷新的时间阈值（秒）
    REFRESH_THRESHOLD_SECONDS: int = 60

    @classmethod
    async def get_access_token(cls, key: "ProviderAPIKey") -> str:
        """
        获取有效的 access_token

        如果缓存中的 Token 有效，直接返回。
        如果过期或即将过期，自动刷新后返回新 Token。

        Args:
            key: ProviderAPIKey 实例

        Returns:
            有效的 access_token

        Raises:
            OAuth2AuthError: 获取或刷新失败
        """
        key_id = key.id
        auth_type = getattr(key, "auth_type", "")

        # 检查缓存
        if key_id in cls._token_cache:
            token, expires_at = cls._token_cache[key_id]
            if time.time() < expires_at - cls.REFRESH_THRESHOLD_SECONDS:
                # Token 仍然有效，使用缓存
                cls._token_cache.move_to_end(key_id)  # LRU: 移动到末尾
                return token

        # 需要刷新 Token - 使用锁防止并发刷新
        if key_id not in cls._refresh_locks:
            cls._refresh_locks[key_id] = asyncio.Lock()

        async with cls._refresh_locks[key_id]:
            # 获取锁后再次检查（可能其他协程已经刷新了）
            if key_id in cls._token_cache:
                token, expires_at = cls._token_cache[key_id]
                if time.time() < expires_at - cls.REFRESH_THRESHOLD_SECONDS:
                    return token

            # 执行刷新
            return await cls._refresh_token(key)

    @classmethod
    async def _refresh_token(cls, key: "ProviderAPIKey") -> str:
        """
        刷新 Token

        Args:
            key: ProviderAPIKey 实例

        Returns:
            新的 access_token

        Raises:
            OAuth2AuthError: 刷新失败
        """
        from src.core.crypto import crypto_service
        from src.core.oauth2_providers import OAuth2ProviderRegistry
        from src.core.oauth2_providers.models import OAuth2TokenData

        auth_type = key.auth_type
        key_id = key.id

        # 获取 Provider
        provider = OAuth2ProviderRegistry.get_provider(auth_type)
        if not provider:
            raise OAuth2AuthError(f"Unknown OAuth2 provider: {auth_type}")

        # 解密 auth_config
        if not key.auth_config:
            raise OAuth2AuthError(f"OAuth2 key {key_id} missing auth_config")

        try:
            decrypted = crypto_service.decrypt(key.auth_config)
            auth_config = json.loads(decrypted)

            # 支持两种格式：完整的 OAuth2AuthConfig 或直接的 token_data
            if "token_data" in auth_config:
                token_data_dict = auth_config["token_data"]
            else:
                token_data_dict = auth_config

            token_data = OAuth2TokenData(**token_data_dict)

        except Exception as e:
            logger.error(f"[OAuth2TokenStore] Failed to decrypt auth_config for key {key_id}: {e}")
            raise OAuth2AuthError(f"Failed to decrypt OAuth2 auth_config: {e}")

        # 检查 refresh_token
        if not token_data.refresh_token:
            raise OAuth2AuthError(f"OAuth2 key {key_id} missing refresh_token")

        # 使用 Provider 刷新 Token
        try:
            logger.debug(
                f"[OAuth2TokenStore] Refreshing token for key {key_id} "
                f"(provider: {auth_type}, refresh_token: {_mask_token(token_data.refresh_token)})"
            )

            new_token_info = await provider.exchange_refresh_token(token_data.refresh_token)

        except OAuth2AuthError:
            raise
        except Exception as e:
            logger.error(f"[OAuth2TokenStore] Token refresh failed for key {key_id}: {e}")
            raise OAuth2AuthError(f"Token refresh failed: {e}")

        # 更新缓存
        cls._token_cache[key_id] = (new_token_info.access_token, new_token_info.expires_at)
        cls._token_cache.move_to_end(key_id)

        # LRU 淘汰
        while len(cls._token_cache) > cls._cache_max_size:
            oldest_key = next(iter(cls._token_cache))
            del cls._token_cache[oldest_key]
            logger.debug(f"[OAuth2TokenStore] Evicted oldest cache entry: {oldest_key}")

        logger.info(
            f"[OAuth2TokenStore] Token refreshed for key {key_id} "
            f"(provider: {auth_type}, expires_in: {int(new_token_info.expires_at - time.time())}s, "
            f"cache_size: {len(cls._token_cache)})"
        )

        # 异步更新数据库
        asyncio.create_task(cls._update_key_tokens(key_id, new_token_info, auth_config))

        return new_token_info.access_token

    @classmethod
    async def _update_key_tokens(
        cls,
        key_id: str,
        token_info: "OAuth2TokenInfo",
        original_config: dict[str, Any],
    ) -> None:
        """
        异步更新数据库中的 Token

        此方法在后台运行，不阻塞 API 响应。

        Args:
            key_id: Key ID
            token_info: 新的 Token 信息
            original_config: 原始的 auth_config（用于保留其他字段）
        """
        from src.core.crypto import crypto_service
        from src.database import get_db_context

        try:
            async with get_db_context() as db:
                from src.models.database import ProviderAPIKey

                key = db.query(ProviderAPIKey).filter(ProviderAPIKey.id == key_id).first()
                if not key:
                    logger.warning(f"[OAuth2TokenStore] Key {key_id} not found when updating tokens")
                    return

                # 构建新的 auth_config
                new_token_data = {
                    "access_token": token_info.access_token,
                    "refresh_token": token_info.refresh_token or original_config.get("token_data", {}).get("refresh_token"),
                    "expires_at": token_info.expires_at,
                    "token_type": token_info.token_type,
                    "scope": token_info.scope,
                    "obtained_at": time.time(),
                }

                # 保留原有的其他配置
                new_config = original_config.copy()
                new_config["token_data"] = new_token_data

                # 加密并保存
                key.auth_config = crypto_service.encrypt(json.dumps(new_config))
                db.commit()

                logger.debug(f"[OAuth2TokenStore] Updated tokens in database for key {key_id}")

        except Exception as e:
            logger.error(f"[OAuth2TokenStore] Failed to update tokens in database for key {key_id}: {e}")

    @classmethod
    def invalidate_cache(cls, key_id: str | None = None) -> None:
        """
        使缓存失效

        Args:
            key_id: 指定要失效的 Key，None 表示清除全部缓存
        """
        if key_id:
            cls._token_cache.pop(key_id, None)
            cls._refresh_locks.pop(key_id, None)
            logger.debug(f"[OAuth2TokenStore] Invalidated cache for key {key_id}")
        else:
            cls._token_cache.clear()
            cls._refresh_locks.clear()
            logger.debug("[OAuth2TokenStore] Cleared all cache")

    @classmethod
    def get_cache_stats(cls) -> dict[str, Any]:
        """
        获取缓存统计信息

        Returns:
            缓存统计字典
        """
        now = time.time()
        valid_count = sum(
            1 for _, expires_at in cls._token_cache.values()
            if now < expires_at - cls.REFRESH_THRESHOLD_SECONDS
        )
        return {
            "total_cached": len(cls._token_cache),
            "valid_cached": valid_count,
            "max_size": cls._cache_max_size,
            "refresh_threshold_seconds": cls.REFRESH_THRESHOLD_SECONDS,
        }

    @classmethod
    async def warm_up_token(cls, key: "ProviderAPIKey") -> bool:
        """
        预热 Token 缓存

        在 Key 创建或更新后调用，提前将 Token 加载到缓存。

        Args:
            key: ProviderAPIKey 实例

        Returns:
            是否成功预热
        """
        try:
            from src.core.crypto import crypto_service
            from src.core.oauth2_providers.models import OAuth2TokenData

            if not key.auth_config:
                return False

            decrypted = crypto_service.decrypt(key.auth_config)
            auth_config = json.loads(decrypted)

            if "token_data" in auth_config:
                token_data_dict = auth_config["token_data"]
            else:
                token_data_dict = auth_config

            token_data = OAuth2TokenData(**token_data_dict)

            # 只有 Token 未过期时才缓存
            if not token_data.is_expired(cls.REFRESH_THRESHOLD_SECONDS):
                cls._token_cache[key.id] = (token_data.access_token, token_data.expires_at)
                logger.debug(f"[OAuth2TokenStore] Warmed up cache for key {key.id}")
                return True

            return False

        except Exception as e:
            logger.warning(f"[OAuth2TokenStore] Failed to warm up cache for key {key.id}: {e}")
            return False
