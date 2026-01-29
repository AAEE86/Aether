"""
封装请求执行逻辑，包含并发控制与链路追踪。
"""

import time
from dataclasses import dataclass
from typing import Any

from collections.abc import Callable

from sqlalchemy.orm import Session

from src.core.api_format import APIFormat
from src.core.exceptions import ConcurrencyLimitError
from src.core.logger import logger
from src.services.health.monitor import health_monitor
from src.services.provider.format import normalize_api_format
from src.services.rate_limit.adaptive_reservation import get_adaptive_reservation_manager
from src.services.request.candidate import RequestCandidateService


@dataclass
class ExecutionContext:
    candidate_id: str
    candidate_index: int
    provider_id: str
    endpoint_id: str
    key_id: str
    user_id: str | None
    api_key_id: str | None
    is_cached_user: bool
    start_time: float | None = None
    elapsed_ms: int | None = None
    concurrent_requests: int | None = None


@dataclass
class ExecutionResult:
    response: Any
    context: ExecutionContext


class ExecutionError(Exception):
    def __init__(self, cause: Exception, context: ExecutionContext):
        super().__init__(str(cause))
        self.cause = cause
        self.context = context


class RequestExecutor:
    def __init__(self, db: Session, concurrency_manager, adaptive_manager):
        self.db = db
        self.concurrency_manager = concurrency_manager
        self.adaptive_manager = adaptive_manager

    async def execute(
        self,
        *,
        candidate,
        candidate_id: str,
        candidate_index: int,
        user_api_key,
        request_func: Callable,
        request_id: str | None,
        api_format: str | APIFormat,
        model_name: str,
        is_stream: bool = False,
    ) -> ExecutionResult:
        provider = candidate.provider
        endpoint = candidate.endpoint
        key = candidate.key
        is_cached_user = bool(candidate.is_cached)

        # 标记候选开始执行
        RequestCandidateService.mark_candidate_started(
            db=self.db,
            candidate_id=candidate_id,
        )

        context = ExecutionContext(
            candidate_id=candidate_id,
            candidate_index=candidate_index,
            provider_id=provider.id,
            endpoint_id=endpoint.id,
            key_id=key.id,
            user_id=user_api_key.user_id,
            api_key_id=user_api_key.id,
            is_cached_user=is_cached_user,
        )

        try:
            # 计算动态预留比例
            reservation_manager = get_adaptive_reservation_manager()
            # 获取当前 RPM 计数用于计算负载
            # 注意：key 侧返回的是 RPM 计数（不会在请求结束时减少，靠 TTL 过期）
            try:
                _, current_key_rpm = await self.concurrency_manager.get_current_concurrency(
                    endpoint_id=endpoint.id,
                    key_id=key.id,
                )
            except Exception as e:
                logger.debug(f"获取 RPM 计数失败（用于预留计算）: {e}")
                current_key_rpm = 0

            # 获取有效的 RPM 限制（自适应或固定）
            if key.rpm_limit is None:
                # 自适应模式：使用学习值，未学习时为 None（不限制，等待碰壁学习）
                effective_key_limit = (
                    int(key.learned_rpm_limit) if key.learned_rpm_limit is not None else None
                )
            else:
                effective_key_limit = int(key.rpm_limit)

            reservation_result = reservation_manager.calculate_reservation(
                key=key,
                current_usage=current_key_rpm,
                effective_limit=effective_key_limit,
            )
            dynamic_reservation_ratio = reservation_result.ratio

            logger.debug(
                f"[Executor] 动态预留: key={key.id[:8]}..., "
                f"ratio={dynamic_reservation_ratio:.0%}, phase={reservation_result.phase}, "
                f"confidence={reservation_result.confidence:.0%}"
            )

            async with self.concurrency_manager.rpm_guard(
                key_id=key.id,
                key_rpm_limit=effective_key_limit,
                is_cached_user=is_cached_user,
                cache_reservation_ratio=dynamic_reservation_ratio,
            ):
                # 获取当前 RPM 计数（guard 内再次获取以获得最新值）
                try:
                    key_rpm_count = await self.concurrency_manager.get_key_rpm_count(
                        key_id=key.id,
                    )
                except Exception as e:
                    logger.debug(f"获取 RPM 计数失败（guard 内）: {e}")
                    key_rpm_count = None

                context.concurrent_requests = key_rpm_count  # 用于记录，实际是 RPM 计数
                context.start_time = time.time()

                response = await request_func(provider, endpoint, key, candidate)

                context.elapsed_ms = int((time.time() - context.start_time) * 1000)

                provider_api_format = getattr(endpoint, "api_format", None)
                provider_format_str = (
                    provider_api_format.value
                    if isinstance(provider_api_format, APIFormat)
                    else str(provider_api_format or "")
                )
                client_format_str = (
                    api_format.value if isinstance(api_format, APIFormat) else str(api_format)
                )
                health_format = normalize_api_format(provider_format_str or client_format_str).value

                health_monitor.record_success(
                    db=self.db,
                    key_id=key.id,
                    api_format=health_format,
                    response_time_ms=context.elapsed_ms,
                )

                # 自适应模式：rpm_limit = NULL
                if key.rpm_limit is None and key_rpm_count is not None:
                    self.adaptive_manager.handle_success(
                        db=self.db,
                        key=key,
                        current_rpm=key_rpm_count,
                    )

                # 根据是否为流式请求，标记不同状态
                if is_stream:
                    # 流式请求：标记为 streaming 状态
                    # 此时连接已建立但流传输尚未完成
                    # success 状态会在流完成后由 _record_stream_stats 方法标记
                    RequestCandidateService.mark_candidate_streaming(
                        db=self.db,
                        candidate_id=candidate_id,
                        status_code=200,
                        concurrent_requests=key_rpm_count,
                    )
                else:
                    # 非流式请求：标记为 success 状态
                    RequestCandidateService.mark_candidate_success(
                        db=self.db,
                        candidate_id=candidate_id,
                        status_code=200,
                        latency_ms=context.elapsed_ms,
                        concurrent_requests=key_rpm_count,
                        extra_data={
                            "is_cached_user": is_cached_user,
                            "model_name": model_name,
                            "api_format": (
                                api_format.value
                                if isinstance(api_format, APIFormat)
                                else api_format
                            ),
                        },
                    )

                return ExecutionResult(response=response, context=context)
        except ConcurrencyLimitError as exc:
            raise ExecutionError(exc, context) from exc
        except Exception as exc:
            context.elapsed_ms = (
                int((time.time() - context.start_time) * 1000)
                if context.start_time is not None
                else None
            )
            raise ExecutionError(exc, context) from exc
