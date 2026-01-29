"""
统一定时任务调度器

使用 APScheduler 管理所有定时任务，支持时区配置。
所有定时任务使用应用时区（APP_TIMEZONE）配置执行时间，
数据存储仍然使用 UTC。
"""

from __future__ import annotations

import os
from datetime import datetime

from apscheduler.schedulers.asyncio import AsyncIOScheduler
from apscheduler.triggers.cron import CronTrigger
from apscheduler.triggers.interval import IntervalTrigger

from src.core.logger import logger

# 应用时区配置，默认为 Asia/Shanghai
APP_TIMEZONE = os.getenv("APP_TIMEZONE", "Asia/Shanghai")


class TaskScheduler:
    """统一定时任务调度器"""

    _instance: TaskScheduler | None = None

    def __init__(self):
        self.scheduler = AsyncIOScheduler(timezone=APP_TIMEZONE)
        self._started = False

    @classmethod
    def get_instance(cls) -> TaskScheduler:
        """获取调度器单例"""
        if cls._instance is None:
            cls._instance = TaskScheduler()
        return cls._instance

    def add_cron_job(
        self,
        func,
        hour: int,
        minute: int = 0,
        job_id: str = None,
        name: str = None,
        **kwargs,
    ):
        """
        添加 cron 定时任务

        Args:
            func: 要执行的函数
            hour: 执行时间（小时），使用业务时区
            minute: 执行时间（分钟）
            job_id: 任务ID
            name: 任务名称（用于日志）
            **kwargs: 传递给任务函数的参数
        """
        trigger = CronTrigger(hour=hour, minute=minute, timezone=APP_TIMEZONE)

        job_id = job_id or func.__name__
        display_name = name or job_id

        self.scheduler.add_job(
            func,
            trigger,
            id=job_id,
            name=display_name,
            replace_existing=True,
            kwargs=kwargs,
        )

        logger.info(
            f"已注册定时任务: {display_name}, "
            f"执行时间: {hour:02d}:{minute:02d} ({APP_TIMEZONE})"
        )

    def add_interval_job(
        self,
        func,
        seconds: int = None,
        minutes: int = None,
        hours: int = None,
        job_id: str = None,
        name: str = None,
        **kwargs,
    ):
        """
        添加间隔执行任务

        Args:
            func: 要执行的函数
            seconds: 间隔秒数
            minutes: 间隔分钟数
            hours: 间隔小时数
            job_id: 任务ID
            name: 任务名称
            **kwargs: 传递给任务函数的参数
        """
        # 构建 trigger 参数，过滤掉 None 值
        trigger_kwargs = {}
        if seconds is not None:
            trigger_kwargs["seconds"] = seconds
        if minutes is not None:
            trigger_kwargs["minutes"] = minutes
        if hours is not None:
            trigger_kwargs["hours"] = hours

        trigger = IntervalTrigger(**trigger_kwargs)

        job_id = job_id or func.__name__
        display_name = name or job_id

        # 计算间隔描述
        interval_parts = []
        if hours:
            interval_parts.append(f"{hours}小时")
        if minutes:
            interval_parts.append(f"{minutes}分钟")
        if seconds:
            interval_parts.append(f"{seconds}秒")
        interval_desc = "".join(interval_parts) or "未知间隔"

        self.scheduler.add_job(
            func,
            trigger,
            id=job_id,
            name=display_name,
            replace_existing=True,
            kwargs=kwargs,
        )

        logger.info(f"已注册间隔任务: {display_name}, 执行间隔: {interval_desc}")

    def start(self):
        """启动调度器"""
        if self._started:
            logger.warning("调度器已在运行中")
            return

        self.scheduler.start()
        self._started = True
        logger.info(f"定时任务调度器已启动，应用时区: {APP_TIMEZONE}")

        # 打印下次执行时间
        self._log_next_run_times()

    def stop(self):
        """停止调度器"""
        if not self._started:
            return

        self.scheduler.shutdown(wait=False)
        self._started = False
        logger.info("定时任务调度器已停止")

    def _log_next_run_times(self):
        """记录所有任务的下次执行时间"""
        jobs = self.scheduler.get_jobs()
        if not jobs:
            return

        logger.info("已注册的定时任务:")
        for job in jobs:
            next_run = job.next_run_time
            if next_run:
                # 计算距离下次执行的时间
                now = datetime.now(next_run.tzinfo)
                delta = next_run - now
                hours, remainder = divmod(int(delta.total_seconds()), 3600)
                minutes = remainder // 60

                logger.info(
                    f"  - {job.name}: 下次执行 {next_run.strftime('%Y-%m-%d %H:%M')} "
                    f"({hours}小时{minutes}分钟后)"
                )

    @property
    def is_running(self) -> bool:
        """调度器是否在运行"""
        return self._started


# 便捷函数
def get_scheduler() -> TaskScheduler:
    """获取调度器单例"""
    return TaskScheduler.get_instance()
