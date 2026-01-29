"""
JWT Token 黑名单服务

使用 Redis 存储被撤销的 JWT Token，防止已登出或被撤销的 Token 继续使用
"""

import hashlib
import os
from datetime import datetime, timezone

from src.clients.redis_client import get_redis_client
from src.core.logger import logger


# 安全策略配置：当 Redis 不可用时的行为
# True = fail-closed（安全优先，拒绝访问）
# False = fail-open（可用性优先，允许访问）
BLACKLIST_FAIL_CLOSED = os.getenv("JWT_BLACKLIST_FAIL_CLOSED", "true").lower() == "true"


class JWTBlacklistService:
    """JWT Token 黑名单服务"""

    # Redis key 前缀
    BLACKLIST_PREFIX = "jwt:blacklist:"

    @staticmethod
    def _get_token_hash(token: str) -> str:
        """
        获取 Token 的哈希值（用于 Redis key）

        使用 SHA256 哈希避免直接存储完整 Token
        """
        return hashlib.sha256(token.encode()).hexdigest()

    @staticmethod
    async def add_to_blacklist(token: str, exp_timestamp: int, reason: str = "logout") -> bool:
        """
        将 Token 添加到黑名单

        Args:
            token: JWT token 字符串
            exp_timestamp: Token 的过期时间戳（Unix timestamp）
            reason: 添加到黑名单的原因（logout, revoked, security）

        Returns:
            是否成功添加到黑名单
        """
        redis_client = await get_redis_client(require_redis=False)

        if redis_client is None:
            logger.warning("Redis 不可用，无法将 Token 添加到黑名单（降级模式）")
            return False

        try:
            token_hash = JWTBlacklistService._get_token_hash(token)
            redis_key = f"{JWTBlacklistService.BLACKLIST_PREFIX}{token_hash}"

            # 计算 TTL（Token 过期前的剩余时间）
            now = datetime.now(timezone.utc).timestamp()
            ttl_seconds = max(int(exp_timestamp - now), 0)

            if ttl_seconds <= 0:
                # Token 已经过期，不需要加入黑名单
                token_fp = JWTBlacklistService._get_token_hash(token)[:12]
                logger.debug("Token 已过期，无需加入黑名单: token_fp={}", token_fp)
                return True

            # 存储到 Redis，设置 TTL 为 Token 过期时间
            # 值存储为原因字符串
            await redis_client.setex(redis_key, ttl_seconds, reason)

            token_fp = JWTBlacklistService._get_token_hash(token)[:12]
            logger.info("Token 已加入黑名单: token_fp={} (原因: {}, TTL: {}s)", token_fp, reason, ttl_seconds)
            return True

        except Exception as e:
            logger.error(f"添加 Token 到黑名单失败: {e}")
            return False

    @staticmethod
    async def is_blacklisted(token: str) -> bool:
        """
        检查 Token 是否在黑名单中

        Args:
            token: JWT token 字符串

        Returns:
            Token 是否在黑名单中
        """
        redis_client = await get_redis_client(require_redis=False)

        if redis_client is None:
            # Redis 不可用时，根据安全策略决定行为
            if BLACKLIST_FAIL_CLOSED:
                logger.warning("Redis 不可用，采用 fail-closed 策略拒绝访问（可通过 JWT_BLACKLIST_FAIL_CLOSED=false 改变）")
                return True  # 返回 True 表示在黑名单中，拒绝访问
            else:
                logger.debug("Redis 不可用，采用 fail-open 策略允许访问")
                return False

        try:
            token_hash = JWTBlacklistService._get_token_hash(token)
            redis_key = f"{JWTBlacklistService.BLACKLIST_PREFIX}{token_hash}"

            # 检查 key 是否存在
            exists = await redis_client.exists(redis_key)

            if exists:
                # 获取黑名单原因（可选）
                reason = await redis_client.get(redis_key)
                token_fp = JWTBlacklistService._get_token_hash(token)[:12]
                logger.warning("检测到黑名单 Token: token_fp={} (原因: {})", token_fp, reason)
                return True

            return False

        except Exception as e:
            logger.error(f"检查 Token 黑名单状态失败: {e}")
            # 发生错误时，根据安全策略决定行为
            if BLACKLIST_FAIL_CLOSED:
                logger.warning("黑名单检查失败，采用 fail-closed 策略拒绝访问")
                return True  # 安全优先，拒绝访问
            else:
                logger.warning("黑名单检查失败，采用 fail-open 策略允许访问")
                return False

    @staticmethod
    async def remove_from_blacklist(token: str) -> bool:
        """
        从黑名单中移除 Token（用于测试或特殊情况）

        Args:
            token: JWT token 字符串

        Returns:
            是否成功移除
        """
        redis_client = await get_redis_client(require_redis=False)

        if redis_client is None:
            logger.warning("Redis 不可用，无法从黑名单中移除 Token")
            return False

        try:
            token_hash = JWTBlacklistService._get_token_hash(token)
            redis_key = f"{JWTBlacklistService.BLACKLIST_PREFIX}{token_hash}"

            deleted = await redis_client.delete(redis_key)

            if deleted:
                token_fp = JWTBlacklistService._get_token_hash(token)[:12]
                logger.info("Token 已从黑名单移除: token_fp={}", token_fp)
            else:
                token_fp = JWTBlacklistService._get_token_hash(token)[:12]
                logger.debug("Token 不在黑名单中: token_fp={}", token_fp)

            return bool(deleted)

        except Exception as e:
            logger.error(f"从黑名单移除 Token 失败: {e}")
            return False

    @staticmethod
    async def get_blacklist_stats() -> dict:
        """
        获取黑名单统计信息

        Returns:
            包含统计信息的字典
        """
        redis_client = await get_redis_client(require_redis=False)

        if redis_client is None:
            return {"available": False, "total_blacklisted": 0, "error": "Redis 不可用"}

        try:
            # 扫描黑名单 key
            pattern = f"{JWTBlacklistService.BLACKLIST_PREFIX}*"
            cursor = 0
            total = 0

            while True:
                cursor, keys = await redis_client.scan(cursor=cursor, match=pattern, count=100)
                total += len(keys)

                if cursor == 0:
                    break

            return {"available": True, "total_blacklisted": total}

        except Exception as e:
            logger.error(f"获取黑名单统计失败: {e}")
            return {"available": False, "total_blacklisted": 0, "error": str(e)}
