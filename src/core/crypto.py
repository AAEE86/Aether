"""
加密工具模块
提供API密钥的加密和解密功能

安全说明:
- 生产环境必须设置独立的 ENCRYPTION_KEY
- 加密密钥应独立于 JWT_SECRET_KEY，避免密钥轮换问题
- 使用 PBKDF2 派生密钥时会使用应用级 salt
"""

from __future__ import annotations
import base64
import hashlib

from cryptography.fernet import Fernet
from cryptography.hazmat.primitives import hashes
from cryptography.hazmat.primitives.kdf.pbkdf2 import PBKDF2HMAC

from ..config import config
from ..core.exceptions import DecryptionException
from src.core.logger import logger



class CryptoService:
    """
    加密服务

    提供对称加密功能，用于保护 Provider API Key 等敏感数据。
    使用 Fernet（AES-128-CBC + HMAC-SHA256）确保数据机密性和完整性。
    """

    _instance = None
    _cipher = None
    _key_source: str = "unknown"  # 记录密钥来源，用于调试

    # 应用级 salt（基于应用名称生成，比硬编码更安全）
    # 注意：更改此值会导致所有已加密数据无法解密
    APP_SALT = hashlib.sha256(b"aether-v1").digest()[:16]

    def __new__(cls) -> CryptoService:
        if cls._instance is None:
            cls._instance = super().__new__(cls)
            cls._instance._initialize()
        return cls._instance

    def _initialize(self) -> None:
        """初始化加密服务"""
        logger.info("初始化加密服务")

        encryption_key = config.encryption_key

        if not encryption_key:
            if config.environment == "production":
                raise ValueError(
                    "ENCRYPTION_KEY must be set in production! "
                    "Use 'python generate_keys.py' to generate a secure key."
                )
            # 开发环境：使用固定的开发密钥
            logger.warning("[DEV] 未设置 ENCRYPTION_KEY，使用开发环境默认密钥。")
            encryption_key = "dev-encryption-key-do-not-use-in-production"
            self._key_source = "development_default"
        else:
            self._key_source = "environment_variable"

        # 派生 Fernet 密钥
        key = self._derive_fernet_key(encryption_key)

        self._cipher = Fernet(key)
        logger.info(f"加密服务初始化成功 (key_source={self._key_source})")

    def _derive_fernet_key(self, encryption_key: str) -> bytes:
        """
        从密码/密钥派生 Fernet 兼容的密钥

        Args:
            encryption_key: 原始密钥字符串

        Returns:
            Fernet 兼容的 base64 编码密钥
        """
        # 首先尝试直接作为 Fernet 密钥使用
        try:
            key_bytes = (
                encryption_key.encode() if isinstance(encryption_key, str) else encryption_key
            )
            # 验证是否为有效的 Fernet 密钥（32 字节 base64 编码）
            Fernet(key_bytes)
            return key_bytes
        except Exception:
            pass

        # 不是有效的 Fernet 密钥，使用 PBKDF2 派生
        kdf = PBKDF2HMAC(
            algorithm=hashes.SHA256(),
            length=32,
            salt=self.APP_SALT,
            iterations=100000,
        )
        derived_key = kdf.derive(encryption_key.encode())
        return base64.urlsafe_b64encode(derived_key)

    def encrypt(self, plaintext: str) -> str:
        """
        加密字符串

        Args:
            plaintext: 明文字符串

        Returns:
            加密后的字符串（base64编码）
        """
        if not plaintext:
            return plaintext

        try:
            encrypted = self._cipher.encrypt(plaintext.encode())
            return base64.urlsafe_b64encode(encrypted).decode()
        except Exception as e:
            logger.error(f"Encryption failed: {e}")
            raise ValueError("Failed to encrypt data")

    def decrypt(self, ciphertext: str, silent: bool = False) -> str:
        """
        解密字符串

        Args:
            ciphertext: 加密的字符串（base64编码）
            silent: 是否静默模式（失败时不打印错误日志）

        Returns:
            解密后的明文字符串

        Raises:
            DecryptionException: 解密失败时抛出异常
        """
        if not ciphertext:
            return ciphertext

        try:
            encrypted = base64.urlsafe_b64decode(ciphertext.encode())
            decrypted = self._cipher.decrypt(encrypted)
            return decrypted.decode()
        except Exception as e:
            if not silent:
                logger.error(f"Decryption failed: {e}")
            # 抛出自定义异常，方便在上层通过类型判断是否需要打印堆栈
            raise DecryptionException(
                message=f"解密失败: {str(e)}。可能原因: ENCRYPTION_KEY 已改变或数据已损坏。解决方案: 请在管理面板重新设置 Provider API Key。",
                details={"original_error": str(e), "key_source": self._key_source},
            )

    def hash_api_key(self, api_key: str) -> str:
        """
        对API密钥进行哈希（用于查找）

        Args:
            api_key: API密钥明文

        Returns:
            哈希后的值
        """
        return hashlib.sha256(api_key.encode()).hexdigest()


# 创建全局加密服务实例
crypto_service = CryptoService()
