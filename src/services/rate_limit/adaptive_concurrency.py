"""
自适应并发调整器 - 基于滑动窗口利用率的并发限制调整

核心改进（相对于旧版基于"持续高利用率"的方案）：
- 使用滑动窗口采样，容忍并发波动
- 基于窗口内高利用率采样比例决策，而非要求连续高利用率
- 增加探测性扩容机制，长时间稳定时主动尝试扩容

AIMD 参数说明：
- 扩容：加性增加 (+INCREASE_STEP)
- 缩容：乘性减少 (*DECREASE_MULTIPLIER，默认 0.85)
"""

from datetime import datetime, timezone
from typing import Any, Dict, List, Optional, cast

from sqlalchemy.orm import Session

from src.config.constants import ConcurrencyDefaults
from src.core.batch_committer import get_batch_committer
from src.core.logger import logger
from src.models.database import ProviderAPIKey
from src.services.rate_limit.detector import RateLimitInfo, RateLimitType


class AdaptiveStrategy:
    """自适应策略类型"""

    AIMD = "aimd"  # 加性增-乘性减 (Additive Increase Multiplicative Decrease)
    CONSERVATIVE = "conservative"  # 保守策略（只减不增）
    AGGRESSIVE = "aggressive"  # 激进策略（快速探测）


class AdaptiveConcurrencyManager:
    """
    自适应并发管理器

    核心算法：基于滑动窗口利用率的 AIMD
    - 滑动窗口记录最近 N 次请求的利用率
    - 当窗口内高利用率采样比例 >= 60% 时触发扩容
    - 遇到 429 错误时乘性减少 (*0.85)
    - 长时间无 429 且有流量时触发探测性扩容

    扩容条件（满足任一即可）：
    1. 滑动窗口扩容：窗口内 >= 60% 的采样利用率 >= 70%，且不在冷却期
    2. 探测性扩容：距上次 429 超过 30 分钟，且期间有足够请求量

    关键特性：
    1. 滑动窗口容忍并发波动，不会因单次低利用率重置
    2. 区分并发限制和 RPM 限制
    3. 探测性扩容避免长期卡在低限制
    4. 记录调整历史
    """

    # 默认配置 - 使用统一常量
    DEFAULT_INITIAL_LIMIT = ConcurrencyDefaults.INITIAL_LIMIT
    MIN_CONCURRENT_LIMIT = ConcurrencyDefaults.MIN_CONCURRENT_LIMIT
    MAX_CONCURRENT_LIMIT = ConcurrencyDefaults.MAX_CONCURRENT_LIMIT

    # AIMD 参数
    INCREASE_STEP = ConcurrencyDefaults.INCREASE_STEP
    DECREASE_MULTIPLIER = ConcurrencyDefaults.DECREASE_MULTIPLIER

    # 滑动窗口参数
    UTILIZATION_WINDOW_SIZE = ConcurrencyDefaults.UTILIZATION_WINDOW_SIZE
    UTILIZATION_WINDOW_SECONDS = ConcurrencyDefaults.UTILIZATION_WINDOW_SECONDS
    UTILIZATION_THRESHOLD = ConcurrencyDefaults.UTILIZATION_THRESHOLD
    HIGH_UTILIZATION_RATIO = ConcurrencyDefaults.HIGH_UTILIZATION_RATIO
    MIN_SAMPLES_FOR_DECISION = ConcurrencyDefaults.MIN_SAMPLES_FOR_DECISION

    # 探测性扩容参数
    PROBE_INCREASE_INTERVAL_MINUTES = ConcurrencyDefaults.PROBE_INCREASE_INTERVAL_MINUTES
    PROBE_INCREASE_MIN_REQUESTS = ConcurrencyDefaults.PROBE_INCREASE_MIN_REQUESTS

    # 记录历史数量
    MAX_HISTORY_RECORDS = 20

    def __init__(self, strategy: str = AdaptiveStrategy.AIMD):
        """
        初始化自适应并发管理器

        Args:
            strategy: 调整策略
        """
        self.strategy = strategy

    def handle_429_error(
        self,
        db: Session,
        key: ProviderAPIKey,
        rate_limit_info: RateLimitInfo,
        current_concurrent: Optional[int] = None,
    ) -> int:
        """
        处理429错误，调整并发限制

        Args:
            db: 数据库会话
            key: API Key对象
            rate_limit_info: 速率限制信息
            current_concurrent: 当前并发数

        Returns:
            调整后的并发限制
        """
        # max_concurrent=NULL 表示启用自适应，max_concurrent=数字 表示固定限制
        is_adaptive = key.max_concurrent is None

        if not is_adaptive:
            logger.debug(
                f"Key {key.id} 设置了固定并发限制 ({key.max_concurrent})，跳过自适应调整"
            )
            return int(key.max_concurrent)  # type: ignore[arg-type]

        # 更新429统计
        key.last_429_at = datetime.now(timezone.utc)  # type: ignore[assignment]
        key.last_429_type = rate_limit_info.limit_type  # type: ignore[assignment]
        key.last_concurrent_peak = current_concurrent  # type: ignore[assignment]

        # 遇到 429 错误，清空利用率采样窗口（重新开始收集）
        key.utilization_samples = []  # type: ignore[assignment]

        if rate_limit_info.limit_type == RateLimitType.CONCURRENT:
            # 并发限制：减少并发数
            key.concurrent_429_count = int(key.concurrent_429_count or 0) + 1  # type: ignore[assignment]

            # 获取当前有效限制（自适应模式使用 learned_max_concurrent）
            old_limit = int(key.learned_max_concurrent or self.DEFAULT_INITIAL_LIMIT)
            new_limit = self._decrease_limit(old_limit, current_concurrent)

            logger.warning(
                f"[CONCURRENT] 并发限制触发: Key {key.id[:8]}... | "
                f"当前并发: {current_concurrent} | "
                f"调整: {old_limit} -> {new_limit}"
            )

            # 记录调整历史
            self._record_adjustment(
                key,
                old_limit=old_limit,
                new_limit=new_limit,
                reason="concurrent_429",
                current_concurrent=current_concurrent,
            )

            # 更新学习到的并发限制
            key.learned_max_concurrent = new_limit  # type: ignore[assignment]

        elif rate_limit_info.limit_type == RateLimitType.RPM:
            # RPM限制：不调整并发，只记录
            key.rpm_429_count = int(key.rpm_429_count or 0) + 1  # type: ignore[assignment]

            logger.info(
                f"[RPM] RPM限制触发: Key {key.id[:8]}... | "
                f"retry_after: {rate_limit_info.retry_after}s | "
                f"不调整并发限制"
            )

        else:
            # 未知类型：保守处理，轻微减少
            logger.warning(
                f"[UNKNOWN] 未知429类型: Key {key.id[:8]}... | "
                f"当前并发: {current_concurrent} | "
                f"保守减少并发"
            )

            old_limit = int(key.learned_max_concurrent or self.DEFAULT_INITIAL_LIMIT)
            new_limit = max(int(old_limit * 0.9), self.MIN_CONCURRENT_LIMIT)  # 减少10%

            self._record_adjustment(
                key,
                old_limit=old_limit,
                new_limit=new_limit,
                reason="unknown_429",
                current_concurrent=current_concurrent,
            )

            key.learned_max_concurrent = new_limit  # type: ignore[assignment]

        db.flush()
        get_batch_committer().mark_dirty(db)

        return int(key.learned_max_concurrent or self.DEFAULT_INITIAL_LIMIT)

    def handle_success(
        self,
        db: Session,
        key: ProviderAPIKey,
        current_concurrent: int,
    ) -> Optional[int]:
        """
        处理成功请求，基于滑动窗口利用率考虑增加并发限制

        Args:
            db: 数据库会话
            key: API Key对象
            current_concurrent: 当前并发数（必需，用于计算利用率）

        Returns:
            调整后的并发限制（如果有调整），否则返回 None
        """
        # max_concurrent=NULL 表示启用自适应
        is_adaptive = key.max_concurrent is None

        if not is_adaptive:
            return None

        current_limit = int(key.learned_max_concurrent or self.DEFAULT_INITIAL_LIMIT)

        # 计算当前利用率
        utilization = float(current_concurrent / current_limit) if current_limit > 0 else 0.0

        now = datetime.now(timezone.utc)
        now_ts = now.timestamp()

        # 更新滑动窗口
        samples = self._update_utilization_window(key, now_ts, utilization)

        # 检查是否满足扩容条件
        increase_reason = self._check_increase_conditions(key, samples, now)

        if increase_reason and current_limit < self.MAX_CONCURRENT_LIMIT:
            old_limit = current_limit
            new_limit = self._increase_limit(current_limit)

            # 计算窗口统计用于日志
            avg_util = sum(s["util"] for s in samples) / len(samples) if samples else 0
            high_util_count = sum(1 for s in samples if s["util"] >= self.UTILIZATION_THRESHOLD)
            high_util_ratio = high_util_count / len(samples) if samples else 0

            logger.info(
                f"[INCREASE] {increase_reason}: Key {key.id[:8]}... | "
                f"窗口采样: {len(samples)} | "
                f"平均利用率: {avg_util:.1%} | "
                f"高利用率比例: {high_util_ratio:.1%} | "
                f"调整: {old_limit} -> {new_limit}"
            )

            # 记录调整历史
            self._record_adjustment(
                key,
                old_limit=old_limit,
                new_limit=new_limit,
                reason=increase_reason,
                avg_utilization=round(avg_util, 2),
                high_util_ratio=round(high_util_ratio, 2),
                sample_count=len(samples),
                current_concurrent=current_concurrent,
            )

            # 更新限制
            key.learned_max_concurrent = new_limit  # type: ignore[assignment]

            # 如果是探测性扩容，更新探测时间
            if increase_reason == "probe_increase":
                key.last_probe_increase_at = now  # type: ignore[assignment]

            # 扩容后清空采样窗口，重新开始收集
            key.utilization_samples = []  # type: ignore[assignment]

            db.flush()
            get_batch_committer().mark_dirty(db)

            return new_limit

        # 定期持久化采样数据（每5个采样保存一次）
        if len(samples) % 5 == 0:
            db.flush()
            get_batch_committer().mark_dirty(db)

        return None

    def _update_utilization_window(
        self, key: ProviderAPIKey, now_ts: float, utilization: float
    ) -> List[Dict[str, Any]]:
        """
        更新利用率滑动窗口

        Args:
            key: API Key对象
            now_ts: 当前时间戳
            utilization: 当前利用率

        Returns:
            更新后的采样列表
        """
        samples: List[Dict[str, Any]] = list(key.utilization_samples or [])

        # 添加新采样
        samples.append({"ts": now_ts, "util": round(utilization, 3)})

        # 移除过期采样（超过时间窗口）
        cutoff_ts = now_ts - self.UTILIZATION_WINDOW_SECONDS
        samples = [s for s in samples if s["ts"] > cutoff_ts]

        # 限制采样数量
        if len(samples) > self.UTILIZATION_WINDOW_SIZE:
            samples = samples[-self.UTILIZATION_WINDOW_SIZE:]

        # 更新到 key 对象
        key.utilization_samples = samples  # type: ignore[assignment]

        return samples

    def _check_increase_conditions(
        self, key: ProviderAPIKey, samples: List[Dict[str, Any]], now: datetime
    ) -> Optional[str]:
        """
        检查是否满足扩容条件

        Args:
            key: API Key对象
            samples: 利用率采样列表
            now: 当前时间

        Returns:
            扩容原因（如果满足条件），否则返回 None
        """
        # 检查是否在冷却期
        if self._is_in_cooldown(key):
            return None

        # 条件1：滑动窗口扩容
        if len(samples) >= self.MIN_SAMPLES_FOR_DECISION:
            high_util_count = sum(1 for s in samples if s["util"] >= self.UTILIZATION_THRESHOLD)
            high_util_ratio = high_util_count / len(samples)

            if high_util_ratio >= self.HIGH_UTILIZATION_RATIO:
                return "high_utilization"

        # 条件2：探测性扩容（长时间无 429 且有流量）
        if self._should_probe_increase(key, samples, now):
            return "probe_increase"

        return None

    def _should_probe_increase(
        self, key: ProviderAPIKey, samples: List[Dict[str, Any]], now: datetime
    ) -> bool:
        """
        检查是否应该进行探测性扩容

        条件：
        1. 距上次 429 超过 PROBE_INCREASE_INTERVAL_MINUTES 分钟
        2. 距上次探测性扩容超过 PROBE_INCREASE_INTERVAL_MINUTES 分钟
        3. 期间有足够的请求量（采样数 >= PROBE_INCREASE_MIN_REQUESTS）
        4. 平均利用率 > 30%（说明确实有使用需求）

        Args:
            key: API Key对象
            samples: 利用率采样列表
            now: 当前时间

        Returns:
            是否应该探测性扩容
        """
        probe_interval_seconds = self.PROBE_INCREASE_INTERVAL_MINUTES * 60

        # 检查距上次 429 的时间
        if key.last_429_at:
            last_429_at = cast(datetime, key.last_429_at)
            time_since_429 = (now - last_429_at).total_seconds()
            if time_since_429 < probe_interval_seconds:
                return False

        # 检查距上次探测性扩容的时间
        if key.last_probe_increase_at:
            last_probe = cast(datetime, key.last_probe_increase_at)
            time_since_probe = (now - last_probe).total_seconds()
            if time_since_probe < probe_interval_seconds:
                return False

        # 检查请求量
        if len(samples) < self.PROBE_INCREASE_MIN_REQUESTS:
            return False

        # 检查平均利用率（确保确实有使用需求）
        avg_util = sum(s["util"] for s in samples) / len(samples)
        if avg_util < 0.3:  # 至少 30% 利用率
            return False

        return True

    def _is_in_cooldown(self, key: ProviderAPIKey) -> bool:
        """
        检查是否在 429 错误后的冷却期内

        Args:
            key: API Key对象

        Returns:
            True 如果在冷却期内，否则 False
        """
        if key.last_429_at is None:
            return False

        last_429_at = cast(datetime, key.last_429_at)
        time_since_429 = (datetime.now(timezone.utc) - last_429_at).total_seconds()
        cooldown_seconds = ConcurrencyDefaults.COOLDOWN_AFTER_429_MINUTES * 60

        return bool(time_since_429 < cooldown_seconds)

    def _decrease_limit(
        self,
        current_limit: int,
        current_concurrent: Optional[int] = None,
    ) -> int:
        """
        减少并发限制

        策略：
        - 如果知道当前并发数，设置为当前并发的70%
        - 否则，使用乘性减少
        """
        if current_concurrent:
            # 基于当前并发数减少
            new_limit = max(
                int(current_concurrent * self.DECREASE_MULTIPLIER), self.MIN_CONCURRENT_LIMIT
            )
        else:
            # 乘性减少
            new_limit = max(
                int(current_limit * self.DECREASE_MULTIPLIER), self.MIN_CONCURRENT_LIMIT
            )

        return new_limit

    def _increase_limit(self, current_limit: int) -> int:
        """
        增加并发限制

        策略：加性增加 (+1)
        """
        new_limit = min(current_limit + self.INCREASE_STEP, self.MAX_CONCURRENT_LIMIT)
        return new_limit

    def _record_adjustment(
        self,
        key: ProviderAPIKey,
        old_limit: int,
        new_limit: int,
        reason: str,
        **extra_data: Any,
    ) -> None:
        """
        记录并发调整历史

        Args:
            key: API Key对象
            old_limit: 原限制
            new_limit: 新限制
            reason: 调整原因
            **extra_data: 额外数据
        """
        history: List[Dict[str, Any]] = list(key.adjustment_history or [])

        record = {
            "timestamp": datetime.now(timezone.utc).isoformat(),
            "old_limit": old_limit,
            "new_limit": new_limit,
            "reason": reason,
            **extra_data,
        }
        history.append(record)

        # 保留最近N条记录
        if len(history) > self.MAX_HISTORY_RECORDS:
            history = history[-self.MAX_HISTORY_RECORDS:]

        key.adjustment_history = history  # type: ignore[assignment]

    def get_adjustment_stats(self, key: ProviderAPIKey) -> Dict[str, Any]:
        """
        获取调整统计信息

        Args:
            key: API Key对象

        Returns:
            统计信息
        """
        history: List[Dict[str, Any]] = list(key.adjustment_history or [])
        samples: List[Dict[str, Any]] = list(key.utilization_samples or [])

        # max_concurrent=NULL 表示自适应，否则为固定限制
        is_adaptive = key.max_concurrent is None
        current_limit = int(key.learned_max_concurrent or self.DEFAULT_INITIAL_LIMIT)
        effective_limit = current_limit if is_adaptive else int(key.max_concurrent)  # type: ignore

        # 计算窗口统计
        avg_utilization: Optional[float] = None
        high_util_ratio: Optional[float] = None
        if samples:
            avg_utilization = sum(s["util"] for s in samples) / len(samples)
            high_util_count = sum(1 for s in samples if s["util"] >= self.UTILIZATION_THRESHOLD)
            high_util_ratio = high_util_count / len(samples)

        last_429_at_str: Optional[str] = None
        if key.last_429_at:
            last_429_at_str = cast(datetime, key.last_429_at).isoformat()

        last_probe_at_str: Optional[str] = None
        if key.last_probe_increase_at:
            last_probe_at_str = cast(datetime, key.last_probe_increase_at).isoformat()

        return {
            "adaptive_mode": is_adaptive,
            "max_concurrent": key.max_concurrent,  # NULL=自适应，数字=固定限制
            "effective_limit": effective_limit,  # 当前有效限制
            "learned_limit": key.learned_max_concurrent,  # 学习到的限制
            "concurrent_429_count": int(key.concurrent_429_count or 0),
            "rpm_429_count": int(key.rpm_429_count or 0),
            "last_429_at": last_429_at_str,
            "last_429_type": key.last_429_type,
            "adjustment_count": len(history),
            "recent_adjustments": history[-5:] if history else [],
            # 滑动窗口相关
            "window_sample_count": len(samples),
            "window_avg_utilization": round(avg_utilization, 3) if avg_utilization else None,
            "window_high_util_ratio": round(high_util_ratio, 3) if high_util_ratio else None,
            "utilization_threshold": self.UTILIZATION_THRESHOLD,
            "high_util_ratio_threshold": self.HIGH_UTILIZATION_RATIO,
            "min_samples_for_decision": self.MIN_SAMPLES_FOR_DECISION,
            # 探测性扩容相关
            "last_probe_increase_at": last_probe_at_str,
            "probe_increase_interval_minutes": self.PROBE_INCREASE_INTERVAL_MINUTES,
        }

    def reset_learning(self, db: Session, key: ProviderAPIKey) -> None:
        """
        重置学习状态（管理员功能）

        Args:
            db: 数据库会话
            key: API Key对象
        """
        logger.info(f"[RESET] 重置学习状态: Key {key.id[:8]}...")

        key.learned_max_concurrent = None  # type: ignore[assignment]
        key.concurrent_429_count = 0  # type: ignore[assignment]
        key.rpm_429_count = 0  # type: ignore[assignment]
        key.last_429_at = None  # type: ignore[assignment]
        key.last_429_type = None  # type: ignore[assignment]
        key.last_concurrent_peak = None  # type: ignore[assignment]
        key.adjustment_history = []  # type: ignore[assignment]
        key.utilization_samples = []  # type: ignore[assignment]
        key.last_probe_increase_at = None  # type: ignore[assignment]

        db.flush()
        get_batch_committer().mark_dirty(db)


# 全局单例
_adaptive_manager: Optional[AdaptiveConcurrencyManager] = None


def get_adaptive_manager() -> AdaptiveConcurrencyManager:
    """获取全局自适应管理器单例"""
    global _adaptive_manager
    if _adaptive_manager is None:
        _adaptive_manager = AdaptiveConcurrencyManager()
    return _adaptive_manager
