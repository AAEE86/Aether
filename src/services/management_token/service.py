"""Management Token 服务"""

import ipaddress
from datetime import datetime, timezone

from sqlalchemy.exc import IntegrityError
from sqlalchemy.orm import Session

from src.config.settings import config
from src.core.logger import logger
from src.models.database import ManagementToken


def validate_ip_list(ips: list[str] | None) -> list[str] | None:
    """验证 IP 白名单格式

    - None: 不限制 IP
    - 非空列表: 只允许列表中的 IP
    - 空列表: 不允许（会抛出错误）
    """
    if ips is None:
        return None
    if len(ips) == 0:
        raise ValueError("IP 白名单不能为空列表，如需取消限制请不提供此字段")

    validated = []
    for i, ip_str in enumerate(ips):
        original = ip_str
        ip_str = ip_str.strip()
        if not ip_str:
            raise ValueError(f"IP 白名单第 {i + 1} 项为空")
        try:
            if "/" in ip_str:
                ipaddress.ip_network(ip_str, strict=False)
            else:
                ipaddress.ip_address(ip_str)
            validated.append(ip_str)
        except ValueError:
            raise ValueError(f"无效的 IP 地址或 CIDR: {original}")

    if not validated:
        raise ValueError("IP 白名单不能为空，如需取消限制请不提供此字段")

    return validated


def parse_expires_at(v, allow_past: bool = False) -> datetime | None:
    """解析过期时间，确保时区安全

    前端 datetime-local 输入返回本地时间字符串（无时区信息）。
    后端要求：
    - 如果是字符串且不含时区，视为 UTC
    - 如果是 datetime 且无时区，视为 UTC
    - 带时区的输入直接使用
    - 默认要求过期时间必须在未来（allow_past=False）

    Args:
        v: 时间值（字符串或 datetime）
        allow_past: 是否允许过去的时间（用于清除过期时间等场景）

    Returns:
        解析后的 datetime 或 None
    """
    if v is None:
        return None
    if isinstance(v, str):
        if not v.strip():
            return None
        try:
            dt = datetime.fromisoformat(v.replace("Z", "+00:00"))
            if dt.tzinfo is None:
                dt = dt.replace(tzinfo=timezone.utc)
        except ValueError as e:
            raise ValueError(f"无效的时间格式: {v}") from e
    elif isinstance(v, datetime):
        dt = v
        if dt.tzinfo is None:
            dt = dt.replace(tzinfo=timezone.utc)
    else:
        raise ValueError(f"不支持的时间类型: {type(v)}")

    if not allow_past and dt <= datetime.now(timezone.utc):
        raise ValueError("过期时间必须在未来")

    return dt


def token_to_dict(
    token: ManagementToken,
    raw_token: str | None = None,
    include_user: bool = False,
) -> dict:
    """将 ManagementToken 转换为字典

    Args:
        token: ManagementToken 实例
        raw_token: 明文 Token（仅在创建/重新生成时提供）
        include_user: 是否包含用户信息（管理员视图使用）

    Returns:
        Token 字典表示
    """
    result = {
        "id": token.id,
        "user_id": token.user_id,
        "name": token.name,
        "description": token.description,
        "token_display": token.get_display_token(),
        "allowed_ips": token.allowed_ips,
        "expires_at": token.expires_at.isoformat() if token.expires_at else None,
        "last_used_at": token.last_used_at.isoformat() if token.last_used_at else None,
        "last_used_ip": token.last_used_ip,
        "usage_count": token.usage_count,
        "is_active": token.is_active,
        "created_at": token.created_at.isoformat() if token.created_at else None,
        "updated_at": token.updated_at.isoformat() if token.updated_at else None,
    }
    if raw_token:
        result["token"] = raw_token
    if include_user and token.user:
        result["user"] = {
            "id": token.user.id,
            "email": token.user.email,
            "username": token.user.username,
            "role": token.user.role.value if token.user.role else None,
        }
    return result


class ManagementTokenService:
    """Management Token 服务类"""

    @staticmethod
    def create_token(
        db: Session,
        user_id: str,
        name: str,
        description: str | None = None,
        allowed_ips: list[str] | None = None,
        expires_at: datetime | None = None,
    ) -> tuple[ManagementToken, str]:
        """创建 Management Token

        Args:
            db: 数据库会话
            user_id: 用户 ID
            name: Token 名称
            description: 描述
            allowed_ips: IP 白名单
            expires_at: 过期时间

        Returns:
            (ManagementToken, 明文 Token) 元组

        Raises:
            ValueError: 如果名称已存在或超过数量限制
        """
        # 检查用户 Token 数量限制
        token_count = (
            db.query(ManagementToken).filter(ManagementToken.user_id == user_id).count()
        )
        max_tokens = config.management_token_max_per_user
        if token_count >= max_tokens:
            raise ValueError(f"已达到 Token 数量上限（{max_tokens}）")

        # 检查名称是否已存在
        existing = (
            db.query(ManagementToken)
            .filter(ManagementToken.user_id == user_id, ManagementToken.name == name)
            .first()
        )
        if existing:
            raise ValueError(f"已存在名为 '{name}' 的 Token")

        # 生成 Token
        raw_token = ManagementToken.generate_token()

        # 创建记录
        token = ManagementToken(
            user_id=user_id,
            name=name,
            description=description,
            allowed_ips=allowed_ips,
            expires_at=expires_at,
        )
        token.set_token(raw_token)

        db.add(token)
        try:
            db.commit()
        except IntegrityError:
            db.rollback()
            # 并发创建导致唯一约束冲突
            raise ValueError(f"已存在名为 '{name}' 的 Token")
        db.refresh(token)

        logger.info(f"创建 Management Token: {token.id} for user {user_id}")

        return token, raw_token

    @staticmethod
    def get_token_by_id(
        db: Session, token_id: str, user_id: str | None = None
    ) -> ManagementToken | None:
        """根据 ID 获取 Token

        Args:
            db: 数据库会话
            token_id: Token ID
            user_id: 用户 ID（如果提供，则只查询该用户的 Token）

        Returns:
            ManagementToken 或 None
        """
        query = db.query(ManagementToken).filter(ManagementToken.id == token_id)
        if user_id:
            query = query.filter(ManagementToken.user_id == user_id)
        return query.first()

    @staticmethod
    def list_tokens(
        db: Session,
        user_id: str | None = None,
        is_active: bool | None = None,
        skip: int = 0,
        limit: int = 100,
    ) -> tuple[list[ManagementToken], int]:
        """列出 Tokens

        Args:
            db: 数据库会话
            user_id: 用户 ID（如果提供，则只查询该用户的 Token）
            is_active: 筛选激活状态
            skip: 跳过记录数
            limit: 返回记录数

        Returns:
            (Token 列表, 总数) 元组
        """
        query = db.query(ManagementToken)

        if user_id:
            query = query.filter(ManagementToken.user_id == user_id)
        if is_active is not None:
            query = query.filter(ManagementToken.is_active == is_active)

        total = query.count()
        tokens = query.order_by(ManagementToken.created_at.desc()).offset(skip).limit(limit).all()

        return tokens, total

    @staticmethod
    def update_token(
        db: Session,
        token_id: str,
        user_id: str | None = None,
        name: str | None = None,
        description: str | None = None,
        allowed_ips: list[str] | None = None,
        expires_at: datetime | None = None,
        is_active: bool | None = None,
        clear_description: bool = False,
        clear_allowed_ips: bool = False,
        clear_expires_at: bool = False,
    ) -> ManagementToken | None:
        """更新 Token

        Args:
            db: 数据库会话
            token_id: Token ID
            user_id: 用户 ID（如果提供，则只更新该用户的 Token）
            name: 新名称
            description: 新描述
            allowed_ips: 新 IP 白名单
            expires_at: 新过期时间
            is_active: 新激活状态
            clear_description: 是否清空描述（True 时 description 被忽略）
            clear_allowed_ips: 是否清空 IP 白名单（True 时 allowed_ips 被忽略）
            clear_expires_at: 是否清空过期时间（True 时 expires_at 被忽略）

        Returns:
            更新后的 ManagementToken 或 None

        Raises:
            ValueError: 如果新名称已被其他 Token 使用
        """
        token = ManagementTokenService.get_token_by_id(db, token_id, user_id)
        if not token:
            return None

        # 如果更新名称，检查是否与其他 Token 冲突
        if name is not None and name != token.name:
            existing = (
                db.query(ManagementToken)
                .filter(
                    ManagementToken.user_id == token.user_id,
                    ManagementToken.name == name,
                    ManagementToken.id != token_id,
                )
                .first()
            )
            if existing:
                raise ValueError(f"已存在名为 '{name}' 的 Token")
            token.name = name

        # 处理 description：支持清空
        if clear_description:
            token.description = None
        elif description is not None:
            token.description = description

        # 处理 allowed_ips：支持清空
        if clear_allowed_ips:
            token.allowed_ips = None
        elif allowed_ips is not None:
            token.allowed_ips = allowed_ips if allowed_ips else None

        # 处理 expires_at：支持清空
        if clear_expires_at:
            token.expires_at = None
        elif expires_at is not None:
            token.expires_at = expires_at

        if is_active is not None:
            token.is_active = is_active

        db.commit()
        db.refresh(token)

        logger.info(f"更新 Management Token: {token.id}")

        return token

    @staticmethod
    def delete_token(
        db: Session, token_id: str, user_id: str | None = None
    ) -> bool:
        """删除 Token

        Args:
            db: 数据库会话
            token_id: Token ID
            user_id: 用户 ID（如果提供，则只删除该用户的 Token）

        Returns:
            是否删除成功
        """
        token = ManagementTokenService.get_token_by_id(db, token_id, user_id)
        if not token:
            return False

        db.delete(token)
        db.commit()

        logger.info(f"删除 Management Token: {token_id}")

        return True

    @staticmethod
    def toggle_status(
        db: Session, token_id: str, user_id: str | None = None
    ) -> ManagementToken | None:
        """切换 Token 状态

        Args:
            db: 数据库会话
            token_id: Token ID
            user_id: 用户 ID（如果提供，则只操作该用户的 Token）

        Returns:
            更新后的 ManagementToken 或 None
        """
        token = ManagementTokenService.get_token_by_id(db, token_id, user_id)
        if not token:
            return None

        token.is_active = not token.is_active
        db.commit()
        db.refresh(token)

        logger.info(f"切换 Management Token 状态: {token.id} -> {token.is_active}")

        return token

    @staticmethod
    def regenerate_token(
        db: Session, token_id: str, user_id: str | None = None
    ) -> tuple[ManagementToken | None, str | None, str | None]:
        """重新生成 Token

        Args:
            db: 数据库会话
            token_id: Token ID
            user_id: 用户 ID（如果提供，则只操作该用户的 Token）

        Returns:
            (ManagementToken, 新的明文 Token, 旧的 token_hash) 元组，失败返回 (None, None, None)
        """
        token = ManagementTokenService.get_token_by_id(db, token_id, user_id)
        if not token:
            return None, None, None

        # 保存旧的 token_hash 用于审计
        old_token_hash = token.token_hash

        # 生成新 Token
        raw_token = ManagementToken.generate_token()
        token.set_token(raw_token)

        db.commit()
        db.refresh(token)

        logger.info(f"重新生成 Management Token: {token.id}")

        return token, raw_token, old_token_hash
