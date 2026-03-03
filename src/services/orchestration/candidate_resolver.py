"""
候选解析器

负责获取和排序可用的 Provider/Endpoint/Key 组合
"""

import uuid
from datetime import datetime, timezone
from typing import Any

from sqlalchemy.orm import Session

from src.core.exceptions import ProviderNotAvailableException
from src.core.logger import logger
from src.models.database import ApiKey
from src.services.provider.format import normalize_endpoint_signature
from src.services.scheduling.aware_scheduler import CacheAwareScheduler, ProviderCandidate


class CandidateResolver:
    """
    候选解析器 - 负责获取和排序可用的 Provider 组合

    职责：
    1. 从 CacheAwareScheduler 获取所有可用候选
    2. 创建候选记录（用于追踪）
    3. 提供候选的迭代和过滤功能
    """

    def __init__(
        self,
        db: Session,
        cache_scheduler: CacheAwareScheduler,
    ) -> None:
        """
        初始化候选解析器

        Args:
            db: 数据库会话
            cache_scheduler: 缓存感知调度器
        """
        self.db = db
        self.cache_scheduler = cache_scheduler

    async def fetch_candidates(
        self,
        api_format: str,
        model_name: str,
        affinity_key: str,
        user_api_key: ApiKey | None = None,
        request_id: str | None = None,
        is_stream: bool = False,
        capability_requirements: dict[str, bool] | None = None,
        preferred_key_ids: list[str] | None = None,
        request_body: dict | None = None,
    ) -> tuple[list[ProviderCandidate], str]:
        """
        获取所有可用候选

        Args:
            api_format: API 格式
            model_name: 模型名称
            affinity_key: 亲和性标识符（通常为API Key ID，用于缓存亲和性）
            user_api_key: 用户 API Key（用于 allowed_providers/allowed_api_formats 过滤）
            request_id: 请求 ID（用于日志）
            is_stream: 是否是流式请求，如果为 True 则过滤不支持流式的 Provider
            capability_requirements: 能力需求（用于过滤不满足能力要求的 Key）
            preferred_key_ids: 优先使用的 Provider Key ID 列表（匹配则置顶）

        Returns:
            (所有候选组合的列表, global_model_id)

        Raises:
            ProviderNotAvailableException: 没有找到任何可用候选时
        """
        all_candidates: list[ProviderCandidate] = []
        provider_offset = 0
        provider_batch_size = 20
        global_model_id: str | None = None
        api_format_norm = normalize_endpoint_signature(api_format)

        logger.debug(
            "[CandidateResolver] fetch_candidates starting: model={}, api_format={}",
            model_name,
            api_format_norm,
        )

        while True:
            candidates, resolved_global_model_id, provider_batch_count = (
                await self.cache_scheduler.list_all_candidates(
                    db=self.db,
                    api_format=api_format_norm,
                    model_name=model_name,
                    affinity_key=affinity_key,
                    user_api_key=user_api_key,
                    provider_offset=provider_offset,
                    provider_limit=provider_batch_size,
                    is_stream=is_stream,
                    capability_requirements=capability_requirements,
                    request_body=request_body,
                )
            )

            logger.debug(
                "[CandidateResolver] list_all_candidates batch: offset={}, providers={}, returned={} candidates",
                provider_offset,
                provider_batch_count,
                len(candidates),
            )

            if resolved_global_model_id and global_model_id is None:
                global_model_id = resolved_global_model_id

            if provider_batch_count == 0:
                break

            all_candidates.extend(candidates)
            provider_offset += provider_batch_size

            if provider_batch_count < provider_batch_size:
                break

        logger.debug(
            "[CandidateResolver] fetch_candidates completed: total={} candidates",
            len(all_candidates),
        )

        if not all_candidates:
            logger.error(f"  [{request_id}] 没有找到任何可用的 Provider/Endpoint/Key 组合")
            request_type = "流式" if is_stream else "非流式"
            raise ProviderNotAvailableException(
                f"没有可用的 Provider 支持模型 {model_name} 的{request_type}请求"
            )

        logger.debug(f"  [{request_id}] 获取到 {len(all_candidates)} 个候选组合")

        # Provider 分页会导致候选在全局维度上排序失真（尤其是 global_key / 降级分组 / cache_affinity）。
        # 这里在汇总后再次应用全局排序规则，保证遍历顺序符合当前调度配置。
        try:
            all_candidates = await self.cache_scheduler.reorder_candidates(
                candidates=all_candidates,
                db=self.db,
                affinity_key=affinity_key,
                api_format=api_format_norm,
                global_model_id=global_model_id,
            )
        except Exception as exc:
            logger.warning(
                "[CandidateResolver] global reorder failed, keep paged order: {}",
                exc,
            )

        if preferred_key_ids:
            preferred_set = {str(kid) for kid in preferred_key_ids if kid}
            if preferred_set:
                preferred_candidates = [
                    c for c in all_candidates if c.key and str(c.key.id) in preferred_set
                ]
                other_candidates = [
                    c for c in all_candidates if not (c.key and str(c.key.id) in preferred_set)
                ]
                if preferred_candidates:
                    matched_key_ids = [str(c.key.id) for c in preferred_candidates if c.key]
                    logger.debug(
                        f"  [{request_id}] 优先候选命中: {len(preferred_candidates)} 个 "
                        f"(key_ids={matched_key_ids[:3]}{'...' if len(matched_key_ids) > 3 else ''})"
                    )
                else:
                    logger.debug(
                        f"  [{request_id}] 优先候选未命中: 请求的 key_ids={list(preferred_set)[:3]} "
                        "不在可用候选中，将使用普通优先级"
                    )
                all_candidates = preferred_candidates + other_candidates

        # 如果没有解析到 global_model_id，使用原始 model_name 作为后备
        return all_candidates, global_model_id or model_name

    def create_candidate_records(
        self,
        all_candidates: list[ProviderCandidate],
        request_id: str | None,
        user_id: str,
        user_api_key: ApiKey,
        required_capabilities: dict[str, bool] | None = None,
        *,
        expand_retries: bool = True,
    ) -> dict[tuple[int, int], str]:
        """
        为所有候选预先创建 available 状态记录（批量插入优化）

        Args:
            all_candidates: 所有候选组合
            request_id: 请求 ID
            user_id: 用户 ID
            user_api_key: 用户 API Key 对象
            required_capabilities: 请求需要的能力标签

        Returns:
            candidate_record_map: {(candidate_index, retry_index): candidate_record_id}
        """
        from src.models.database import RequestCandidate

        candidate_records_to_insert: list[dict[str, Any]] = []
        candidate_record_map: dict[tuple[int, int], str] = {}

        # 只保存启用的能力（值为 True 的）
        active_capabilities = None
        if required_capabilities:
            active_capabilities = {k: v for k, v in required_capabilities.items() if v}
            if not active_capabilities:
                active_capabilities = None

        for candidate_index, candidate in enumerate(all_candidates):
            provider = candidate.provider
            endpoint = candidate.endpoint
            key = candidate.key
            pool_extra = (
                getattr(candidate, "_pool_extra_data", None)
                if isinstance(getattr(candidate, "_pool_extra_data", None), dict)
                else {}
            )

            if candidate.is_skipped:
                record_id = str(uuid.uuid4())
                candidate_records_to_insert.append(
                    {
                        "id": record_id,
                        "request_id": request_id,
                        "candidate_index": candidate_index,
                        "retry_index": 0,
                        "user_id": user_id,
                        "api_key_id": user_api_key.id if user_api_key else None,
                        "provider_id": provider.id,
                        "endpoint_id": endpoint.id,
                        "key_id": key.id,
                        "status": "skipped",
                        "skip_reason": candidate.skip_reason,
                        "is_cached": candidate.is_cached,
                        "extra_data": {
                            "needs_conversion": candidate.needs_conversion,
                            "provider_api_format": candidate.provider_api_format or None,
                            "mapping_matched_model": candidate.mapping_matched_model or None,
                            **pool_extra,
                        },
                        "required_capabilities": active_capabilities,
                        "created_at": datetime.now(timezone.utc),
                    }
                )
                candidate_record_map[(candidate_index, 0)] = record_id
            else:
                # max_retries 已从 Endpoint 迁移到 Provider（Endpoint 仍可能保留旧字段用于兼容）
                if not expand_retries:
                    max_retries_for_candidate = 1
                else:
                    max_retries_for_candidate = (
                        int(provider.max_retries or 2) if candidate.is_cached else 1
                    )

                for retry_index in range(max_retries_for_candidate):
                    record_id = str(uuid.uuid4())
                    candidate_records_to_insert.append(
                        {
                            "id": record_id,
                            "request_id": request_id,
                            "candidate_index": candidate_index,
                            "retry_index": retry_index,
                            "user_id": user_id,
                            "api_key_id": user_api_key.id if user_api_key else None,
                            "provider_id": provider.id,
                            "endpoint_id": endpoint.id,
                            "key_id": key.id,
                            "status": "available",
                            "is_cached": candidate.is_cached,
                            "extra_data": {
                                "needs_conversion": candidate.needs_conversion,
                                "provider_api_format": candidate.provider_api_format or None,
                                "mapping_matched_model": candidate.mapping_matched_model or None,
                                **pool_extra,
                            },
                            "required_capabilities": active_capabilities,
                            "created_at": datetime.now(timezone.utc),
                        }
                    )
                    candidate_record_map[(candidate_index, retry_index)] = record_id

        if candidate_records_to_insert:
            self.db.bulk_insert_mappings(
                RequestCandidate, candidate_records_to_insert  # type: ignore
            )
            self.db.flush()

            logger.debug(
                f"  [{request_id}] 批量插入完成: {len(candidate_records_to_insert)} 条记录"
            )

        return candidate_record_map

    def get_active_candidates(
        self,
        all_candidates: list[ProviderCandidate],
    ) -> list[tuple[int, ProviderCandidate]]:
        """
        获取所有非跳过的候选（带索引）

        Args:
            all_candidates: 所有候选组合

        Returns:
            List of (index, candidate) for non-skipped candidates
        """
        return [(i, c) for i, c in enumerate(all_candidates) if not c.is_skipped]

    def count_total_attempts(
        self,
        all_candidates: list[ProviderCandidate],
    ) -> int:
        """
        计算总尝试次数

        Args:
            all_candidates: 所有候选组合

        Returns:
            总尝试次数
        """
        total = 0
        for candidate in all_candidates:
            if not candidate.is_skipped:
                provider = candidate.provider
                max_retries = int(provider.max_retries or 2) if candidate.is_cached else 1
                total += max_retries
        return total
