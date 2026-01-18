"""
Provider 操作服务

提供操作执行、凭据管理、缓存等业务逻辑。
"""

import asyncio
import json
from dataclasses import asdict
from datetime import datetime, timezone
from typing import Any, Dict, List, Optional

from sqlalchemy.orm import Session

from src.core.cache_service import CacheService
from src.core.crypto import CryptoService
from src.core.logger import logger
from src.database import create_session
from src.models.database import Provider
from src.services.provider_ops.architectures import ProviderArchitecture, ProviderConnector
from src.services.provider_ops.registry import get_registry
from src.services.provider_ops.types import (
    ActionResult,
    ActionStatus,
    BalanceInfo,
    ConnectorAuthType,
    ConnectorState,
    ConnectorStatus,
    ProviderActionType,
    ProviderOpsConfig,
)

# 余额缓存 TTL（24 小时）
BALANCE_CACHE_TTL = 86400


class ProviderOpsService:
    """
    Provider 操作服务

    提供：
    - 凭据管理（加密存储、读取）
    - 连接管理（建立、断开、状态检查）
    - 操作执行（余额查询、签到等）
    """

    # 凭据中需要加密的字段
    SENSITIVE_FIELDS = {"api_key", "password", "session_token", "cookie_string", "cookies"}

    def __init__(self, db: Session):
        self.db = db
        self.crypto = CryptoService()

        # 连接器缓存 {provider_id: ProviderConnector}
        self._connectors: Dict[str, ProviderConnector] = {}

    # ==================== 配置管理 ====================

    def get_config(self, provider_id: str) -> Optional[ProviderOpsConfig]:
        """
        获取 Provider 的操作配置

        Args:
            provider_id: Provider ID

        Returns:
            配置对象，未配置则返回 None
        """
        provider = self._get_provider(provider_id)
        if not provider:
            return None

        config_data = (provider.config or {}).get("provider_ops")
        if not config_data:
            return None

        return ProviderOpsConfig.from_dict(config_data)

    def save_config(
        self,
        provider_id: str,
        config: ProviderOpsConfig,
    ) -> bool:
        """
        保存 Provider 的操作配置

        Args:
            provider_id: Provider ID
            config: 配置对象

        Returns:
            是否保存成功
        """
        provider = self._get_provider(provider_id)
        if not provider:
            return False

        # 加密敏感凭据
        encrypted_credentials = self._encrypt_credentials(config.connector_credentials)
        logger.debug(
            f"加密凭据: provider_id={provider_id}, "
            f"input_keys={list(config.connector_credentials.keys())}, "
            f"output_keys={list(encrypted_credentials.keys())}, "
            f"has_api_key={bool(config.connector_credentials.get('api_key'))}"
        )

        # 构建配置
        config_dict = config.to_dict()
        config_dict["connector"]["credentials"] = encrypted_credentials

        # 更新 Provider 配置
        provider_config = dict(provider.config or {})
        provider_config["provider_ops"] = config_dict
        provider.config = provider_config

        self.db.commit()

        # 清除连接器缓存
        if provider_id in self._connectors:
            del self._connectors[provider_id]

        logger.info(f"保存 Provider 操作配置: provider_id={provider_id}")
        return True

    def delete_config(self, provider_id: str) -> bool:
        """
        删除 Provider 的操作配置

        Args:
            provider_id: Provider ID

        Returns:
            是否删除成功
        """
        provider = self._get_provider(provider_id)
        if not provider:
            return False

        provider_config = dict(provider.config or {})
        if "provider_ops" in provider_config:
            del provider_config["provider_ops"]
            provider.config = provider_config
            self.db.commit()

        # 清除连接器缓存
        if provider_id in self._connectors:
            del self._connectors[provider_id]

        return True

    # ==================== 连接管理 ====================

    async def connect(
        self,
        provider_id: str,
        credentials: Optional[Dict[str, Any]] = None,
    ) -> tuple[bool, str]:
        """
        建立与 Provider 的连接

        Args:
            provider_id: Provider ID
            credentials: 凭据（如果为 None 则使用已保存的凭据）

        Returns:
            (是否成功, 消息)
        """
        provider = self._get_provider(provider_id)
        if not provider:
            return False, "Provider 不存在"

        config = self.get_config(provider_id)
        if not config:
            return False, "未配置操作设置"

        # 获取架构
        registry = get_registry()
        architecture = registry.get_or_default(config.architecture_id)

        # 获取 base_url
        base_url = self._get_provider_base_url(provider)
        if not base_url:
            return False, "Provider 未配置 base_url"

        # 创建连接器
        try:
            connector = architecture.get_connector(
                base_url=base_url,
                auth_type=config.connector_auth_type,
                config=config.connector_config,
            )
        except ValueError as e:
            return False, str(e)

        # 使用提供的凭据或已保存的凭据
        if credentials:
            actual_credentials = credentials
        else:
            actual_credentials = self._decrypt_credentials(config.connector_credentials)
            logger.debug(
                f"解密凭据: provider_id={provider_id}, "
                f"encrypted_keys={list(config.connector_credentials.keys())}, "
                f"decrypted_keys={list(actual_credentials.keys())}, "
                f"has_api_key={bool(actual_credentials.get('api_key'))}"
            )

        if not actual_credentials:
            return False, "未提供凭据"

        # 建立连接
        logger.info(
            f"尝试连接: provider_id={provider_id}, "
            f"credentials_keys={list(actual_credentials.keys())}"
        )
        success = await connector.connect(actual_credentials)
        if success:
            self._connectors[provider_id] = connector
            return True, "连接成功"
        else:
            state = connector.get_state()
            return False, state.last_error or "连接失败"

    async def disconnect(self, provider_id: str) -> bool:
        """
        断开与 Provider 的连接

        Args:
            provider_id: Provider ID

        Returns:
            是否成功
        """
        connector = self._connectors.get(provider_id)
        if connector:
            await connector.disconnect()
            del self._connectors[provider_id]
        return True

    def get_connection_status(self, provider_id: str) -> ConnectorState:
        """
        获取连接状态

        Args:
            provider_id: Provider ID

        Returns:
            连接器状态
        """
        connector = self._connectors.get(provider_id)
        if connector:
            return connector.get_state()

        # 未连接
        config = self.get_config(provider_id)
        return ConnectorState(
            status=ConnectorStatus.DISCONNECTED,
            auth_type=config.connector_auth_type if config else ConnectorAuthType.NONE,
        )

    # ==================== 操作执行 ====================

    async def execute_action(
        self,
        provider_id: str,
        action_type: ProviderActionType,
        action_config: Optional[Dict[str, Any]] = None,
    ) -> ActionResult:
        """
        执行操作

        Args:
            provider_id: Provider ID
            action_type: 操作类型
            action_config: 操作配置（覆盖默认配置）

        Returns:
            操作结果
        """
        # 检查连接状态
        connector = self._connectors.get(provider_id)
        if not connector:
            # 尝试自动连接
            success, message = await self.connect(provider_id)
            if not success:
                return ActionResult(
                    status=ActionStatus.AUTH_FAILED,
                    action_type=action_type,
                    message=f"连接失败: {message}",
                )
            connector = self._connectors.get(provider_id)

        if not connector or not await connector.is_authenticated():
            return ActionResult(
                status=ActionStatus.AUTH_EXPIRED,
                action_type=action_type,
                message="认证已过期，请重新连接",
            )

        # 获取配置
        config = self.get_config(provider_id)
        if not config:
            return ActionResult(
                status=ActionStatus.NOT_CONFIGURED,
                action_type=action_type,
                message="未配置操作设置",
            )

        # 获取架构
        registry = get_registry()
        architecture = registry.get_or_default(config.architecture_id)

        # 检查是否支持该操作
        if not architecture.supports_action(action_type):
            return ActionResult(
                status=ActionStatus.NOT_SUPPORTED,
                action_type=action_type,
                message=f"架构 {architecture.architecture_id} 不支持 {action_type.value} 操作",
            )

        # 合并操作配置
        saved_action_config = config.actions.get(action_type.value, {}).get("config", {})
        merged_config = {**saved_action_config, **(action_config or {})}

        # 创建操作实例
        action = architecture.get_action(action_type, merged_config)

        # 执行操作
        async with connector.get_client() as client:
            result = await action.execute(client)

        return result

    async def query_balance(
        self,
        provider_id: str,
        config: Optional[Dict[str, Any]] = None,
    ) -> ActionResult:
        """
        查询余额（快捷方法）

        Args:
            provider_id: Provider ID
            config: 操作配置

        Returns:
            操作结果
        """
        result = await self.execute_action(
            provider_id, ProviderActionType.QUERY_BALANCE, config
        )

        # 成功时更新缓存
        if result.status == ActionStatus.SUCCESS and result.data:
            await self._cache_balance(provider_id, result)

        return result

    async def query_balance_with_cache(
        self,
        provider_id: str,
        trigger_refresh: bool = True,
    ) -> ActionResult:
        """
        查询余额（优先返回缓存，可触发异步刷新）

        Args:
            provider_id: Provider ID
            trigger_refresh: 是否触发后台异步刷新

        Returns:
            操作结果（可能是缓存的）
        """
        # 尝试从缓存获取
        cached = await self._get_cached_balance(provider_id)

        if cached:
            # 有缓存，可选触发后台刷新
            if trigger_refresh:
                # 后台任务内部已处理异常并记录日志，无需额外回调
                asyncio.create_task(self._refresh_balance_async(provider_id))
            return cached

        # 没有缓存，同步查询一次（首次访问）
        logger.info(f"余额缓存未命中，同步查询: provider_id={provider_id}")
        return await self.query_balance(provider_id)

    async def _refresh_balance_async(self, provider_id: str) -> None:
        """后台异步刷新余额（使用独立的数据库 session）"""
        try:
            # 后台任务需要创建独立的 session，因为原请求的 session 可能已关闭
            with create_session() as db:
                service = ProviderOpsService(db)
                await service.query_balance(provider_id)
        except Exception as e:
            logger.warning(f"异步刷新余额失败: provider_id={provider_id}, error={e}")

    async def _cache_balance(self, provider_id: str, result: ActionResult) -> None:
        """缓存余额结果"""
        cache_key = f"provider_ops:balance:{provider_id}"

        # 序列化 BalanceInfo
        data = result.data
        if isinstance(data, BalanceInfo):
            data = asdict(data)

        cache_data = {
            "status": result.status.value,
            "data": data,
            "executed_at": result.executed_at.isoformat(),
            "response_time_ms": result.response_time_ms,
        }

        await CacheService.set(cache_key, cache_data, BALANCE_CACHE_TTL)

    async def _cache_balance_from_verify(self, provider_id: str, quota_usd: float) -> None:
        """
        从验证结果缓存余额

        Args:
            provider_id: Provider ID
            quota_usd: 已转换为美元的余额值
        """
        cache_key = f"provider_ops:balance:{provider_id}"

        # 构建与 BalanceAction 兼容的缓存数据
        # 注意：验证接口只返回 quota，没有 total_granted/total_used
        cache_data = {
            "status": "success",
            "data": {
                "total_granted": None,
                "total_used": None,
                "total_available": quota_usd,
                "currency": "USD",
                "extra": {},
            },
            "executed_at": datetime.now(timezone.utc).isoformat(),
            "response_time_ms": None,
        }

        await CacheService.set(cache_key, cache_data, BALANCE_CACHE_TTL)
        logger.debug(f"验证成功，缓存余额: provider_id={provider_id}, quota_usd={quota_usd}")

    async def _get_cached_balance(self, provider_id: str) -> Optional[ActionResult]:
        """获取缓存的余额"""
        cache_key = f"provider_ops:balance:{provider_id}"
        cached = await CacheService.get(cache_key)

        if not cached:
            return None

        # 反序列化
        try:
            data = cached.get("data")
            if data and isinstance(data, dict):
                # 转回 BalanceInfo
                data = BalanceInfo(
                    total_granted=data.get("total_granted"),
                    total_used=data.get("total_used"),
                    total_available=data.get("total_available"),
                    currency=data.get("currency", "USD"),
                    extra=data.get("extra", {}),
                )

            executed_at_str = cached.get("executed_at")
            executed_at = (
                datetime.fromisoformat(executed_at_str)
                if executed_at_str
                else datetime.now(timezone.utc)
            )

            return ActionResult(
                status=ActionStatus(cached.get("status", "success")),
                action_type=ProviderActionType.QUERY_BALANCE,
                data=data,
                executed_at=executed_at,
                response_time_ms=cached.get("response_time_ms"),
                cache_ttl_seconds=BALANCE_CACHE_TTL,
            )
        except Exception as e:
            logger.warning(f"解析缓存余额失败: provider_id={provider_id}, error={e}")
            return None

    async def checkin(
        self,
        provider_id: str,
        config: Optional[Dict[str, Any]] = None,
    ) -> ActionResult:
        """
        签到（快捷方法）

        Args:
            provider_id: Provider ID
            config: 操作配置

        Returns:
            操作结果
        """
        return await self.execute_action(provider_id, ProviderActionType.CHECKIN, config)

    # ==================== 辅助方法 ====================

    def _get_provider(self, provider_id: str) -> Optional[Provider]:
        """获取 Provider"""
        return self.db.query(Provider).filter(Provider.id == provider_id).first()

    def _get_provider_base_url(self, provider: Provider) -> Optional[str]:
        """从 Provider 获取 base_url"""
        # 优先从第一个 endpoint 获取
        if provider.endpoints:
            for endpoint in provider.endpoints:
                if endpoint.base_url:
                    return endpoint.base_url

        # 从 config 获取
        config = provider.config or {}
        if "base_url" in config:
            return config["base_url"]

        # 从 website 获取
        if provider.website:
            return provider.website

        return None

    def _encrypt_credentials(self, credentials: Dict[str, Any]) -> Dict[str, Any]:
        """加密凭据中的敏感字段"""
        encrypted = {}
        for key, value in credentials.items():
            if key in self.SENSITIVE_FIELDS and isinstance(value, str):
                if value:  # 只加密非空值
                    encrypted[key] = self.crypto.encrypt(value)
                    logger.debug(f"加密字段 {key}: 原始长度={len(value)}, 加密后长度={len(encrypted[key])}")
                else:
                    logger.warning(f"跳过空值字段 {key}")
                    encrypted[key] = value
            elif key == "cookies" and isinstance(value, dict):
                # cookies 整体加密
                encrypted[key] = self.crypto.encrypt(json.dumps(value))
            else:
                encrypted[key] = value
        return encrypted

    def _decrypt_credentials(self, credentials: Dict[str, Any]) -> Dict[str, Any]:
        """解密凭据中的敏感字段"""
        decrypted = {}
        for key, value in credentials.items():
            if key in self.SENSITIVE_FIELDS and isinstance(value, str):
                try:
                    decrypted[key] = self.crypto.decrypt(value)
                except Exception as e:
                    logger.warning(f"解密字段 {key} 失败: {e}")
                    decrypted[key] = value  # 解密失败则保持原值
            elif key == "cookies" and isinstance(value, str):
                try:
                    decrypted[key] = json.loads(self.crypto.decrypt(value))
                except Exception as e:
                    logger.warning(f"解密 cookies 失败: {e}")
                    decrypted[key] = value
            else:
                decrypted[key] = value
        return decrypted

    def get_masked_credentials(self, credentials: Dict[str, Any]) -> Dict[str, Any]:
        """
        获取脱敏后的凭据

        解密凭据并对敏感字段进行脱敏处理（显示部分字符）。

        Args:
            credentials: 加密的凭据

        Returns:
            脱敏后的凭据
        """
        decrypted = self._decrypt_credentials(credentials)

        for field in self.SENSITIVE_FIELDS:
            if field in decrypted and decrypted[field]:
                value = str(decrypted[field])
                # 显示前4位和后4位，中间固定4个 *（如 sk-x****a12k）
                if len(value) > 12:
                    decrypted[field] = value[:4] + "****" + value[-4:]
                elif len(value) > 8:
                    decrypted[field] = value[:2] + "****" + value[-2:]
                else:
                    decrypted[field] = "*" * len(value)

        return decrypted

    def merge_credentials_with_saved(
        self,
        provider_id: str,
        credentials: Dict[str, Any],
    ) -> Dict[str, Any]:
        """
        合并凭据：如果请求中的敏感字段为空，使用已保存的凭据

        用于验证和保存配置时，当用户未重新输入敏感字段时保留原有值。

        Args:
            provider_id: Provider ID
            credentials: 请求中的凭据

        Returns:
            合并后的凭据
        """
        merged = dict(credentials)
        saved_config = self.get_config(provider_id)

        if saved_config:
            saved_credentials = self._decrypt_credentials(saved_config.connector_credentials)
            sensitive_fields = ["api_key", "password", "session_token", "cookie_string", "cookies"]

            for field in sensitive_fields:
                # 如果请求中该字段为空或只包含星号（脱敏值），使用已保存的值
                req_value = merged.get(field, "")
                if not req_value or (isinstance(req_value, str) and set(req_value) <= {"*"}):
                    if field in saved_credentials:
                        merged[field] = saved_credentials[field]
                        logger.debug(f"合并凭据 - 使用已保存的 {field}")

        return merged

    # ==================== 批量操作 ====================

    async def batch_query_balance(
        self, provider_ids: Optional[List[str]] = None
    ) -> Dict[str, ActionResult]:
        """
        批量查询余额（优先返回缓存，后台异步刷新）

        Args:
            provider_ids: Provider ID 列表，None 表示查询所有已配置的

        Returns:
            {provider_id: result}
        """
        if provider_ids is None:
            # 查询所有已配置的 Provider
            providers = self.db.query(Provider).filter(Provider.is_active.is_(True)).all()
            provider_ids = [
                p.id
                for p in providers
                if p.config and p.config.get("provider_ops")
            ]

        # 并行查询，使用缓存优先策略
        tasks = [
            self.query_balance_with_cache(provider_id, trigger_refresh=True)
            for provider_id in provider_ids
        ]
        results_list = await asyncio.gather(*tasks, return_exceptions=True)

        results = {}
        for provider_id, result in zip(provider_ids, results_list):
            if isinstance(result, Exception):
                logger.warning(f"查询余额失败: provider_id={provider_id}, error={result}")
                results[provider_id] = ActionResult(
                    status=ActionStatus.UNKNOWN_ERROR,
                    action_type=ProviderActionType.QUERY_BALANCE,
                    message=str(result),
                )
            else:
                results[provider_id] = result

        return results

    # ==================== 认证验证 ====================

    async def verify_auth(
        self,
        base_url: str,
        architecture_id: str,
        auth_type: ConnectorAuthType,
        config: Dict[str, Any],
        credentials: Dict[str, Any],
        provider_id: Optional[str] = None,
    ) -> Dict[str, Any]:
        """
        验证认证配置

        在保存前测试认证是否有效。
        认证逻辑委托给对应的 Architecture 实现。

        Args:
            base_url: API 基础地址
            architecture_id: 架构 ID
            auth_type: 认证类型
            config: 连接器配置
            credentials: 凭据
            provider_id: Provider ID（可选，用于缓存余额）

        Returns:
            验证结果
        """
        import httpx

        # 移除 base_url 末尾的斜杠
        base_url = base_url.rstrip("/")

        # 获取架构实例
        registry = get_registry()
        architecture = registry.get_or_default(architecture_id)

        # 使用架构的方法构建请求
        verify_endpoint = f"{base_url}{architecture.get_verify_endpoint()}"
        headers = architecture.build_verify_headers(config, credentials)

        logger.debug(
            f"验证认证: architecture={architecture_id}, "
            f"endpoint={verify_endpoint}, headers={list(headers.keys())}"
        )

        try:
            async with httpx.AsyncClient(timeout=30.0) as client:
                response = await client.get(verify_endpoint, headers=headers)

                # 尝试解析 JSON
                try:
                    data = response.json()
                except Exception:
                    data = {}

                # 使用架构的方法解析响应
                result = architecture.parse_verify_response(response.status_code, data)
                result_dict = result.to_dict()

                # 验证成功且有 provider_id 时，缓存余额
                if result.success and provider_id and result.quota is not None:
                    # 从架构的默认配置获取 quota_divisor
                    balance_config = architecture.default_action_configs.get(
                        ProviderActionType.QUERY_BALANCE, {}
                    )
                    quota_divisor = balance_config.get("quota_divisor", 1)
                    # 转换为美元值后缓存
                    quota_usd = result.quota / quota_divisor
                    await self._cache_balance_from_verify(provider_id, quota_usd)

                return result_dict

        except httpx.TimeoutException:
            return {"success": False, "message": "连接超时"}
        except httpx.ConnectError as e:
            return {"success": False, "message": f"连接失败: {str(e)}"}
        except Exception as e:
            logger.error(f"验证认证失败: {e}")
            return {"success": False, "message": f"验证失败: {str(e)}"}
