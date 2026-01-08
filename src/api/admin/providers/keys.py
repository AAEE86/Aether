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
    获取完整的 Shared API Key
    """
    adapter = AdminRevealProviderKeyAdapter(key_id=key_id)
    return await pipeline.run(adapter=adapter, http_request=request, db=db, mode=adapter.mode)


@router.put("/{provider_id}/keys/batch-priority")
async def batch_update_provider_key_priority(
    provider_id: str,
    priorities: dict,
    request: Request,
    db: Session = Depends(get_db),
) -> dict:
    """
    批量更新 Provider 共享 Keys 的优先级
    """
    adapter = AdminBatchUpdateProviderKeyPriorityAdapter(
        provider_id=provider_id,
        priorities=priorities.get("priorities", [])
    )
    return await pipeline.run(adapter=adapter, http_request=request, db=db, mode=adapter.mode)


# -------- Adapters --------


@dataclass
class AdminListProviderKeysAdapter(AdminApiAdapter):
    provider_id: str
    skip: int
    limit: int

    async def handle(self, context):
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

    async def handle(self, context):
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

    async def handle(self, context):
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

    async def handle(self, context):
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
    key_id: str

    async def handle(self, context):
        db = context.db
        key = db.query(ProviderAPIKey).filter(ProviderAPIKey.id == self.key_id, ProviderAPIKey.is_shared == True).first()
        if not key:
            raise NotFoundException(f"共享 Key {self.key_id} 不存在")

        try:
            decrypted_key = crypto_service.decrypt(key.api_key)
        except Exception as e:
            logger.error(f"解密 Key 失败: ID={self.key_id}, Error={e}")
            raise InvalidRequestException(
                "无法解密 API Key，可能是加密密钥已更改。"
            )

        logger.info(f"[REVEAL] 查看完整共享 Key: ID={self.key_id}")
        return {"api_key": decrypted_key}


@dataclass
class AdminBatchUpdateProviderKeyPriorityAdapter(AdminApiAdapter):
    provider_id: str
    priorities: List[dict]

    async def handle(self, context):
        db = context.db
        provider = db.query(Provider).filter(Provider.id == self.provider_id).first()
        if not provider:
            raise NotFoundException(f"Provider {self.provider_id} 不存在")

        updated_count = 0
        for item in self.priorities:
            key_id = item.get("key_id")
            internal_priority = item.get("internal_priority")
            
            if key_id is None or internal_priority is None:
                continue

            key = db.query(ProviderAPIKey).filter(
                ProviderAPIKey.id == key_id,
                ProviderAPIKey.provider_id == self.provider_id,
                ProviderAPIKey.is_shared == True
            ).first()
            
            if key:
                key.internal_priority = internal_priority
                key.updated_at = datetime.now(timezone.utc)
                updated_count += 1

        db.commit()
        logger.info(f"[OK] 批量更新共享 Key 优先级: Provider={self.provider_id}, Count={updated_count}")
        
        return {
            "message": f"已更新 {updated_count} 个共享 Key 的优先级",
            "updated_count": updated_count
        }
