"""
超时保护工具

为异步函数和操作提供超时保护
"""

import asyncio
from functools import wraps
from typing import Any, TypeVar

from collections.abc import Callable

from src.core.logger import logger


T = TypeVar("T")


class AsyncTimeoutError(TimeoutError):
    """异步操作超时错误"""

    def __init__(self, message: str, operation: str, timeout: float):
        super().__init__(message)
        self.operation = operation
        self.timeout = timeout


def with_timeout(seconds: float, operation_name: str | None = None):
    """
    装饰器：为异步函数添加超时保护

    Args:
        seconds: 超时时间（秒）
        operation_name: 操作名称（用于日志，默认使用函数名）

    Usage:
        @with_timeout(30.0)
        async def my_async_function():
            ...

        @with_timeout(60.0, operation_name="API请求")
        async def api_call():
            ...
    """

    def decorator(func: Callable[..., Any]) -> Callable[..., Any]:
        @wraps(func)
        async def wrapper(*args, **kwargs):
            op_name = operation_name or func.__name__
            try:
                return await asyncio.wait_for(func(*args, **kwargs), timeout=seconds)
            except TimeoutError:
                logger.warning(f"操作超时: {op_name} (timeout={seconds}s)")
                raise AsyncTimeoutError(
                    f"{op_name} 操作超时（{seconds}秒）",
                    operation=op_name,
                    timeout=seconds,
                )

        return wrapper

    return decorator


async def run_with_timeout(
    coro,
    timeout: float,
    operation_name: str = "operation",
    default: T = None,
    raise_on_timeout: bool = True,
) -> T:
    """
    为协程添加超时保护（函数式调用）

    Args:
        coro: 协程对象
        timeout: 超时时间（秒）
        operation_name: 操作名称（用于日志）
        default: 超时时返回的默认值（仅在 raise_on_timeout=False 时有效）
        raise_on_timeout: 超时时是否抛出异常

    Returns:
        协程的返回值，或超时时的默认值

    Usage:
        result = await run_with_timeout(
            my_async_function(),
            timeout=30.0,
            operation_name="API请求"
        )
    """
    try:
        return await asyncio.wait_for(coro, timeout=timeout)
    except TimeoutError:
        logger.warning(f"操作超时: {operation_name} (timeout={timeout}s)")
        if raise_on_timeout:
            raise AsyncTimeoutError(
                f"{operation_name} 操作超时（{timeout}秒）",
                operation=operation_name,
                timeout=timeout,
            )
        return default


class TimeoutContext:
    """
    超时上下文管理器

    Usage:
        async with TimeoutContext(30.0, "数据库查询") as ctx:
            result = await db.query(...)
            # 如果超过30秒会抛出 AsyncTimeoutError
    """

    def __init__(self, timeout: float, operation_name: str = "operation"):
        self.timeout = timeout
        self.operation_name = operation_name
        self._task: asyncio.Task | None = None

    async def __aenter__(self):
        return self

    async def __aexit__(self, exc_type, exc_val, exc_tb):
        # asyncio.timeout 在 Python 3.11+ 可用
        # 这里使用更通用的方式
        pass


async def with_timeout_context(timeout: float, operation_name: str = "operation"):
    """
    超时上下文管理器（Python 3.11+ asyncio.timeout 的替代）

    Usage:
        async with with_timeout_context(30.0, "API请求"):
            result = await api_call()
    """
    try:
        # Python 3.11+ 使用内置的 asyncio.timeout
        return asyncio.timeout(timeout)
    except AttributeError:
        # Python 3.10 及以下版本的兼容实现
        # 注意：这个简单实现不支持嵌套取消
        pass


async def read_first_chunk_with_ttfb_timeout(
    byte_iterator: Any,
    timeout: float,
    request_id: str,
    provider_name: str,
) -> tuple[bytes, Any]:
    """
    读取流的首字节并应用 TTFB 超时检测

    首字节超时（Time To First Byte）用于检测慢响应的 Provider，
    超时时触发故障转移到其他可用的 Provider。

    Args:
        byte_iterator: 异步字节流迭代器
        timeout: TTFB 超时时间（秒）
        request_id: 请求 ID（用于日志）
        provider_name: Provider 名称（用于日志和异常）

    Returns:
        (first_chunk, aiter): 首个字节块和异步迭代器

    Raises:
        ProviderTimeoutException: 如果首字节超时
    """
    from src.core.exceptions import ProviderTimeoutException

    aiter = byte_iterator.__aiter__()

    try:
        first_chunk = await asyncio.wait_for(aiter.__anext__(), timeout=timeout)
        return first_chunk, aiter
    except TimeoutError:
        # 完整的资源清理：先关闭迭代器，再关闭底层响应
        await _cleanup_iterator_resources(aiter, request_id)
        logger.warning(
            f"  [{request_id}] 流首字节超时 (TTFB): "
            f"Provider={provider_name}, timeout={timeout}s"
        )
        raise ProviderTimeoutException(
            provider_name=provider_name,
            timeout=int(timeout),
        )


async def _cleanup_iterator_resources(aiter: Any, request_id: str) -> None:
    """
    清理异步迭代器及其底层资源

    确保在 TTFB 超时后正确释放 HTTP 连接，避免连接泄漏。

    Args:
        aiter: 异步迭代器
        request_id: 请求 ID（用于日志）
    """
    # 1. 关闭迭代器本身
    if hasattr(aiter, "aclose"):
        try:
            await aiter.aclose()
        except Exception as e:
            logger.debug(f"  [{request_id}] 关闭迭代器失败: {e}")

    # 2. 关闭底层响应对象（httpx.Response）
    # 迭代器可能持有 _response 属性指向底层响应
    response = getattr(aiter, "_response", None)
    if response is not None and hasattr(response, "aclose"):
        try:
            await response.aclose()
        except Exception as e:
            logger.debug(f"  [{request_id}] 关闭底层响应失败: {e}")

    # 3. 尝试关闭 httpx 流（如果迭代器是 httpx 的 aiter_bytes）
    # httpx 的 Response.aiter_bytes() 返回的生成器可能有 _stream 属性
    stream = getattr(aiter, "_stream", None)
    if stream is not None and hasattr(stream, "aclose"):
        try:
            await stream.aclose()
        except Exception as e:
            logger.debug(f"  [{request_id}] 关闭流对象失败: {e}")
