"""
认证服务
"""

import os
import hashlib
import secrets
from datetime import datetime, timedelta, timezone
from typing import Any, Dict, Optional

import jwt
from fastapi import HTTPException, status
from fastapi.concurrency import run_in_threadpool
from sqlalchemy.orm import Session, joinedload

from src.config import config
from src.core.crypto import crypto_service
from src.core.logger import logger
from src.core.enums import AuthSource
from src.models.database import ApiKey, User, UserRole
from src.services.auth.jwt_blacklist import JWTBlacklistService
from src.services.auth.ldap import LDAPService
from src.services.cache.user_cache import UserCacheService
from src.services.user.apikey import ApiKeyService


# JWT配置从config读取
if not config.jwt_secret_key:
    # 如果没有配置，生成一个随机密钥并警告
    if config.environment == "production":
        raise ValueError("JWT_SECRET_KEY must be set in production environment!")
    config.jwt_secret_key = secrets.token_urlsafe(32)
    logger.warning("JWT_SECRET_KEY未在环境变量中找到，已生成随机密钥用于开发")
    logger.warning("生产环境请设置JWT_SECRET_KEY环境变量!")

JWT_SECRET_KEY = config.jwt_secret_key
JWT_ALGORITHM = config.jwt_algorithm
JWT_EXPIRATION_HOURS = config.jwt_expiration_hours
# Refresh token 有效期设为7天
REFRESH_TOKEN_EXPIRATION_DAYS = 7


class AuthService:
    """认证服务"""

    @staticmethod
    def create_access_token(data: dict) -> str:
        """创建JWT访问令牌"""
        to_encode = data.copy()
        expire = datetime.now(timezone.utc) + timedelta(hours=JWT_EXPIRATION_HOURS)
        to_encode.update({"exp": expire, "type": "access"})
        encoded_jwt = jwt.encode(to_encode, JWT_SECRET_KEY, algorithm=JWT_ALGORITHM)
        return encoded_jwt

    @staticmethod
    def create_refresh_token(data: dict) -> str:
        """创建JWT刷新令牌"""
        to_encode = data.copy()
        expire = datetime.now(timezone.utc) + timedelta(days=REFRESH_TOKEN_EXPIRATION_DAYS)
        to_encode.update({"exp": expire, "type": "refresh"})
        encoded_jwt = jwt.encode(to_encode, JWT_SECRET_KEY, algorithm=JWT_ALGORITHM)
        return encoded_jwt

    @staticmethod
    async def verify_token(token: str, token_type: Optional[str] = None) -> Dict[str, Any]:
        """验证JWT令牌

        Args:
            token: JWT token字符串
            token_type: 期望的token类型 ('access' 或 'refresh')，None表示不验证类型
        """
        try:
            payload = jwt.decode(token, JWT_SECRET_KEY, algorithms=[JWT_ALGORITHM])

            # 验证token类型（如果指定）
            if token_type:
                actual_type = payload.get("type")
                if actual_type != token_type:
                    raise HTTPException(
                        status_code=status.HTTP_401_UNAUTHORIZED,
                        detail=f"Token类型错误: 期望 {token_type}, 实际 {actual_type}",
                    )

            # 检查 Token 是否在黑名单中
            is_blacklisted = await JWTBlacklistService.is_blacklisted(token)
            if is_blacklisted:
                raise HTTPException(
                    status_code=status.HTTP_401_UNAUTHORIZED, detail="Token已被撤销"
                )

            return payload
        except jwt.ExpiredSignatureError:
            raise HTTPException(status_code=status.HTTP_401_UNAUTHORIZED, detail="Token已过期")
        except jwt.InvalidTokenError:
            raise HTTPException(status_code=status.HTTP_401_UNAUTHORIZED, detail="无效的Token")

    @staticmethod
    async def authenticate_user(
        db: Session, email: str, password: str, auth_type: str = "local"
    ) -> Optional[User]:
        """用户登录认证

        Args:
            db: 数据库会话
            email: 邮箱/用户名
            password: 密码
            auth_type: 认证类型 ("local" 或 "ldap")
        """
        if auth_type == "ldap":
            # LDAP 认证
            if not LDAPService.is_ldap_enabled(db):
                logger.warning("登录失败 - LDAP 认证未启用")
                return None

            # 预取配置，避免将 Session 传递到线程池
            config_data = LDAPService.get_config_data(db)
            if not config_data:
                logger.warning("登录失败 - 无法获取 LDAP 配置或解密失败")
                return None

            # 在线程池中执行阻塞的 LDAP 网络请求，避免阻塞事件循环
            ldap_user = await run_in_threadpool(
                LDAPService.authenticate_with_config, config_data, email, password
            )
            if not ldap_user:
                return None

            # 获取或创建本地用户
            user = await AuthService._get_or_create_ldap_user(db, ldap_user)
            if not user:
                # 已有本地账号但来源不匹配等情况
                return None
            if not user.is_active:
                logger.warning(f"登录失败 - 用户已禁用: {email}")
                return None
            return user

        # 本地认证
        # 登录校验必须读取密码哈希，不能使用不包含 password_hash 的缓存对象
        user = db.query(User).filter(User.email == email).first()

        if not user:
            logger.warning(f"登录失败 - 用户不存在: {email}")
            return None

        # 检查 LDAP exclusive 模式：仅允许本地管理员登录（紧急恢复通道）
        if LDAPService.is_ldap_exclusive(db):
            if user.role != UserRole.ADMIN or user.auth_source != AuthSource.LOCAL:
                logger.warning(f"登录失败 - 仅允许 LDAP 登录（管理员除外）: {email}")
                return None
            logger.info(f"LDAP exclusive 模式下允许本地管理员登录: {email}")

        # 检查用户认证来源
        if user.auth_source == AuthSource.LDAP:
            logger.warning(f"登录失败 - 该用户使用 LDAP 认证: {email}")
            return None

        if not user.verify_password(password):
            logger.warning(f"登录失败 - 密码错误: {email}")
            return None

        if not user.is_active:
            logger.warning(f"登录失败 - 用户已禁用: {email}")
            return None

        # 更新最后登录时间
        user.last_login_at = datetime.now(timezone.utc)
        db.commit()  # 立即提交事务,释放数据库锁
        # 清除缓存，因为用户信息已更新
        await UserCacheService.invalidate_user_cache(user.id, user.email)

        logger.info(f"用户登录成功: {email} (ID: {user.id})")
        return user

    @staticmethod
    async def _get_or_create_ldap_user(db: Session, ldap_user: dict) -> Optional[User]:
        """获取或创建 LDAP 用户

        Args:
            ldap_user: LDAP 用户信息 {username, email, display_name}
        """
        # 先按 email 查找
        user = db.query(User).filter(User.email == ldap_user["email"]).first()

        if user:
            # 更新 auth_source（如果是首次 LDAP 登录）
            if user.auth_source != AuthSource.LDAP:
                # 避免覆盖已有本地账户（不同来源时拒绝登录）
                logger.warning(
                    f"LDAP 登录拒绝 - 账户来源不匹配(现有:{user.auth_source}, 请求:LDAP): {ldap_user['email']}"
                )
                return None
            user.last_login_at = datetime.now(timezone.utc)
            db.commit()
            await UserCacheService.invalidate_user_cache(user.id, user.email)
            logger.info(f"LDAP 用户登录成功: {ldap_user['email']} (ID: {user.id})")
            return user

        # 检查 username 是否已被占用
        username = ldap_user["username"]
        existing_user_with_username = db.query(User).filter(User.username == username).first()
        if existing_user_with_username:
            # 如果 username 已存在，添加后缀使其唯一
            base_username = username
            counter = 1
            while db.query(User).filter(User.username == username).first():
                username = f"{base_username}_ldap_{counter}"
                counter += 1
            logger.info(f"LDAP 用户名冲突，使用新用户名: {base_username} -> {username}")

        # 创建新用户
        user = User(
            email=ldap_user["email"],
            username=username,
            password_hash="",  # LDAP 用户无本地密码
            auth_source=AuthSource.LDAP,
            role=UserRole.USER,
            is_active=True,
            last_login_at=datetime.now(timezone.utc),
        )
        db.add(user)
        db.commit()
        db.refresh(user)
        logger.info(f"LDAP 用户创建成功: {ldap_user['email']} (ID: {user.id})")
        return user

    @staticmethod
    def authenticate_api_key(db: Session, api_key: str) -> Optional[tuple[User, ApiKey]]:
        """API密钥认证"""
        # 对API密钥进行哈希查找，预加载 user 关系以支持后续访问限制检查
        key_hash = ApiKey.hash_key(api_key)
        key_record = (
            db.query(ApiKey)
            .options(joinedload(ApiKey.user))
            .filter(ApiKey.key_hash == key_hash)
            .first()
        )

        if not key_record:
            # 只记录认证失败事件，不记录任何 key 信息以防止信息泄露
            logger.warning("API认证失败 - 密钥不存在或无效")
            return None

        if not key_record.is_active:
            logger.warning("API认证失败 - 密钥已禁用")
            return None

        # 检查过期时间
        if key_record.expires_at:
            # 确保 expires_at 是 aware datetime
            expires_at = key_record.expires_at
            if expires_at.tzinfo is None:
                # 如果没有时区信息，假定为 UTC
                expires_at = expires_at.replace(tzinfo=timezone.utc)

            if expires_at < datetime.now(timezone.utc):
                logger.warning("API认证失败 - 密钥已过期")
                return None

        # 检查余额限制（仅独立Key）
        is_balance_ok, remaining = ApiKeyService.check_balance(key_record)
        if not is_balance_ok:
            # 获取剩余余额用于日志
            remaining_balance = ApiKeyService.get_remaining_balance(key_record)
            logger.warning(f"API认证失败 - 余额不足 "
                f"(已用: ${key_record.balance_used_usd:.4f}, 剩余: ${remaining_balance:.4f})")
            return None

        # 获取用户
        user = key_record.user
        if not user.is_active:
            logger.warning(f"API认证失败 - 用户已禁用: {user.email}")
            return None

        # 更新最后使用时间
        key_record.last_used_at = datetime.now(timezone.utc)
        db.commit()  # 立即提交事务,释放数据库锁,避免阻塞后续请求

        api_key_fp = hashlib.sha256(api_key.encode()).hexdigest()[:12]
        logger.debug("API认证成功: 用户 {} (api_key_fp={})", user.email, api_key_fp)
        return user, key_record

    @staticmethod
    def check_user_quota(user: User, estimated_cost: float = 0) -> bool:
        """检查用户配额"""
        if user.role == UserRole.ADMIN:
            return True  # 管理员无限制

        # NULL 表示无限制
        if user.quota_usd is None:
            return True

        # 检查美元配额
        if user.used_usd + estimated_cost > user.quota_usd:
            logger.warning(f"用户配额不足: {user.email} (已用: ${user.used_usd:.2f}, 配额: ${user.quota_usd:.2f})")
            return False

        return True

    @staticmethod
    def check_permission(user: User, required_role: UserRole = UserRole.USER) -> bool:
        """检查用户权限"""
        if user.role == UserRole.ADMIN:
            return True

        # 避免使用字符串比较导致权限判断错误（例如 'user' >= 'admin'）
        role_rank = {UserRole.USER: 0, UserRole.ADMIN: 1}
        # 未知用户角色默认 -1（拒绝），未知要求角色默认 999（拒绝）
        if role_rank.get(user.role, -1) >= role_rank.get(required_role, 999):
            return True

        logger.warning(f"权限不足: 用户 {user.email} 角色 {user.role.value} < 需要 {required_role.value}")
        return False

    @staticmethod
    async def logout(token: str) -> bool:
        """
        用户登出，将 Token 加入黑名单

        Args:
            token: JWT token字符串

        Returns:
            是否成功登出
        """
        try:
            # 解码 Token 获取过期时间（不验证黑名单）
            payload = jwt.decode(token, JWT_SECRET_KEY, algorithms=[JWT_ALGORITHM])
            exp_timestamp = payload.get("exp")

            if not exp_timestamp:
                logger.warning("Token 缺少过期时间，无法加入黑名单")
                return False

            # 将 Token 加入黑名单
            success = await JWTBlacklistService.add_to_blacklist(
                token=token, exp_timestamp=exp_timestamp, reason="logout"
            )

            if success:
                user_id = payload.get("user_id")
                logger.info(f"用户登出成功: user_id={user_id}")

            return success

        except jwt.InvalidTokenError as e:
            logger.warning(f"登出失败 - 无效的 Token: {e}")
            return False
        except Exception as e:
            logger.error(f"登出失败: {e}")
            return False

    @staticmethod
    async def revoke_token(token: str, reason: str = "revoked") -> bool:
        """
        撤销 Token（管理员操作）

        Args:
            token: JWT token字符串
            reason: 撤销原因

        Returns:
            是否成功撤销
        """
        try:
            # 解码 Token 获取过期时间
            payload = jwt.decode(token, JWT_SECRET_KEY, algorithms=[JWT_ALGORITHM])
            exp_timestamp = payload.get("exp")

            if not exp_timestamp:
                logger.warning("Token 缺少过期时间，无法撤销")
                return False

            # 将 Token 加入黑名单
            success = await JWTBlacklistService.add_to_blacklist(
                token=token, exp_timestamp=exp_timestamp, reason=reason
            )

            if success:
                user_id = payload.get("sub")
                logger.warning(f"Token 已被撤销: user_id={user_id}, reason={reason}")

            return success

        except jwt.InvalidTokenError as e:
            logger.warning(f"撤销失败 - 无效的 Token: {e}")
            return False
        except Exception as e:
            logger.error(f"撤销 Token 失败: {e}")
            return False
