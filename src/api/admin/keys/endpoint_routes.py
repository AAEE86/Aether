"""
Endpoint API Keys 路由
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
    AdminGetKeysGroupedByFormatAdapter,
    AdminListKeysAdapter,
    AdminRevealKeyAdapter,
    AdminUpdateKeyAdapter,
)

router = APIRouter(tags=["Endpoint Keys"])
pipeline = ApiRequestPipeline()


@router.get("/{endpoint_id}/keys", response_model=List[EndpointAPIKeyResponse])
async def list_endpoint_keys(
    endpoint_id: str,
    request: Request,
    skip: int = Query(0, ge=0, description="跳过的记录数"),
    limit: int = Query(100, ge=1, le=1000, description="返回的最大记录数"),
    db: Session = Depends(get_db),
) -> List[EndpointAPIKeyResponse]:
    """
    获取 Endpoint 的所有 Keys

    获取指定 Endpoint 下的所有 API Key 列表，包括 Key 的配置、统计信息等。
    结果按优先级和创建时间排序。

    **路径参数**:
    - `endpoint_id`: Endpoint ID

    **查询参数**:
    - `skip`: 跳过的记录数，用于分页（默认 0）
    - `limit`: 返回的最大记录数（1-1000，默认 100）

    **返回字段**:
    - `id`: Key ID
    - `name`: Key 名称
    - `api_key_masked`: 脱敏后的 API Key
    - `internal_priority`: 内部优先级
    - `global_priority`: 全局优先级
    - `rate_multiplier`: 速率倍数
    - `max_concurrent`: 最大并发数（null 表示自适应模式）
    - `is_adaptive`: 是否为自适应并发模式
    - `effective_limit`: 有效并发限制
    - `success_rate`: 成功率
    - `avg_response_time_ms`: 平均响应时间（毫秒）
    - 其他配置和统计字段
    """
    adapter = AdminListKeysAdapter(
        parent_id=endpoint_id,
        parent_type="endpoint",
        skip=skip,
        limit=limit,
    )
    return await pipeline.run(adapter=adapter, http_request=request, db=db, mode=adapter.mode)


@router.post("/{endpoint_id}/keys", response_model=EndpointAPIKeyResponse)
async def add_endpoint_key(
    endpoint_id: str,
    key_data: EndpointAPIKeyCreate,
    request: Request,
    db: Session = Depends(get_db),
) -> EndpointAPIKeyResponse:
    """
    为 Endpoint 添加 Key

    为指定 Endpoint 添加新的 API Key，支持配置并发限制、速率倍数、
    优先级、配额限制、能力限制等。

    **路径参数**:
    - `endpoint_id`: Endpoint ID

    **请求体字段**:
    - `endpoint_id`: Endpoint ID（必须与路径参数一致）
    - `api_key`: API Key 原文（将被加密存储）
    - `name`: Key 名称
    - `note`: 备注（可选）
    - `rate_multiplier`: 速率倍数（默认 1.0）
    - `internal_priority`: 内部优先级（默认 100）
    - `max_concurrent`: 最大并发数（null 表示自适应模式）
    - `rate_limit`: 每分钟请求限制（可选）
    - `daily_limit`: 每日请求限制（可选）
    - `monthly_limit`: 每月请求限制（可选）
    - `allowed_models`: 允许的模型列表（可选）
    - `capabilities`: 能力配置（可选）

    **返回字段**:
    - 包含完整的 Key 信息，其中 `api_key_plain` 为原文（仅在创建时返回）
    """
    adapter = AdminCreateKeyAdapter(
        parent_id=endpoint_id,
        parent_type="endpoint",
        key_data=key_data,
    )
    return await pipeline.run(adapter=adapter, http_request=request, db=db, mode=adapter.mode)


@router.put("/keys/{key_id}", response_model=EndpointAPIKeyResponse)
async def update_endpoint_key(
    key_id: str,
    key_data: EndpointAPIKeyUpdate,
    request: Request,
    db: Session = Depends(get_db),
) -> EndpointAPIKeyResponse:
    """
    更新 Endpoint Key

    更新指定 Key 的配置，支持修改并发限制、速率倍数、优先级、
    配额限制、能力限制等。支持部分更新。

    **路径参数**:
    - `key_id`: Key ID

    **请求体字段**（均为可选）:
    - `api_key`: 新的 API Key 原文
    - `name`: Key 名称
    - `note`: 备注
    - `rate_multiplier`: 速率倍数
    - `internal_priority`: 内部优先级
    - `max_concurrent`: 最大并发数（设置为 null 可切换到自适应模式）
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
        parent_type="endpoint",
    )
    return await pipeline.run(adapter=adapter, http_request=request, db=db, mode=adapter.mode)


@router.get("/keys/grouped-by-format")
async def get_keys_grouped_by_format(
    request: Request,
    db: Session = Depends(get_db),
) -> dict:
    """
    获取按 API 格式分组的所有 Keys

    获取所有活跃的 Key，按 API 格式分组返回，用于全局优先级管理。
    每个 Key 包含基本信息、健康度指标、能力标签等。

    **返回字段**:
    - 返回一个字典，键为 API 格式，值为该格式下的 Key 列表
    - 每个 Key 包含：
      - `id`: Key ID
      - `name`: Key 名称
      - `api_key_masked`: 脱敏后的 API Key
      - `internal_priority`: 内部优先级
      - `global_priority`: 全局优先级
      - `rate_multiplier`: 速率倍数
      - `is_active`: 是否活跃
      - `circuit_breaker_open`: 熔断器状态
      - `provider_name`: Provider 名称
      - `endpoint_base_url`: Endpoint 基础 URL
      - `api_format`: API 格式
      - `capabilities`: 能力简称列表
      - `success_rate`: 成功率
      - `avg_response_time_ms`: 平均响应时间
      - `request_count`: 请求总数
    """
    adapter = AdminGetKeysGroupedByFormatAdapter()
    return await pipeline.run(adapter=adapter, http_request=request, db=db, mode=adapter.mode)


@router.get("/keys/{key_id}/reveal")
async def reveal_endpoint_key(
    key_id: str,
    request: Request,
    db: Session = Depends(get_db),
) -> dict:
    """
    获取完整的 API Key

    解密并返回指定 Key 的完整原文，用于查看和复制。
    此操作会被记录到审计日志。

    **路径参数**:
    - `key_id`: Key ID

    **返回字段**:
    - `api_key`: 完整的 API Key 原文
    """
    adapter = AdminRevealKeyAdapter(
        key_id=key_id,
        parent_type="endpoint",
    )
    return await pipeline.run(adapter=adapter, http_request=request, db=db, mode=adapter.mode)


@router.delete("/keys/{key_id}")
async def delete_endpoint_key(
    key_id: str,
    request: Request,
    db: Session = Depends(get_db),
) -> dict:
    """
    删除 Endpoint Key

    删除指定的 API Key。此操作不可逆，请谨慎使用。

    **路径参数**:
    - `key_id`: Key ID

    **返回字段**:
    - `message`: 操作结果消息
    """
    adapter = AdminDeleteKeyAdapter(
        key_id=key_id,
        parent_type="endpoint",
    )
    return await pipeline.run(adapter=adapter, http_request=request, db=db, mode=adapter.mode)


@router.put("/{endpoint_id}/keys/batch-priority")
async def batch_update_key_priority(
    endpoint_id: str,
    request: Request,
    priority_data: BatchUpdateKeyPriorityRequest,
    db: Session = Depends(get_db),
) -> dict:
    """
    批量更新 Endpoint 下 Keys 的优先级

    批量更新指定 Endpoint 下多个 Key 的内部优先级，用于拖动排序。
    所有 Key 必须属于指定的 Endpoint。

    **路径参数**:
    - `endpoint_id`: Endpoint ID

    **请求体字段**:
    - `priorities`: 优先级列表
      - `key_id`: Key ID
      - `internal_priority`: 新的内部优先级

    **返回字段**:
    - `message`: 操作结果消息
    - `updated_count`: 实际更新的 Key 数量
    """
    adapter = AdminBatchUpdateKeyPriorityAdapter(
        parent_id=endpoint_id,
        parent_type="endpoint",
        priority_data=priority_data,
    )
    return await pipeline.run(adapter=adapter, http_request=request, db=db, mode=adapter.mode)