"""
统一的 API Keys 管理 Adapters
支持 Provider 共享 Keys 和 Endpoint 专用 Keys
"""

import uuid
from dataclasses import dataclass
from datetime import datetime, timezone
from typing import Dict, List, Literal, Optional

from sqlalchemy.orm import Session

from src.api.base.admin_adapter import AdminApiAdapter
from src.core.crypto import crypto_service
from src.core.exceptions import InvalidRequestException, NotFoundException
from src.core.key_capabilities import get_capability
from src.core.logger import logger
from src.models.database import Provider, ProviderAPIKey, ProviderEndpoint
from src.services.cache.provider_cache import ProviderCacheService
from src.models.endpoint_models import (
    BatchUpdateKeyPriorityRequest,
    EndpointAPIKeyCreate,
    EndpointAPIKeyResponse,
    EndpointAPIKeyUpdate,
)


# -------- Helper Functions --------


def _build_key_response(
    key: ProviderAPIKey,
    decrypted_key: Optional[str] = None,
    include_plain: bool = False,
) -> EndpointAPIKeyResponse:
    """构建 Key 响应对象的通用方法"""
    try:
        if decrypted_key is None:
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
            "api_key_plain": decrypted_key if include_plain else None,
            "success_rate": success_rate,
            "avg_response_time_ms": round(avg_response_time_ms, 2),
            "is_adaptive": is_adaptive,
            "effective_limit": (
                key.learned_max_concurrent if is_adaptive else key.max_concurrent
            ),
        }
    )
    return EndpointAPIKeyResponse(**key_dict)


# -------- List Keys Adapter --------


@dataclass
class AdminListKeysAdapter(AdminApiAdapter):
    """统一的 List Keys Adapter,支持 Provider 和 Endpoint"""

    parent_id: str
    parent_type: Literal["provider", "endpoint"]
    skip: int
    limit: int

    async def handle(self, context):  # type: ignore[override]
        db = context.db

        if self.parent_type == "provider":
            # 验证 Provider 存在
            parent = db.query(Provider).filter(Provider.id == self.parent_id).first()
            if not parent:
                raise NotFoundException(f"Provider {self.parent_id} 不存在")

            # 查询共享 Keys
            keys = (
                db.query(ProviderAPIKey)
                .filter(
                    ProviderAPIKey.provider_id == self.parent_id,
                    ProviderAPIKey.is_shared == True,
                )
                .order_by(
                    ProviderAPIKey.internal_priority.asc(), ProviderAPIKey.created_at.asc()
                )
                .offset(self.skip)
                .limit(self.limit)
                .all()
            )
        else:  # endpoint
            # 验证 Endpoint 存在
            parent = (
                db.query(ProviderEndpoint).filter(ProviderEndpoint.id == self.parent_id).first()
            )
            if not parent:
                raise NotFoundException(f"Endpoint {self.parent_id} 不存在")

            # 查询 Endpoint Keys
            keys = (
                db.query(ProviderAPIKey)
                .filter(ProviderAPIKey.endpoint_id == self.parent_id)
                .order_by(
                    ProviderAPIKey.internal_priority.asc(), ProviderAPIKey.created_at.asc()
                )
                .offset(self.skip)
                .limit(self.limit)
                .all()
            )

        result: List[EndpointAPIKeyResponse] = []
        for key in keys:
            key_response = _build_key_response(key)
            # 添加共享 Key 特有字段
            if self.parent_type == "provider":
                key_response.is_shared = True
                key_response.provider_id = self.parent_id
                key_response.endpoint_id = None
            result.append(key_response)

        return result


# -------- Create Key Adapter --------


@dataclass
class AdminCreateKeyAdapter(AdminApiAdapter):
    """统一的 Create Key Adapter,支持 Provider 和 Endpoint"""

    parent_id: str
    parent_type: Literal["provider", "endpoint"]
    key_data: EndpointAPIKeyCreate

    async def handle(self, context):  # type: ignore[override]
        db = context.db

        if self.parent_type == "provider":
            # 验证 Provider 存在
            parent = db.query(Provider).filter(Provider.id == self.parent_id).first()
            if not parent:
                raise NotFoundException(f"Provider {self.parent_id} 不存在")

            # 共享 Key 不能指定 endpoint_id
            if self.key_data.endpoint_id is not None:
                raise InvalidRequestException("创建共享 Key 时不能指定 endpoint_id")

            # 创建共享 Key
            provider_id = self.parent_id
            endpoint_id = None
            is_shared = True
            log_prefix = "共享 Key"
        else:  # endpoint
            # 验证 Endpoint 存在
            parent = (
                db.query(ProviderEndpoint).filter(ProviderEndpoint.id == self.parent_id).first()
            )
            if not parent:
                raise NotFoundException(f"Endpoint {self.parent_id} 不存在")

            # Endpoint Key 必须匹配 endpoint_id
            if self.key_data.endpoint_id != self.parent_id:
                raise InvalidRequestException("endpoint_id 不匹配")

            # 创建 Endpoint Key
            provider_id = None
            endpoint_id = self.parent_id
            is_shared = False
            log_prefix = "Key"

        encrypted_key = crypto_service.encrypt(self.key_data.api_key)
        now = datetime.now(timezone.utc)

        new_key = ProviderAPIKey(
            id=str(uuid.uuid4()),
            provider_id=provider_id,
            endpoint_id=endpoint_id,
            is_shared=is_shared,
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

        parent_desc = f"Provider={self.parent_id}" if is_shared else f"Endpoint={self.parent_id}"
        logger.info(
            f"[OK] 添加{log_prefix}: {parent_desc}, Key=***{self.key_data.api_key[-4:]}, ID={new_key.id}"
        )

        response = _build_key_response(new_key, self.key_data.api_key, include_plain=True)
        if is_shared:
            response.is_shared = True
            response.endpoint_id = None

        return response


# -------- Update Key Adapter --------


@dataclass
class AdminUpdateKeyAdapter(AdminApiAdapter):
    """统一的 Update Key Adapter,支持 Provider 和 Endpoint"""

    key_id: str
    key_data: EndpointAPIKeyUpdate
    parent_type: Literal["provider", "endpoint"]

    async def handle(self, context):  # type: ignore[override]
        db = context.db

        # 查询 Key
        if self.parent_type == "provider":
            key = (
                db.query(ProviderAPIKey)
                .filter(ProviderAPIKey.id == self.key_id, ProviderAPIKey.is_shared == True)
                .first()
            )
            if not key:
                raise NotFoundException(f"共享 Key {self.key_id} 不存在")
            log_prefix = "共享 Key"
        else:  # endpoint
            key = db.query(ProviderAPIKey).filter(ProviderAPIKey.id == self.key_id).first()
            if not key:
                raise NotFoundException(f"Key {self.key_id} 不存在")
            log_prefix = "Key"

        # 构建更新数据
        update_data = self.key_data.model_dump(exclude_unset=True)
        if "api_key" in update_data:
            update_data["api_key"] = crypto_service.encrypt(update_data["api_key"])

        # 特殊处理 max_concurrent
        if "max_concurrent" in self.key_data.model_fields_set:
            update_data["max_concurrent"] = self.key_data.max_concurrent
            if self.key_data.max_concurrent is None:
                update_data["learned_max_concurrent"] = None
                logger.info("Key %s 切换为自适应并发模式", self.key_id)

        # 应用更新
        for field, value in update_data.items():
            setattr(key, field, value)
        key.updated_at = datetime.now(timezone.utc)

        db.commit()
        db.refresh(key)

        # 如果更新了 rate_multiplier,清除缓存
        if "rate_multiplier" in update_data:
            await ProviderCacheService.invalidate_provider_api_key_cache(self.key_id)

        logger.info("[OK] 更新%s: ID=%s, Updates=%s", log_prefix, self.key_id, list(update_data.keys()))

        return _build_key_response(key)


# -------- Reveal Key Adapter --------


@dataclass
class AdminRevealKeyAdapter(AdminApiAdapter):
    """统一的 Reveal Key Adapter,支持 Provider 和 Endpoint"""

    key_id: str
    parent_type: Literal["provider", "endpoint"]

    async def handle(self, context):  # type: ignore[override]
        db = context.db

        # 查询 Key
        if self.parent_type == "provider":
            key = (
                db.query(ProviderAPIKey)
                .filter(ProviderAPIKey.id == self.key_id, ProviderAPIKey.is_shared == True)
                .first()
            )
            if not key:
                raise NotFoundException(f"共享 Key {self.key_id} 不存在")
            log_prefix = "共享 Key"
        else:  # endpoint
            key = db.query(ProviderAPIKey).filter(ProviderAPIKey.id == self.key_id).first()
            if not key:
                raise NotFoundException(f"Key {self.key_id} 不存在")
            log_prefix = "完整 Key"

        try:
            decrypted_key = crypto_service.decrypt(key.api_key)
        except Exception as e:
            logger.error(f"解密 Key 失败: ID={self.key_id}, Error={e}")
            raise InvalidRequestException("无法解密 API Key,可能是加密密钥已更改。请重新添加该密钥。")

        logger.info(f"[REVEAL] 查看{log_prefix}: ID={self.key_id}, Name={key.name}")
        return {"api_key": decrypted_key}


# -------- Delete Key Adapter --------


@dataclass
class AdminDeleteKeyAdapter(AdminApiAdapter):
    """统一的 Delete Key Adapter,支持 Provider 和 Endpoint"""

    key_id: str
    parent_type: Literal["provider", "endpoint"]

    async def handle(self, context):  # type: ignore[override]
        db = context.db

        # 查询 Key
        if self.parent_type == "provider":
            key = (
                db.query(ProviderAPIKey)
                .filter(ProviderAPIKey.id == self.key_id, ProviderAPIKey.is_shared == True)
                .first()
            )
            if not key:
                raise NotFoundException(f"共享 Key {self.key_id} 不存在")
            parent_id = key.provider_id
            log_prefix = "共享 Key"
            parent_desc = f"Provider={parent_id}"
        else:  # endpoint
            key = db.query(ProviderAPIKey).filter(ProviderAPIKey.id == self.key_id).first()
            if not key:
                raise NotFoundException(f"Key {self.key_id} 不存在")
            parent_id = key.endpoint_id
            log_prefix = "Key"
            parent_desc = f"Endpoint={parent_id}"

        try:
            db.delete(key)
            db.commit()
        except Exception as exc:
            db.rollback()
            logger.error(f"删除{log_prefix}失败: ID={self.key_id}, Error={exc}")
            raise

        logger.warning(f"[DELETE] 删除{log_prefix}: ID={self.key_id}, {parent_desc}")
        return {"message": f"{log_prefix} {self.key_id} 已删除"}


# -------- Batch Update Priority Adapter --------


@dataclass
class AdminBatchUpdateKeyPriorityAdapter(AdminApiAdapter):
    """统一的 Batch Update Priority Adapter,支持 Provider 和 Endpoint"""

    parent_id: str
    parent_type: Literal["provider", "endpoint"]
    priority_data: BatchUpdateKeyPriorityRequest

    async def handle(self, context):  # type: ignore[override]
        db = context.db

        if self.parent_type == "provider":
            # 验证 Provider 存在
            parent = db.query(Provider).filter(Provider.id == self.parent_id).first()
            if not parent:
                raise NotFoundException(f"Provider {self.parent_id} 不存在")

            # 获取所有需要更新的 Key ID
            key_ids = [item.key_id for item in self.priority_data.priorities]

            # 验证所有 Key 都属于该 Provider 且为共享 Key
            keys = (
                db.query(ProviderAPIKey)
                .filter(
                    ProviderAPIKey.id.in_(key_ids),
                    ProviderAPIKey.provider_id == self.parent_id,
                    ProviderAPIKey.is_shared == True,
                )
                .all()
            )

            if len(keys) != len(key_ids):
                found_ids = {k.id for k in keys}
                missing_ids = set(key_ids) - found_ids
                raise InvalidRequestException(
                    f"Keys 不属于该 Provider、不是共享 Key 或不存在: {missing_ids}"
                )

            log_prefix = "共享 Key"
            parent_desc = f"Provider={self.parent_id}"
        else:  # endpoint
            # 验证 Endpoint 存在
            parent = (
                db.query(ProviderEndpoint).filter(ProviderEndpoint.id == self.parent_id).first()
            )
            if not parent:
                raise NotFoundException(f"Endpoint {self.parent_id} 不存在")

            # 获取所有需要更新的 Key ID
            key_ids = [item.key_id for item in self.priority_data.priorities]

            # 验证所有 Key 都属于该 Endpoint
            keys = (
                db.query(ProviderAPIKey)
                .filter(
                    ProviderAPIKey.id.in_(key_ids),
                    ProviderAPIKey.endpoint_id == self.parent_id,
                )
                .all()
            )

            if len(keys) != len(key_ids):
                found_ids = {k.id for k in keys}
                missing_ids = set(key_ids) - found_ids
                raise InvalidRequestException(f"Keys 不属于该 Endpoint 或不存在: {missing_ids}")

            log_prefix = "Key"
            parent_desc = f"Endpoint={self.parent_id}"

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

        logger.info(
            f"[OK] 批量更新{log_prefix}优先级: {parent_desc}, Updated={updated_count}/{len(key_ids)}"
        )
        return {"message": f"已更新 {updated_count} 个{log_prefix}的优先级", "updated_count": updated_count}


# -------- Get Keys Grouped By Format Adapter (Endpoint Only) --------


class AdminGetKeysGroupedByFormatAdapter(AdminApiAdapter):
    """获取按 API 格式分组的所有 Keys (仅用于 Endpoint Keys)"""

    async def handle(self, context):  # type: ignore[override]
        db = context.db

        # 查询端点密钥(endpoint_id 不为空)
        endpoint_keys = (
            db.query(ProviderAPIKey, ProviderEndpoint, Provider)
            .join(ProviderEndpoint, ProviderAPIKey.endpoint_id == ProviderEndpoint.id)
            .join(Provider, ProviderEndpoint.provider_id == Provider.id)
            .filter(
                ProviderAPIKey.is_active.is_(True),
                ProviderAPIKey.is_shared.is_(False),
                ProviderEndpoint.is_active.is_(True),
                Provider.is_active.is_(True),
            )
            .all()
        )

        # 查询共享密钥(provider_id 不为空,endpoint_id 为空)
        shared_keys = (
            db.query(ProviderAPIKey, Provider)
            .join(Provider, ProviderAPIKey.provider_id == Provider.id)
            .filter(
                ProviderAPIKey.is_active.is_(True),
                ProviderAPIKey.is_shared.is_(True),
                Provider.is_active.is_(True),
            )
            .all()
        )

        # 合并两种密钥的结果
        all_keys = []

        # 处理端点密钥
        for key, endpoint, provider in endpoint_keys:
            all_keys.append(
                {
                    "key": key,
                    "provider": provider,
                    "api_format": endpoint.api_format,
                    "endpoint_base_url": endpoint.base_url,
                }
            )

        # 处理共享密钥 - 为每个 Provider 的每个活跃 Endpoint 的 api_format 创建条目
        for key, provider in shared_keys:
            # 获取该 Provider 下所有活跃的 Endpoints
            provider_endpoints = (
                db.query(ProviderEndpoint)
                .filter(
                    ProviderEndpoint.provider_id == provider.id,
                    ProviderEndpoint.is_active.is_(True),
                )
                .all()
            )

            # 如果 Provider 没有活跃的 Endpoint,跳过这个共享密钥
            if not provider_endpoints:
                continue

            # 为每个 Endpoint 的 api_format 创建一个条目
            for endpoint in provider_endpoints:
                all_keys.append(
                    {
                        "key": key,
                        "provider": provider,
                        "api_format": endpoint.api_format,
                        "endpoint_base_url": "shared",
                    }
                )

        # 按优先级排序
        all_keys.sort(
            key=lambda x: (
                x["key"].global_priority
                if x["key"].global_priority is not None
                else float("inf"),
                x["key"].internal_priority,
            )
        )

        grouped: Dict[str, List[dict]] = {}
        for item in all_keys:
            key = item["key"]
            provider = item["provider"]
            api_format = item["api_format"]
            endpoint_base_url = item["endpoint_base_url"]

            if api_format not in grouped:
                grouped[api_format] = []

            try:
                decrypted_key = crypto_service.decrypt(key.api_key)
                masked_key = f"{decrypted_key[:8]}***{decrypted_key[-4:]}"
            except Exception:
                masked_key = "***ERROR***"

            # 计算健康度指标
            success_rate = (
                key.success_count / key.request_count if key.request_count > 0 else None
            )
            avg_response_time_ms = (
                round(key.total_response_time_ms / key.success_count, 2)
                if key.success_count > 0
                else None
            )

            # 将 capabilities dict 转换为启用的能力简短名称列表
            caps_list = []
            if key.capabilities:
                for cap_name, enabled in key.capabilities.items():
                    if enabled:
                        cap_def = get_capability(cap_name)
                        caps_list.append(cap_def.short_name if cap_def else cap_name)

            grouped[api_format].append(
                {
                    "id": key.id,
                    "name": key.name,
                    "api_key_masked": masked_key,
                    "internal_priority": key.internal_priority,
                    "global_priority": key.global_priority,
                    "rate_multiplier": key.rate_multiplier,
                    "is_active": key.is_active,
                    "circuit_breaker_open": key.circuit_breaker_open,
                    "provider_name": provider.display_name or provider.name,
                    "endpoint_base_url": endpoint_base_url,
                    "api_format": api_format,
                    "capabilities": caps_list,
                    "health_score": key.health_score,
                    "success_rate": success_rate,
                    "avg_response_time_ms": avg_response_time_ms,
                    "request_count": key.request_count,
                }
            )

        return grouped