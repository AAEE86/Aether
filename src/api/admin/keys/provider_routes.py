"""
Provider 共享 API Keys 路由
"""

from typing import List

from fastapi import APIRouter, Depends, Query, Request
from sqlalchemy.orm import Session

from src.api.base.pipeline import ApiRequestPipeline
from src.database import get_db
from src.models.endpoint_models import (
    BatchUpdateKeyPriorityRequest,
    EndpointAPIKeyCreate,
    EndpointAPIKeyResponse,
    EndpointAPIKeyUpdate,
)
from .adapters import (
    AdminBatchUpdateKeyPriorityAdapter,
    AdminCreateKeyAdapter,
    AdminDeleteKeyAdapter,
    AdminListKeysAdapter,
    AdminRevealKeyAdapter,
    AdminUpdateKeyAdapter,
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
    adapter = AdminListKeysAdapter(
        parent_id=provider_id,
        parent_type="provider",
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
    adapter = AdminCreateKeyAdapter(
        parent_id=provider_id,
        parent_type="provider",
        key_data=key_data,
    )
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
    adapter = AdminUpdateKeyAdapter(
        key_id=key_id,
        key_data=key_data,
        parent_type="provider",
    )
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
    adapter = AdminDeleteKeyAdapter(
        key_id=key_id,
        parent_type="provider",
    )
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
    adapter = AdminRevealKeyAdapter(
        key_id=key_id,
        parent_type="provider",
    )
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
    adapter = AdminBatchUpdateKeyPriorityAdapter(
        parent_id=provider_id,
        parent_type="provider",
        priority_data=priority_data,
    )
    return await pipeline.run(adapter=adapter, http_request=request, db=db, mode=adapter.mode)