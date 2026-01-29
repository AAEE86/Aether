"""
Provider 操作 API 路由

提供 Provider 操作相关的 API 端点：
- 架构列表
- 连接管理
- 操作执行（余额查询、签到等）
- 配置管理
"""

from dataclasses import asdict, is_dataclass
from typing import Any

from fastapi import APIRouter, Depends, HTTPException
from pydantic import BaseModel, Field
from sqlalchemy.orm import Session

from src.database import get_db
from src.models.database import Provider, User
from src.services.provider_ops import (
    ConnectorAuthType,
    ProviderActionType,
    ProviderOpsConfig,
    ProviderOpsService,
    get_registry,
)
from src.utils.auth_utils import require_admin

router = APIRouter(prefix="/api/admin/provider-ops", tags=["Provider Operations"])


# ==================== Request/Response Models ====================


class ArchitectureInfo(BaseModel):
    """架构信息"""

    architecture_id: str
    display_name: str
    description: str
    supported_auth_types: list[dict[str, str]]
    supported_actions: list[dict[str, Any]]
    default_connector: str | None


class ConnectorConfigRequest(BaseModel):
    """连接器配置请求"""

    auth_type: str = Field(..., description="认证类型")
    config: dict[str, Any] = Field(default_factory=dict, description="连接器配置")
    credentials: dict[str, Any] = Field(default_factory=dict, description="凭据信息")


class ActionConfigRequest(BaseModel):
    """操作配置请求"""

    enabled: bool = Field(True, description="是否启用")
    config: dict[str, Any] = Field(default_factory=dict, description="操作配置")


class SaveConfigRequest(BaseModel):
    """保存配置请求"""

    architecture_id: str = Field("generic_api", description="架构 ID")
    base_url: str | None = Field(None, description="API 基础地址")
    connector: ConnectorConfigRequest
    actions: dict[str, ActionConfigRequest] = Field(default_factory=dict)
    schedule: dict[str, str] = Field(default_factory=dict, description="定时任务配置")


class ConnectRequest(BaseModel):
    """连接请求"""

    credentials: dict[str, Any] | None = Field(None, description="凭据（可选，使用已保存的）")


class ExecuteActionRequest(BaseModel):
    """执行操作请求"""

    config: dict[str, Any] | None = Field(None, description="操作配置（覆盖默认）")


class ConnectionStatusResponse(BaseModel):
    """连接状态响应"""

    status: str
    auth_type: str
    connected_at: str | None
    expires_at: str | None
    last_error: str | None


class ActionResultResponse(BaseModel):
    """操作结果响应"""

    status: str
    action_type: str
    data: Any | None
    message: str | None
    executed_at: str
    response_time_ms: int | None
    cache_ttl_seconds: int


class ProviderOpsStatusResponse(BaseModel):
    """Provider 操作状态响应"""

    provider_id: str
    is_configured: bool
    architecture_id: str | None
    connection_status: ConnectionStatusResponse
    enabled_actions: list[str]


class ProviderOpsConfigResponse(BaseModel):
    """Provider 操作配置响应（脱敏）"""

    provider_id: str
    is_configured: bool
    architecture_id: str | None = None
    base_url: str | None = None
    connector: dict[str, Any] | None = None  # 脱敏后的连接器配置


class VerifyAuthResponse(BaseModel):
    """验证认证响应"""

    success: bool
    message: str | None = None
    data: dict[str, Any] | None = None


# ==================== Helper Functions ====================


def _serialize_data(data: Any) -> Any:
    """序列化 dataclass 为字典，用于 JSON 响应"""
    if data is None:
        return None
    if is_dataclass(data) and not isinstance(data, type):
        return asdict(data)
    return data


# ==================== Routes ====================


@router.get("/architectures", response_model=list[ArchitectureInfo])
async def list_architectures(_: User = Depends(require_admin)):
    """获取所有可用的架构"""
    registry = get_registry()
    return registry.to_dict_list()


@router.get("/architectures/{architecture_id}", response_model=ArchitectureInfo)
async def get_architecture(architecture_id: str, _: User = Depends(require_admin)):
    """获取指定架构的详情"""
    registry = get_registry()
    arch = registry.get(architecture_id)
    if not arch:
        raise HTTPException(status_code=404, detail=f"架构 {architecture_id} 不存在")
    return arch.to_dict()


@router.get("/providers/{provider_id}/status", response_model=ProviderOpsStatusResponse)
async def get_provider_ops_status(
    provider_id: str,
    db: Session = Depends(get_db),
    _: User = Depends(require_admin),
):
    """获取 Provider 的操作状态"""
    service = ProviderOpsService(db)

    config = service.get_config(provider_id)
    conn_state = service.get_connection_status(provider_id)

    enabled_actions = []
    if config:
        for action_type, action_config in config.actions.items():
            if action_config.get("enabled", True):
                enabled_actions.append(action_type)

    return ProviderOpsStatusResponse(
        provider_id=provider_id,
        is_configured=config is not None,
        architecture_id=config.architecture_id if config else None,
        connection_status=ConnectionStatusResponse(
            status=conn_state.status.value,
            auth_type=conn_state.auth_type.value,
            connected_at=conn_state.connected_at.isoformat() if conn_state.connected_at else None,
            expires_at=conn_state.expires_at.isoformat() if conn_state.expires_at else None,
            last_error=conn_state.last_error,
        ),
        enabled_actions=enabled_actions,
    )


@router.get("/providers/{provider_id}/config", response_model=ProviderOpsConfigResponse)
async def get_provider_ops_config(
    provider_id: str,
    db: Session = Depends(get_db),
    _: User = Depends(require_admin),
):
    """
    获取 Provider 的操作配置（脱敏）

    返回已保存的配置，但敏感字段（如 api_key）会被脱敏处理。
    """
    service = ProviderOpsService(db)
    config = service.get_config(provider_id)

    if not config:
        return ProviderOpsConfigResponse(
            provider_id=provider_id,
            is_configured=False,
        )

    # 获取 base_url：优先从 provider_ops 配置读取，否则回退到 endpoint/provider
    base_url = config.base_url
    if not base_url:
        provider = db.query(Provider).filter(Provider.id == provider_id).first()
        if provider:
            if provider.endpoints:
                for endpoint in provider.endpoints:
                    if endpoint.base_url:
                        base_url = endpoint.base_url
                        break
            if not base_url:
                provider_config = provider.config or {}
                base_url = provider_config.get("base_url") or provider.website

    # 获取脱敏后的凭据
    masked_credentials = service.get_masked_credentials(config.connector_credentials)

    return ProviderOpsConfigResponse(
        provider_id=provider_id,
        is_configured=True,
        architecture_id=config.architecture_id,
        base_url=base_url,
        connector={
            "auth_type": config.connector_auth_type.value,
            "config": config.connector_config,
            "credentials": masked_credentials,
        },
    )


@router.put("/providers/{provider_id}/config")
async def save_provider_ops_config(
    provider_id: str,
    request: SaveConfigRequest,
    db: Session = Depends(get_db),
    _: User = Depends(require_admin),
):
    """保存 Provider 的操作配置"""
    service = ProviderOpsService(db)

    # 合并凭据：如果请求中的敏感字段为空，使用已保存的凭据
    credentials = service.merge_credentials_with_saved(
        provider_id, dict(request.connector.credentials)
    )

    # 构建配置对象
    config = ProviderOpsConfig(
        architecture_id=request.architecture_id,
        base_url=request.base_url,
        connector_auth_type=ConnectorAuthType(request.connector.auth_type),
        connector_config=request.connector.config,
        connector_credentials=credentials,
        actions={
            action_type: {"enabled": action_config.enabled, "config": action_config.config}
            for action_type, action_config in request.actions.items()
        },
        schedule=request.schedule,
    )

    success = service.save_config(provider_id, config)
    if not success:
        raise HTTPException(status_code=404, detail="Provider 不存在")

    return {"success": True, "message": "配置保存成功"}


@router.post("/providers/{provider_id}/verify", response_model=VerifyAuthResponse)
async def verify_provider_auth(
    provider_id: str,
    request: SaveConfigRequest,
    db: Session = Depends(get_db),
    _: User = Depends(require_admin),
):
    """
    验证 Provider 认证配置

    在保存前测试认证是否有效。
    如果凭据中的敏感字段为空，会使用已保存的凭据。
    """
    service = ProviderOpsService(db)

    # 获取 base_url
    base_url = request.base_url
    if not base_url:
        # 尝试从 Provider 获取
        provider = db.query(Provider).filter(Provider.id == provider_id).first()
        if provider:
            # 从 endpoints 或 config 获取
            if provider.endpoints:
                for endpoint in provider.endpoints:
                    if endpoint.base_url:
                        base_url = endpoint.base_url
                        break
            if not base_url and provider.config:
                base_url = provider.config.get("base_url")

    if not base_url:
        return VerifyAuthResponse(
            success=False,
            message="请提供 API 地址",
        )

    # 合并凭据：如果请求中的敏感字段为空，使用已保存的凭据
    credentials = service.merge_credentials_with_saved(
        provider_id, dict(request.connector.credentials)
    )

    result = await service.verify_auth(
        base_url=base_url,
        architecture_id=request.architecture_id,
        auth_type=ConnectorAuthType(request.connector.auth_type),
        config=request.connector.config,
        credentials=credentials,
        provider_id=provider_id,
    )

    return VerifyAuthResponse(
        success=result.get("success", False),
        message=result.get("message"),
        data=result.get("data"),
    )


@router.delete("/providers/{provider_id}/config")
async def delete_provider_ops_config(
    provider_id: str,
    db: Session = Depends(get_db),
    _: User = Depends(require_admin),
):
    """删除 Provider 的操作配置"""
    service = ProviderOpsService(db)
    success = service.delete_config(provider_id)

    if not success:
        raise HTTPException(status_code=404, detail="Provider 不存在")

    return {"success": True, "message": "配置已删除"}


@router.post("/providers/{provider_id}/connect")
async def connect_provider(
    provider_id: str,
    request: ConnectRequest,
    db: Session = Depends(get_db),
    _: User = Depends(require_admin),
):
    """建立与 Provider 的连接"""
    service = ProviderOpsService(db)

    success, message = await service.connect(provider_id, request.credentials)

    if not success:
        raise HTTPException(status_code=400, detail=message)

    return {"success": True, "message": message}


@router.post("/providers/{provider_id}/disconnect")
async def disconnect_provider(
    provider_id: str,
    db: Session = Depends(get_db),
    _: User = Depends(require_admin),
):
    """断开与 Provider 的连接"""
    service = ProviderOpsService(db)
    await service.disconnect(provider_id)

    return {"success": True, "message": "已断开连接"}


@router.post(
    "/providers/{provider_id}/actions/{action_type}",
    response_model=ActionResultResponse,
)
async def execute_action(
    provider_id: str,
    action_type: str,
    request: ExecuteActionRequest,
    db: Session = Depends(get_db),
    _: User = Depends(require_admin),
):
    """执行指定操作"""
    service = ProviderOpsService(db)

    try:
        action_type_enum = ProviderActionType(action_type)
    except ValueError:
        raise HTTPException(status_code=400, detail=f"无效的操作类型: {action_type}")

    result = await service.execute_action(provider_id, action_type_enum, request.config)

    return ActionResultResponse(
        status=result.status.value,
        action_type=result.action_type.value,
        data=_serialize_data(result.data),
        message=result.message,
        executed_at=result.executed_at.isoformat(),
        response_time_ms=result.response_time_ms,
        cache_ttl_seconds=result.cache_ttl_seconds,
    )


@router.get("/providers/{provider_id}/balance", response_model=ActionResultResponse)
async def get_balance(
    provider_id: str,
    refresh: bool = True,
    db: Session = Depends(get_db),
    _: User = Depends(require_admin),
):
    """
    获取余额（优先返回缓存，后台异步刷新）

    - refresh=True（默认）：返回缓存并触发后台刷新
    - refresh=False：仅返回缓存，不触发刷新
    """
    service = ProviderOpsService(db)
    result = await service.query_balance_with_cache(provider_id, trigger_refresh=refresh)

    return ActionResultResponse(
        status=result.status.value,
        action_type=result.action_type.value,
        data=_serialize_data(result.data),
        message=result.message,
        executed_at=result.executed_at.isoformat(),
        response_time_ms=result.response_time_ms,
        cache_ttl_seconds=result.cache_ttl_seconds,
    )


@router.post("/providers/{provider_id}/balance", response_model=ActionResultResponse)
async def refresh_balance(
    provider_id: str,
    db: Session = Depends(get_db),
    _: User = Depends(require_admin),
):
    """立即刷新余额（同步等待结果）"""
    service = ProviderOpsService(db)
    result = await service.query_balance(provider_id)

    return ActionResultResponse(
        status=result.status.value,
        action_type=result.action_type.value,
        data=_serialize_data(result.data),
        message=result.message,
        executed_at=result.executed_at.isoformat(),
        response_time_ms=result.response_time_ms,
        cache_ttl_seconds=result.cache_ttl_seconds,
    )


@router.post("/providers/{provider_id}/checkin", response_model=ActionResultResponse)
async def checkin(
    provider_id: str,
    db: Session = Depends(get_db),
    _: User = Depends(require_admin),
):
    """签到（快捷方法）"""
    service = ProviderOpsService(db)
    result = await service.checkin(provider_id)

    return ActionResultResponse(
        status=result.status.value,
        action_type=result.action_type.value,
        data=_serialize_data(result.data),
        message=result.message,
        executed_at=result.executed_at.isoformat(),
        response_time_ms=result.response_time_ms,
        cache_ttl_seconds=result.cache_ttl_seconds,
    )


@router.post("/batch/balance")
async def batch_query_balance(
    provider_ids: list[str] | None = None,
    db: Session = Depends(get_db),
    _: User = Depends(require_admin),
):
    """批量查询余额"""
    service = ProviderOpsService(db)
    results = await service.batch_query_balance(provider_ids)

    return {
        provider_id: ActionResultResponse(
            status=result.status.value,
            action_type=result.action_type.value,
            data=_serialize_data(result.data),
            message=result.message,
            executed_at=result.executed_at.isoformat(),
            response_time_ms=result.response_time_ms,
            cache_ttl_seconds=result.cache_ttl_seconds,
        )
        for provider_id, result in results.items()
    }
