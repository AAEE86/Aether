"""
滑动窗口算法速率限制策略
精确的速率限制，不允许突发

WARNING: 多进程环境注意事项
=============================
此插件的窗口状态存储在进程内存中。如果使用 Gunicorn/uvicorn 多 worker 模式，
每个 worker 进程有独立的限流状态，可能导致：
- 实际允许的请求数 = 配置限制 * worker数量
- 限流效果大打折扣

解决方案：
1. 单 worker 模式：适用于低流量场景
2. Redis 共享状态：使用 Redis 实现分布式滑动窗口
3. 使用 token_bucket.py：令牌桶策略可以更容易迁移到 Redis

目前项目已有 Redis 依赖（src/clients/redis_client.py），
建议在生产环境使用 Redis 实现分布式限流。
"""

import asyncio
import time
from collections import deque
from datetime import datetime, timezone
from typing import Any

from src.core.logger import logger
from .base import RateLimitResult, RateLimitStrategy



class SlidingWindow:
    """滑动窗口实现"""

    def __init__(self, window_size: int, max_requests: int):
        """
        初始化滑动窗口

        Args:
            window_size: 窗口大小（秒）
            max_requests: 窗口内最大请求数
        """
        self.window_size = window_size
        self.max_requests = max_requests
        self.requests: deque[float] = deque()
        self.last_access_time: float = time.time()

    def _cleanup(self):
        """清理过期的请求记录"""
        current_time = time.time()
        self.last_access_time = current_time  # 更新最后访问时间
        cutoff_time = current_time - self.window_size

        # 移除窗口外的请求
        while self.requests and self.requests[0] < cutoff_time:
            self.requests.popleft()

    def can_accept(self, amount: int = 1) -> bool:
        """
        检查是否可以接受新请求

        Args:
            amount: 请求数量

        Returns:
            是否可以接受
        """
        self._cleanup()
        return len(self.requests) + amount <= self.max_requests

    def add_request(self, amount: int = 1) -> bool:
        """
        添加请求记录

        Args:
            amount: 请求数量

        Returns:
            是否成功添加
        """
        if not self.can_accept(amount):
            return False

        current_time = time.time()
        for _ in range(amount):
            self.requests.append(current_time)
        return True

    def get_remaining(self) -> int:
        """获取剩余配额"""
        self._cleanup()
        return max(0, self.max_requests - len(self.requests))

    def get_reset_time(self) -> datetime:
        """获取最早的重置时间"""
        self._cleanup()
        if not self.requests:
            return datetime.now(timezone.utc)

        # 最早的请求将在window_size秒后过期
        oldest_request = self.requests[0]
        reset_time = oldest_request + self.window_size
        return datetime.fromtimestamp(reset_time, tz=timezone.utc)


class SlidingWindowStrategy(RateLimitStrategy):
    """
    滑动窗口算法速率限制策略

    特点：
    - 精确的速率限制
    - 不允许突发流量
    - 适合需要严格速率控制的场景
    - 自动清理长时间不活跃的窗口，防止内存泄漏
    """

    # 默认最大缓存窗口数量
    DEFAULT_MAX_WINDOWS = 10000
    # 默认窗口过期时间（秒）- 超过此时间未访问的窗口将被清理
    DEFAULT_WINDOW_EXPIRY = 3600  # 1小时

    def __init__(self):
        super().__init__("sliding_window")
        self.windows: dict[str, SlidingWindow] = {}
        self._lock = asyncio.Lock()

        # 默认配置
        self.default_window_size = 60  # 默认60秒窗口
        self.default_max_requests = 100  # 默认100个请求

        # 内存管理配置
        self.max_windows = self.DEFAULT_MAX_WINDOWS
        self.window_expiry = self.DEFAULT_WINDOW_EXPIRY
        self._last_cleanup_time: float = time.time()
        self._cleanup_interval = 300  # 每5分钟检查一次是否需要清理

    def _cleanup_expired_windows(self) -> int:
        """
        清理过期的窗口，防止内存泄漏

        Returns:
            清理的窗口数量
        """
        current_time = time.time()
        expired_keys = []

        for key, window in self.windows.items():
            # 检查窗口是否过期（长时间未访问）
            if current_time - window.last_access_time > self.window_expiry:
                expired_keys.append(key)

        # 删除过期窗口
        for key in expired_keys:
            del self.windows[key]

        if expired_keys:
            logger.info(f"清理了 {len(expired_keys)} 个过期的滑动窗口")

        return len(expired_keys)

    def _evict_lru_windows(self, count: int) -> int:
        """
        使用 LRU 策略淘汰最久未使用的窗口

        Args:
            count: 需要淘汰的数量

        Returns:
            实际淘汰的数量
        """
        if not self.windows or count <= 0:
            return 0

        # 按最后访问时间排序，淘汰最久未访问的
        sorted_keys = sorted(self.windows.keys(), key=lambda k: self.windows[k].last_access_time)

        evicted = 0
        for key in sorted_keys[:count]:
            del self.windows[key]
            evicted += 1

        if evicted:
            logger.warning(f"LRU 淘汰了 {evicted} 个滑动窗口（达到容量上限）")

        return evicted

    async def _maybe_cleanup(self):
        """检查是否需要执行清理操作"""
        current_time = time.time()

        # 定期清理过期窗口
        if current_time - self._last_cleanup_time > self._cleanup_interval:
            self._cleanup_expired_windows()
            self._last_cleanup_time = current_time

        # 如果超过容量上限，执行 LRU 淘汰
        if len(self.windows) >= self.max_windows:
            # 淘汰 10% 的窗口
            evict_count = max(1, self.max_windows // 10)
            self._evict_lru_windows(evict_count)

    def _get_window(self, key: str) -> SlidingWindow:
        """
        获取或创建滑动窗口

        Args:
            key: 限制键

        Returns:
            滑动窗口实例
        """
        if key not in self.windows:
            # 根据key的不同前缀使用不同的配置
            if key.startswith("api_key:"):
                window_size = self.config.get("api_key_window_size", self.default_window_size)
                max_requests = self.config.get("api_key_max_requests", self.default_max_requests)
            elif key.startswith("user:"):
                window_size = self.config.get("user_window_size", self.default_window_size)
                max_requests = self.config.get("user_max_requests", self.default_max_requests * 2)
            else:
                window_size = self.default_window_size
                max_requests = self.default_max_requests

            self.windows[key] = SlidingWindow(window_size, max_requests)

        return self.windows[key]

    async def check_limit(self, key: str, **kwargs) -> RateLimitResult:
        """
        检查速率限制

        Args:
            key: 限制键

        Returns:
            速率限制检查结果
        """
        async with self._lock:
            # 检查是否需要清理过期窗口
            await self._maybe_cleanup()

            window = self._get_window(key)
            amount = kwargs.get("amount", 1)

            # 检查是否可以接受请求
            allowed = window.can_accept(amount)
            remaining = window.get_remaining()
            reset_at = window.get_reset_time()

            retry_after = None
            if not allowed:
                # 计算需要等待的时间（最早请求过期的时间）
                retry_after = int((reset_at - datetime.now(timezone.utc)).total_seconds()) + 1

            return RateLimitResult(
                allowed=allowed,
                remaining=remaining,
                reset_at=reset_at,
                retry_after=retry_after,
                message=(
                    None
                    if allowed
                    else f"Rate limit exceeded. Please retry after {retry_after} seconds."
                ),
            )

    async def consume(self, key: str, amount: int = 1, **kwargs) -> bool:
        """
        消费配额

        Args:
            key: 限制键
            amount: 消费数量

        Returns:
            是否成功消费
        """
        async with self._lock:
            window = self._get_window(key)
            success = window.add_request(amount)

            if success:
                logger.debug(f"滑动窗口请求记录成功")
            else:
                logger.warning(f"滑动窗口请求被拒绝：超出速率限制")

            return success

    async def reset(self, key: str):
        """
        重置滑动窗口

        Args:
            key: 限制键
        """
        async with self._lock:
            if key in self.windows:
                window = self.windows[key]
                window.requests.clear()

                logger.info(f"滑动窗口已重置")

    async def get_stats(self, key: str) -> dict[str, Any]:
        """
        获取统计信息

        Args:
            key: 限制键

        Returns:
            统计信息
        """
        async with self._lock:
            window = self._get_window(key)
            window._cleanup()  # 先清理过期请求

            return {
                "strategy": "sliding_window",
                "key": key,
                "window_size": window.window_size,
                "max_requests": window.max_requests,
                "current_requests": len(window.requests),
                "remaining": window.get_remaining(),
                "reset_at": window.get_reset_time().isoformat(),
            }

    def configure(self, config: dict[str, Any]):
        """
        配置策略

        支持的配置项：
        - api_key_window_size: API Key的窗口大小（秒）
        - api_key_max_requests: API Key的最大请求数
        - user_window_size: 用户的窗口大小（秒）
        - user_max_requests: 用户的最大请求数
        - max_windows: 最大缓存窗口数量（防止内存泄漏）
        - window_expiry: 窗口过期时间（秒）
        - cleanup_interval: 清理检查间隔（秒）
        """
        super().configure(config)
        self.default_window_size = config.get("default_window_size", self.default_window_size)
        self.default_max_requests = config.get("default_max_requests", self.default_max_requests)
        self.max_windows = config.get("max_windows", self.max_windows)
        self.window_expiry = config.get("window_expiry", self.window_expiry)
        self._cleanup_interval = config.get("cleanup_interval", self._cleanup_interval)

    def get_memory_stats(self) -> dict[str, Any]:
        """
        获取内存使用统计信息

        Returns:
            内存使用统计
        """
        return {
            "total_windows": len(self.windows),
            "max_windows": self.max_windows,
            "window_expiry": self.window_expiry,
            "cleanup_interval": self._cleanup_interval,
            "last_cleanup_time": self._last_cleanup_time,
            "usage_percent": (
                (len(self.windows) / self.max_windows * 100) if self.max_windows > 0 else 0
            ),
        }
