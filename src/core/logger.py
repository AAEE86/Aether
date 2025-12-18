"""
统一日志系统 - 基于 loguru

日志级别策略:
- DEBUG: 开发调试，详细执行流程、变量值、缓存操作
- INFO:  生产环境，关键业务操作、状态变更、请求处理
- WARNING: 潜在问题、降级处理、资源警告
- ERROR: 异常错误、需要关注的故障

输出策略:
- 控制台: 开发环境=DEBUG, 生产环境=INFO (通过 LOG_LEVEL 控制)
- 文件: 始终保存 DEBUG 级别，保留30天，按大小轮转 (100MB)

使用方式:
    from src.core.logger import logger

    logger.info("消息")
    logger.debug("调试信息")
    logger.warning("警告")
    logger.error("错误")
    logger.exception("异常，带堆栈")
"""

import logging
import os
import sys
from pathlib import Path

from loguru import logger

# ============================================================================
# 环境检测
# ============================================================================

IS_DOCKER = (
    os.path.exists("/.dockerenv")
    or os.environ.get("DOCKER_CONTAINER", "false").lower() == "true"
)

# 日志级别: 默认开发环境 DEBUG, 生产环境 INFO
LOG_LEVEL = os.getenv("LOG_LEVEL", "DEBUG" if not IS_DOCKER else "INFO").upper()

# 是否禁用文件日志 (用于测试或特殊场景)
DISABLE_FILE_LOG = os.getenv("LOG_DISABLE_FILE", "false").lower() == "true"

# 项目根目录
PROJECT_ROOT = Path(__file__).resolve().parent.parent.parent

# ============================================================================
# 日志格式定义
# ============================================================================

CONSOLE_FORMAT_DEV = (
    "<green>{time:HH:mm:ss}</green> | "
    "<level>{level: <8}</level> | "
    "<cyan>{message}</cyan>"
)

CONSOLE_FORMAT_PROD = "{time:YYYY-MM-DD HH:mm:ss} | {level: <8} | {message}"

FILE_FORMAT = "{time:YYYY-MM-DD HH:mm:ss.SSS} | {level: <8} | {name}:{function}:{line} | {message}"

# ============================================================================
# 日志配置
# ============================================================================

logger.remove()


def _log_filter(record: dict) -> bool:  # type: ignore[type-arg]
    return "watchfiles" not in record["name"]


if IS_DOCKER:
    # 生产环境：禁用 backtrace 和 diagnose，减少日志噪音
    logger.add(
        sys.stdout,
        format=CONSOLE_FORMAT_PROD,
        level=LOG_LEVEL,
        filter=_log_filter,  # type: ignore[arg-type]
        colorize=False,
        backtrace=False,
        diagnose=False,
    )
else:
    logger.add(
        sys.stdout,
        format=CONSOLE_FORMAT_DEV,
        level=LOG_LEVEL,
        filter=_log_filter,  # type: ignore[arg-type]
        colorize=True,
    )

if not DISABLE_FILE_LOG:
    log_dir = PROJECT_ROOT / "logs"
    log_dir.mkdir(exist_ok=True)

    # 文件日志通用配置
    file_log_config = {
        "format": FILE_FORMAT,
        "filter": _log_filter,
        "rotation": "100 MB",
        "retention": "30 days",
        "compression": "gz",
        "enqueue": True,
        "encoding": "utf-8",
        "catch": True,
    }

    # 生产环境禁用详细堆栈
    if IS_DOCKER:
        file_log_config["backtrace"] = False
        file_log_config["diagnose"] = False

    # 主日志文件 - 所有级别
    logger.add(
        log_dir / "app.log",
        level="DEBUG",
        **file_log_config,  # type: ignore[arg-type]
    )

    # 错误日志文件 - 仅 ERROR 及以上
    error_log_config = file_log_config.copy()
    error_log_config["rotation"] = "50 MB"
    logger.add(
        log_dir / "error.log",
        level="ERROR",
        **error_log_config,  # type: ignore[arg-type]
    )

# ============================================================================
# 禁用第三方库噪音日志
# ============================================================================

logging.getLogger("watchfiles").setLevel(logging.ERROR)
logging.getLogger("watchfiles.main").setLevel(logging.ERROR)
logging.getLogger("httpx").setLevel(logging.WARNING)
logging.getLogger("httpcore").setLevel(logging.WARNING)
logging.getLogger("uvicorn.access").setLevel(logging.WARNING)

# ============================================================================
# 导出
# ============================================================================

__all__ = ["logger"]
