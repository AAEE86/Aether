"""
缓存工具类

提供同步缓存接口，用于不适合使用异步缓存的场景
"""

import threading
import time
from collections import OrderedDict
from typing import Any


class SyncLRUCache:
    """
    同步 LRU 缓存（带 TTL 和线程安全）

    用于需要同步访问的场景，如 ModelMapperMiddleware
    """

    def __init__(self, max_size: int = 1000, ttl: int = 300) -> None:
        """
        初始化缓存

        Args:
            max_size: 最大缓存条目数
            ttl: 过期时间（秒）
        """
        self._cache: OrderedDict = OrderedDict()
        self._expiry: dict[Any, float] = {}
        self.max_size = max_size
        self.ttl = ttl
        self._lock = threading.RLock()

    def _is_expired(self, key: Any) -> bool:
        """检查 key 是否过期（调用者需确保已持有锁）"""
        if key in self._expiry:
            return time.time() > self._expiry[key]
        return False

    def _delete_key(self, key: Any) -> None:
        """删除 key（调用者需确保已持有锁）"""
        if key in self._cache:
            del self._cache[key]
        if key in self._expiry:
            del self._expiry[key]

    def get(self, key: Any, default: Any = None) -> Any:
        """获取缓存值"""
        with self._lock:
            if key not in self._cache:
                return default

            if self._is_expired(key):
                self._delete_key(key)
                return default

            self._cache.move_to_end(key)
            return self._cache[key]

    def set(self, key: Any, value: Any, ttl: int | None = None) -> None:
        """设置缓存值"""
        with self._lock:
            if ttl is None:
                ttl = self.ttl

            if key in self._cache:
                self._cache.move_to_end(key)

            self._cache[key] = value
            self._expiry[key] = time.time() + ttl

            while len(self._cache) > self.max_size:
                oldest = next(iter(self._cache))
                self._delete_key(oldest)

    def delete(self, key: Any) -> None:
        """删除缓存值"""
        with self._lock:
            self._delete_key(key)

    def clear(self) -> None:
        """清空缓存"""
        with self._lock:
            self._cache.clear()
            self._expiry.clear()

    def __contains__(self, key: Any) -> bool:
        """检查 key 是否存在"""
        with self._lock:
            if key not in self._cache:
                return False
            if self._is_expired(key):
                self._delete_key(key)
                return False
            return True

    def __getitem__(self, key: Any) -> Any:
        """获取缓存值（通过索引）"""
        with self._lock:
            if key not in self._cache:
                raise KeyError(key)

            if self._is_expired(key):
                self._delete_key(key)
                raise KeyError(key)

            self._cache.move_to_end(key)
            return self._cache[key]

    def __setitem__(self, key: Any, value: Any) -> None:
        """设置缓存值（通过索引）"""
        self.set(key, value)

    def __delitem__(self, key: Any) -> None:
        """删除缓存值（通过索引）"""
        self.delete(key)

    def keys(self) -> list:
        """返回所有未过期的 key"""
        with self._lock:
            now = time.time()
            return [
                k for k in self._cache.keys() if k not in self._expiry or now <= self._expiry[k]
            ]

    def get_stats(self) -> dict[str, Any]:
        """获取缓存统计信息"""
        with self._lock:
            return {
                "size": len(self._cache),
                "max_size": self.max_size,
                "ttl": self.ttl,
            }
