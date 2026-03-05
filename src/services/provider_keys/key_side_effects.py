"""
Provider Key 写操作后的副作用处理。
"""

from __future__ import annotations

from sqlalchemy.orm import Session

from src.core.logger import logger
from src.models.database import ProviderAPIKey
from src.services.cache.model_list_cache import invalidate_models_list_cache
from src.services.cache.provider_cache import ProviderCacheService


async def run_update_key_side_effects(
    db: Session,
    key: ProviderAPIKey,
    key_id: str,
    auto_fetch_enabled_before: bool,
    auto_fetch_enabled_after: bool,
    include_patterns_before: list[str] | None,
    exclude_patterns_before: list[str] | None,
    allowed_models_before: set[str],
) -> None:
    """执行更新 Key 后的副作用。"""
    if not auto_fetch_enabled_before and auto_fetch_enabled_after:
        # 刚刚开启了 auto_fetch_models，同步执行模型获取
        logger.info("[AUTO_FETCH] Key {} 开启自动获取模型，同步执行模型获取", key_id)
        try:
            from src.services.model.fetch_scheduler import get_model_fetch_scheduler

            scheduler = get_model_fetch_scheduler()
            # 同步等待模型获取完成，确保前端刷新时能看到最新数据
            await scheduler._fetch_models_for_key_by_id(key_id)
            # fetch_scheduler 可能在独立 session 更新 allowed_models，需刷新当前对象避免后续比较使用旧值。
            db.refresh(key)
        except Exception as e:
            logger.error(f"触发模型获取失败: {e}")
            # 不抛出异常，避免影响 Key 更新操作
    elif auto_fetch_enabled_before and not auto_fetch_enabled_after:
        # 关闭了 auto_fetch_models，只保留锁定的模型，清除自动获取的模型
        locked = key.locked_models or []
        if locked:
            key.allowed_models = locked
            logger.info(
                "[AUTO_FETCH] Key {} 关闭自动获取模型，保留 {} 个锁定模型",
                key_id,
                len(locked),
            )
        else:
            key.allowed_models = None
            logger.info(
                "[AUTO_FETCH] Key {} 关闭自动获取模型，无锁定模型，清空 allowed_models",
                key_id,
            )
        db.commit()
        db.refresh(key)
    elif auto_fetch_enabled_after:
        # auto_fetch_models 保持开启状态，检查过滤规则是否变更
        include_patterns_after = key.model_include_patterns
        exclude_patterns_after = key.model_exclude_patterns
        patterns_changed = (
            include_patterns_before != include_patterns_after
            or exclude_patterns_before != exclude_patterns_after
        )
        if patterns_changed:
            # 过滤规则变更，重新应用过滤（使用缓存的上游模型数据）
            logger.info("[AUTO_FETCH] Key {} 过滤规则变更，重新应用过滤", key_id)
            try:
                from src.services.model.fetch_scheduler import get_model_fetch_scheduler

                scheduler = get_model_fetch_scheduler()
                await scheduler._fetch_models_for_key_by_id(key_id)
                # 重新应用过滤后，刷新当前对象以读取最新 allowed_models。
                db.refresh(key)
            except Exception as e:
                logger.error(f"重新应用过滤规则失败: {e}")

    # 任何字段更新都清除缓存，确保缓存一致性
    # 包括 is_active、allowed_models、capabilities 等影响权限和行为的字段
    await ProviderCacheService.invalidate_provider_api_key_cache(key_id)

    # 检查 allowed_models 是否有变化，触发缓存失效和自动关联
    allowed_models_after = set(key.allowed_models or [])
    if allowed_models_before != allowed_models_after and key.provider_id:
        from src.services.model.global_model import on_key_allowed_models_changed

        await on_key_allowed_models_changed(
            db=db,
            provider_id=key.provider_id,
            allowed_models=list(key.allowed_models or []),
        )
    else:
        # allowed_models 未变化时，仍需清除 /v1/models 缓存（is_active、api_formats 变更会影响模型可用性）
        await invalidate_models_list_cache()


async def run_create_key_side_effects(
    db: Session,
    provider_id: str,
    key: ProviderAPIKey,
) -> None:
    """执行创建 Key 后的副作用。"""
    # 如果开启了 auto_fetch_models，同步执行模型获取
    if key.auto_fetch_models:
        logger.info("[AUTO_FETCH] 新 Key {} 开启自动获取模型，同步执行模型获取", key.id)
        try:
            from src.services.model.fetch_scheduler import get_model_fetch_scheduler

            scheduler = get_model_fetch_scheduler()
            # 同步等待模型获取完成，确保前端刷新时能看到最新数据
            await scheduler._fetch_models_for_key_by_id(key.id)
        except Exception as e:
            logger.error(f"触发模型获取失败: {e}")
            # 不抛出异常，避免影响 Key 创建操作

    # 如果创建时指定了 allowed_models，触发自动关联检查（内部会清除 /v1/models 缓存）
    if key.allowed_models:
        from src.services.model.global_model import on_key_allowed_models_changed

        await on_key_allowed_models_changed(
            db=db,
            provider_id=provider_id,
            allowed_models=list(key.allowed_models),
        )
    else:
        # 没有 allowed_models 时，仍需清除 /v1/models 缓存
        await invalidate_models_list_cache()


async def run_delete_key_side_effects(
    db: Session,
    provider_id: str | None,
    deleted_key_allowed_models: list[str] | None,
) -> None:
    """执行删除 Key 后的副作用。"""
    # 触发缓存失效和自动解除关联检查
    # 注意：删除后是否需要解除关联，应基于“删除后的活跃 Key 集合”判断。
    # 不能仅凭被删除 Key 的 allowed_models 是否为 null 来跳过 disassociate。
    _ = deleted_key_allowed_models
    if provider_id:
        from src.services.model.global_model import on_key_allowed_models_changed

        await on_key_allowed_models_changed(
            db=db,
            provider_id=provider_id,
        )
    else:
        # 无 provider_id 时仅清除缓存
        await invalidate_models_list_cache()
