"""
API密钥管理服务
"""

from datetime import datetime, timedelta, timezone
from typing import Any

from sqlalchemy import func
from sqlalchemy.orm import Session

from src.core.crypto import crypto_service
from src.core.logger import logger
from src.models.database import ApiKey, Usage



class ApiKeyService:
    """API密钥管理服务"""

    @staticmethod
    def create_api_key(
        db: Session,
        user_id: str,  # UUID
        name: str | None = None,
        allowed_providers: list[str] | None = None,
        allowed_api_formats: list[str] | None = None,
        allowed_models: list[str] | None = None,
        rate_limit: int | None = None,
        concurrent_limit: int = 5,
        expire_days: int | None = None,
        expires_at: datetime | None = None,  # 直接传入过期时间，优先于 expire_days
        initial_balance_usd: float | None = None,
        is_standalone: bool = False,
        auto_delete_on_expiry: bool = False,
    ) -> tuple[ApiKey, str]:
        """创建新的API密钥，返回密钥对象和明文密钥

        Args:
            db: 数据库会话
            user_id: 用户ID
            name: 密钥名称
            allowed_providers: 允许的提供商列表
            allowed_api_formats: 允许的 API 格式列表
            allowed_models: 允许的模型列表
            rate_limit: 速率限制
            concurrent_limit: 并发限制
            expire_days: 过期天数，None = 永不过期
            expires_at: 直接指定过期时间，优先于 expire_days
            initial_balance_usd: 初始余额（USD），仅用于独立Key，None = 无限制
            is_standalone: 是否为独立余额Key（仅管理员可创建）
            auto_delete_on_expiry: 过期后是否自动删除（True=物理删除，False=仅禁用）
        """

        # 生成密钥
        key = ApiKey.generate_key()
        key_hash = ApiKey.hash_key(key)
        key_encrypted = crypto_service.encrypt(key)  # 加密存储密钥

        # 计算过期时间：优先使用 expires_at，其次使用 expire_days
        final_expires_at = expires_at
        if final_expires_at is None and expire_days:
            final_expires_at = datetime.now(timezone.utc) + timedelta(days=expire_days)

        # 空数组转为 None（表示不限制）
        api_key = ApiKey(
            user_id=user_id,
            key_hash=key_hash,
            key_encrypted=key_encrypted,
            name=name or f"API Key {datetime.now(timezone.utc).strftime('%Y%m%d%H%M%S')}",
            allowed_providers=allowed_providers or None,
            allowed_api_formats=allowed_api_formats or None,
            allowed_models=allowed_models or None,
            rate_limit=rate_limit,
            concurrent_limit=concurrent_limit,
            expires_at=final_expires_at,
            balance_used_usd=0.0,
            current_balance_usd=initial_balance_usd,  # 直接使用初始余额，None = 无限制
            is_standalone=is_standalone,
            auto_delete_on_expiry=auto_delete_on_expiry,
            is_active=True,
        )

        db.add(api_key)
        db.commit()
        db.refresh(api_key)

        logger.info(f"创建API密钥: 用户ID {user_id}, 密钥名 {api_key.name}, "
            f"独立Key={is_standalone}, 初始余额={initial_balance_usd}")
        return api_key, key  # 返回密钥对象和明文密钥

    @staticmethod
    def get_api_key(db: Session, key_id: str) -> ApiKey | None:  # UUID
        """获取API密钥"""
        return db.query(ApiKey).filter(ApiKey.id == key_id).first()

    @staticmethod
    def get_api_key_by_key(db: Session, key: str) -> ApiKey | None:
        """通过密钥字符串获取API密钥"""
        key_hash = ApiKey.hash_key(key)
        return db.query(ApiKey).filter(ApiKey.key_hash == key_hash).first()

    @staticmethod
    def list_user_api_keys(
        db: Session, user_id: str, is_active: bool | None = None  # UUID
    ) -> list[ApiKey]:
        """列出用户的所有API密钥（不包括独立Key）"""
        query = db.query(ApiKey).filter(
            ApiKey.user_id == user_id, ApiKey.is_standalone == False  # 排除独立Key
        )

        if is_active is not None:
            query = query.filter(ApiKey.is_active == is_active)

        return query.order_by(ApiKey.created_at.desc()).all()

    @staticmethod
    def list_standalone_api_keys(db: Session, is_active: bool | None = None) -> list[ApiKey]:
        """列出所有独立余额Key（仅管理员可用）"""
        query = db.query(ApiKey).filter(ApiKey.is_standalone == True)

        if is_active is not None:
            query = query.filter(ApiKey.is_active == is_active)

        return query.order_by(ApiKey.created_at.desc()).all()

    @staticmethod
    def update_api_key(db: Session, key_id: str, **kwargs) -> ApiKey | None:  # UUID
        """更新API密钥"""
        api_key = db.query(ApiKey).filter(ApiKey.id == key_id).first()
        if not api_key:
            return None

        # 可更新的字段
        updatable_fields = [
            "name",
            "allowed_providers",
            "allowed_api_formats",
            "allowed_models",
            "rate_limit",
            "concurrent_limit",
            "is_active",
            "expires_at",
            "balance_limit_usd",
            "auto_delete_on_expiry",
        ]

        # 允许显式设置为空数组/None 的字段（空数组会转为 None，表示"全部"）
        nullable_list_fields = {"allowed_providers", "allowed_api_formats", "allowed_models"}

        # 允许显式设置为 None 的字段（如 expires_at=None 表示永不过期，rate_limit=None 表示无限制）
        nullable_fields = {"expires_at", "rate_limit"}

        for field, value in kwargs.items():
            if field not in updatable_fields:
                continue
            # 对于 nullable_list_fields，空数组应该转为 None（表示不限制）
            if field in nullable_list_fields:
                if value is not None:
                    # 空数组转为 None（表示允许全部）
                    setattr(api_key, field, value if value else None)
            elif field in nullable_fields:
                # 这些字段允许显式设置为 None
                setattr(api_key, field, value)
            elif value is not None:
                setattr(api_key, field, value)

        api_key.updated_at = datetime.now(timezone.utc)
        db.commit()
        db.refresh(api_key)

        logger.debug(f"更新API密钥: ID {key_id}")
        return api_key

    @staticmethod
    def delete_api_key(db: Session, key_id: str) -> bool:  # UUID
        """删除API密钥（禁用）"""
        api_key = db.query(ApiKey).filter(ApiKey.id == key_id).first()
        if not api_key:
            return False

        api_key.is_active = False
        api_key.updated_at = datetime.now(timezone.utc)
        db.commit()

        logger.info(f"删除API密钥: ID {key_id}")
        return True

    @staticmethod
    def get_remaining_balance(api_key: ApiKey) -> float | None:
        """计算剩余余额（仅用于独立Key）

        Returns:
            剩余余额，None 表示无限制或非独立Key
        """
        if not api_key.is_standalone:
            return None

        if api_key.current_balance_usd is None:
            return None

        # 剩余余额 = 当前余额 - 已使用余额
        remaining = api_key.current_balance_usd - (api_key.balance_used_usd or 0)
        return max(0, remaining)  # 不能为负数

    @staticmethod
    def check_balance(api_key: ApiKey) -> tuple[bool, float | None]:
        """检查余额限制（仅用于独立Key）

        Returns:
            (is_allowed, remaining_balance): 是否允许请求，剩余余额（None表示无限制）
        """
        if not api_key.is_standalone:
            # 非独立Key不检查余额
            return True, None

        # 使用新的预付费模式: current_balance_usd
        if api_key.current_balance_usd is None:
            # 无余额限制
            return True, None

        # 使用统一的余额计算方法
        remaining = ApiKeyService.get_remaining_balance(api_key)
        is_allowed = remaining > 0 if remaining is not None else True

        if not is_allowed:
            logger.warning(f"API密钥余额不足: Key ID {api_key.id}, " f"剩余余额 ${remaining:.4f}")

        return is_allowed, remaining

    @staticmethod
    def check_rate_limit(db: Session, api_key: ApiKey, window_minutes: int = 1) -> tuple[bool, int]:
        """检查速率限制

        Returns:
            (is_allowed, remaining): 是否允许请求，剩余可用次数
            当 rate_limit 为 None 时表示不限制，返回 (True, -1)
        """
        # 如果 rate_limit 为 None，表示不限制
        if api_key.rate_limit is None:
            return True, -1  # -1 表示无限制

        # 计算时间窗口
        window_start = datetime.now(timezone.utc) - timedelta(minutes=window_minutes)

        # 统计窗口内的请求数
        request_count = (
            db.query(func.count(Usage.id))
            .filter(Usage.api_key_id == api_key.id, Usage.created_at >= window_start)
            .scalar()
            or 0
        )

        # 检查是否超限
        is_allowed = request_count < api_key.rate_limit

        if not is_allowed:
            logger.warning(f"API密钥速率限制: Key ID {api_key.id}, 请求数 {request_count}/{api_key.rate_limit}")

        return is_allowed, api_key.rate_limit - request_count

    @staticmethod
    def add_balance(db: Session, key_id: str, amount_usd: float) -> ApiKey | None:
        """为独立余额Key调整余额

        Args:
            db: 数据库会话
            key_id: API Key ID
            amount_usd: 要调整的余额金额（USD），正数为增加，负数为扣除

        Returns:
            更新后的API Key对象，如果Key不存在或不是独立Key则返回None
        """
        api_key = db.query(ApiKey).filter(ApiKey.id == key_id).first()
        if not api_key:
            logger.warning(f"余额调整失败: Key ID {key_id} 不存在")
            return None

        if not api_key.is_standalone:
            logger.warning(f"余额调整失败: Key ID {key_id} 不是独立余额Key")
            return None

        if amount_usd == 0:
            logger.warning(f"余额调整失败: 调整金额不能为0，当前值 ${amount_usd}")
            return None

        # 如果是扣除（负数），检查是否超过当前余额
        if amount_usd < 0:
            current = api_key.current_balance_usd or 0
            if abs(amount_usd) > current:
                logger.warning(f"余额扣除失败: 扣除金额 ${abs(amount_usd):.4f} 超过当前余额 ${current:.4f}")
                return None

        # 调整当前余额
        if api_key.current_balance_usd is None:
            api_key.current_balance_usd = amount_usd if amount_usd > 0 else 0
        else:
            api_key.current_balance_usd = max(0, api_key.current_balance_usd + amount_usd)

        api_key.updated_at = datetime.now(timezone.utc)
        db.commit()
        db.refresh(api_key)

        action = "增加" if amount_usd > 0 else "扣除"
        logger.info(f"余额调整成功: Key ID {key_id}, {action} ${abs(amount_usd):.4f}, "
            f"新余额 ${api_key.current_balance_usd:.4f}")
        return api_key

    @staticmethod
    def cleanup_expired_keys(db: Session, auto_delete: bool = False) -> int:
        """清理过期的API密钥

        Args:
            db: 数据库会话
            auto_delete: 全局默认行为（True=物理删除，False=仅禁用）
                        单个Key的 auto_delete_on_expiry 字段会覆盖此设置

        Returns:
            int: 清理的密钥数量
        """
        now = datetime.now(timezone.utc)
        expired_keys = (
            db.query(ApiKey)
            .filter(ApiKey.expires_at <= now, ApiKey.is_active == True)  # 只处理仍然活跃的
            .all()
        )

        count = 0
        for api_key in expired_keys:
            # 优先使用Key自身的auto_delete_on_expiry设置,否则使用全局设置
            should_delete = (
                api_key.auto_delete_on_expiry
                if api_key.auto_delete_on_expiry is not None
                else auto_delete
            )

            if should_delete:
                # 物理删除（Usage记录会保留，因为是 SET NULL）
                db.delete(api_key)
                logger.info(f"删除过期API密钥: ID {api_key.id}, 名称 {api_key.name}, "
                    f"过期时间 {api_key.expires_at}")
            else:
                # 仅禁用
                api_key.is_active = False
                api_key.updated_at = now
                logger.info(f"禁用过期API密钥: ID {api_key.id}, 名称 {api_key.name}, "
                    f"过期时间 {api_key.expires_at}")
            count += 1

        if count > 0:
            db.commit()

        return count

    @staticmethod
    def get_api_key_stats(
        db: Session,
        key_id: str,  # UUID
        start_date: datetime | None = None,
        end_date: datetime | None = None,
    ) -> dict[str, Any]:
        """获取API密钥使用统计"""

        api_key = db.query(ApiKey).filter(ApiKey.id == key_id).first()
        if not api_key:
            return {}

        query = db.query(Usage).filter(Usage.api_key_id == key_id)

        if start_date:
            query = query.filter(Usage.created_at >= start_date)
        if end_date:
            query = query.filter(Usage.created_at <= end_date)

        # 统计数据
        stats = db.query(
            func.count(Usage.id).label("requests"),
            func.sum(Usage.total_tokens).label("tokens"),
            func.sum(Usage.total_cost_usd).label("cost_usd"),
            func.avg(Usage.response_time_ms).label("avg_response_time"),
        ).filter(Usage.api_key_id == key_id)

        if start_date:
            stats = stats.filter(Usage.created_at >= start_date)
        if end_date:
            stats = stats.filter(Usage.created_at <= end_date)

        result = stats.first()

        # 按天统计
        daily_stats = db.query(
            func.date(Usage.created_at).label("date"),
            func.count(Usage.id).label("requests"),
            func.sum(Usage.total_tokens).label("tokens"),
            func.sum(Usage.total_cost_usd).label("cost_usd"),
        ).filter(Usage.api_key_id == key_id)

        if start_date:
            daily_stats = daily_stats.filter(Usage.created_at >= start_date)
        if end_date:
            daily_stats = daily_stats.filter(Usage.created_at <= end_date)

        daily_stats = daily_stats.group_by(func.date(Usage.created_at)).all()

        return {
            "key_id": key_id,
            "key_name": api_key.name,
            "total_requests": result.requests or 0,
            "total_tokens": result.tokens or 0,
            "total_cost_usd": float(result.cost_usd or 0),
            "avg_response_time_ms": float(result.avg_response_time or 0),
            "daily_stats": [
                {
                    "date": stat.date.isoformat() if stat.date else None,
                    "requests": stat.requests,
                    "tokens": stat.tokens,
                    "cost_usd": float(stat.cost_usd),
                }
                for stat in daily_stats
            ],
        }
