"""
RPM (Requests Per Minute) 限流服务
"""

import time
from datetime import datetime, timedelta, timezone
from typing import Dict, Tuple

from sqlalchemy.orm import Session

from src.core.batch_committer import get_batch_committer
from src.core.logger import logger
from src.models.database import Provider
from src.models.database_extensions import ProviderUsageTracking



class RPMLimiter:
    """RPM限流器"""

    def __init__(self, db: Session):
        self.db = db
        # 内存中的RPM计数器 {provider_id: (count, window_start)}
        self._rpm_counters: Dict[str, Tuple[int, float]] = {}

    def check_and_increment(self, provider_id: str) -> bool:
        """
        检查并递增RPM计数

        Returns:
            True if allowed, False if rate limited
        """
        provider = self.db.query(Provider).filter(Provider.id == provider_id).first()
        if not provider:
            return True

        rpm_limit = provider.rpm_limit
        if rpm_limit is None:
            # 未设置限制
            return True

        if rpm_limit == 0:
            logger.warning(f"Provider {provider.name} is fully restricted by RPM limit=0")
            return False

        current_time = time.time()

        # 检查是否需要重置
        if provider.rpm_reset_at and provider.rpm_reset_at < datetime.now(timezone.utc):
            provider.rpm_used = 0
            provider.rpm_reset_at = datetime.fromtimestamp(current_time + 60, tz=timezone.utc)
            self.db.commit()  # 立即提交事务,释放数据库锁

        # 检查是否超限
        if provider.rpm_used >= rpm_limit:
            logger.warning(f"Provider {provider.name} RPM limit exceeded")
            return False

        # 递增计数
        provider.rpm_used += 1
        if not provider.rpm_reset_at:
            provider.rpm_reset_at = datetime.fromtimestamp(current_time + 60, tz=timezone.utc)

        self.db.commit()  # 立即提交事务,释放数据库锁
        return True

    def record_usage(
        self, provider_id: str, success: bool, response_time_ms: float, cost_usd: float
    ):
        """记录使用情况到追踪表"""

        # 获取当前分钟窗口
        now = datetime.now(timezone.utc)
        window_start = now.replace(second=0, microsecond=0)
        window_end = window_start + timedelta(minutes=1)

        # 查找或创建追踪记录
        tracking = (
            self.db.query(ProviderUsageTracking)
            .filter(
                ProviderUsageTracking.provider_id == provider_id,
                ProviderUsageTracking.window_start == window_start,
            )
            .first()
        )

        if not tracking:
            tracking = ProviderUsageTracking(
                provider_id=provider_id, window_start=window_start, window_end=window_end
            )
            self.db.add(tracking)

        # 更新统计
        tracking.total_requests += 1
        if success:
            tracking.successful_requests += 1
        else:
            tracking.failed_requests += 1

        tracking.total_response_time_ms += response_time_ms
        tracking.avg_response_time_ms = tracking.total_response_time_ms / tracking.total_requests
        tracking.total_cost_usd += cost_usd

        self.db.flush()  # 只 flush，不立即 commit
        # RPM 使用统计是非关键数据，使用批量提交
        get_batch_committer().mark_dirty(self.db)

        logger.debug(f"Recorded usage for provider {provider_id}")

    def get_rpm_status(self, provider_id: str) -> Dict:
        """获取提供商的RPM状态"""
        provider = self.db.query(Provider).filter(Provider.id == provider_id).first()
        if not provider:
            return {"error": "Provider not found"}

        return {
            "provider_id": provider_id,
            "provider_name": provider.name,
            "rpm_limit": provider.rpm_limit,
            "rpm_used": provider.rpm_used,
            "rpm_reset_at": provider.rpm_reset_at.isoformat() if provider.rpm_reset_at else None,
            "available": (
                provider.rpm_limit - provider.rpm_used if provider.rpm_limit is not None else None
            ),
        }

    def reset_rpm_counter(self, provider_id: str):
        """手动重置RPM计数器"""
        provider = self.db.query(Provider).filter(Provider.id == provider_id).first()
        if provider:
            provider.rpm_used = 0
            provider.rpm_reset_at = None
            self.db.commit()  # 立即提交事务,释放数据库锁

            logger.info(f"Reset RPM counter for provider {provider.name}")
