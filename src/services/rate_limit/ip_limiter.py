"""
IP 级别的速率限制服务

提供基于 IP 地址的速率限制，防止暴力破解和 DDoS 攻击
"""

import ipaddress

from src.clients.redis_client import get_redis_client
from src.core.logger import logger



class IPRateLimiter:
    """IP 速率限制服务"""

    # Redis key 前缀
    RATE_LIMIT_PREFIX = "ip:rate_limit:"
    BLACKLIST_PREFIX = "ip:blacklist:"
    WHITELIST_KEY = "ip:whitelist"

    # 默认限制配置（每分钟）
    DEFAULT_LIMITS = {
        "default": 100,  # 默认限制
        "login": 20,  # 登录接口
        "register": 10,  # 注册接口
        "api": 60,  # API 接口
        "public": 60,  # 公共接口
        "verification_send": 5,  # 发送验证码接口
        "verification_verify": 20,  # 验证验证码接口
    }

    @staticmethod
    async def check_limit(
        ip_address: str, endpoint_type: str = "default", limit: int | None = None
    ) -> tuple[bool, int, int]:
        """
        检查 IP 是否超过速率限制

        Args:
            ip_address: IP 地址
            endpoint_type: 端点类型（default, login, register, api, public）
            limit: 自定义限制值，None 则使用默认值

        Returns:
            (是否允许, 剩余次数, 重置时间秒数)
        """
        # 检查白名单
        if await IPRateLimiter.is_whitelisted(ip_address):
            return True, 999999, 60

        # 检查黑名单
        if await IPRateLimiter.is_blacklisted(ip_address):
            logger.warning(f"黑名单 IP 尝试访问: {ip_address}, 类型: {endpoint_type}")
            return False, 0, 0

        redis_client = await get_redis_client(require_redis=False)

        if redis_client is None:
            # Redis 不可用时降级：允许访问但记录警告
            logger.warning("Redis 不可用，跳过 IP 速率限制（降级模式）")
            return True, 0, 60

        # 确定限制值
        rate_limit = (
            limit if limit is not None else IPRateLimiter.DEFAULT_LIMITS.get(endpoint_type, 100)
        )

        try:
            # Redis key: ip:rate_limit:{type}:{ip}
            redis_key = f"{IPRateLimiter.RATE_LIMIT_PREFIX}{endpoint_type}:{ip_address}"

            # 使用 Redis 的滑动窗口计数器
            # INCR 并设置过期时间
            count = await redis_client.incr(redis_key)

            # 第一次访问时设置过期时间
            if count == 1:
                await redis_client.expire(redis_key, 60)  # 60秒窗口

            # 获取 TTL（剩余过期时间）
            ttl = await redis_client.ttl(redis_key)
            if ttl < 0:
                # 如果没有过期时间，重新设置
                await redis_client.expire(redis_key, 60)
                ttl = 60

            remaining = max(0, rate_limit - count)
            allowed = count <= rate_limit

            if not allowed:
                logger.warning(f"IP 速率限制触发: {ip_address}, 类型: {endpoint_type}, 计数: {count}/{rate_limit}")

            return allowed, remaining, ttl

        except Exception as e:
            logger.error(f"检查 IP 速率限制失败: {e}")
            # 发生错误时允许访问，避免误杀
            return True, 0, 60

    @staticmethod
    async def add_to_blacklist(
        ip_address: str, reason: str = "manual", ttl: int | None = None
    ) -> bool:
        """
        将 IP 加入黑名单

        Args:
            ip_address: IP 地址
            reason: 加入黑名单的原因
            ttl: 过期时间（秒），None 表示永久

        Returns:
            是否成功
        """
        redis_client = await get_redis_client(require_redis=False)

        if redis_client is None:
            logger.warning("Redis 不可用，无法将 IP 加入黑名单")
            return False

        try:
            redis_key = f"{IPRateLimiter.BLACKLIST_PREFIX}{ip_address}"

            if ttl is not None:
                await redis_client.setex(redis_key, ttl, reason)
            else:
                await redis_client.set(redis_key, reason)

            logger.warning(f"IP 已加入黑名单: {ip_address}, 原因: {reason}, TTL: {ttl or '永久'}")
            return True

        except Exception as e:
            logger.error(f"添加 IP 到黑名单失败: {e}")
            return False

    @staticmethod
    async def remove_from_blacklist(ip_address: str) -> bool:
        """
        从黑名单移除 IP

        Args:
            ip_address: IP 地址

        Returns:
            是否成功
        """
        redis_client = await get_redis_client(require_redis=False)

        if redis_client is None:
            logger.warning("Redis 不可用，无法从黑名单移除 IP")
            return False

        try:
            redis_key = f"{IPRateLimiter.BLACKLIST_PREFIX}{ip_address}"
            deleted = await redis_client.delete(redis_key)

            if deleted:
                logger.info(f"IP 已从黑名单移除: {ip_address}")

            return bool(deleted)

        except Exception as e:
            logger.error(f"从黑名单移除 IP 失败: {e}")
            return False

    @staticmethod
    async def is_blacklisted(ip_address: str) -> bool:
        """
        检查 IP 是否在黑名单中

        Args:
            ip_address: IP 地址

        Returns:
            是否在黑名单中
        """
        redis_client = await get_redis_client(require_redis=False)

        if redis_client is None:
            return False

        try:
            redis_key = f"{IPRateLimiter.BLACKLIST_PREFIX}{ip_address}"
            exists = await redis_client.exists(redis_key)
            return bool(exists)

        except Exception as e:
            logger.error(f"检查 IP 黑名单状态失败: {e}")
            return False

    @staticmethod
    async def add_to_whitelist(ip_address: str) -> bool:
        """
        将 IP 加入白名单

        Args:
            ip_address: IP 地址或 CIDR 格式（如 192.168.1.0/24）

        Returns:
            是否成功
        """
        redis_client = await get_redis_client(require_redis=False)

        if redis_client is None:
            logger.warning("Redis 不可用，无法将 IP 加入白名单")
            return False

        try:
            # 验证 IP 格式
            try:
                ipaddress.ip_network(ip_address, strict=False)
            except ValueError as e:
                logger.error(f"无效的 IP 地址格式: {ip_address}, 错误: {e}")
                return False

            # 使用 Redis Set 存储白名单
            await redis_client.sadd(IPRateLimiter.WHITELIST_KEY, ip_address)

            logger.info(f"IP 已加入白名单: {ip_address}")
            return True

        except Exception as e:
            logger.error(f"添加 IP 到白名单失败: {e}")
            return False

    @staticmethod
    async def remove_from_whitelist(ip_address: str) -> bool:
        """
        从白名单移除 IP

        Args:
            ip_address: IP 地址

        Returns:
            是否成功
        """
        redis_client = await get_redis_client(require_redis=False)

        if redis_client is None:
            logger.warning("Redis 不可用，无法从白名单移除 IP")
            return False

        try:
            removed = await redis_client.srem(IPRateLimiter.WHITELIST_KEY, ip_address)

            if removed:
                logger.info(f"IP 已从白名单移除: {ip_address}")

            return bool(removed)

        except Exception as e:
            logger.error(f"从白名单移除 IP 失败: {e}")
            return False

    @staticmethod
    async def is_whitelisted(ip_address: str) -> bool:
        """
        检查 IP 是否在白名单中（支持 CIDR 匹配）

        Args:
            ip_address: IP 地址

        Returns:
            是否在白名单中
        """
        redis_client = await get_redis_client(require_redis=False)

        if redis_client is None:
            return False

        try:
            # 获取所有白名单条目
            whitelist = await redis_client.smembers(IPRateLimiter.WHITELIST_KEY)

            if not whitelist:
                return False

            # 将 IP 地址转换为 ip_address 对象
            try:
                ip_obj = ipaddress.ip_address(ip_address)
            except ValueError:
                return False

            # 检查是否匹配白名单中的任何条目
            for entry in whitelist:
                try:
                    network = ipaddress.ip_network(entry, strict=False)
                    if ip_obj in network:
                        return True
                except ValueError:
                    # 如果条目格式无效，跳过
                    continue

            return False

        except Exception as e:
            logger.error(f"检查 IP 白名单状态失败: {e}")
            return False

    @staticmethod
    async def get_blacklist_stats() -> dict:
        """
        获取黑名单统计信息

        Returns:
            统计信息字典
        """
        redis_client = await get_redis_client(require_redis=False)

        if redis_client is None:
            return {"available": False, "total": 0, "error": "Redis 不可用"}

        try:
            pattern = f"{IPRateLimiter.BLACKLIST_PREFIX}*"
            cursor = 0
            total = 0

            while True:
                cursor, keys = await redis_client.scan(cursor=cursor, match=pattern, count=100)
                total += len(keys)

                if cursor == 0:
                    break

            return {"available": True, "total": total}

        except Exception as e:
            logger.error(f"获取黑名单统计失败: {e}")
            return {"available": False, "total": 0, "error": str(e)}

    @staticmethod
    async def get_whitelist() -> set[str]:
        """
        获取白名单列表

        Returns:
            白名单 IP 集合
        """
        redis_client = await get_redis_client(require_redis=False)

        if redis_client is None:
            return set()

        try:
            whitelist = await redis_client.smembers(IPRateLimiter.WHITELIST_KEY)
            return whitelist if whitelist else set()

        except Exception as e:
            logger.error(f"获取白名单失败: {e}")
            return set()
