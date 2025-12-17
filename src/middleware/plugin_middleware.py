"""
统一的插件中间件
负责协调所有插件的调用
"""

import time
from typing import Any, Awaitable, Callable, Optional

from fastapi import HTTPException, Request
from fastapi.responses import JSONResponse
from sqlalchemy.orm import Session
from starlette.middleware.base import BaseHTTPMiddleware
from starlette.responses import Response as StarletteResponse

from src.config import config
from src.core.logger import logger
from src.plugins.manager import get_plugin_manager
from src.plugins.rate_limit.base import RateLimitResult



class PluginMiddleware(BaseHTTPMiddleware):
    """
    统一的插件调用中间件

    职责:
    - 性能监控
    - 限流控制 (可选)

    注意: 认证由各路由通过 Depends() 显式声明，不在中间件层处理
    """

    def __init__(self, app: Any) -> None:
        super().__init__(app)
        self.plugin_manager = get_plugin_manager()

        # 从配置读取速率限制值
        self.llm_api_rate_limit = config.llm_api_rate_limit
        self.public_api_rate_limit = config.public_api_rate_limit

        # 完全跳过限流的路径（静态资源、文档等）
        self.skip_rate_limit_paths = [
            "/health",
            "/docs",
            "/redoc",
            "/openapi.json",
            "/favicon.ico",
            "/static/",
            "/assets/",
            "/api/admin/",  # 管理后台已有JWT认证，不需要额外限流
            "/api/auth/",  # 认证端点（由路由层的 IPRateLimiter 处理）
            "/api/users/",  # 用户端点
            "/api/monitoring/",  # 监控端点
        ]

        # LLM API 端点（需要特殊的速率限制策略）
        self.llm_api_paths = [
            "/v1/messages",
            "/v1/chat/completions",
            "/v1/responses",
            "/v1/completions",
        ]

    async def dispatch(
        self, request: Request, call_next: Callable[[Request], Awaitable[StarletteResponse]]
    ) -> StarletteResponse:
        """处理请求并调用相应插件"""

        # 记录请求开始时间
        start_time = time.time()
        request.state.request_id = request.headers.get("x-request-id", "")
        request.state.start_time = start_time
        # 标记：若请求过程中通过 Depends(get_db) 创建了会话，则由本中间件统一管理其生命周期
        request.state.db_managed_by_middleware = True

        response = None
        exception_to_raise = None

        try:
            # 1. 限流插件调用（可选功能）
            rate_limit_result = await self._call_rate_limit_plugins(request)
            if rate_limit_result and not rate_limit_result.allowed:
                # 限流触发，返回429
                headers = rate_limit_result.headers or {}
                raise HTTPException(
                    status_code=429,
                    detail=rate_limit_result.message or "Rate limit exceeded",
                    headers=headers,
                )

            # 2. 预处理插件调用
            await self._call_pre_request_plugins(request)

            # 处理请求
            response = await call_next(request)

            # 3. 提交关键数据库事务（在返回响应前）
            # 这确保了 Usage 记录、配额扣减等关键数据在响应返回前持久化
            try:
                db = getattr(request.state, "db", None)
                if isinstance(db, Session):
                    db.commit()
            except Exception as commit_error:
                logger.error(f"关键事务提交失败: {commit_error}")
                try:
                    if isinstance(db, Session):
                        db.rollback()
                except Exception:
                    pass
                await self._call_error_plugins(request, commit_error, start_time)
                # 返回 500 错误，因为数据可能不一致
                response = JSONResponse(
                    status_code=500,
                    content={
                        "type": "error",
                        "error": {
                            "type": "database_error",
                            "message": "数据保存失败，请重试",
                        },
                    },
                )
                # 跳过后处理插件，直接返回错误响应
                return response

            # 4. 后处理插件调用（监控等，非关键操作）
            # 这些操作失败不应影响用户响应
            await self._call_post_request_plugins(request, response, start_time)

            # 注意：不在此处添加限流响应头，因为在BaseHTTPMiddleware中
            # 响应返回后修改headers会导致Content-Length不匹配错误
            # 限流响应头已在返回429错误时正确包含（见上面的HTTPException）

        except RuntimeError as e:
            if str(e) == "No response returned.":
                db = getattr(request.state, "db", None)
                if isinstance(db, Session):
                    try:
                        db.rollback()
                    except Exception:
                        pass

                logger.error("Downstream handler completed without returning a response")

                await self._call_error_plugins(request, e, start_time)

                if isinstance(db, Session):
                    try:
                        db.commit()
                    except Exception:
                        pass

                response = JSONResponse(
                    status_code=500,
                    content={
                        "type": "error",
                        "error": {
                            "type": "internal_error",
                            "message": "Internal server error: downstream handler returned no response.",
                        },
                    },
                )
            else:
                exception_to_raise = e

        except Exception as e:
            # 回滚数据库事务
            db = getattr(request.state, "db", None)
            if isinstance(db, Session):
                try:
                    db.rollback()
                except Exception:
                    pass

            # 错误处理插件调用
            await self._call_error_plugins(request, e, start_time)

            # 尝试提交错误日志
            if isinstance(db, Session):
                try:
                    db.commit()
                except:
                    pass

            exception_to_raise = e

        finally:
            db = getattr(request.state, "db", None)
            if isinstance(db, Session):
                try:
                    db.close()
                except Exception as close_error:
                    # 连接池会处理连接的回收，这里的异常不应影响响应
                    logger.debug(f"关闭数据库连接时出错（可忽略）: {close_error}")

        # 在 finally 块之后处理异常和响应
        if exception_to_raise:
            raise exception_to_raise

        return response

    def _get_client_ip(self, request: Request) -> str:
        """
        获取客户端 IP 地址，支持代理头
        """
        # 优先从代理头获取真实 IP
        forwarded_for = request.headers.get("x-forwarded-for")
        if forwarded_for:
            # X-Forwarded-For 可能包含多个 IP，取第一个
            return forwarded_for.split(",")[0].strip()

        real_ip = request.headers.get("x-real-ip")
        if real_ip:
            return real_ip.strip()

        # 回退到直连 IP
        if request.client:
            return request.client.host

        return "unknown"

    def _is_llm_api_path(self, path: str) -> bool:
        """检查是否为 LLM API 端点"""
        for llm_path in self.llm_api_paths:
            if path.startswith(llm_path):
                return True
        return False

    async def _get_rate_limit_key_and_config(
        self, request: Request
    ) -> tuple[Optional[str], Optional[int]]:
        """
        获取速率限制的key和配置

        策略说明:
        - /v1/messages, /v1/chat/completions 等 LLM API: 按 API Key 限流
        - /api/public/* 端点: 使用服务器级别 IP 限制
        - /api/admin/* 端点: 跳过（在 skip_rate_limit_paths 中跳过）
        - /api/auth/* 端点: 跳过（由路由层的 IPRateLimiter 处理）

        Returns:
            (key, rate_limit_value) - key用于标识限制对象，rate_limit_value是限制值
        """
        path = request.url.path

        # LLM API 端点: 按 API Key 或 IP 限流
        if self._is_llm_api_path(path):
            # 尝试从请求头获取 API Key
            auth_header = request.headers.get("authorization", "")
            api_key = request.headers.get("x-api-key", "")

            if auth_header.startswith("Bearer "):
                api_key = auth_header[7:]

            if api_key:
                # 使用 API Key 的哈希作为限制 key（避免日志泄露完整 key）
                import hashlib

                key_hash = hashlib.sha256(api_key.encode()).hexdigest()[:16]
                key = f"llm_api_key:{key_hash}"
                request.state.rate_limit_key_type = "api_key"
            else:
                # 无 API Key 时使用 IP 限制（更严格）
                client_ip = self._get_client_ip(request)
                key = f"llm_ip:{client_ip}"
                request.state.rate_limit_key_type = "ip"

            rate_limit = self.llm_api_rate_limit
            request.state.rate_limit_value = rate_limit
            return key, rate_limit

        # /api/public/* 端点: 使用服务器级别 IP 地址作为限制 key
        if path.startswith("/api/public/"):
            client_ip = self._get_client_ip(request)
            key = f"public_ip:{client_ip}"
            rate_limit = self.public_api_rate_limit
            request.state.rate_limit_key_type = "public_ip"
            request.state.rate_limit_value = rate_limit
            return key, rate_limit

        # 其他端点不应用速率限制（或已在 skip_rate_limit_paths 中跳过）
        return None, None

    async def _call_rate_limit_plugins(self, request: Request) -> Optional[RateLimitResult]:
        """调用限流插件"""

        # 跳过不需要限流的路径（支持前缀匹配）
        for skip_path in self.skip_rate_limit_paths:
            if request.url.path == skip_path or request.url.path.startswith(skip_path):
                return None

        # 获取限流插件
        rate_limit_plugin = self.plugin_manager.get_plugin("rate_limit")
        if not rate_limit_plugin or not rate_limit_plugin.enabled:
            # 如果没有限流插件，允许通过
            return None

        # 获取速率限制的 key 和配置
        key, rate_limit_value = await self._get_rate_limit_key_and_config(request)
        if not key:
            # 不需要限流的端点（如未分类路径），静默跳过
            return None

        try:
            # 检查速率限制，传入数据库配置的限制值
            result = await rate_limit_plugin.check_limit(
                key=key,
                endpoint=request.url.path,
                method=request.method,
                rate_limit=rate_limit_value,  # 传入配置的限制值
            )
            # 类型检查：确保返回的是RateLimitResult类型
            if isinstance(result, RateLimitResult):
                # 如果检查通过，实际消耗令牌
                if result.allowed:
                    await rate_limit_plugin.consume(
                        key=key,
                        amount=1,
                        rate_limit=rate_limit_value,
                    )
                else:
                    # 限流触发，记录日志
                    logger.warning(f"速率限制触发: {getattr(request.state, 'rate_limit_key_type', 'unknown')}")
                return result
            return None
        except Exception as e:
            logger.error(f"Rate limit error: {e}")
            # 发生错误时允许请求通过
            return None

    async def _call_pre_request_plugins(self, request: Request) -> None:
        """调用请求前的插件（当前保留扩展点）"""
        pass

    async def _call_post_request_plugins(
        self, request: Request, response: StarletteResponse, start_time: float
    ) -> None:
        """调用请求后的插件"""

        duration = time.time() - start_time

        # 监控插件 - 记录指标
        monitor_plugin = self.plugin_manager.get_plugin("monitor")
        if monitor_plugin and monitor_plugin.enabled:
            try:
                monitor_labels = {
                    "method": request.method,
                    "endpoint": request.url.path,
                    "status": str(response.status_code),
                    "status_class": f"{response.status_code // 100}xx",
                }

                # 记录请求计数
                await monitor_plugin.increment(
                    "http_requests_total",
                    labels=monitor_labels,
                )

                # 记录请求时长
                await monitor_plugin.timing(
                    "http_request_duration",
                    duration,
                    labels=monitor_labels,
                )
            except Exception as e:
                logger.error(f"Monitor plugin failed: {e}")

    async def _call_error_plugins(
        self, request: Request, error: Exception, start_time: float
    ) -> None:
        """调用错误处理插件"""

        duration = time.time() - start_time

        # 通知插件 - 发送严重错误通知
        if not isinstance(error, HTTPException) or error.status_code >= 500:
            notification_plugin = self.plugin_manager.get_plugin("notification")
            if notification_plugin and notification_plugin.enabled:
                try:
                    await notification_plugin.send_error(
                        error=error,
                        context={
                            "endpoint": f"{request.method} {request.url.path}",
                            "request_id": request.state.request_id,
                            "duration": duration,
                        },
                    )
                except Exception as e:
                    logger.error(f"Notification plugin failed: {e}")
