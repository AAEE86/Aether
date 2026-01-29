"""
输入验证器
包含密码复杂度验证和其他输入验证
"""

import re


class PasswordValidator:
    """密码复杂度验证器"""

    MIN_LENGTH = 6  # 降低到6位
    MAX_LENGTH = 128

    @classmethod
    def validate(cls, password: str) -> tuple[bool, str | None]:
        """
        验证密码复杂度

        要求：
        - 长度至少6个字符

        Args:
            password: 待验证的密码

        Returns:
            (是否通过, 错误消息)
        """
        if not password:
            return False, "密码不能为空"

        if len(password) < cls.MIN_LENGTH:
            return False, f"密码长度至少为{cls.MIN_LENGTH}个字符"

        if len(password) > cls.MAX_LENGTH:
            return False, f"密码长度不能超过{cls.MAX_LENGTH}个字符"

        # 简化密码复杂度要求 - 只检查长度
        # 不再要求大小写字母、数字和特殊字符

        # 检查常见弱密码
        weak_passwords = [
            "password123",
            "admin123",
            "12345678",
            "qwerty123",
            "password@123",
            "admin@123",
            "Password123!",
            "Admin123!",
        ]
        if password.lower() in [p.lower() for p in weak_passwords]:
            return False, "密码过于简单，请使用更复杂的密码"

        return True, None

    @classmethod
    def get_password_strength(cls, password: str) -> str:
        """
        获取密码强度评级

        Args:
            password: 密码

        Returns:
            强度评级: 弱、中、强、非常强
        """
        if not password:
            return "无效"

        score = 0

        # 长度评分
        if len(password) >= 8:
            score += 1
        if len(password) >= 12:
            score += 1
        if len(password) >= 16:
            score += 1

        # 字符类型评分
        if re.search(r"[a-z]", password):
            score += 1
        if re.search(r"[A-Z]", password):
            score += 1
        if re.search(r"\d", password):
            score += 1
        if re.search(r'[!@#$%^&*()_+\-=\[\]{};:\'",.<>?/\\|`~]', password):
            score += 2

        # 额外复杂度评分
        if re.search(r"[^\w\s]", password):  # 非字母数字字符
            score += 1

        if score < 3:
            return "弱"
        elif score < 5:
            return "中"
        elif score < 7:
            return "强"
        else:
            return "非常强"


class EmailValidator:
    """邮箱验证器"""

    EMAIL_REGEX = re.compile(r"^[a-zA-Z0-9._%+-]+@[a-zA-Z0-9.-]+\.[a-zA-Z]{2,}$")

    @classmethod
    def validate(cls, email: str) -> tuple[bool, str | None]:
        """
        验证邮箱格式

        Args:
            email: 待验证的邮箱

        Returns:
            (是否通过, 错误消息)
        """
        if not email:
            return False, "邮箱不能为空"

        if len(email) > 255:
            return False, "邮箱长度不能超过255个字符"

        if not cls.EMAIL_REGEX.match(email):
            return False, "邮箱格式不正确"

        return True, None


class UsernameValidator:
    """用户名验证器"""

    MIN_LENGTH = 3
    MAX_LENGTH = 30
    USERNAME_REGEX = re.compile(r"^[a-zA-Z0-9_.\-]+$")

    @classmethod
    def validate(cls, username: str) -> tuple[bool, str | None]:
        """
        验证用户名

        Args:
            username: 待验证的用户名

        Returns:
            (是否通过, 错误消息)
        """
        if not username:
            return False, "用户名不能为空"

        if len(username) < cls.MIN_LENGTH:
            return False, f"用户名长度至少为{cls.MIN_LENGTH}个字符"

        if len(username) > cls.MAX_LENGTH:
            return False, f"用户名长度不能超过{cls.MAX_LENGTH}个字符"

        if not cls.USERNAME_REGEX.match(username):
            return False, "用户名只能包含字母、数字、下划线、连字符和点号"

        # 检查保留用户名
        reserved_names = [
            "admin",
            "root",
            "system",
            "api",
            "test",
            "demo",
            "user",
            "guest",
            "bot",
            "webhook",
            "support",
        ]
        if username.lower() in reserved_names:
            return False, "该用户名为系统保留用户名"

        return True, None
