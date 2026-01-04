"""LDAP 认证服务"""

from typing import Optional, Tuple

from sqlalchemy.orm import Session

from src.core.logger import logger
from src.models.database import LDAPConfig

# LDAP 连接超时时间（秒）
LDAP_CONNECT_TIMEOUT = 10


def escape_ldap_filter(value: str) -> str:
    """
    转义 LDAP 过滤器中的特殊字符，防止 LDAP 注入攻击

    Args:
        value: 需要转义的字符串

    Returns:
        转义后的安全字符串
    """
    # LDAP 过滤器特殊字符: \ * ( ) NUL
    escape_chars = {
        "\\": r"\5c",
        "*": r"\2a",
        "(": r"\28",
        ")": r"\29",
        "\x00": r"\00",
    }
    for char, escaped in escape_chars.items():
        value = value.replace(char, escaped)
    return value


class LDAPService:
    """LDAP 认证服务"""

    @staticmethod
    def get_config(db: Session) -> Optional[LDAPConfig]:
        """获取 LDAP 配置"""
        return db.query(LDAPConfig).first()

    @staticmethod
    def is_ldap_enabled(db: Session) -> bool:
        """检查 LDAP 是否启用"""
        config = LDAPService.get_config(db)
        return config.is_enabled if config else False

    @staticmethod
    def is_ldap_exclusive(db: Session) -> bool:
        """检查是否仅允许 LDAP 登录"""
        config = LDAPService.get_config(db)
        return config.is_exclusive if config and config.is_enabled else False

    @staticmethod
    def authenticate(db: Session, username: str, password: str) -> Optional[dict]:
        """
        LDAP bind 验证

        Args:
            db: 数据库会话
            username: 用户名
            password: 密码

        Returns:
            用户属性 dict {username, email, display_name} 或 None
        """
        try:
            import ldap3
            from ldap3 import Server, Connection, SUBTREE
            from ldap3.core.exceptions import LDAPBindError, LDAPSocketOpenError
        except ImportError:
            logger.error("ldap3 库未安装")
            return None

        config = LDAPService.get_config(db)
        if not config or not config.is_enabled:
            logger.warning("LDAP 未配置或未启用")
            return None

        try:
            # 创建服务器连接
            use_ssl = config.server_url.startswith("ldaps://")
            server = Server(
                config.server_url,
                use_ssl=use_ssl,
                get_info=ldap3.ALL,
                connect_timeout=LDAP_CONNECT_TIMEOUT,
            )

            # 使用管理员账号连接
            bind_password = config.get_bind_password()
            admin_conn = Connection(server, user=config.bind_dn, password=bind_password)

            if config.use_starttls and not use_ssl:
                admin_conn.start_tls()

            if not admin_conn.bind():
                logger.error(f"LDAP 管理员绑定失败: {admin_conn.result}")
                return None

            # 搜索用户（转义用户名防止 LDAP 注入）
            safe_username = escape_ldap_filter(username)
            search_filter = config.user_search_filter.replace("{username}", safe_username)
            admin_conn.search(
                search_base=config.base_dn,
                search_filter=search_filter,
                search_scope=SUBTREE,
                attributes=[config.username_attr, config.email_attr, config.display_name_attr],
            )

            if not admin_conn.entries:
                logger.warning(f"LDAP 用户未找到: {username}")
                admin_conn.unbind()
                return None

            user_entry = admin_conn.entries[0]
            user_dn = user_entry.entry_dn
            admin_conn.unbind()

            # 用户密码验证
            user_conn = Connection(server, user=user_dn, password=password)
            if config.use_starttls and not use_ssl:
                user_conn.start_tls()

            if not user_conn.bind():
                logger.warning(f"LDAP 密码验证失败: {username}")
                return None

            user_conn.unbind()

            # 提取用户属性
            email = str(getattr(user_entry, config.email_attr, "")) or f"{username}@ldap.local"
            display_name = str(getattr(user_entry, config.display_name_attr, "")) or username

            logger.info(f"LDAP 认证成功: {username}")
            return {
                "username": username,
                "email": email,
                "display_name": display_name,
            }

        except LDAPSocketOpenError as e:
            logger.error(f"LDAP 服务器连接失败: {e}")
            return None
        except LDAPBindError as e:
            logger.error(f"LDAP 绑定失败: {e}")
            return None
        except Exception as e:
            logger.error(f"LDAP 认证异常: {e}")
            return None

    @staticmethod
    def test_connection(db: Session) -> Tuple[bool, str]:
        """
        测试 LDAP 连接

        Returns:
            (success, message)
        """
        try:
            import ldap3
            from ldap3 import Server, Connection
        except ImportError:
            return False, "ldap3 库未安装"

        config = LDAPService.get_config(db)
        if not config:
            return False, "LDAP 配置不存在"

        try:
            use_ssl = config.server_url.startswith("ldaps://")
            server = Server(
                config.server_url,
                use_ssl=use_ssl,
                get_info=ldap3.ALL,
                connect_timeout=LDAP_CONNECT_TIMEOUT,
            )
            bind_password = config.get_bind_password()
            conn = Connection(server, user=config.bind_dn, password=bind_password)

            if config.use_starttls and not use_ssl:
                conn.start_tls()

            if not conn.bind():
                return False, f"绑定失败: {conn.result}"

            conn.unbind()
            return True, "连接成功"

        except Exception as e:
            return False, f"连接失败: {str(e)}"
