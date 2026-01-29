"""
模型可用性查询模块

将所有系统级「可用性」条件集中管理，作为模型查询的单一来源。

职责边界：
- 本模块只负责系统级可用性（对所有请求一致）
- API Key/User 的请求级访问限制由 models_service.AccessRestrictions 处理
"""



from sqlalchemy import or_
from sqlalchemy.orm import Query, Session, contains_eager

from src.core.logger import logger
from src.models.database import (
    GlobalModel,
    Model,
    Provider,
    ProviderAPIKey,
    ProviderEndpoint,
)


class ModelAvailabilityQuery:
    """
    模型可用性查询构建器

    设计原则：
    1. 单一来源：所有可用性条件定义在此类中
    2. 内连接 GlobalModel：未关联的 Model 不参与路由（global_model_id=NULL 不返回）
    3. 完整过滤：包含 is_active 与 is_available（is_available=NULL 视为可用，兼容历史数据）
    """

    @staticmethod
    def base_active_models(db: Session, eager_load: bool = False) -> Query:
        """
        返回基础的可用模型查询（实体为 Model）

        已包含条件：
        - Model.is_active = True
        - Model.is_available = True 或 NULL（NULL 视为可用）
        - Provider.is_active = True
        - GlobalModel.is_active = True
        - Model 必须关联 GlobalModel（内连接，排除 global_model_id=NULL）

        Args:
            db: 数据库会话
            eager_load: 是否预加载 Provider 与 GlobalModel（复用 join，避免重复 JOIN）
        """
        # 使用关系路径 join，与 contains_eager 兼容
        query = (
            db.query(Model)
            .join(Model.provider)
            .join(Model.global_model)
            .filter(
                Model.is_active.is_(True),
                or_(Model.is_available.is_(True), Model.is_available.is_(None)),
                Provider.is_active.is_(True),
                GlobalModel.is_active.is_(True),
            )
        )

        if eager_load:
            query = query.options(
                contains_eager(Model.provider),
                contains_eager(Model.global_model),
            )

        return query

    @staticmethod
    def get_providers_with_active_endpoints(
        db: Session,
        api_formats: list[str],
    ) -> dict[str, set[str]]:
        """
        获取有活跃端点的 Provider 及其支持的格式集合

        条件：
        - Provider.is_active = True（提前过滤，减少无效候选）
        - ProviderEndpoint.is_active = True
        - ProviderEndpoint.api_format 匹配

        Returns:
            {provider_id: {format1, format2, ...}}
        """
        target_formats = {f.upper() for f in api_formats}

        endpoint_rows = (
            db.query(ProviderEndpoint.provider_id, ProviderEndpoint.api_format)
            .join(Provider, ProviderEndpoint.provider_id == Provider.id)
            .filter(
                Provider.is_active.is_(True),
                ProviderEndpoint.api_format.in_(list(target_formats)),
                ProviderEndpoint.is_active.is_(True),
            )
            .all()
        )

        provider_to_formats: dict[str, set[str]] = {}
        for provider_id, fmt in endpoint_rows:
            if provider_id and fmt:
                provider_to_formats.setdefault(provider_id, set()).add(str(fmt).upper())

        return provider_to_formats

    @staticmethod
    def get_providers_with_active_keys(
        db: Session,
        provider_ids: set[str],
        api_formats: list[str],
        provider_to_endpoint_formats: dict[str, set[str]],
    ) -> set[str]:
        """
        过滤出有活跃 Key 支持指定格式的 Provider

        条件：
        - ProviderAPIKey.is_active = True
        - Key.api_formats 与 Endpoint 格式与请求格式有交集
        """
        if not provider_ids:
            return set()

        target_formats = {f.upper() for f in api_formats}

        key_rows = (
            db.query(ProviderAPIKey.provider_id, ProviderAPIKey.api_formats)
            .filter(
                ProviderAPIKey.provider_id.in_(provider_ids),
                ProviderAPIKey.is_active.is_(True),
            )
            .all()
        )

        available_provider_ids: set[str] = set()
        for provider_id, key_formats in key_rows:
            if not provider_id:
                continue

            endpoint_formats = provider_to_endpoint_formats.get(provider_id)
            if not endpoint_formats:
                continue

            # 类型兜底：key_formats 是 JSON 字段
            if not isinstance(key_formats, list):
                if key_formats is not None:
                    logger.warning(
                        f"[ModelAvailability] Key api_formats 类型异常, "
                        f"provider_id={provider_id}, type={type(key_formats).__name__}"
                    )
                continue

            key_formats_upper: set[str] = {str(f).upper() for f in key_formats if isinstance(f, str)}

            if key_formats_upper & endpoint_formats & target_formats:
                available_provider_ids.add(provider_id)

        return available_provider_ids

    @staticmethod
    def get_provider_key_rules(
        db: Session,
        provider_ids: set[str],
        api_formats: list[str],
        provider_to_endpoint_formats: dict[str, set[str]],
    ) -> dict[str, list[tuple[list[str] | None, set[str]]]]:
        """
        获取每个 Provider 的 Key 权限规则

        Returns:
            {provider_id: [(allowed_models, usable_formats), ...]}

        注意：
        - allowed_models 是 JSON 字段，此方法会进行类型兜底处理
        - 非预期类型会跳过该 Key 并打日志（安全优先，不放大权限）
        """
        if not provider_ids:
            return {}

        target_formats = {f.upper() for f in api_formats}

        key_rows = (
            db.query(
                ProviderAPIKey.id,
                ProviderAPIKey.provider_id,
                ProviderAPIKey.allowed_models,
                ProviderAPIKey.api_formats,
            )
            .filter(
                ProviderAPIKey.provider_id.in_(provider_ids),
                ProviderAPIKey.is_active.is_(True),
            )
            .all()
        )

        provider_key_rules: dict[str, list[tuple[list[str] | None, set[str]]]] = {}
        for key_id, provider_id, allowed_models_raw, key_formats in key_rows:
            if not provider_id:
                continue

            endpoint_formats = provider_to_endpoint_formats.get(provider_id)
            if not endpoint_formats:
                continue

            # 类型兜底：key_formats
            if not isinstance(key_formats, list):
                if key_formats is not None:
                    logger.warning(
                        f"[ModelAvailability] Key api_formats 类型异常, "
                        f"key_id={key_id}, type={type(key_formats).__name__}"
                    )
                continue

            key_formats_upper: set[str] = {str(f).upper() for f in key_formats if isinstance(f, str)}
            usable_formats = key_formats_upper & endpoint_formats & target_formats
            if not usable_formats:
                continue

            # 类型兜底：allowed_models（安全优先）
            allowed_models: list[str] | None
            if allowed_models_raw is None:
                # None = 不限制
                allowed_models = None
            elif isinstance(allowed_models_raw, list):
                allowed_models = [m for m in allowed_models_raw if isinstance(m, str)]
            else:
                logger.warning(
                    f"[ModelAvailability] Key allowed_models 类型异常, "
                    f"key_id={key_id}, type={type(allowed_models_raw).__name__}, 跳过该 Key"
                )
                continue

            provider_key_rules.setdefault(provider_id, []).append((allowed_models, usable_formats))

        return provider_key_rules

    @staticmethod
    def find_by_global_model_name(
        db: Session,
        model_name: str,
        provider_ids: set[str] | None = None,
        eager_load: bool = False,
    ) -> Query:
        """
        按 GlobalModel.name 查找模型

        条件：
        - 基础可用性条件（base_active_models）
        - GlobalModel.name = model_name
        - 可选：限制到指定 Provider
        """
        query = ModelAvailabilityQuery.base_active_models(db, eager_load=eager_load).filter(
            GlobalModel.name == model_name
        )

        if provider_ids is not None:
            query = query.filter(Model.provider_id.in_(provider_ids))

        return query

