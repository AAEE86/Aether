"""
数据库事务管理工具
提供事务装饰器和事务上下文管理器
支持同步和异步函数
"""

import functools
import inspect
from contextlib import contextmanager
from typing import Any

from collections.abc import Callable
from collections.abc import Generator

from sqlalchemy.exc import DatabaseError, IntegrityError
from sqlalchemy.orm import Session

from src.core.logger import logger



class TransactionError(Exception):
    """事务处理异常"""

    pass


def _find_db_session(args, kwargs) -> Session | None:
    """从参数中查找数据库会话"""
    # 从位置参数中查找Session
    for arg in args:
        if isinstance(arg, Session):
            return arg

    # 从关键字参数中查找Session
    for value in kwargs.values():
        if isinstance(value, Session):
            return value

    return None


def transactional(commit: bool = True, rollback_on_error: bool = True):
    """
    事务装饰器，支持同步和异步函数

    Args:
        commit: 是否在成功时自动提交，默认True
        rollback_on_error: 是否在错误时自动回滚，默认True

    Usage:
        @transactional()
        def create_user_with_api_key(db: Session, ...):
            # 同步方法会在事务中执行
            pass

        @transactional()
        async def create_user_async(db: Session, ...):
            # 异步方法也会在事务中执行
            pass
    """

    def decorator(func: Callable) -> Callable:
        # 检查是否是异步函数
        if inspect.iscoroutinefunction(func):
            @functools.wraps(func)
            async def async_wrapper(*args, **kwargs) -> Any:
                db_session = _find_db_session(args, kwargs)

                if not db_session:
                    raise TransactionError(
                        f"No SQLAlchemy Session found in arguments for {func.__name__}"
                    )

                # 检查是否已经在事务中
                if db_session.in_transaction():
                    return await func(*args, **kwargs)

                transaction_id = f"{func.__module__}.{func.__name__}"
                logger.debug(f"开始异步事务: {transaction_id}")

                try:
                    result = await func(*args, **kwargs)

                    if commit:
                        db_session.commit()
                        logger.debug(f"异步事务提交成功: {transaction_id}")

                    return result

                except Exception as e:
                    if rollback_on_error:
                        try:
                            db_session.rollback()
                        except Exception:
                            pass
                        logger.error(
                            f"异步事务回滚: {transaction_id} - {type(e).__name__}: {str(e)}"
                        )
                    else:
                        logger.error(
                            f"异步事务异常（未回滚）: {transaction_id} - {type(e).__name__}: {str(e)}"
                        )
                    raise

            return async_wrapper
        else:
            @functools.wraps(func)
            def sync_wrapper(*args, **kwargs) -> Any:
                db_session = _find_db_session(args, kwargs)

                if not db_session:
                    raise TransactionError(
                        f"No SQLAlchemy Session found in arguments for {func.__name__}"
                    )

                # 检查是否已经在事务中
                if db_session.in_transaction():
                    return func(*args, **kwargs)

                transaction_id = f"{func.__module__}.{func.__name__}"
                logger.debug(f"开始事务: {transaction_id}")

                try:
                    result = func(*args, **kwargs)

                    if commit:
                        db_session.commit()
                        logger.debug(f"事务提交成功: {transaction_id}")

                    return result

                except Exception as e:
                    if rollback_on_error:
                        try:
                            db_session.rollback()
                        except Exception:
                            pass
                        logger.error(
                            f"事务回滚: {transaction_id} - {type(e).__name__}: {str(e)}"
                        )
                    else:
                        logger.error(
                            f"事务异常（未回滚）: {transaction_id} - {type(e).__name__}: {str(e)}"
                        )
                    raise

            return sync_wrapper

    return decorator


@contextmanager
def transaction_scope(
    db: Session,
    commit_on_success: bool = True,
    rollback_on_error: bool = True,
    operation_name: str | None = None,
) -> Generator[Session]:
    """
    事务上下文管理器

    Args:
        db: 数据库会话
        commit_on_success: 成功时是否自动提交
        rollback_on_error: 失败时是否自动回滚
        operation_name: 操作名称，用于日志

    Usage:
        with transaction_scope(db, operation_name="create_user") as tx:
            user = User(...)
            tx.add(user)
            # 自动提交或回滚
    """
    operation_name = operation_name or "database_operation"

    # 检查是否已经在事务中
    if db.in_transaction():
        # 已经在事务中，直接返回session
        logger.debug(f"使用现有事务: {operation_name}")
        yield db
        return

    logger.debug(f"开始事务范围: {operation_name}")

    try:
        yield db

        if commit_on_success:
            db.commit()
            logger.debug(f"事务范围提交成功: {operation_name}")

    except Exception as e:
        if rollback_on_error:
            db.rollback()
            logger.error(f"事务范围回滚: {operation_name} - {type(e).__name__}: {str(e)}")
        raise


def retry_on_database_error(max_retries: int = 3, delay: float = 0.1):
    """
    数据库错误重试装饰器

    Args:
        max_retries: 最大重试次数
        delay: 重试延迟（秒）
    """

    def decorator(func: Callable) -> Callable:
        @functools.wraps(func)
        def wrapper(*args, **kwargs) -> Any:
            import random
            import time

            for attempt in range(max_retries):
                try:
                    return func(*args, **kwargs)

                except (DatabaseError, IntegrityError) as e:
                    if attempt < max_retries - 1:
                        # 随机化延迟，避免多个请求同时重试
                        actual_delay = delay * (2**attempt) + random.uniform(0, 0.1)
                        logger.warning(f"数据库操作失败，{actual_delay:.2f}秒后重试 (尝试 {attempt + 1}/{max_retries}): {str(e)}")
                        time.sleep(actual_delay)
                        continue
                    else:
                        logger.error(
                            f"数据库操作最终失败，已达最大重试次数({max_retries}): {func.__name__} - {str(e)}"
                        )
                        raise

        return wrapper

    return decorator


class BatchOperation:
    """
    批量操作管理器
    用于处理大量数据插入/更新操作
    """

    def __init__(self, db: Session, batch_size: int = 100):
        self.db = db
        self.batch_size = batch_size
        self.operations = []
        self.operation_count = 0

    def add(self, obj):
        """添加对象到批处理"""
        self.operations.append(("add", obj))
        self.operation_count += 1

        if self.operation_count >= self.batch_size:
            self.flush()

    def update(self, obj):
        """添加更新操作到批处理"""
        self.operations.append(("merge", obj))
        self.operation_count += 1

        if self.operation_count >= self.batch_size:
            self.flush()

    def flush(self):
        """执行当前批次的所有操作"""
        if not self.operations:
            return

        logger.debug(f"执行批量操作: {len(self.operations)} 项")

        try:
            for operation, obj in self.operations:
                if operation == "add":
                    self.db.add(obj)
                elif operation == "merge":
                    self.db.merge(obj)

            self.db.flush()  # 只flush，不提交
            logger.debug(f"批量操作flush完成: {len(self.operations)} 项")

        except Exception as e:
            logger.error(f"批量操作失败({len(self.operations)}项): {type(e).__name__}: {str(e)}")
            raise

        finally:
            # 清空操作列表
            self.operations.clear()
            self.operation_count = 0

    def commit(self):
        """提交所有操作"""
        self.flush()  # 确保所有操作都已flush
        self.db.commit()
        logger.debug("批量操作提交完成")

    def __enter__(self):
        return self

    def __exit__(self, exc_type, exc_val, exc_tb):
        if exc_type is None:
            # 正常退出，提交事务
            self.commit()
        else:
            # 异常退出，回滚事务
            self.db.rollback()
            logger.error(f"批量操作异常退出，已回滚: {str(exc_val)}")
