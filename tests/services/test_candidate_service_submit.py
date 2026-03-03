from types import SimpleNamespace
from typing import Any
from unittest.mock import AsyncMock, MagicMock

import httpx
import pytest

from src.config.settings import config
from src.services.candidate.submit import AllCandidatesFailedError, UpstreamClientRequestError
from src.services.task.service import TaskService


def _make_candidate(
    *,
    provider_id: str = "p1",
    provider_name: str = "prov",
    provider_config: dict[str, Any] | None = None,
    endpoint_id: str = "e1",
    key_id: str = "k1",
    key_name: str = "key",
    auth_type: str = "api_key",
    priority: int = 0,
    is_cached: bool = False,
    is_skipped: bool = False,
    skip_reason: str | None = None,
    needs_conversion: bool = False,
) -> SimpleNamespace:
    provider = SimpleNamespace(id=provider_id, name=provider_name, config=provider_config or {})
    endpoint = SimpleNamespace(id=endpoint_id)
    key = SimpleNamespace(id=key_id, name=key_name, auth_type=auth_type, priority=priority)
    return SimpleNamespace(
        provider=provider,
        endpoint=endpoint,
        key=key,
        is_cached=is_cached,
        is_skipped=is_skipped,
        skip_reason=skip_reason,
        needs_conversion=needs_conversion,
    )


@pytest.mark.asyncio
async def test_submit_with_failover_continues_on_http_400_without_stop_rule_then_succeeds(
    monkeypatch: pytest.MonkeyPatch,
) -> None:
    db = MagicMock()
    svc = TaskService(db)

    monkeypatch.setattr(
        "src.services.system.config.SystemConfigService.get_config",
        lambda *_args, **_kwargs: "provider",
    )
    monkeypatch.setattr(
        "src.services.scheduling.aware_scheduler.get_cache_aware_scheduler",
        AsyncMock(return_value=None),
    )
    monkeypatch.setattr(
        "src.services.candidate.resolver.CandidateResolver.fetch_candidates",
        AsyncMock(
            return_value=(
                [
                    _make_candidate(provider_id="p1", endpoint_id="e1", key_id="k1"),
                    _make_candidate(provider_id="p2", endpoint_id="e2", key_id="k2"),
                ],
                "gm1",
            )
        ),
    )
    responses = [
        httpx.Response(400, json={"error": {"type": "invalid_request_error", "message": "bad"}}),
        httpx.Response(200, json={"id": "task-123"}),
    ]
    submit = AsyncMock(side_effect=responses)

    outcome = await svc.submit_with_failover(
        api_format="openai:video",
        model_name="sora",
        affinity_key="a1",
        user_api_key=MagicMock(),
        request_id=None,
        task_type="video",
        submit_func=submit,
        extract_external_task_id=lambda payload: payload.get("id"),
        supported_auth_types={"api_key"},
        allow_format_conversion=False,
        max_candidates=10,
    )

    assert outcome.external_task_id == "task-123"
    assert outcome.candidate.provider.id == "p2"
    assert outcome.candidate_keys[1]["selected"] is True
    assert submit.await_count == 2


@pytest.mark.asyncio
async def test_submit_with_failover_stops_on_provider_error_stop_rule(
    monkeypatch: pytest.MonkeyPatch,
) -> None:
    db = MagicMock()
    svc = TaskService(db)

    monkeypatch.setattr(
        "src.services.system.config.SystemConfigService.get_config",
        lambda *_args, **_kwargs: "provider",
    )
    monkeypatch.setattr(
        "src.services.scheduling.aware_scheduler.get_cache_aware_scheduler",
        AsyncMock(return_value=None),
    )
    monkeypatch.setattr(
        "src.services.candidate.resolver.CandidateResolver.fetch_candidates",
        AsyncMock(
            return_value=(
                [
                    _make_candidate(
                        provider_config={
                            "failover_rules": {
                                "error_stop_patterns": [
                                    {"pattern": "invalid_request_error", "status_codes": [400]}
                                ]
                            }
                        }
                    )
                ],
                "gm1",
            )
        ),
    )

    response = httpx.Response(
        400,
        json={"error": {"type": "invalid_request_error", "message": "bad request"}},
    )
    submit = AsyncMock(return_value=response)

    with pytest.raises(UpstreamClientRequestError):
        await svc.submit_with_failover(
            api_format="openai:video",
            model_name="sora",
            affinity_key="a1",
            user_api_key=MagicMock(),
            request_id=None,
            task_type="video",
            submit_func=submit,
            extract_external_task_id=lambda payload: payload.get("id"),
            supported_auth_types={"api_key"},
            allow_format_conversion=False,
            max_candidates=10,
        )


@pytest.mark.asyncio
async def test_submit_with_failover_continues_on_provider_success_failover_rule(
    monkeypatch: pytest.MonkeyPatch,
) -> None:
    db = MagicMock()
    svc = TaskService(db)

    monkeypatch.setattr(
        "src.services.system.config.SystemConfigService.get_config",
        lambda *_args, **_kwargs: "provider",
    )
    monkeypatch.setattr(
        "src.services.scheduling.aware_scheduler.get_cache_aware_scheduler",
        AsyncMock(return_value=None),
    )
    monkeypatch.setattr(
        "src.services.candidate.resolver.CandidateResolver.fetch_candidates",
        AsyncMock(
            return_value=(
                [
                    _make_candidate(
                        provider_id="p1",
                        endpoint_id="e1",
                        key_id="k1",
                        provider_config={
                            "failover_rules": {
                                "success_failover_patterns": [{"pattern": "fallback_me"}]
                            }
                        },
                    ),
                    _make_candidate(provider_id="p2", endpoint_id="e2", key_id="k2"),
                ],
                "gm1",
            )
        ),
    )

    responses = [
        httpx.Response(200, json={"id": "task-should-not-be-used", "message": "fallback_me"}),
        httpx.Response(200, json={"id": "task-123"}),
    ]
    submit = AsyncMock(side_effect=responses)

    outcome = await svc.submit_with_failover(
        api_format="openai:video",
        model_name="sora",
        affinity_key="a1",
        user_api_key=MagicMock(),
        request_id=None,
        task_type="video",
        submit_func=submit,
        extract_external_task_id=lambda payload: payload.get("id"),
        supported_auth_types={"api_key"},
        allow_format_conversion=False,
        max_candidates=10,
    )

    assert outcome.external_task_id == "task-123"
    assert outcome.candidate.provider.id == "p2"
    assert submit.await_count == 2


@pytest.mark.asyncio
async def test_submit_with_failover_no_eligible_candidates_due_to_auth_type(
    monkeypatch: pytest.MonkeyPatch,
) -> None:
    db = MagicMock()
    svc = TaskService(db)

    monkeypatch.setattr(
        "src.services.system.config.SystemConfigService.get_config",
        lambda *_args, **_kwargs: "provider",
    )
    monkeypatch.setattr(
        "src.services.scheduling.aware_scheduler.get_cache_aware_scheduler",
        AsyncMock(return_value=None),
    )
    monkeypatch.setattr(
        "src.services.candidate.resolver.CandidateResolver.fetch_candidates",
        AsyncMock(return_value=([_make_candidate(auth_type="vertex_ai")], "gm1")),
    )

    with pytest.raises(AllCandidatesFailedError) as excinfo:
        await svc.submit_with_failover(
            api_format="openai:video",
            model_name="sora",
            affinity_key="a1",
            user_api_key=MagicMock(),
            request_id=None,
            task_type="video",
            submit_func=AsyncMock(),
            extract_external_task_id=lambda payload: payload.get("id"),
            supported_auth_types={"api_key"},
            allow_format_conversion=False,
            max_candidates=10,
        )

    assert excinfo.value.reason == "no_eligible_candidates"


@pytest.mark.asyncio
async def test_submit_with_failover_filters_missing_billing_rule(
    monkeypatch: pytest.MonkeyPatch,
) -> None:
    db = MagicMock()
    svc = TaskService(db)

    monkeypatch.setattr(
        "src.services.system.config.SystemConfigService.get_config",
        lambda *_args, **_kwargs: "provider",
    )
    monkeypatch.setattr(
        "src.services.scheduling.aware_scheduler.get_cache_aware_scheduler",
        AsyncMock(return_value=None),
    )
    monkeypatch.setattr(
        "src.services.candidate.resolver.CandidateResolver.fetch_candidates",
        AsyncMock(
            return_value=(
                [
                    _make_candidate(provider_id="p1", endpoint_id="e1", key_id="k1"),
                    _make_candidate(provider_id="p2", endpoint_id="e2", key_id="k2"),
                ],
                "gm1",
            )
        ),
    )
    # enable require_rule
    old = config.billing_require_rule
    try:
        config.billing_require_rule = True

        def _find_rule(
            _db: Any, *, provider_id: str, model_name: str, task_type: str
        ) -> object | None:
            return None if provider_id == "p1" else object()

        monkeypatch.setattr(
            "src.services.billing.rule_service.BillingRuleService.find_rule", _find_rule
        )

        submit = AsyncMock(return_value=httpx.Response(200, json={"id": "task-999"}))
        outcome = await svc.submit_with_failover(
            api_format="openai:video",
            model_name="sora",
            affinity_key="a1",
            user_api_key=MagicMock(),
            request_id=None,
            task_type="video",
            submit_func=submit,
            extract_external_task_id=lambda payload: payload.get("id"),
            supported_auth_types={"api_key"},
            allow_format_conversion=False,
            max_candidates=10,
        )
        assert outcome.candidate.provider.id == "p2"
        assert outcome.external_task_id == "task-999"
        # only called once because p1 skipped
        assert submit.await_count == 1
    finally:
        config.billing_require_rule = old


@pytest.mark.asyncio
async def test_submit_with_failover_applies_pool_reorder_before_submit(
    monkeypatch: pytest.MonkeyPatch,
) -> None:
    db = MagicMock()
    svc = TaskService(db)

    monkeypatch.setattr(
        "src.services.system.config.SystemConfigService.get_config",
        lambda *_args, **_kwargs: "provider",
    )
    monkeypatch.setattr(
        "src.services.scheduling.aware_scheduler.get_cache_aware_scheduler",
        AsyncMock(return_value=None),
    )

    pool_candidate_a = _make_candidate(
        provider_id="pool-1",
        provider_name="pool-provider",
        endpoint_id="ep-1",
        key_id="k-a",
        key_name="key-a",
    )
    pool_candidate_b = _make_candidate(
        provider_id="pool-1",
        provider_name="pool-provider",
        endpoint_id="ep-1",
        key_id="k-b",
        key_name="key-b",
    )

    fetch_candidates = AsyncMock(
        return_value=(
            [pool_candidate_a, pool_candidate_b],
            "gm1",
        )
    )
    monkeypatch.setattr(
        "src.services.candidate.resolver.CandidateResolver.fetch_candidates",
        fetch_candidates,
    )

    reordered = [pool_candidate_b, pool_candidate_a]
    apply_pool_reorder = AsyncMock(return_value=(reordered, []))
    monkeypatch.setattr(svc, "_apply_pool_reorder", apply_pool_reorder)

    submit = AsyncMock(return_value=httpx.Response(200, json={"id": "task-pooled"}))
    body = {"session_id": "sid-123"}
    outcome = await svc.submit_with_failover(
        api_format="openai:video",
        model_name="sora",
        affinity_key="a1",
        user_api_key=MagicMock(),
        request_id=None,
        task_type="video",
        submit_func=submit,
        extract_external_task_id=lambda payload: payload.get("id"),
        supported_auth_types={"api_key"},
        allow_format_conversion=False,
        max_candidates=10,
        request_body=body,
    )

    assert outcome.external_task_id == "task-pooled"
    assert outcome.candidate.key.id == "k-b"
    assert submit.await_count == 1
    fetch_candidates.assert_awaited_once()
    assert fetch_candidates.await_args.kwargs.get("request_body") == body
    apply_pool_reorder.assert_awaited_once_with(
        [pool_candidate_a, pool_candidate_b],
        request_body=body,
    )
