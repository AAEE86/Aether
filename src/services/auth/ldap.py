"""LDAP 认证服务"""

from typing import Any, Dict, Optional, Tuple
from urllib.parse import urlparse

from sqlalchemy.orm import Session

from src.core.logger import logger
from src.models.database import LDAPConfig

# LDAP 连接默认超时时间（秒）
DEFAULT_LDAP_CONNECT_TIMEOUT = 10


def parse_ldap_server_url(server_url: str) -> tuple[str, int, bool]:
    """
    解析 LDAP 服务器地址，支持：
    - ldap://host:389
    - ldaps://host:636
    - host:389（无 scheme 时默认 ldap）

    Returns:
        (host, port, use_ssl)
    """
    raw = (server_url or "").strip()
    if not raw:
        raise ValueError("LDAP server_url is required")

    parsed = urlparse(raw)
    if parsed.scheme in {"ldap", "ldaps"}:
        host = parsed.hostname
        if not host:
            raise ValueError("Invalid LDAP server_url")
        use_ssl = parsed.scheme == "ldaps"
        port = parsed.port or (636 if use_ssl else 389)
        return host, port, use_ssl

    # 兼容无 scheme：按 ldap:// 解析
    parsed = urlparse(f"ldap://{raw}")
    host = parsed.hostname
    if not host:
        raise ValueError("Invalid LDAP server_url")
    port = parsed.port or 389
    return host, port, False


def escape_ldap_filter(value: str, max_length: int = 128) -> str:
    """
    转义 LDAP 过滤器中的特殊字符，防止 LDAP 注入攻击（RFC 4515）

    Args:
        value: 需要转义的字符串
        max_length: 最大允许长度，默认 128 字符（覆盖大多数企业邮箱用户名）

    Returns:
        转义后的安全字符串

    Raises:
        ValueError: 输入值过长
    """
    import unicodedata

    # 先检查原始长度，防止 DoS 攻击
    # 128 字符足够覆盖大多数企业用户名和邮箱地址
    if len(value) > max_length:
        raise ValueError(f"LDAP filter value too long (max {max_length} characters)")

    # Unicode 规范化（使用 NFC 而非 NFKC，避免兼容性字符转换导致安全问题）
    value = unicodedata.normalize("NFC", value)

    # 再次检查规范化后的长度（防止规范化后长度突增）
    if len(value) > max_length:
        raise ValueError(f"LDAP filter value too long after normalization (max {max_length})")

    # LDAP 过滤器特殊字符（RFC 4515 + 扩展）
    # 使用显式顺序处理，确保反斜杠首先转义
    value = value.replace("\\", r"\5c")  # 反斜杠必须首先转义
    value = value.replace("*", r"\2a")
    value = value.replace("(", r"\28")
    value = value.replace(")", r"\29")
    value = value.replace("\x00", r"\00")  # NUL
    value = value.replace("&", r"\26")
    value = value.replace("|", r"\7c")
    value = value.replace("=", r"\3d")
    value = value.replace(">", r"\3e")
    value = value.replace("<", r"\3c")
    value = value.replace("~", r"\7e")
    value = value.replace("!", r"\21")
    return value


def _get_attr_value(entry: Any, attr_name: str, default: str = "") -> str:
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
        """检查 LDAP 是否可用（已启用且绑定密码可解密）"""
        return LDAPService.get_config_data(db) is not None

    @staticmethod
    def is_ldap_exclusive(db: Session) -> bool:
        """检查是否仅允许 LDAP 登录（仅在 LDAP 可用时生效，避免误锁定）"""
        config = LDAPService.get_config(db)
        if not config or config.is_exclusive is not True:
            return False
        return LDAPService.get_config_data(db) is not None

    @staticmethod
    def get_config_data(db: Session) -> Optional[Dict[str, Any]]:
        """
        提前获取并解密配置，供线程池使用，避免跨线程共享 Session。
        """
        config = LDAPService.get_config(db)
        if not config or config.is_enabled is not True:
            return None

        try:
            bind_password = config.get_bind_password()
        except Exception as e:
            logger.error(f"LDAP 绑定密码解密失败: {e}")
            return None

        # 绑定密码为空时无法进行 LDAP 认证
        if not bind_password:
            logger.warning("LDAP 绑定密码未配置，无法进行 LDAP 认证")
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
    def authenticate_with_config(config: Dict[str, Any], username: str, password: str) -> Optional[dict]:
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
            server_host, server_port, use_ssl = parse_ldap_server_url(server_url)
            timeout = config.get("connect_timeout", DEFAULT_LDAP_CONNECT_TIMEOUT)
            server = Server(
                server_host,
                port=server_port,
                use_ssl=use_ssl,
                get_info=ldap3.ALL,
                connect_timeout=timeout,
            )

            # 使用管理员账号连接
            bind_password = config["bind_password"]
            admin_conn = Connection(
                server,
                user=config["bind_dn"],
                password=bind_password,
                receive_timeout=timeout,  # 添加读取超时，避免服务器响应缓慢时阻塞
            )

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
                size_limit=2,  # 防止过滤器误配导致匹配多用户
                time_limit=timeout,  # 添加搜索超时，防止大型目录搜索阻塞
                attributes=[
                    config["username_attr"],
                    config["email_attr"],
                    config["display_name_attr"],
                ],
            )

            if len(admin_conn.entries) != 1:
                # 统一错误信息，避免泄露用户是否存在；日志仅记录结果数量，不泄露敏感信息
                logger.warning(
                    f"LDAP 认证失败（用户查找阶段）: 搜索返回 {len(admin_conn.entries)} 条结果"
                )
                return None

            user_entry = admin_conn.entries[0]
            user_dn = user_entry.entry_dn

            # 用户密码验证
            user_conn = Connection(
                server,
                user=user_dn,
                password=password,
                receive_timeout=timeout,  # 添加读取超时
            )
            if config.get("use_starttls") and not use_ssl:
                user_conn.start_tls()

            if not user_conn.bind():
                # 统一错误信息，避免泄露密码是否正确；日志仅记录错误码，不泄露用户 DN
                bind_result = user_conn.result.get("description", "unknown")
                logger.warning(f"LDAP 认证失败（密码验证阶段）: {bind_result}")
                return None

            # 提取用户属性（优先用 LDAP 提供的值，不合法则回退默认）
            ldap_username = _get_attr_value(user_entry, config["username_attr"], username)
            email = _get_attr_value(
                user_entry, config["email_attr"], f"{username}@ldap.local"
            )
            display_name = _get_attr_value(user_entry, config["display_name_attr"], username)

            logger.info(f"LDAP 认证成功: {username}")
            return {
                "username": ldap_username,
                "ldap_username": ldap_username,
                "ldap_dn": user_dn,
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
            # 使用循环确保即使第一个 unbind 失败，后续连接仍会尝试关闭
            for conn, name in [(admin_conn, "admin"), (user_conn, "user")]:
                if conn:
                    try:
                        conn.unbind()
                    except Exception as e:
                        logger.warning(f"LDAP {name} 连接关闭失败: {e}")

    @staticmethod
    def test_connection_with_config(config: Dict[str, Any]) -> Tuple[bool, str]:
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

        conn = None
        try:
            server_url = config["server_url"]
            server_host, server_port, use_ssl = parse_ldap_server_url(server_url)
            timeout = config.get("connect_timeout", DEFAULT_LDAP_CONNECT_TIMEOUT)
            server = Server(
                server_host,
                port=server_port,
                use_ssl=use_ssl,
                get_info=ldap3.ALL,
                connect_timeout=timeout,
            )
            bind_password = config["bind_password"]
            conn = Connection(
                server,
                user=config["bind_dn"],
                password=bind_password,
                receive_timeout=timeout,  # 添加读取超时
            )

            if config.get("use_starttls") and not use_ssl:
                conn.start_tls()

            if not conn.bind():
                return False, f"绑定失败: {conn.result}"

            return True, "连接成功"

        except Exception as e:
            # 记录详细错误到日志，但只返回通用信息给前端，避免泄露敏感信息
            logger.error(f"LDAP 测试连接失败: {type(e).__name__}: {e}")
            return False, "连接失败，请检查服务器地址、端口和凭据"
        finally:
            if conn:
                try:
                    conn.unbind()
                except Exception as e:
                    logger.warning(f"LDAP 测试连接关闭失败: {e}")

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
