"""
能力需求解析器

负责从各种来源解析请求的能力需求：
1. 用户模型级配置 (User.model_capability_settings)
2. 用户 API Key 强制配置 (ApiKey.force_capabilities)
3. 请求头 X-Require-Capability（显式声明）
4. Adapter 的 detect_capability_requirements（如 Claude 的 anthropic-beta）
5. 显式传入 (用于重试升级)
"""

from typing import Any

from collections.abc import Callable

from src.core.key_capabilities import (
    CAPABILITY_DEFINITIONS,
    CapabilityConfigMode,
    get_user_configurable_capabilities,
)
from src.core.api_format import get_header_value
from src.core.logger import logger

# Adapter 检测器类型：接受 headers 和可选的 request_body，返回能力需求字典
type AdapterDetectorType = Callable[[dict[str, str], dict[str, Any] | None], dict[str, bool]]


class CapabilityResolver:
    """能力需求解析器"""

    @staticmethod
    def resolve_requirements(
        user: Any | None = None,
        user_api_key: Any | None = None,
        model_name: str | None = None,
        request_headers: dict[str, str] | None = None,
        request_body: dict[str, Any] | None = None,
        explicit_requirements: dict[str, bool] | None = None,
        adapter_detector: AdapterDetectorType | None = None,
    ) -> dict[str, bool]:
        """
        解析请求的能力需求

        来源优先级（后者覆盖前者）:
        1. 用户模型级配置 (User.model_capability_settings)
        2. 用户 API Key 强制配置 (ApiKey.force_capabilities)
        3. 请求头 X-Require-Capability（显式声明）
        4. Adapter 的 detect_capability_requirements（如 Claude 的 anthropic-beta）
        5. 显式传入的 explicit_requirements（用于重试升级）

        Args:
            user: User 对象
            user_api_key: 用户 ApiKey 对象
            model_name: 模型名称（用于查找模型级配置）
            request_headers: 请求头
            request_body: 请求体（可选，部分 Adapter 可能需要）
            explicit_requirements: 显式传入的需求（重试时使用）
            adapter_detector: Adapter 的能力检测方法

        Returns:
            能力需求字典，如 {"cache_1h": True, "context_1m": False}
        """
        requirements: dict[str, bool] = {}

        # 1. 从用户模型级配置获取（仅用户可配置型能力）
        if user and model_name:
            model_settings = getattr(user, "model_capability_settings", None) or {}
            model_caps = model_settings.get(model_name, {})
            if model_caps:
                for cap_name, cap_value in model_caps.items():
                    cap_def = CAPABILITY_DEFINITIONS.get(cap_name)
                    if cap_def and cap_def.config_mode == CapabilityConfigMode.USER_CONFIGURABLE:
                        requirements[cap_name] = bool(cap_value)
                        logger.debug(
                            f"[CapabilityResolver] 从用户模型配置获取 {cap_name}={cap_value} "
                            f"(model={model_name})"
                        )

        # 2. 从用户 API Key 强制配置获取（覆盖模型级配置）
        if user_api_key:
            force_caps = getattr(user_api_key, "force_capabilities", None) or {}
            if force_caps:
                for cap_name, cap_value in force_caps.items():
                    cap_def = CAPABILITY_DEFINITIONS.get(cap_name)
                    if cap_def and cap_def.config_mode == CapabilityConfigMode.USER_CONFIGURABLE:
                        requirements[cap_name] = bool(cap_value)
                        logger.debug(
                            f"[CapabilityResolver] 从 API Key 强制配置获取 {cap_name}={cap_value}"
                        )

        # 3. 从请求头 X-Require-Capability 获取（显式声明）
        if request_headers:
            header_caps = get_header_value(request_headers, "X-Require-Capability")
            if header_caps:
                for cap in header_caps.split(","):
                    cap = cap.strip()
                    if not cap:
                        continue
                    if cap.startswith("-"):
                        # -cache_1h 表示不需要
                        cap_name = cap[1:]
                        requirements[cap_name] = False
                    else:
                        requirements[cap] = True
                    logger.debug(
                        f"[CapabilityResolver] 从请求头获取 {cap_name if cap.startswith('-') else cap}"
                    )

        # 4. 从 Adapter 的 detect_capability_requirements 获取
        if adapter_detector and request_headers:
            detected = adapter_detector(request_headers, request_body)
            for cap_name, cap_value in detected.items():
                # 只有尚未设置的能力才从 Adapter 检测
                if cap_name not in requirements:
                    requirements[cap_name] = cap_value
                    logger.debug(
                        f"[CapabilityResolver] 从 Adapter 检测到 {cap_name}={cap_value}"
                    )

        # 5. 显式覆盖（重试时使用）
        if explicit_requirements:
            for cap_name, cap_value in explicit_requirements.items():
                requirements[cap_name] = cap_value
                logger.debug(f"[CapabilityResolver] 显式覆盖 {cap_name}={cap_value}")

        return requirements

    @staticmethod
    def get_default_requirements_for_model(
        user: Any | None = None,
        model_name: str | None = None,
    ) -> dict[str, bool]:
        """
        获取用户对特定模型的默认能力需求

        仅返回用户可配置型能力的配置。

        Args:
            user: User 对象
            model_name: 模型名称

        Returns:
            能力需求字典
        """
        requirements: dict[str, bool] = {}

        if not user or not model_name:
            return requirements

        model_settings = getattr(user, "model_capability_settings", None) or {}
        model_caps = model_settings.get(model_name, {})

        for cap_def in get_user_configurable_capabilities():
            if cap_def.name in model_caps:
                requirements[cap_def.name] = bool(model_caps[cap_def.name])

        return requirements

    @staticmethod
    def merge_requirements(
        base: dict[str, bool] | None,
        override: dict[str, bool] | None,
    ) -> dict[str, bool]:
        """
        合并两个能力需求字典

        Args:
            base: 基础需求
            override: 覆盖需求

        Returns:
            合并后的需求
        """
        result = dict(base or {})
        if override:
            result.update(override)
        return result
