"""LDAP 认证服务"""

from typing import Dict, Optional, Tuple

from sqlalchemy.orm import Session

from src.core.logger import logger
from src.models.database import LDAPConfig

# LDAP 连接默认超时时间（秒）
DEFAULT_LDAP_CONNECT_TIMEOUT = 10


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


def _get_attr_value(entry, attr_name: str, default: str = "") -> str:
    """
    提取 LDAP 条目属性的首个值，避免返回字符串化的列表表示。
    """
    attr = getattr(entry, attr_name, None)
    if not attr:
        return default
    # ldap3 的 EntryAttribute.value 已经是单值或列表，根据类型取首个
    val = getattr(attr, "value", None)
    if isinstance(val, list):
        val = val[0] if val else default
    if val is None:
        return default
    return str(val)


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
    def get_config_data(db: Session) -> Optional[Dict[str, str]]:
        """
        提前获取并解密配置，供线程池使用，避免跨线程共享 Session。
        """
        config = LDAPService.get_config(db)
        if not config or not config.is_enabled:
            return None

        try:
            bind_password = config.get_bind_password()
        except Exception as e:
            logger.error(f"LDAP 绑定密码解密失败: {e}")
            return None

        return {
            "server_url": config.server_url,
            "bind_dn": config.bind_dn,
            "bind_password": bind_password,
            "base_dn": config.base_dn,
            "user_search_filter": config.user_search_filter,
            "username_attr": config.username_attr,
            "email_attr": config.email_attr,
            "display_name_attr": config.display_name_attr,
            "use_starttls": config.use_starttls,
            "connect_timeout": config.connect_timeout or DEFAULT_LDAP_CONNECT_TIMEOUT,
        }

    @staticmethod
    def authenticate_with_config(config: Dict[str, str], username: str, password: str) -> Optional[dict]:
        """
        LDAP bind 验证

        Args:
            config: 已解密的 LDAP 配置
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

        if not config:
            logger.warning("LDAP 未配置或未启用")
            return None

        admin_conn = None
        user_conn = None

        try:
            # 创建服务器连接
            server_url = config["server_url"]
            use_ssl = server_url.startswith("ldaps://")
            timeout = config.get("connect_timeout", DEFAULT_LDAP_CONNECT_TIMEOUT)
            server = Server(
                server_url,
                use_ssl=use_ssl,
                get_info=ldap3.ALL,
                connect_timeout=timeout,
            )

            # 使用管理员账号连接
            bind_password = config["bind_password"]
            admin_conn = Connection(server, user=config["bind_dn"], password=bind_password)

            if config.get("use_starttls") and not use_ssl:
                admin_conn.start_tls()

            if not admin_conn.bind():
                logger.error(f"LDAP 管理员绑定失败: {admin_conn.result}")
                return None

            # 搜索用户（转义用户名防止 LDAP 注入）
            safe_username = escape_ldap_filter(username)
            search_filter = config["user_search_filter"].replace("{username}", safe_username)
            admin_conn.search(
                search_base=config["base_dn"],
                search_filter=search_filter,
                search_scope=SUBTREE,
                attributes=[
                    config["username_attr"],
                    config["email_attr"],
                    config["display_name_attr"],
                ],
            )

            if not admin_conn.entries:
                logger.warning(f"LDAP 用户未找到: {username}")
                return None

            user_entry = admin_conn.entries[0]
            user_dn = user_entry.entry_dn

            # 用户密码验证
            user_conn = Connection(server, user=user_dn, password=password)
            if config.get("use_starttls") and not use_ssl:
                user_conn.start_tls()

            if not user_conn.bind():
                logger.warning(f"LDAP 密码验证失败: {username}")
                return None

            # 提取用户属性（优先用 LDAP 提供的值，不合法则回退默认）
            email = _get_attr_value(
                user_entry, config["email_attr"], f"{username}@ldap.local"
            )
            display_name = _get_attr_value(user_entry, config["display_name_attr"], username)

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
        finally:
            # 确保连接关闭，避免失败路径泄漏
            if admin_conn:
                try:
                    admin_conn.unbind()
                except Exception:
                    pass
            if user_conn:
                try:
                    user_conn.unbind()
                except Exception:
                    pass

    @staticmethod
    def test_connection_with_config(config: Dict[str, str]) -> Tuple[bool, str]:
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

        if not config:
            return False, "LDAP 配置不存在"

        try:
            server_url = config["server_url"]
            use_ssl = server_url.startswith("ldaps://")
            timeout = config.get("connect_timeout", DEFAULT_LDAP_CONNECT_TIMEOUT)
            server = Server(
                server_url,
                use_ssl=use_ssl,
                get_info=ldap3.ALL,
                connect_timeout=timeout,
            )
            bind_password = config["bind_password"]
            conn = Connection(server, user=config["bind_dn"], password=bind_password)

            if config.get("use_starttls") and not use_ssl:
                conn.start_tls()

            if not conn.bind():
                return False, f"绑定失败: {conn.result}"

            conn.unbind()
            return True, "连接成功"

        except Exception as e:
            return False, f"连接失败: {str(e)}"

    # 兼容旧接口：如果其他代码直接调用
    @staticmethod
    def authenticate(db: Session, username: str, password: str) -> Optional[dict]:
        config = LDAPService.get_config_data(db)
        return LDAPService.authenticate_with_config(config, username, password) if config else None

    @staticmethod
    def test_connection(db: Session) -> Tuple[bool, str]:
        config = LDAPService.get_config_data(db)
        if not config:
            return False, "LDAP 配置不存在或未启用"
        return LDAPService.test_connection_with_config(config)
