"""
Provider Shared API Keys 管理
"""

import uuid
from dataclasses import dataclass
from datetime import datetime, timezone
from typing import List

from fastapi import APIRouter, Depends, Query, Request
from sqlalchemy.orm import Session

from src.api.base.admin_adapter import AdminApiAdapter
from src.api.base.pipeline import ApiRequestPipeline
from src.core.crypto import crypto_service
from src.core.exceptions import InvalidRequestException, NotFoundException
from src.core.logger import logger
from src.database import get_db
from src.models.database import Provider, ProviderAPIKey
from src.services.cache.provider_cache import ProviderCacheService
from src.models.endpoint_models import (
    BatchUpdateKeyPriorityRequest,
    EndpointAPIKeyCreate,
    EndpointAPIKeyResponse,
    EndpointAPIKeyUpdate,
)

router = APIRouter(tags=["Provider Shared Keys"])
pipeline = ApiRequestPipeline()


@router.get("/{provider_id}/keys", response_model=List[EndpointAPIKeyResponse])
async def list_provider_keys(
    provider_id: str,
    request: Request,
    skip: int = Query(0, ge=0, description="跳过的记录数"),
    limit: int = Query(100, ge=1, le=1000, description="返回的最大记录数"),
    db: Session = Depends(get_db),
) -> List[EndpointAPIKeyResponse]:
    """
    获取 Provider 的所有共享 Keys

    获取指定 Provider 下的所有共享 API Key 列表,包括 Key 的配置、统计信息等。
    结果按优先级和创建时间排序。

    **路径参数**:
    - `provider_id`: Provider ID

    **查询参数**:
    - `skip`: 跳过的记录数,用于分页(默认 0)
    - `limit`: 返回的最大记录数(1-1000,默认 100)

    **返回字段**:
    - `id`: Key ID
    - `name`: Key 名称
    - `api_key_masked`: 脱敏后的 API Key
    - `internal_priority`: 内部优先级
    - `global_priority`: 全局优先级
    - `rate_multiplier`: 速率倍数
    - `max_concurrent`: 最大并发数(null 表示自适应模式)
    - `is_adaptive`: 是否为自适应并发模式
    - `effective_limit`: 有效并发限制
    - `success_rate`: 成功率
    - `avg_response_time_ms`: 平均响应时间(毫秒)
    - `is_shared`: 是否为共享 Key(true)
    - `provider_id`: 所属 Provider ID
    - 其他配置和统计字段
    """
    adapter = AdminListProviderKeysAdapter(
        provider_id=provider_id,
        skip=skip,
        limit=limit,
    )
    return await pipeline.run(adapter=adapter, http_request=request, db=db, mode=adapter.mode)


@router.post("/{provider_id}/keys", response_model=EndpointAPIKeyResponse)
async def add_provider_key(
    provider_id: str,
    key_data: EndpointAPIKeyCreate,
    request: Request,
    db: Session = Depends(get_db),
) -> EndpointAPIKeyResponse:
    """
    为 Provider 添加共享 Key

    为指定 Provider 添加新的共享 API Key,该 Key 可被 Provider 下所有 Endpoint 使用。
    支持配置并发限制、速率倍数、优先级、配额限制、能力限制等。

    **路径参数**:
    - `provider_id`: Provider ID

    **请求体字段**:
    - `api_key`: API Key 原文(将被加密存储)
    - `name`: Key 名称
    - `note`: 备注(可选)
    - `rate_multiplier`: 速率倍数(默认 1.0)
    - `internal_priority`: 内部优先级(默认 100)
    - `max_concurrent`: 最大并发数(null 表示自适应模式)
    - `rate_limit`: 每分钟请求限制(可选)
    - `daily_limit`: 每日请求限制(可选)
    - `monthly_limit`: 每月请求限制(可选)
    - `allowed_models`: 允许的模型列表(可选)
    - `capabilities`: 能力配置(可选)
    - `endpoint_id`: 必须为 null(共享 Key 不绑定特定 Endpoint)

    **返回字段**:
    - 包含完整的 Key 信息,其中 `api_key_plain` 为原文(仅在创建时返回)
    """
    adapter = AdminCreateProviderKeyAdapter(provider_id=provider_id, key_data=key_data)
    return await pipeline.run(adapter=adapter, http_request=request, db=db, mode=adapter.mode)


@router.put("/keys/{key_id}", response_model=EndpointAPIKeyResponse)
async def update_provider_key(
    key_id: str,
    key_data: EndpointAPIKeyUpdate,
    request: Request,
    db: Session = Depends(get_db),
) -> EndpointAPIKeyResponse:
    """
    更新 Provider 共享 Key

    更新指定共享 Key 的配置,支持修改并发限制、速率倍数、优先级、
    配额限制、能力限制等。支持部分更新。

    **路径参数**:
    - `key_id`: Key ID

    **请求体字段**(均为可选):
    - `api_key`: 新的 API Key 原文
    - `name`: Key 名称
    - `note`: 备注
    - `rate_multiplier`: 速率倍数
    - `internal_priority`: 内部优先级
    - `max_concurrent`: 最大并发数(设置为 null 可切换到自适应模式)
    - `rate_limit`: 每分钟请求限制
    - `daily_limit`: 每日请求限制
    - `monthly_limit`: 每月请求限制
    - `allowed_models`: 允许的模型列表
    - `capabilities`: 能力配置
    - `is_active`: 是否活跃

    **返回字段**:
    - 包含更新后的完整 Key 信息
    """
    adapter = AdminUpdateProviderKeyAdapter(key_id=key_id, key_data=key_data)
    return await pipeline.run(adapter=adapter, http_request=request, db=db, mode=adapter.mode)


@router.delete("/keys/{key_id}")
async def delete_provider_key(
    key_id: str,
    request: Request,
    db: Session = Depends(get_db),
) -> dict:
    """
    删除 Provider 共享 Key

    删除指定的共享 API Key。此操作不可逆,请谨慎使用。

    **路径参数**:
    - `key_id`: Key ID

    **返回字段**:
    - `message`: 操作结果消息
    """
    adapter = AdminDeleteProviderKeyAdapter(key_id=key_id)
    return await pipeline.run(adapter=adapter, http_request=request, db=db, mode=adapter.mode)


@router.get("/keys/{key_id}/reveal")
async def reveal_provider_key(
    key_id: str,
    request: Request,
    db: Session = Depends(get_db),
) -> dict:
    """
    获取完整的共享 API Key

    解密并返回指定共享 Key 的完整原文,用于查看和复制。
    此操作会被记录到审计日志。

    **路径参数**:
    - `key_id`: Key ID

    **返回字段**:
    - `api_key`: 完整的 API Key 原文
    """
    adapter = AdminRevealProviderKeyAdapter(key_id=key_id)
    return await pipeline.run(adapter=adapter, http_request=request, db=db, mode=adapter.mode)


@router.put("/{provider_id}/keys/batch-priority")
async def batch_update_provider_key_priority(
    provider_id: str,
    request: Request,
    priority_data: BatchUpdateKeyPriorityRequest,
    db: Session = Depends(get_db),
) -> dict:
    """
    批量更新 Provider 共享 Keys 的优先级

    批量更新指定 Provider 下多个共享 Key 的内部优先级,用于拖动排序。
    所有 Key 必须属于指定的 Provider 且为共享 Key。

    **路径参数**:
    - `provider_id`: Provider ID

    **请求体字段**:
    - `priorities`: 优先级列表
      - `key_id`: Key ID
      - `internal_priority`: 新的内部优先级

    **返回字段**:
    - `message`: 操作结果消息
    - `updated_count`: 实际更新的 Key 数量
    """
    adapter = AdminBatchUpdateProviderKeyPriorityAdapter(
        provider_id=provider_id,
        priority_data=priority_data
    )
    return await pipeline.run(adapter=adapter, http_request=request, db=db, mode=adapter.mode)


# -------- Adapters --------


@dataclass
class AdminListProviderKeysAdapter(AdminApiAdapter):
    provider_id: str
    skip: int
    limit: int

    async def handle(self, context):  # type: ignore[override]
        db = context.db
        provider = db.query(Provider).filter(Provider.id == self.provider_id).first()
        if not provider:
            raise NotFoundException(f"Provider {self.provider_id} 不存在")

        keys = (
            db.query(ProviderAPIKey)
            .filter(
                ProviderAPIKey.provider_id == self.provider_id,
                ProviderAPIKey.is_shared == True
            )
            .order_by(ProviderAPIKey.internal_priority.asc(), ProviderAPIKey.created_at.asc())
            .offset(self.skip)
            .limit(self.limit)
            .all()
        )

        result: List[EndpointAPIKeyResponse] = []
        for key in keys:
            try:
                decrypted_key = crypto_service.decrypt(key.api_key)
                masked_key = f"{decrypted_key[:8]}***{decrypted_key[-4:]}"
            except Exception:
                masked_key = "***ERROR***"

            success_rate = key.success_count / key.request_count if key.request_count > 0 else 0.0
            avg_response_time_ms = (
                key.total_response_time_ms / key.success_count if key.success_count > 0 else 0.0
            )

            is_adaptive = key.max_concurrent is None
            key_dict = key.__dict__.copy()
            key_dict.pop("_sa_instance_state", None)
            key_dict.update(
                {
                    "api_key_masked": masked_key,
                    "api_key_plain": None,
                    "success_rate": success_rate,
                    "avg_response_time_ms": round(avg_response_time_ms, 2),
                    "is_adaptive": is_adaptive,
                    "effective_limit": (
                        key.learned_max_concurrent if is_adaptive else key.max_concurrent
                    ),
                    "is_shared": True,
                    "provider_id": self.provider_id,
                    "endpoint_id": None
                }
            )
            result.append(EndpointAPIKeyResponse(**key_dict))

        return result


@dataclass
class AdminCreateProviderKeyAdapter(AdminApiAdapter):
    provider_id: str
    key_data: EndpointAPIKeyCreate

    async def handle(self, context):  # type: ignore[override]
        db = context.db
        provider = db.query(Provider).filter(Provider.id == self.provider_id).first()
        if not provider:
            raise NotFoundException(f"Provider {self.provider_id} 不存在")

        if self.key_data.endpoint_id is not None:
             raise InvalidRequestException("创建共享 Key 时不能指定 endpoint_id")

        encrypted_key = crypto_service.encrypt(self.key_data.api_key)
        now = datetime.now(timezone.utc)
        
        new_key = ProviderAPIKey(
            id=str(uuid.uuid4()),
            provider_id=self.provider_id,
            endpoint_id=None,
            is_shared=True,
            api_key=encrypted_key,
            name=self.key_data.name,
            note=self.key_data.note,
            rate_multiplier=self.key_data.rate_multiplier,
            internal_priority=self.key_data.internal_priority,
            max_concurrent=self.key_data.max_concurrent,
            rate_limit=self.key_data.rate_limit,
            daily_limit=self.key_data.daily_limit,
            monthly_limit=self.key_data.monthly_limit,
            allowed_models=self.key_data.allowed_models if self.key_data.allowed_models else None,
            capabilities=self.key_data.capabilities if self.key_data.capabilities else None,
            request_count=0,
            success_count=0,
            error_count=0,
            total_response_time_ms=0,
            is_active=True,
            last_used_at=None,
            created_at=now,
            updated_at=now,
        )

        db.add(new_key)
        db.commit()
        db.refresh(new_key)

        logger.info(f"[OK] 添加共享 Key: Provider={self.provider_id}, Key=***{self.key_data.api_key[-4:]}, ID={new_key.id}")

        masked_key = f"{self.key_data.api_key[:8]}***{self.key_data.api_key[-4:]}"
        is_adaptive = new_key.max_concurrent is None
        response_dict = new_key.__dict__.copy()
        response_dict.pop("_sa_instance_state", None)
        response_dict.update(
            {
                "api_key_masked": masked_key,
                "api_key_plain": self.key_data.api_key,
                "success_rate": 0.0,
                "avg_response_time_ms": 0.0,
                "is_adaptive": is_adaptive,
                "effective_limit": (
                    new_key.learned_max_concurrent if is_adaptive else new_key.max_concurrent
                ),
                "is_shared": True,
                "endpoint_id": None
            }
        )

        return EndpointAPIKeyResponse(**response_dict)


@dataclass
class AdminUpdateProviderKeyAdapter(AdminApiAdapter):
    key_id: str
    key_data: EndpointAPIKeyUpdate

    async def handle(self, context):  # type: ignore[override]
        db = context.db
        key = db.query(ProviderAPIKey).filter(ProviderAPIKey.id == self.key_id, ProviderAPIKey.is_shared == True).first()
        if not key:
            raise NotFoundException(f"共享 Key {self.key_id} 不存在")

        update_data = self.key_data.model_dump(exclude_unset=True)
        if "api_key" in update_data:
            update_data["api_key"] = crypto_service.encrypt(update_data["api_key"])

        if "max_concurrent" in self.key_data.model_fields_set:
            update_data["max_concurrent"] = self.key_data.max_concurrent
            if self.key_data.max_concurrent is None:
                update_data["learned_max_concurrent"] = None
                logger.info("Key %s 切换为自适应并发模式", self.key_id)

        for field, value in update_data.items():
            setattr(key, field, value)
        key.updated_at = datetime.now(timezone.utc)

        db.commit()
        db.refresh(key)

        if "rate_multiplier" in update_data:
            await ProviderCacheService.invalidate_provider_api_key_cache(self.key_id)

        logger.info("[OK] 更新共享 Key: ID=%s, Updates=%s", self.key_id, list(update_data.keys()))

        try:
            decrypted_key = crypto_service.decrypt(key.api_key)
            masked_key = f"{decrypted_key[:8]}***{decrypted_key[-4:]}"
        except Exception:
            masked_key = "***ERROR***"

        success_rate = key.success_count / key.request_count if key.request_count > 0 else 0.0
        avg_response_time_ms = (
            key.total_response_time_ms / key.success_count if key.success_count > 0 else 0.0
        )

        is_adaptive = key.max_concurrent is None
        response_dict = key.__dict__.copy()
        response_dict.pop("_sa_instance_state", None)
        response_dict.update(
            {
                "api_key_masked": masked_key,
                "api_key_plain": None,
                "success_rate": success_rate,
                "avg_response_time_ms": round(avg_response_time_ms, 2),
                "is_adaptive": is_adaptive,
                "effective_limit": (
                    key.learned_max_concurrent if is_adaptive else key.max_concurrent
                ),
            }
        )
        return EndpointAPIKeyResponse(**response_dict)


@dataclass
class AdminDeleteProviderKeyAdapter(AdminApiAdapter):
    key_id: str

    async def handle(self, context):  # type: ignore[override]
        db = context.db
        key = db.query(ProviderAPIKey).filter(ProviderAPIKey.id == self.key_id, ProviderAPIKey.is_shared == True).first()
        if not key:
            raise NotFoundException(f"共享 Key {self.key_id} 不存在")

        provider_id = key.provider_id
        try:
            db.delete(key)
            db.commit()
        except Exception as exc:
            db.rollback()
            logger.error(f"删除共享 Key 失败: ID={self.key_id}, Error={exc}")
            raise

        logger.warning(f"[DELETE] 删除共享 Key: ID={self.key_id}, Provider={provider_id}")
        return {"message": f"共享 Key {self.key_id} 已删除"}


@dataclass
class AdminRevealProviderKeyAdapter(AdminApiAdapter):
    """获取完整的共享 API Key(用于查看和复制)"""

    key_id: str

    async def handle(self, context):  # type: ignore[override]
        db = context.db
        key = db.query(ProviderAPIKey).filter(ProviderAPIKey.id == self.key_id, ProviderAPIKey.is_shared == True).first()
        if not key:
            raise NotFoundException(f"共享 Key {self.key_id} 不存在")

        try:
            decrypted_key = crypto_service.decrypt(key.api_key)
        except Exception as e:
            logger.error(f"解密 Key 失败: ID={self.key_id}, Error={e}")
            raise InvalidRequestException(
                "无法解密 API Key，可能是加密密钥已更改。请重新添加该密钥。"
            )

        logger.info(f"[REVEAL] 查看完整共享 Key: ID={self.key_id}, Name={key.name}")
        return {"api_key": decrypted_key}


@dataclass
class AdminBatchUpdateProviderKeyPriorityAdapter(AdminApiAdapter):
    provider_id: str
    priority_data: BatchUpdateKeyPriorityRequest

    async def handle(self, context):  # type: ignore[override]
        db = context.db
        provider = db.query(Provider).filter(Provider.id == self.provider_id).first()
        if not provider:
            raise NotFoundException(f"Provider {self.provider_id} 不存在")

        # 获取所有需要更新的 Key ID
        key_ids = [item.key_id for item in self.priority_data.priorities]

        # 验证所有 Key 都属于该 Provider 且为共享 Key
        keys = (
            db.query(ProviderAPIKey)
            .filter(
                ProviderAPIKey.id.in_(key_ids),
                ProviderAPIKey.provider_id == self.provider_id,
                ProviderAPIKey.is_shared == True,
            )
            .all()
        )

        if len(keys) != len(key_ids):
            found_ids = {k.id for k in keys}
            missing_ids = set(key_ids) - found_ids
            raise InvalidRequestException(f"Keys 不属于该 Provider、不是共享 Key 或不存在: {missing_ids}")

        # 批量更新优先级
        key_map = {k.id: k for k in keys}
        updated_count = 0
        for item in self.priority_data.priorities:
            key = key_map.get(item.key_id)
            if key and key.internal_priority != item.internal_priority:
                key.internal_priority = item.internal_priority
                key.updated_at = datetime.now(timezone.utc)
                updated_count += 1

        db.commit()

        logger.info(f"[OK] 批量更新共享 Key 优先级: Provider={self.provider_id}, Updated={updated_count}/{len(key_ids)}")
        return {"message": f"已更新 {updated_count} 个共享 Key 的优先级", "updated_count": updated_count}
