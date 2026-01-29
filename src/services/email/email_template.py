"""
邮件模板
提供验证码邮件的 HTML 和纯文本模板，支持从数据库加载自定义模板
"""

import html
import re
from html.parser import HTMLParser
from typing import Any

from sqlalchemy.orm import Session

from src.services.system.config import SystemConfigService


class HTMLToTextParser(HTMLParser):
    """HTML 转纯文本解析器"""

    def __init__(self):
        super().__init__()
        self.text_parts = []
        self.skip_data = False

    def handle_starttag(self, tag, attrs):  # noqa: ARG002
        if tag in ("script", "style", "head"):
            self.skip_data = True
        elif tag == "br":
            self.text_parts.append("\n")
        elif tag in ("p", "div", "tr", "h1", "h2", "h3", "h4", "h5", "h6"):
            self.text_parts.append("\n")

    def handle_endtag(self, tag):
        if tag in ("script", "style", "head"):
            self.skip_data = False
        elif tag in ("p", "div", "tr", "h1", "h2", "h3", "h4", "h5", "h6", "td"):
            self.text_parts.append("\n")

    def handle_data(self, data):
        if not self.skip_data:
            text = data.strip()
            if text:
                self.text_parts.append(text)


class EmailTemplate:
    """邮件模板类"""

    # 模板类型定义
    TEMPLATE_VERIFICATION = "verification"
    TEMPLATE_PASSWORD_RESET = "password_reset"

    # 支持的模板类型及其变量
    TEMPLATE_TYPES = {
        TEMPLATE_VERIFICATION: {
            "name": "注册验证码",
            "variables": ["app_name", "code", "expire_minutes", "email"],
            "default_subject": "验证码",
        },
        TEMPLATE_PASSWORD_RESET: {
            "name": "找回密码",
            "variables": ["app_name", "reset_link", "expire_minutes", "email"],
            "default_subject": "密码重置",
        },
    }

    # Literary Tech 主题色 - 与网页保持一致
    PRIMARY_COLOR = "#c96442"  # book-cloth
    PRIMARY_LIGHT = "#e4b2a0"  # kraft
    BG_WARM = "#faf9f5"  # ivory-light
    BG_MEDIUM = "#e9e6dc"  # ivory-medium / cloud-medium
    TEXT_DARK = "#3d3929"  # slate-dark
    TEXT_MUTED = "#6c695c"  # slate-medium
    BORDER_COLOR = "rgba(61, 57, 41, 0.12)"

    @staticmethod
    def get_default_verification_html() -> str:
        """获取默认的验证码邮件 HTML 模板 - Literary Tech 风格"""
        return """<!DOCTYPE html>
<html lang="zh-CN">
<head>
    <meta charset="UTF-8">
    <meta name="viewport" content="width=device-width, initial-scale=1.0">
    <title>验证码</title>
</head>
<body style="margin: 0; padding: 0; background-color: #faf9f5; font-family: Georgia, 'Times New Roman', 'Songti SC', 'STSong', serif;">
    <table width="100%" cellpadding="0" cellspacing="0" style="background-color: #faf9f5; padding: 40px 20px;">
        <tr>
            <td align="center">
                <table width="100%" cellpadding="0" cellspacing="0" style="max-width: 480px;">
                    <!-- Header -->
                    <tr>
                        <td style="padding: 0 0 32px; text-align: center;">
                            <div style="font-size: 13px; font-family: 'SF Mono', Monaco, 'Courier New', monospace; color: #6c695c; letter-spacing: 0.15em; text-transform: uppercase;">
                                {{app_name}}
                            </div>
                        </td>
                    </tr>

                    <!-- Main Card -->
                    <tr>
                        <td>
                            <table width="100%" cellpadding="0" cellspacing="0" style="background-color: #ffffff; border: 1px solid rgba(61, 57, 41, 0.1); border-radius: 6px;">
                                <!-- Content -->
                                <tr>
                                    <td style="padding: 48px 40px;">
                                        <h1 style="margin: 0 0 24px; font-size: 24px; font-weight: 500; color: #3d3929; text-align: center; letter-spacing: -0.02em;">
                                            验证码
                                        </h1>

                                        <p style="margin: 0 0 32px; font-size: 15px; color: #6c695c; line-height: 1.7; text-align: center;">
                                            您正在注册账户，请使用以下验证码完成验证。
                                        </p>

                                        <!-- Code Box -->
                                        <div style="background-color: #faf9f5; border: 1px solid rgba(61, 57, 41, 0.08); border-radius: 4px; padding: 32px 20px; text-align: center; margin-bottom: 32px;">
                                            <div style="font-size: 40px; font-weight: 500; color: #c96442; letter-spacing: 12px; font-family: 'SF Mono', Monaco, 'Courier New', monospace;">
                                                {{code}}
                                            </div>
                                        </div>

                                        <p style="margin: 0; font-size: 14px; color: #6c695c; line-height: 1.6; text-align: center;">
                                            验证码将在 <span style="color: #3d3929; font-weight: 500;">{{expire_minutes}} 分钟</span>后失效
                                        </p>
                                    </td>
                                </tr>
                            </table>
                        </td>
                    </tr>

                    <!-- Footer -->
                    <tr>
                        <td style="padding: 32px 0 0; text-align: center;">
                            <p style="margin: 0 0 8px; font-size: 12px; color: #6c695c;">
                                如果这不是您的操作，请忽略此邮件。
                            </p>
                            <p style="margin: 0; font-size: 11px; color: rgba(108, 105, 92, 0.6);">
                                此邮件由系统自动发送，请勿回复
                            </p>
                        </td>
                    </tr>
                </table>
            </td>
        </tr>
    </table>
</body>
</html>"""

    @staticmethod
    def get_default_password_reset_html() -> str:
        """获取默认的密码重置邮件 HTML 模板 - Literary Tech 风格"""
        return """<!DOCTYPE html>
<html lang="zh-CN">
<head>
    <meta charset="UTF-8">
    <meta name="viewport" content="width=device-width, initial-scale=1.0">
    <title>密码重置</title>
</head>
<body style="margin: 0; padding: 0; background-color: #faf9f5; font-family: Georgia, 'Times New Roman', 'Songti SC', 'STSong', serif;">
    <table width="100%" cellpadding="0" cellspacing="0" style="background-color: #faf9f5; padding: 40px 20px;">
        <tr>
            <td align="center">
                <table width="100%" cellpadding="0" cellspacing="0" style="max-width: 480px;">
                    <!-- Header -->
                    <tr>
                        <td style="padding: 0 0 32px; text-align: center;">
                            <div style="font-size: 13px; font-family: 'SF Mono', Monaco, 'Courier New', monospace; color: #6c695c; letter-spacing: 0.15em; text-transform: uppercase;">
                                {{app_name}}
                            </div>
                        </td>
                    </tr>

                    <!-- Main Card -->
                    <tr>
                        <td>
                            <table width="100%" cellpadding="0" cellspacing="0" style="background-color: #ffffff; border: 1px solid rgba(61, 57, 41, 0.1); border-radius: 6px;">
                                <!-- Content -->
                                <tr>
                                    <td style="padding: 48px 40px;">
                                        <h1 style="margin: 0 0 24px; font-size: 24px; font-weight: 500; color: #3d3929; text-align: center; letter-spacing: -0.02em;">
                                            重置密码
                                        </h1>

                                        <p style="margin: 0 0 32px; font-size: 15px; color: #6c695c; line-height: 1.7; text-align: center;">
                                            您正在重置账户密码，请点击下方按钮完成操作。
                                        </p>

                                        <!-- Button -->
                                        <div style="text-align: center; margin-bottom: 32px;">
                                            <a href="{{reset_link}}" style="display: inline-block; padding: 14px 36px; background-color: #c96442; color: #ffffff; text-decoration: none; border-radius: 4px; font-size: 15px; font-weight: 500; font-family: -apple-system, BlinkMacSystemFont, 'Segoe UI', sans-serif;">
                                                重置密码
                                            </a>
                                        </div>

                                        <p style="margin: 0; font-size: 14px; color: #6c695c; line-height: 1.6; text-align: center;">
                                            链接将在 <span style="color: #3d3929; font-weight: 500;">{{expire_minutes}} 分钟</span>后失效
                                        </p>
                                    </td>
                                </tr>
                            </table>
                        </td>
                    </tr>

                    <!-- Footer -->
                    <tr>
                        <td style="padding: 32px 0 0; text-align: center;">
                            <p style="margin: 0 0 8px; font-size: 12px; color: #6c695c;">
                                如果您没有请求重置密码，请忽略此邮件。
                            </p>
                            <p style="margin: 0; font-size: 11px; color: rgba(108, 105, 92, 0.6);">
                                此邮件由系统自动发送，请勿回复
                            </p>
                        </td>
                    </tr>
                </table>
            </td>
        </tr>
    </table>
</body>
</html>"""

    @staticmethod
    def get_default_template(template_type: str) -> dict[str, str]:
        """
        获取默认模板

        Args:
            template_type: 模板类型

        Returns:
            包含 subject 和 html 的字典
        """
        if template_type == EmailTemplate.TEMPLATE_VERIFICATION:
            return {
                "subject": "验证码",
                "html": EmailTemplate.get_default_verification_html(),
            }
        elif template_type == EmailTemplate.TEMPLATE_PASSWORD_RESET:
            return {
                "subject": "密码重置",
                "html": EmailTemplate.get_default_password_reset_html(),
            }
        else:
            return {"subject": "通知", "html": ""}

    @staticmethod
    def get_template(db: Session, template_type: str) -> dict[str, str]:
        """
        从数据库获取模板，如果不存在则返回默认模板

        Args:
            db: 数据库会话
            template_type: 模板类型

        Returns:
            包含 subject 和 html 的字典
        """
        default = EmailTemplate.get_default_template(template_type)

        # 从数据库获取自定义模板
        subject_key = f"email_template_{template_type}_subject"
        html_key = f"email_template_{template_type}_html"

        custom_subject = SystemConfigService.get_config(db, subject_key, default=None)
        custom_html = SystemConfigService.get_config(db, html_key, default=None)

        return {
            "subject": custom_subject if custom_subject else default["subject"],
            "html": custom_html if custom_html else default["html"],
        }

    @staticmethod
    def render_template(template_html: str, variables: dict[str, Any]) -> str:
        """
        渲染模板，替换 {{variable}} 格式的变量

        Args:
            template_html: HTML 模板
            variables: 变量字典

        Returns:
            渲染后的 HTML
        """
        result = template_html
        for key, value in variables.items():
            # HTML 转义变量值，防止 XSS
            escaped_value = html.escape(str(value))
            # 替换 {{key}} 格式的变量
            pattern = r"\{\{\s*" + re.escape(key) + r"\s*\}\}"
            result = re.sub(pattern, escaped_value, result)
        return result

    @staticmethod
    def html_to_text(html: str) -> str:
        """
        从 HTML 提取纯文本

        Args:
            html: HTML 内容

        Returns:
            纯文本内容
        """
        parser = HTMLToTextParser()
        parser.feed(html)
        text = " ".join(parser.text_parts)
        # 清理多余空白
        text = re.sub(r"\n\s*\n", "\n\n", text)
        text = re.sub(r" +", " ", text)
        return text.strip()

    @staticmethod
    def get_verification_code_html(
        code: str, expire_minutes: int = 5, db: Session | None = None, **kwargs
    ) -> str:
        """
        获取验证码邮件 HTML

        Args:
            code: 验证码
            expire_minutes: 过期时间（分钟）
            db: 数据库会话（用于获取自定义模板）
            **kwargs: 其他模板变量

        Returns:
            渲染后的 HTML
        """
        app_name = kwargs.get("app_name", "Aether")
        email = kwargs.get("email", "")

        # 获取模板
        if db:
            template = EmailTemplate.get_template(db, EmailTemplate.TEMPLATE_VERIFICATION)
        else:
            template = EmailTemplate.get_default_template(EmailTemplate.TEMPLATE_VERIFICATION)

        # 渲染变量
        variables = {
            "app_name": app_name,
            "code": code,
            "expire_minutes": expire_minutes,
            "email": email,
        }

        return EmailTemplate.render_template(template["html"], variables)

    @staticmethod
    def get_verification_code_text(
        code: str, expire_minutes: int = 5, db: Session | None = None, **kwargs
    ) -> str:
        """
        获取验证码邮件纯文本（从 HTML 自动生成）

        Args:
            code: 验证码
            expire_minutes: 过期时间（分钟）
            db: 数据库会话
            **kwargs: 其他模板变量

        Returns:
            纯文本邮件内容
        """
        html = EmailTemplate.get_verification_code_html(code, expire_minutes, db, **kwargs)
        return EmailTemplate.html_to_text(html)

    @staticmethod
    def get_password_reset_html(
        reset_link: str, expire_minutes: int = 30, db: Session | None = None, **kwargs
    ) -> str:
        """
        获取密码重置邮件 HTML

        Args:
            reset_link: 重置链接
            expire_minutes: 过期时间（分钟）
            db: 数据库会话
            **kwargs: 其他模板变量

        Returns:
            渲染后的 HTML
        """
        app_name = kwargs.get("app_name", "Aether")
        email = kwargs.get("email", "")

        # 获取模板
        if db:
            template = EmailTemplate.get_template(db, EmailTemplate.TEMPLATE_PASSWORD_RESET)
        else:
            template = EmailTemplate.get_default_template(EmailTemplate.TEMPLATE_PASSWORD_RESET)

        # 渲染变量
        variables = {
            "app_name": app_name,
            "reset_link": reset_link,
            "expire_minutes": expire_minutes,
            "email": email,
        }

        return EmailTemplate.render_template(template["html"], variables)

    @staticmethod
    def get_password_reset_text(
        reset_link: str, expire_minutes: int = 30, db: Session | None = None, **kwargs
    ) -> str:
        """
        获取密码重置邮件纯文本（从 HTML 自动生成）

        Args:
            reset_link: 重置链接
            expire_minutes: 过期时间（分钟）
            db: 数据库会话
            **kwargs: 其他模板变量

        Returns:
            纯文本邮件内容
        """
        html = EmailTemplate.get_password_reset_html(reset_link, expire_minutes, db, **kwargs)
        return EmailTemplate.html_to_text(html)

    @staticmethod
    def get_subject(
        template_type: str = "verification", db: Session | None = None
    ) -> str:
        """
        获取邮件主题

        Args:
            template_type: 模板类型
            db: 数据库会话

        Returns:
            邮件主题
        """
        if db:
            template = EmailTemplate.get_template(db, template_type)
            return template["subject"]

        default_subjects = {
            "verification": "验证码",
            "welcome": "欢迎加入",
            "password_reset": "密码重置",
        }
        return default_subjects.get(template_type, "通知")
