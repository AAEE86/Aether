"""管理员独立余额 API Key 管理路由。

独立余额Key：不关联用户配额，有独立余额限制，用于给非注册用户使用。
"""

import os
from datetime import datetime, timedelta, timezone
from zoneinfo import ZoneInfo

from fastapi import APIRouter, Depends, HTTPException, Query, Request
from sqlalchemy.orm import Session

from src.api.base.admin_adapter import AdminApiAdapter
from src.api.base.pipeline import ApiRequestPipeline
from src.core.exceptions import InvalidRequestException, NotFoundException
from src.core.logger import logger
from src.database import get_db
from src.models.api import CreateApiKeyRequest
from src.models.database import ApiKey
from src.services.user.apikey import ApiKeyService


# 应用时区配置，默认为 Asia/Shanghai
APP_TIMEZONE = ZoneInfo(os.getenv("APP_TIMEZONE", "Asia/Shanghai"))


def parse_expiry_date(date_str: str | None) -> datetime | None:
    """解析过期日期字符串为 datetime 对象。

    Args:
        date_str: 日期字符串，支持 "YYYY-MM-DD" 或 ISO 格式

    Returns:
        datetime 对象（当天 23:59:59.999999，应用时区），或 None 如果输入为空

    Raises:
        BadRequestException: 日期格式无效
    """
    if not date_str or not date_str.strip():
        return None

    date_str = date_str.strip()

    # 尝试 YYYY-MM-DD 格式
    try:
        parsed_date = datetime.strptime(date_str, "%Y-%m-%d")
        # 设置为当天结束时间 (23:59:59.999999，应用时区)
        return parsed_date.replace(
            hour=23, minute=59, second=59, microsecond=999999, tzinfo=APP_TIMEZONE
        )
    except ValueError:
        pass

    # 尝试完整 ISO 格式
    try:
        return datetime.fromisoformat(date_str.replace("Z", "+00:00"))
    except ValueError:
        pass

    raise InvalidRequestException(f"无效的日期格式: {date_str}，请使用 YYYY-MM-DD 格式")


router = APIRouter(prefix="/api/admin/api-keys", tags=["Admin - API Keys (Standalone)"])
pipeline = ApiRequestPipeline()


@router.get("")
async def list_standalone_api_keys(
    request: Request,
    skip: int = Query(0, ge=0),
    limit: int = Query(100, ge=1, le=500),
    is_active: bool | None = None,
    db: Session = Depends(get_db),
):
    """
    列出所有独立余额 API Keys

    获取系统中所有独立余额 API Key 的列表。独立余额 Key 不关联用户配额，
    有独立的余额限制，主要用于给非注册用户使用。

    **查询参数**:
    - `skip`: 跳过的记录数（分页偏移量），默认 0
    - `limit`: 返回的记录数（分页限制），默认 100，最大 500
    - `is_active`: 可选，根据启用状态筛选（true/false）

    **返回字段**:
    - `api_keys`: API Key 列表，包含 id, name, key_display, is_active, current_balance_usd,
      balance_used_usd, total_requests, total_cost_usd, rate_limit, allowed_providers,
      allowed_api_formats, allowed_models, last_used_at, expires_at, created_at, updated_at,
      auto_delete_on_expiry 等字段
    - `total`: 符合条件的总记录数
    - `limit`: 当前分页限制
    - `skip`: 当前分页偏移量
    """
    adapter = AdminListStandaloneKeysAdapter(skip=skip, limit=limit, is_active=is_active)
    return await pipeline.run(adapter=adapter, http_request=request, db=db, mode=adapter.mode)


@router.post("")
async def create_standalone_api_key(
    request: Request,
    key_data: CreateApiKeyRequest,
    db: Session = Depends(get_db),
):
    """
    创建独立余额 API Key

    创建一个新的独立余额 API Key。独立余额 Key 必须设置初始余额限制。

    **请求体字段**:
    - `name`: API Key 的名称
    - `initial_balance_usd`: 必需，初始余额（美元），必须大于 0
    - `allowed_providers`: 可选，允许使用的提供商列表
    - `allowed_api_formats`: 可选，允许使用的 API 格式列表
    - `allowed_models`: 可选，允许使用的模型列表
    - `rate_limit`: 可选，速率限制配置（请求数/秒）
    - `expire_days`: 可选，过期天数（兼容旧版）
    - `expires_at`: 可选，过期时间（ISO 格式或 YYYY-MM-DD 格式，优先级高于 expire_days）
    - `auto_delete_on_expiry`: 可选，过期后是否自动删除

    **返回字段**:
    - `id`: API Key ID
    - `key`: 完整的 API Key（仅在创建时返回一次）
    - `name`: API Key 名称
    - `key_display`: 脱敏显示的 Key
    - `is_standalone`: 是否为独立余额 Key（始终为 true）
    - `current_balance_usd`: 当前余额
    - `balance_used_usd`: 已使用余额
    - `rate_limit`: 速率限制配置
    - `expires_at`: 过期时间
    - `created_at`: 创建时间
    - `message`: 提示信息
    """
    adapter = AdminCreateStandaloneKeyAdapter(key_data=key_data)
    return await pipeline.run(adapter=adapter, http_request=request, db=db, mode=adapter.mode)


@router.put("/{key_id}")
async def update_api_key(
    key_id: str, request: Request, key_data: CreateApiKeyRequest, db: Session = Depends(get_db)
):
    """
    更新独立余额 API Key

    更新指定 ID 的独立余额 API Key 的配置信息。

    **路径参数**:
    - `key_id`: API Key ID

    **请求体字段**:
    - `name`: 可选，API Key 的名称
    - `rate_limit`: 可选，速率限制配置（null 表示无限制）
    - `allowed_providers`: 可选，允许使用的提供商列表
    - `allowed_api_formats`: 可选，允许使用的 API 格式列表
    - `allowed_models`: 可选，允许使用的模型列表
    - `expire_days`: 可选，过期天数（兼容旧版）
    - `expires_at`: 可选，过期时间（ISO 格式或 YYYY-MM-DD 格式，优先级高于 expire_days，null 或空字符串表示永不过期）
    - `auto_delete_on_expiry`: 可选，过期后是否自动删除

    **返回字段**:
    - `id`: API Key ID
    - `name`: API Key 名称
    - `key_display`: 脱敏显示的 Key
    - `is_active`: 是否启用
    - `current_balance_usd`: 当前余额
    - `balance_used_usd`: 已使用余额
    - `rate_limit`: 速率限制配置
    - `expires_at`: 过期时间
    - `updated_at`: 更新时间
    - `message`: 提示信息
    """
    adapter = AdminUpdateApiKeyAdapter(key_id=key_id, key_data=key_data)
    return await pipeline.run(adapter=adapter, http_request=request, db=db, mode=adapter.mode)


@router.patch("/{key_id}")
async def toggle_api_key(key_id: str, request: Request, db: Session = Depends(get_db)):
    """
    切换 API Key 启用状态

    切换指定 API Key 的启用/禁用状态。

    **路径参数**:
    - `key_id`: API Key ID

    **返回字段**:
    - `id`: API Key ID
    - `is_active`: 新的启用状态
    - `message`: 提示信息
    """
    adapter = AdminToggleApiKeyAdapter(key_id=key_id)
    return await pipeline.run(adapter=adapter, http_request=request, db=db, mode=adapter.mode)


@router.delete("/{key_id}")
async def delete_api_key(key_id: str, request: Request, db: Session = Depends(get_db)):
    """
    删除 API Key

    删除指定的 API Key。此操作不可逆。

    **路径参数**:
    - `key_id`: API Key ID

    **返回字段**:
    - `message`: 提示信息
    """
    adapter = AdminDeleteApiKeyAdapter(key_id=key_id)
    return await pipeline.run(adapter=adapter, http_request=request, db=db, mode=adapter.mode)


@router.patch("/{key_id}/lock")
async def toggle_lock_api_key(key_id: str, request: Request, db: Session = Depends(get_db)):
    """
    切换 API Key 锁定状态

    锁定/解锁指定的 API Key。锁定后用户无法使用和操作此密钥。

    **路径参数**:
    - `key_id`: API Key ID

    **返回字段**:
    - `id`: API Key ID
    - `is_locked`: 新的锁定状态
    - `message`: 提示信息
    """
    adapter = AdminToggleLockApiKeyAdapter(key_id=key_id)
    return await pipeline.run(adapter=adapter, http_request=request, db=db, mode=adapter.mode)


@router.patch("/{key_id}/balance")
async def add_balance_to_key(
    key_id: str,
    request: Request,
    db: Session = Depends(get_db),
):
    """
    调整独立余额 API Key 的余额

    为指定的独立余额 API Key 增加或扣除余额。

    **路径参数**:
    - `key_id`: API Key ID

    **请求体字段**:
    - `amount_usd`: 调整金额（美元），正数为充值，负数为扣除

    **返回字段**:
    - `id`: API Key ID
    - `name`: API Key 名称
    - `current_balance_usd`: 调整后的当前余额
    - `balance_used_usd`: 已使用余额
    - `message`: 提示信息
    """
    # 从请求体获取调整金额
    body = await request.json()
    amount_usd = body.get("amount_usd")

    # 参数校验
    if amount_usd is None:
        raise HTTPException(status_code=400, detail="缺少必需参数: amount_usd")

    if amount_usd == 0:
        raise HTTPException(status_code=400, detail="调整金额不能为 0")

    # 类型校验
    try:
        amount_usd = float(amount_usd)
    except (ValueError, TypeError):
        raise HTTPException(status_code=400, detail="调整金额必须是有效数字")

    # 如果是扣除操作,检查Key是否存在以及余额是否充足
    if amount_usd < 0:
        api_key = db.query(ApiKey).filter(ApiKey.id == key_id).first()
        if not api_key:
            raise HTTPException(status_code=404, detail="API密钥不存在")

        if not api_key.is_standalone:
            raise HTTPException(status_code=400, detail="只能为独立余额Key调整余额")

        if api_key.current_balance_usd is not None:
            if abs(amount_usd) > api_key.current_balance_usd:
                raise HTTPException(
                    status_code=400,
                    detail=f"扣除金额 ${abs(amount_usd):.2f} 超过当前余额 ${api_key.current_balance_usd:.2f}",
                )

    adapter = AdminAddBalanceAdapter(key_id=key_id, amount_usd=amount_usd)
    return await pipeline.run(adapter=adapter, http_request=request, db=db, mode=adapter.mode)


@router.get("/{key_id}")
async def get_api_key_detail(
    key_id: str,
    request: Request,
    include_key: bool = Query(False, description="Include full decrypted key in response"),
    db: Session = Depends(get_db),
):
    """
    获取 API Key 详情

    获取指定 API Key 的详细信息。可选择是否返回完整的解密密钥。

    **路径参数**:
    - `key_id`: API Key ID

    **查询参数**:
    - `include_key`: 是否包含完整的解密密钥，默认 false

    **返回字段**:
    - 当 include_key=false 时，返回基本信息：id, user_id, name, key_display, is_active,
      is_standalone, current_balance_usd, balance_used_usd, total_requests, total_cost_usd,
      rate_limit, allowed_providers, allowed_api_formats, allowed_models, last_used_at,
      expires_at, created_at, updated_at
    - 当 include_key=true 时，返回完整密钥：key
    """
    if include_key:
        adapter = AdminGetFullKeyAdapter(key_id=key_id)
    else:
        # Return basic key info without full key
        adapter = AdminGetKeyDetailAdapter(key_id=key_id)
    return await pipeline.run(adapter=adapter, http_request=request, db=db, mode=adapter.mode)


class AdminListStandaloneKeysAdapter(AdminApiAdapter):
    """列出独立余额Keys"""

    def __init__(
        self,
        skip: int,
        limit: int,
        is_active: bool | None,
    ):
        self.skip = skip
        self.limit = limit
        self.is_active = is_active

    async def handle(self, context):  # type: ignore[override]
        db = context.db
        # 只查询独立余额Keys
        query = db.query(ApiKey).filter(ApiKey.is_standalone == True)

        if self.is_active is not None:
            query = query.filter(ApiKey.is_active == self.is_active)

        total = query.count()
        api_keys = (
            query.order_by(ApiKey.created_at.desc()).offset(self.skip).limit(self.limit).all()
        )

        context.add_audit_metadata(
            action="list_standalone_api_keys",
            filter_is_active=self.is_active,
            limit=self.limit,
            skip=self.skip,
            total=total,
        )

        return {
            "api_keys": [
                {
                    "id": api_key.id,
                    "user_id": api_key.user_id,  # 创建者ID
                    "name": api_key.name,
                    "key_display": api_key.get_display_key(),
                    "is_active": api_key.is_active,
                    "is_locked": api_key.is_locked,
                    "is_standalone": api_key.is_standalone,
                    "current_balance_usd": api_key.current_balance_usd,
                    "balance_used_usd": float(api_key.balance_used_usd or 0),
                    "total_requests": api_key.total_requests,
                    "total_cost_usd": float(api_key.total_cost_usd or 0),
                    "rate_limit": api_key.rate_limit,
                    "allowed_providers": api_key.allowed_providers,
                    "allowed_api_formats": api_key.allowed_api_formats,
                    "allowed_models": api_key.allowed_models,
                    "last_used_at": (
                        api_key.last_used_at.isoformat() if api_key.last_used_at else None
                    ),
                    "expires_at": api_key.expires_at.isoformat() if api_key.expires_at else None,
                    "created_at": api_key.created_at.isoformat(),
                    "updated_at": api_key.updated_at.isoformat() if api_key.updated_at else None,
                    "auto_delete_on_expiry": api_key.auto_delete_on_expiry,
                }
                for api_key in api_keys
            ],
            "total": total,
            "limit": self.limit,
            "skip": self.skip,
        }


class AdminCreateStandaloneKeyAdapter(AdminApiAdapter):
    """创建独立余额Key"""

    def __init__(self, key_data: CreateApiKeyRequest):
        self.key_data = key_data

    async def handle(self, context):  # type: ignore[override]
        db = context.db

        # 独立Key必须设置初始余额
        if not self.key_data.initial_balance_usd or self.key_data.initial_balance_usd <= 0:
            raise HTTPException(
                status_code=400,
                detail="创建独立余额Key必须设置有效的初始余额（initial_balance_usd > 0）",
            )

        # 独立Key需要关联到管理员用户（从context获取）
        admin_user_id = context.user.id

        # 解析过期时间（优先使用 expires_at，其次使用 expire_days）
        expires_at_dt = parse_expiry_date(self.key_data.expires_at)

        # 创建独立Key
        api_key, plain_key = ApiKeyService.create_api_key(
            db=db,
            user_id=admin_user_id,  # 关联到创建者
            name=self.key_data.name,
            allowed_providers=self.key_data.allowed_providers,
            allowed_api_formats=self.key_data.allowed_api_formats,
            allowed_models=self.key_data.allowed_models,
            rate_limit=self.key_data.rate_limit,  # None 表示不限制
            expire_days=self.key_data.expire_days,  # 兼容旧版
            expires_at=expires_at_dt,  # 优先使用
            initial_balance_usd=self.key_data.initial_balance_usd,
            is_standalone=True,  # 标记为独立Key
            auto_delete_on_expiry=self.key_data.auto_delete_on_expiry,
        )

        logger.info(f"管理员创建独立余额Key: ID {api_key.id}, 初始余额 ${self.key_data.initial_balance_usd}")

        context.add_audit_metadata(
            action="create_standalone_api_key",
            key_id=api_key.id,
            initial_balance_usd=self.key_data.initial_balance_usd,
        )

        return {
            "id": api_key.id,
            "key": plain_key,  # 只在创建时返回完整密钥
            "name": api_key.name,
            "key_display": api_key.get_display_key(),
            "is_standalone": True,
            "current_balance_usd": api_key.current_balance_usd,
            "balance_used_usd": 0.0,
            "rate_limit": api_key.rate_limit,
            "expires_at": api_key.expires_at.isoformat() if api_key.expires_at else None,
            "created_at": api_key.created_at.isoformat(),
            "message": "独立余额Key创建成功，请妥善保存完整密钥，后续将无法查看",
        }


class AdminUpdateApiKeyAdapter(AdminApiAdapter):
    """更新独立余额Key"""

    def __init__(self, key_id: str, key_data: CreateApiKeyRequest):
        self.key_id = key_id
        self.key_data = key_data

    async def handle(self, context):  # type: ignore[override]
        db = context.db
        api_key = db.query(ApiKey).filter(ApiKey.id == self.key_id).first()
        if not api_key:
            raise NotFoundException("API密钥不存在", "api_key")

        # 构建更新数据
        update_data = {}
        if self.key_data.name is not None:
            update_data["name"] = self.key_data.name
        # rate_limit: 显式传递时更新（包括 null 表示无限制）
        if "rate_limit" in self.key_data.model_fields_set:
            update_data["rate_limit"] = self.key_data.rate_limit
        if (
            hasattr(self.key_data, "auto_delete_on_expiry")
            and self.key_data.auto_delete_on_expiry is not None
        ):
            update_data["auto_delete_on_expiry"] = self.key_data.auto_delete_on_expiry

        # 访问限制配置（允许设置为空数组来清除限制）
        if hasattr(self.key_data, "allowed_providers"):
            update_data["allowed_providers"] = self.key_data.allowed_providers
        if hasattr(self.key_data, "allowed_api_formats"):
            update_data["allowed_api_formats"] = self.key_data.allowed_api_formats
        if hasattr(self.key_data, "allowed_models"):
            update_data["allowed_models"] = self.key_data.allowed_models

        # 处理过期时间
        # 优先使用 expires_at（如果显式传递且有值）
        if self.key_data.expires_at and self.key_data.expires_at.strip():
            update_data["expires_at"] = parse_expiry_date(self.key_data.expires_at)
        elif "expires_at" in self.key_data.model_fields_set:
            # expires_at 明确传递为 null 或空字符串，设为永不过期
            update_data["expires_at"] = None
        # 兼容旧版 expire_days
        elif "expire_days" in self.key_data.model_fields_set:
            if self.key_data.expire_days is not None and self.key_data.expire_days > 0:
                update_data["expires_at"] = datetime.now(timezone.utc) + timedelta(
                    days=self.key_data.expire_days
                )
            else:
                # expire_days = None/0/负数 表示永不过期
                update_data["expires_at"] = None

        # 使用 ApiKeyService 更新
        updated_key = ApiKeyService.update_api_key(db, self.key_id, **update_data)
        if not updated_key:
            raise NotFoundException("更新失败", "api_key")

        logger.info(f"管理员更新独立余额Key: ID {self.key_id}, 更新字段 {list(update_data.keys())}")

        context.add_audit_metadata(
            action="update_standalone_api_key",
            key_id=self.key_id,
            updated_fields=list(update_data.keys()),
        )

        return {
            "id": updated_key.id,
            "name": updated_key.name,
            "key_display": updated_key.get_display_key(),
            "is_active": updated_key.is_active,
            "current_balance_usd": updated_key.current_balance_usd,
            "balance_used_usd": float(updated_key.balance_used_usd or 0),
            "rate_limit": updated_key.rate_limit,
            "expires_at": updated_key.expires_at.isoformat() if updated_key.expires_at else None,
            "updated_at": updated_key.updated_at.isoformat() if updated_key.updated_at else None,
            "message": "API密钥已更新",
        }


class AdminToggleApiKeyAdapter(AdminApiAdapter):
    def __init__(self, key_id: str):
        self.key_id = key_id

    async def handle(self, context):  # type: ignore[override]
        db = context.db
        api_key = db.query(ApiKey).filter(ApiKey.id == self.key_id).first()
        if not api_key:
            raise NotFoundException("API密钥不存在", "api_key")

        api_key.is_active = not api_key.is_active
        api_key.updated_at = datetime.now(timezone.utc)
        db.commit()
        db.refresh(api_key)

        logger.info(f"管理员切换API密钥状态: Key ID {self.key_id}, 新状态 {'启用' if api_key.is_active else '禁用'}")

        context.add_audit_metadata(
            action="toggle_api_key",
            target_key_id=api_key.id,
            user_id=api_key.user_id,
            new_status="enabled" if api_key.is_active else "disabled",
        )

        return {
            "id": api_key.id,
            "is_active": api_key.is_active,
            "message": f"API密钥已{'启用' if api_key.is_active else '禁用'}",
        }


class AdminToggleLockApiKeyAdapter(AdminApiAdapter):
    """切换API密钥锁定状态"""

    def __init__(self, key_id: str):
        self.key_id = key_id

    async def handle(self, context):  # type: ignore[override]
        db = context.db
        api_key = db.query(ApiKey).filter(ApiKey.id == self.key_id).first()
        if not api_key:
            raise NotFoundException("API密钥不存在", "api_key")

        api_key.is_locked = not api_key.is_locked
        api_key.updated_at = datetime.now(timezone.utc)
        db.commit()
        db.refresh(api_key)

        logger.info(f"管理员切换API密钥锁定状态: Key ID {self.key_id}, 新状态 {'锁定' if api_key.is_locked else '解锁'}")

        context.add_audit_metadata(
            action="toggle_lock_api_key",
            target_key_id=api_key.id,
            user_id=api_key.user_id,
            new_lock_status="locked" if api_key.is_locked else "unlocked",
        )

        return {
            "id": api_key.id,
            "is_locked": api_key.is_locked,
            "message": f"API密钥已{'锁定' if api_key.is_locked else '解锁'}",
        }


class AdminDeleteApiKeyAdapter(AdminApiAdapter):
    def __init__(self, key_id: str):
        self.key_id = key_id

    async def handle(self, context):  # type: ignore[override]
        db = context.db
        api_key = db.query(ApiKey).filter(ApiKey.id == self.key_id).first()
        if not api_key:
            raise HTTPException(status_code=404, detail="API密钥不存在")

        user = api_key.user
        db.delete(api_key)
        db.commit()

        logger.info(f"管理员删除API密钥: Key ID {self.key_id}, 用户 {user.email if user else '未知'}")

        context.add_audit_metadata(
            action="delete_api_key",
            target_key_id=self.key_id,
            user_id=user.id if user else None,
            user_email=user.email if user else None,
        )
        return {"message": "API密钥已删除"}


class AdminAddBalanceAdapter(AdminApiAdapter):
    """为独立余额Key增加余额"""

    def __init__(self, key_id: str, amount_usd: float):
        self.key_id = key_id
        self.amount_usd = amount_usd

    async def handle(self, context):  # type: ignore[override]
        db = context.db

        # 使用 ApiKeyService 增加余额
        updated_key = ApiKeyService.add_balance(db, self.key_id, self.amount_usd)

        if not updated_key:
            raise NotFoundException("余额充值失败：Key不存在或不是独立余额Key", "api_key")

        logger.info(f"管理员为独立余额Key充值: ID {self.key_id}, 充值 ${self.amount_usd:.4f}")

        context.add_audit_metadata(
            action="add_balance_to_key",
            key_id=self.key_id,
            amount_usd=self.amount_usd,
            new_current_balance=updated_key.current_balance_usd,
        )

        return {
            "id": updated_key.id,
            "name": updated_key.name,
            "current_balance_usd": updated_key.current_balance_usd,
            "balance_used_usd": float(updated_key.balance_used_usd or 0),
            "message": f"余额充值成功，充值 ${self.amount_usd:.2f}，当前余额 ${updated_key.current_balance_usd:.2f}",
        }


class AdminGetFullKeyAdapter(AdminApiAdapter):
    """获取完整的API密钥"""

    def __init__(self, key_id: str):
        self.key_id = key_id

    async def handle(self, context):  # type: ignore[override]
        from src.core.crypto import crypto_service

        db = context.db

        # 查找API密钥
        api_key = db.query(ApiKey).filter(ApiKey.id == self.key_id).first()
        if not api_key:
            raise NotFoundException("API密钥不存在", "api_key")

        # 解密完整密钥
        if not api_key.key_encrypted:
            raise HTTPException(status_code=400, detail="该密钥没有存储完整密钥信息")

        try:
            full_key = crypto_service.decrypt(api_key.key_encrypted)
        except Exception as e:
            logger.error(f"解密API密钥失败: Key ID {self.key_id}, 错误: {e}")
            raise HTTPException(status_code=500, detail="解密密钥失败")

        logger.info(f"管理员查看完整API密钥: Key ID {self.key_id}")

        context.add_audit_metadata(
            action="view_full_api_key",
            key_id=self.key_id,
            key_name=api_key.name,
        )

        return {
            "key": full_key,
        }


class AdminGetKeyDetailAdapter(AdminApiAdapter):
    """Get API key detail without full key"""

    def __init__(self, key_id: str):
        self.key_id = key_id

    async def handle(self, context):  # type: ignore[override]
        db = context.db

        api_key = db.query(ApiKey).filter(ApiKey.id == self.key_id).first()
        if not api_key:
            raise NotFoundException("API密钥不存在", "api_key")

        context.add_audit_metadata(
            action="get_api_key_detail",
            key_id=self.key_id,
        )

        return {
            "id": api_key.id,
            "user_id": api_key.user_id,
            "name": api_key.name,
            "key_display": api_key.get_display_key(),
            "is_active": api_key.is_active,
            "is_locked": api_key.is_locked,
            "is_standalone": api_key.is_standalone,
            "current_balance_usd": api_key.current_balance_usd,
            "balance_used_usd": float(api_key.balance_used_usd or 0),
            "total_requests": api_key.total_requests,
            "total_cost_usd": float(api_key.total_cost_usd or 0),
            "rate_limit": api_key.rate_limit,
            "allowed_providers": api_key.allowed_providers,
            "allowed_api_formats": api_key.allowed_api_formats,
            "allowed_models": api_key.allowed_models,
            "last_used_at": api_key.last_used_at.isoformat() if api_key.last_used_at else None,
            "expires_at": api_key.expires_at.isoformat() if api_key.expires_at else None,
            "created_at": api_key.created_at.isoformat(),
            "updated_at": api_key.updated_at.isoformat() if api_key.updated_at else None,
        }
