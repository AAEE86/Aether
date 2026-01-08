"""
全局HTTP客户端池管理
避免每次请求都创建新的AsyncClient,提高性能

性能优化说明：
1. 默认客户端：无代理场景，全局复用单一客户端
2. 代理客户端缓存：相同代理配置复用同一客户端，避免重复创建
3. 连接池复用：Keep-alive 连接减少 TCP 握手开销
"""

import asyncio
import hashlib
import time
from contextlib import asynccontextmanager
from typing import Any, Dict, Optional, Tuple
from urllib.parse import quote, urlparse

import httpx

from src.config import config
from src.core.logger import logger

# 模块级锁，避免类属性延迟初始化的竞态条件
_proxy_clients_lock = asyncio.Lock()
_default_client_lock = asyncio.Lock()


def _compute_proxy_cache_key(proxy_config: Optional[Dict[str, Any]]) -> str:
    """
    计算代理配置的缓存键

    Args:
        proxy_config: 代理配置字典

    Returns:
        缓存键字符串，无代理时返回 "__no_proxy__"
    """
    if not proxy_config:
        return "__no_proxy__"

    # 构建代理 URL 作为缓存键的基础
    proxy_url = build_proxy_url(proxy_config)
    if not proxy_url:
        return "__no_proxy__"

    # 使用 MD5 哈希来避免过长的键名
    return f"proxy:{hashlib.md5(proxy_url.encode()).hexdigest()[:16]}"


def build_proxy_url(proxy_config: Dict[str, Any]) -> Optional[str]:
    """
    根据代理配置构建完整的代理 URL

    Args:
        proxy_config: 代理配置字典，包含 url, username, password, enabled

    Returns:
        完整的代理 URL，如 socks5://user:pass@host:port
        如果 enabled=False 或无配置，返回 None
    """
    if not proxy_config:
        return None

    # 检查 enabled 字段，默认为 True（兼容旧数据）
    if not proxy_config.get("enabled", True):
        return None

    proxy_url = proxy_config.get("url")
    if not proxy_url:
        return None

    username = proxy_config.get("username")
    password = proxy_config.get("password")

    # 只要有用户名就添加认证信息（密码可以为空）
    if username:
        parsed = urlparse(proxy_url)
        # URL 编码用户名和密码，处理特殊字符（如 @, :, /）
        encoded_username = quote(username, safe="")
        encoded_password = quote(password, safe="") if password else ""
        # 重新构建带认证的代理 URL
        if encoded_password:
            auth_proxy = f"{parsed.scheme}://{encoded_username}:{encoded_password}@{parsed.netloc}"
        else:
            auth_proxy = f"{parsed.scheme}://{encoded_username}@{parsed.netloc}"
        if parsed.path:
            auth_proxy += parsed.path
        return auth_proxy

    return proxy_url


class HTTPClientPool:
    """
    全局HTTP客户端池单例

    管理可重用的httpx.AsyncClient实例,避免频繁创建/销毁连接

    性能优化：
    1. 默认客户端：无代理场景复用
    2. 代理客户端缓存：相同代理配置复用同一客户端
    3. LRU 淘汰：代理客户端超过上限时淘汰最久未使用的
    """

    _instance: Optional["HTTPClientPool"] = None
    _default_client: Optional[httpx.AsyncClient] = None
    _clients: Dict[str, httpx.AsyncClient] = {}
    # 代理客户端缓存：{cache_key: (client, last_used_time)}
    _proxy_clients: Dict[str, Tuple[httpx.AsyncClient, float]] = {}
    # 代理客户端缓存上限（避免内存泄漏）
    _max_proxy_clients: int = 50

    def __new__(cls):
        if cls._instance is None:
            cls._instance = super().__new__(cls)
        return cls._instance

    @classmethod
    async def get_default_client_async(cls) -> httpx.AsyncClient:
        """
        获取默认的HTTP客户端（异步线程安全版本）

        用于大多数HTTP请求,具有合理的默认配置
        """
        if cls._default_client is not None:
            return cls._default_client

        async with _default_client_lock:
            # 双重检查，避免重复创建
            if cls._default_client is None:
                cls._default_client = httpx.AsyncClient(
                    http2=False,  # 暂时禁用HTTP/2以提高兼容性
                    verify=True,  # 启用SSL验证
                    timeout=httpx.Timeout(
                        connect=config.http_connect_timeout,
                        read=config.http_read_timeout,
                        write=config.http_write_timeout,
                        pool=config.http_pool_timeout,
                    ),
                    limits=httpx.Limits(
                        max_connections=config.http_max_connections,
                        max_keepalive_connections=config.http_keepalive_connections,
                        keepalive_expiry=config.http_keepalive_expiry,
                    ),
                    follow_redirects=True,  # 跟随重定向
                )
                logger.info(
                    f"全局HTTP客户端池已初始化: "
                    f"max_connections={config.http_max_connections}, "
                    f"keepalive={config.http_keepalive_connections}, "
                    f"keepalive_expiry={config.http_keepalive_expiry}s"
                )
        return cls._default_client

    @classmethod
    def get_default_client(cls) -> httpx.AsyncClient:
        """
        获取默认的HTTP客户端（同步版本，向后兼容）

        ⚠️ 注意：此方法在高并发首次调用时可能存在竞态条件，
        推荐使用 get_default_client_async() 异步版本。
        """
        if cls._default_client is None:
            cls._default_client = httpx.AsyncClient(
                http2=False,  # 暂时禁用HTTP/2以提高兼容性
                verify=True,  # 启用SSL验证
                timeout=httpx.Timeout(
                    connect=config.http_connect_timeout,
                    read=config.http_read_timeout,
                    write=config.http_write_timeout,
                    pool=config.http_pool_timeout,
                ),
                limits=httpx.Limits(
                    max_connections=config.http_max_connections,
                    max_keepalive_connections=config.http_keepalive_connections,
                    keepalive_expiry=config.http_keepalive_expiry,
                ),
                follow_redirects=True,  # 跟随重定向
            )
            logger.info(
                f"全局HTTP客户端池已初始化: "
                f"max_connections={config.http_max_connections}, "
                f"keepalive={config.http_keepalive_connections}, "
                f"keepalive_expiry={config.http_keepalive_expiry}s"
            )
        return cls._default_client

    @classmethod
    def get_client(cls, name: str, **kwargs: Any) -> httpx.AsyncClient:
        """
        获取或创建命名的HTTP客户端

        用于需要特定配置的场景(如不同的超时设置、代理等)

        Args:
            name: 客户端标识符
            **kwargs: httpx.AsyncClient的配置参数
        """
        if name not in cls._clients:
            # 合并默认配置和自定义配置
            default_config = {
                "http2": False,
                "verify": True,
                "timeout": httpx.Timeout(
                    connect=config.http_connect_timeout,
                    read=config.http_read_timeout,
                    write=config.http_write_timeout,
                    pool=config.http_pool_timeout,
                ),
                "follow_redirects": True,
            }
            default_config.update(kwargs)

            cls._clients[name] = httpx.AsyncClient(**default_config)
            logger.debug(f"创建命名HTTP客户端: {name}")

        return cls._clients[name]

    @classmethod
    def _get_proxy_clients_lock(cls) -> asyncio.Lock:
        """获取代理客户端缓存锁（模块级单例，避免竞态条件）"""
        return _proxy_clients_lock

    @classmethod
    async def _evict_lru_proxy_client(cls) -> None:
        """淘汰最久未使用的代理客户端"""
        if len(cls._proxy_clients) < cls._max_proxy_clients:
            return

        # 找到最久未使用的客户端
        oldest_key = min(cls._proxy_clients.keys(), key=lambda k: cls._proxy_clients[k][1])
        old_client, _ = cls._proxy_clients.pop(oldest_key)

        # 异步关闭旧客户端
        try:
            await old_client.aclose()
            logger.debug(f"淘汰代理客户端: {oldest_key}")
        except Exception as e:
            logger.warning(f"关闭代理客户端失败: {e}")

    @classmethod
    async def get_proxy_client(
        cls,
        proxy_config: Optional[Dict[str, Any]] = None,
    ) -> httpx.AsyncClient:
        """
        获取代理客户端（带缓存复用）

        相同代理配置会复用同一个客户端，大幅减少连接建立开销。
        注意：返回的客户端使用默认超时配置，如需自定义超时请在请求时传递 timeout 参数。

        Args:
            proxy_config: 代理配置字典，包含 url, username, password

        Returns:
            可复用的 httpx.AsyncClient 实例
        """
        cache_key = _compute_proxy_cache_key(proxy_config)

        # 无代理时返回默认客户端
        if cache_key == "__no_proxy__":
            return await cls.get_default_client_async()

        lock = cls._get_proxy_clients_lock()
        async with lock:
            # 检查缓存
            if cache_key in cls._proxy_clients:
                client, _ = cls._proxy_clients[cache_key]
                # 健康检查：如果客户端已关闭，移除并重新创建
                if client.is_closed:
                    del cls._proxy_clients[cache_key]
                    logger.debug(f"代理客户端已关闭，将重新创建: {cache_key}")
                else:
                    # 更新最后使用时间
                    cls._proxy_clients[cache_key] = (client, time.time())
                    return client

            # 淘汰旧客户端（如果超过上限）
            await cls._evict_lru_proxy_client()

            # 创建新客户端（使用默认超时，请求时可覆盖）
            client_config: Dict[str, Any] = {
                "http2": False,
                "verify": True,
                "follow_redirects": True,
                "limits": httpx.Limits(
                    max_connections=config.http_max_connections,
                    max_keepalive_connections=config.http_keepalive_connections,
                    keepalive_expiry=config.http_keepalive_expiry,
                ),
                "timeout": httpx.Timeout(
                    connect=config.http_connect_timeout,
                    read=config.http_read_timeout,
                    write=config.http_write_timeout,
                    pool=config.http_pool_timeout,
                ),
            }

            # 添加代理配置
            proxy_url = build_proxy_url(proxy_config) if proxy_config else None
            if proxy_url:
                client_config["proxy"] = proxy_url

            client = httpx.AsyncClient(**client_config)
            cls._proxy_clients[cache_key] = (client, time.time())

            logger.debug(
                f"创建代理客户端(缓存): {proxy_config.get('url', 'unknown') if proxy_config else 'none'}, "
                f"缓存数量: {len(cls._proxy_clients)}"
            )

            return client

    @classmethod
    async def close_all(cls):
        """关闭所有HTTP客户端"""
        if cls._default_client is not None:
            await cls._default_client.aclose()
            cls._default_client = None
            logger.info("默认HTTP客户端已关闭")

        for name, client in cls._clients.items():
            await client.aclose()
            logger.debug(f"命名HTTP客户端已关闭: {name}")

        cls._clients.clear()

        # 关闭代理客户端缓存
        for cache_key, (client, _) in cls._proxy_clients.items():
            try:
                await client.aclose()
                logger.debug(f"代理客户端已关闭: {cache_key}")
            except Exception as e:
                logger.warning(f"关闭代理客户端失败: {e}")

        cls._proxy_clients.clear()
        logger.info("所有HTTP客户端已关闭")

    @classmethod
    @asynccontextmanager
    async def get_temp_client(cls, **kwargs: Any):
        """
        获取临时HTTP客户端(上下文管理器)

        用于一次性请求,使用后自动关闭

        用法:
            async with HTTPClientPool.get_temp_client() as client:
                response = await client.get('https://example.com')
        """
        default_config = {
            "http2": False,
            "verify": True,
            "timeout": httpx.Timeout(
                connect=config.http_connect_timeout,
                read=config.http_read_timeout,
                write=config.http_write_timeout,
                pool=config.http_pool_timeout,
            ),
        }
        default_config.update(kwargs)

        client = httpx.AsyncClient(**default_config)
        try:
            yield client
        finally:
            await client.aclose()

    @classmethod
    def create_client_with_proxy(
        cls,
        proxy_config: Optional[Dict[str, Any]] = None,
        timeout: Optional[httpx.Timeout] = None,
        **kwargs: Any,
    ) -> httpx.AsyncClient:
        """
        创建带代理配置的HTTP客户端

        ⚠️ 性能警告：此方法每次都创建新客户端，推荐使用 get_proxy_client() 复用连接。

        Args:
            proxy_config: 代理配置字典，包含 url, username, password
            timeout: 超时配置
            **kwargs: 其他 httpx.AsyncClient 配置参数

        Returns:
            配置好的 httpx.AsyncClient 实例（调用者需要负责关闭）
        """
        client_config: Dict[str, Any] = {
            "http2": False,
            "verify": True,
            "follow_redirects": True,
        }

        if timeout:
            client_config["timeout"] = timeout
        else:
            client_config["timeout"] = httpx.Timeout(
                connect=config.http_connect_timeout,
                read=config.http_read_timeout,
                write=config.http_write_timeout,
                pool=config.http_pool_timeout,
            )

        # 添加代理配置
        proxy_url = build_proxy_url(proxy_config) if proxy_config else None
        if proxy_url:
            client_config["proxy"] = proxy_url
            logger.debug(f"创建带代理的HTTP客户端(一次性): {proxy_config.get('url', 'unknown')}")

        client_config.update(kwargs)
        return httpx.AsyncClient(**client_config)

    @classmethod
    def get_pool_stats(cls) -> Dict[str, Any]:
        """获取连接池统计信息"""
        return {
            "default_client_active": cls._default_client is not None,
            "named_clients_count": len(cls._clients),
            "proxy_clients_count": len(cls._proxy_clients),
            "max_proxy_clients": cls._max_proxy_clients,
        }


# 便捷访问函数
def get_http_client() -> httpx.AsyncClient:
    """获取默认HTTP客户端的便捷函数"""
    return HTTPClientPool.get_default_client()


async def close_http_clients():
    """关闭所有HTTP客户端的便捷函数"""
    await HTTPClientPool.close_all()
