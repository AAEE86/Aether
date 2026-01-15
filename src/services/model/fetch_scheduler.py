"""
模型自动获取调度器

定时从上游 API 获取可用模型列表，并更新 ProviderAPIKey 的 allowed_models。

功能:
- 扫描所有启用了 auto_fetch_models 的 ProviderAPIKey
- 调用 Adapter.fetch_models() 获取模型列表
- 更新 Key 的 allowed_models（保留 locked_models 中的模型）
- 记录获取结果和错误信息
"""

import asyncio
import os
from datetime import datetime, timezone
from typing import Optional

import httpx
from sqlalchemy.orm import Session, joinedload

from src.core.crypto import crypto_service
from src.core.logger import logger
from src.database import create_session
from src.models.database import Provider, ProviderAPIKey, ProviderEndpoint
from src.services.system.scheduler import get_scheduler

# 从环境变量读取间隔，默认 1440 分钟（1 天），限制在 60-10080 分钟之间
_interval_env = int(os.getenv("MODEL_FETCH_INTERVAL_MINUTES", "1440"))
MODEL_FETCH_INTERVAL_MINUTES = max(60, min(10080, _interval_env))

# 并发请求限制
MAX_CONCURRENT_REQUESTS = 5

# 单个 Key 处理的超时时间（秒）
KEY_FETCH_TIMEOUT_SECONDS = 120


def _get_adapter_for_format(api_format: str) -> Optional[type]:
    """根据 API 格式获取对应的 Adapter 类"""
    # 延迟导入避免循环依赖
    from src.api.handlers.base.chat_adapter_base import get_adapter_class
    from src.api.handlers.base.cli_adapter_base import get_cli_adapter_class

    adapter_class = get_adapter_class(api_format)
    if adapter_class:
        return adapter_class
    cli_adapter_class = get_cli_adapter_class(api_format)
    if cli_adapter_class:
        return cli_adapter_class
    return None


class ModelFetchScheduler:
    """模型自动获取调度器"""

    def __init__(self) -> None:
        self._running = False
        self._lock = asyncio.Lock()
        self._startup_task: Optional[asyncio.Task] = None

    async def start(self) -> None:
        """启动调度器"""
        if self._running:
            logger.warning("ModelFetchScheduler already running")
            return

        self._running = True
        logger.info(f"模型自动获取调度器已启动，间隔: {MODEL_FETCH_INTERVAL_MINUTES} 分钟")

        scheduler = get_scheduler()
        scheduler.add_interval_job(
            self._scheduled_fetch_models,
            minutes=MODEL_FETCH_INTERVAL_MINUTES,
            job_id="model_auto_fetch",
            name="自动获取模型",
        )

        # 启动时延迟执行一次，保存任务引用
        self._startup_task = asyncio.create_task(self._run_startup_task())

    async def stop(self) -> None:
        """停止调度器"""
        self._running = False

        # 取消并等待启动任务完成
        if self._startup_task and not self._startup_task.done():
            self._startup_task.cancel()
            try:
                await self._startup_task
            except asyncio.CancelledError:
                pass

        logger.info("模型自动获取调度器已停止")

    async def _run_startup_task(self) -> None:
        """启动时执行的初始化任务"""
        try:
            await asyncio.sleep(10)  # 等待系统完全启动
            if not self._running:
                return
            logger.info("启动时执行首次模型获取...")
            await self._perform_fetch_all_keys()
        except asyncio.CancelledError:
            logger.debug("启动任务被取消")
            raise
        except Exception:
            logger.exception("启动时模型获取出错")

    async def _scheduled_fetch_models(self) -> None:
        """定时任务入口"""
        async with self._lock:
            await self._perform_fetch_all_keys()

    async def _perform_fetch_all_keys(self) -> None:
        """获取所有启用自动获取的 Key 并拉取模型"""
        logger.info("开始自动获取模型任务...")

        # 统计信息
        success_count = 0
        error_count = 0
        skip_count = 0

        with create_session() as db:
            # 查询所有启用了 auto_fetch_models 的 Key（只获取 ID 列表）
            key_ids = [
                row[0]
                for row in db.query(ProviderAPIKey.id)
                .filter(
                    ProviderAPIKey.auto_fetch_models == True,  # noqa: E712
                    ProviderAPIKey.is_active == True,  # noqa: E712
                )
                .all()
            ]

        if not key_ids:
            logger.debug("没有启用自动获取模型的 Key")
            return

        logger.info(f"找到 {len(key_ids)} 个启用自动获取模型的 Key")

        # 逐个处理每个 Key，每个 Key 使用独立的数据库会话
        for key_id in key_ids:
            if not self._running:
                logger.info("调度器已停止，中断模型获取任务")
                break

            try:
                # 添加超时保护
                result = await asyncio.wait_for(
                    self._fetch_models_for_key_by_id(key_id),
                    timeout=KEY_FETCH_TIMEOUT_SECONDS,
                )
                if result == "success":
                    success_count += 1
                elif result == "skip":
                    skip_count += 1
                else:
                    error_count += 1
            except asyncio.TimeoutError:
                logger.error(f"处理 Key {key_id} 超时（{KEY_FETCH_TIMEOUT_SECONDS}s）")
                self._update_key_error(key_id, f"Timeout after {KEY_FETCH_TIMEOUT_SECONDS}s")
                error_count += 1
            except Exception:
                logger.exception(f"处理 Key {key_id} 时出错")
                error_count += 1

        logger.info(
            f"自动获取模型任务完成: 成功={success_count}, 失败={error_count}, 跳过={skip_count}"
        )

    def _update_key_error(self, key_id: str, error_msg: str) -> None:
        """更新 Key 的错误信息（独立事务）"""
        try:
            with create_session() as db:
                key = db.query(ProviderAPIKey).filter(ProviderAPIKey.id == key_id).first()
                if key:
                    key.last_models_fetch_at = datetime.now(timezone.utc)
                    key.last_models_fetch_error = error_msg
                    db.commit()
        except Exception:
            logger.exception(f"更新 Key {key_id} 错误信息失败")

    async def _fetch_models_for_key_by_id(self, key_id: str) -> str:
        """根据 Key ID 获取模型并更新，返回结果状态"""
        with create_session() as db:
            key = (
                db.query(ProviderAPIKey)
                .options(joinedload(ProviderAPIKey.provider))
                .filter(ProviderAPIKey.id == key_id)
                .first()
            )

            if not key:
                logger.warning(f"Key {key_id} 不存在，跳过")
                return "skip"

            if not key.is_active or not key.auto_fetch_models:
                logger.debug(f"Key {key_id} 已禁用或关闭自动获取，跳过")
                return "skip"

            try:
                result = await self._fetch_models_for_key(db, key)
                db.commit()
                return result
            except Exception:
                db.rollback()
                raise

    async def _fetch_models_for_key(
        self,
        db: "Session",
        key: ProviderAPIKey,
    ) -> str:
        """为单个 Key 获取模型并更新 allowed_models，返回结果状态"""
        now = datetime.now(timezone.utc)
        provider_id = key.provider_id

        # 获取 Provider 和 Endpoints
        provider = (
            db.query(Provider)
            .options(joinedload(Provider.endpoints))
            .filter(Provider.id == provider_id)
            .first()
        )

        if not provider:
            logger.warning(f"Provider {provider_id} 不存在，跳过 Key {key.id}")
            key.last_models_fetch_error = "Provider not found"
            key.last_models_fetch_at = now
            return "error"

        # 解密 API Key
        if not key.api_key:
            logger.warning(f"Key {key.id} 没有 API Key，跳过")
            key.last_models_fetch_error = "No API key configured"
            key.last_models_fetch_at = now
            return "error"

        try:
            api_key_value = crypto_service.decrypt(key.api_key)
        except Exception:
            # 不记录异常详情，避免泄露密钥信息
            logger.error(f"解密 Key {key.id} 失败")
            key.last_models_fetch_error = "Decrypt error"
            key.last_models_fetch_at = now
            return "error"

        # 构建 api_format -> endpoint 映射
        format_to_endpoint: dict[str, ProviderEndpoint] = {}
        for endpoint in provider.endpoints:  # type: ignore[attr-defined]
            if endpoint.is_active:
                format_to_endpoint[endpoint.api_format] = endpoint

        if not format_to_endpoint:
            logger.warning(f"Provider {provider.name} 没有活跃的端点，跳过 Key {key.id}")
            key.last_models_fetch_error = "No active endpoints"
            key.last_models_fetch_at = now
            return "error"

        # 收集端点配置
        endpoint_configs: list[dict] = []
        key_formats = key.api_formats or []
        for fmt in key_formats:
            endpoint = format_to_endpoint.get(fmt)
            if endpoint:
                endpoint_configs.append(
                    {
                        "api_key": api_key_value,
                        "base_url": endpoint.base_url,
                        "api_format": fmt,
                        "extra_headers": endpoint.headers,
                    }
                )

        if not endpoint_configs:
            logger.warning(f"Provider {provider.name} 没有匹配 Key {key.id} 格式的端点配置")
            key.last_models_fetch_error = "No matching endpoints for key formats"
            key.last_models_fetch_at = now
            return "error"

        # 并发获取模型
        all_models, errors, has_success = await self._fetch_models_from_endpoints(endpoint_configs)

        # 记录获取结果
        error_msg = "; ".join(errors) if errors else None
        key.last_models_fetch_at = now
        key.last_models_fetch_error = error_msg

        # 如果没有任何成功的响应，不更新 allowed_models（保留旧数据）
        if not has_success:
            logger.warning(
                f"Provider {provider.name} Key {key.id} 所有端点获取失败，保留现有模型列表"
            )
            if not error_msg:
                key.last_models_fetch_error = "All endpoints failed"
            return "error"

        # 去重获取模型 ID 列表
        fetched_model_ids: set[str] = set()
        for model in all_models:
            model_id = model.get("id")
            if model_id:
                fetched_model_ids.add(model_id)

        logger.info(
            f"Provider {provider.name} Key {key.id} 获取到 {len(fetched_model_ids)} 个唯一模型"
        )

        # 更新 allowed_models（保留 locked_models）
        has_changed = self._update_key_allowed_models(key, fetched_model_ids)

        # 如果白名单有变化，触发缓存失效和自动关联检查
        if has_changed and provider_id:
            from src.services.model.global_model import on_key_allowed_models_changed

            on_key_allowed_models_changed(
                db=db,
                provider_id=provider_id,
                allowed_models=list(key.allowed_models or []),
            )

        return "success"

    def _update_key_allowed_models(self, key: ProviderAPIKey, fetched_model_ids: set[str]) -> bool:
        """
        更新 Key 的 allowed_models，保留 locked_models

        Returns:
            bool: 是否有变化
        """
        # 获取当前锁定的模型
        locked_models = set(key.locked_models or [])

        # 新的 allowed_models = 获取到的模型 + 锁定的模型
        # 锁定模型无论上游是否返回都会保留
        new_allowed_models = list(fetched_model_ids | locked_models)
        new_allowed_models.sort()  # 保持顺序稳定

        # 检查是否有变化
        current_allowed = set(key.allowed_models or [])
        new_allowed_set = set(new_allowed_models)

        if current_allowed != new_allowed_set:
            added = new_allowed_set - current_allowed
            removed = current_allowed - new_allowed_set
            if added:
                logger.info(f"Key {key.id} 新增模型: {sorted(added)}")
            if removed:
                logger.info(f"Key {key.id} 移除模型: {sorted(removed)}")

            key.allowed_models = new_allowed_models
            return True
        else:
            logger.debug(f"Key {key.id} 模型列表无变化")
            return False

    async def _fetch_models_from_endpoints(
        self, endpoint_configs: list[dict]
    ) -> tuple[list[dict], list[str], bool]:
        """从多个端点并发获取模型，返回 (模型列表, 错误列表, 是否有成功)"""
        all_models: list[dict] = []
        errors: list[str] = []
        has_success = False
        semaphore = asyncio.Semaphore(MAX_CONCURRENT_REQUESTS)

        async def fetch_one(
            client: httpx.AsyncClient, config: dict
        ) -> tuple[list, Optional[str], bool]:
            base_url = config["base_url"]
            if not base_url:
                return [], None, False
            base_url = base_url.rstrip("/")
            api_format = config["api_format"]
            api_key_value = config["api_key"]
            extra_headers = config.get("extra_headers")

            try:
                adapter_class = _get_adapter_for_format(api_format)
                if not adapter_class:
                    return [], f"Unknown API format: {api_format}", False

                async with semaphore:
                    models, error = await adapter_class.fetch_models(  # type: ignore[attr-defined]
                        client, base_url, api_key_value, extra_headers
                    )

                for m in models:
                    if "api_format" not in m:
                        m["api_format"] = api_format

                # 即使返回空列表，只要没有错误也算成功
                success = error is None
                return models, error, success
            except httpx.TimeoutException:
                logger.warning(f"获取 {api_format} 模型超时")
                return [], f"{api_format}: timeout", False
            except Exception as e:
                # 只记录异常类型，避免泄露敏感信息
                logger.exception(f"获取 {api_format} 模型出错")
                return [], f"{api_format}: {type(e).__name__}", False

        async with httpx.AsyncClient(timeout=30.0) as client:
            results = await asyncio.gather(*[fetch_one(client, c) for c in endpoint_configs])
            for models, error, success in results:
                all_models.extend(models)
                if error:
                    errors.append(error)
                if success:
                    has_success = True

        return all_models, errors, has_success


# 单例模式
_model_fetch_scheduler: Optional[ModelFetchScheduler] = None


def get_model_fetch_scheduler() -> ModelFetchScheduler:
    """获取模型获取调度器单例"""
    global _model_fetch_scheduler
    if _model_fetch_scheduler is None:
        _model_fetch_scheduler = ModelFetchScheduler()
    return _model_fetch_scheduler
