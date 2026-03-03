from unittest.mock import AsyncMock, MagicMock

import pytest

from src.services.scheduling.aware_scheduler import CacheAwareScheduler


def _mock_key(key_id: str, api_formats: list[str]) -> MagicMock:
    key = MagicMock()
    key.id = key_id
    key.is_active = True
    key.api_formats = api_formats
    key.cache_ttl_minutes = 1
    key.internal_priority = 1
    return key


def _mock_endpoint(api_format: str) -> MagicMock:
    endpoint = MagicMock()
    endpoint.id = f"ep_{api_format.lower().replace(':', '_')}"
    endpoint.is_active = True
    endpoint.api_format = api_format
    endpoint.api_family = api_format.split(":", 1)[0]
    endpoint.endpoint_kind = api_format.split(":", 1)[1]
    endpoint.format_acceptance_config = None
    return endpoint


@pytest.mark.asyncio
async def test_pool_provider_enumerates_all_key_candidates() -> None:
    scheduler = CacheAwareScheduler()
    builder = scheduler._candidate_builder
    builder._check_model_support = AsyncMock(return_value=(True, None, None, {"m"}))  # type: ignore[method-assign]
    builder._check_key_availability = MagicMock(return_value=(True, None, None))  # type: ignore[method-assign]

    provider = MagicMock()
    provider.id = "p_pool"
    provider.name = "pool_provider"
    provider.enable_format_conversion = False
    provider.config = {"pool_advanced": {}}
    provider.endpoints = [_mock_endpoint("openai:chat")]
    provider.api_keys = [
        _mock_key("k1", ["openai:chat"]),
        _mock_key("k2", ["openai:chat"]),
    ]

    candidates = await builder._build_candidates(
        db=MagicMock(),
        providers=[provider],
        client_format="openai:chat",
        model_name="dummy-model",
        affinity_key="aff-1",
        global_conversion_enabled=True,
    )

    assert len(candidates) == 2
    assert {str(c.key.id) for c in candidates} == {"k1", "k2"}
