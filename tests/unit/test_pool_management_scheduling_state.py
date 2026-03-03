"""Tests for pool management scheduling state assembly."""

from __future__ import annotations

from types import SimpleNamespace

from src.api.admin.pool.routes import (
    _build_pool_scheduling_state,
    _is_known_banned_key,
    _is_known_banned_reason,
)


def test_pool_scheduling_state_manual_disabled_is_blocked() -> None:
    (
        status,
        reason,
        _label,
        reasons,
        score,
        candidate_eligible,
        blocked_count,
        _degraded_count,
        dimensions,
    ) = _build_pool_scheduling_state(
        is_active=False,
        cooldown_reason=None,
        cooldown_ttl_seconds=None,
        circuit_breaker_open=False,
        cost_window_usage=0,
        cost_limit=None,
        cost_soft_threshold_percent=80,
        health_score=1.0,
    )

    assert status == "blocked"
    assert reason == "manual_disabled"
    assert candidate_eligible is False
    assert blocked_count >= 1
    assert score < 100
    assert any(item.code == "manual_disabled" for item in reasons)
    assert any(item.code == "manual_disabled" for item in dimensions)


def test_pool_scheduling_state_cooldown_detail_is_mapped() -> None:
    (
        _status,
        reason,
        _label,
        reasons,
        _score,
        _candidate_eligible,
        _blocked_count,
        _degraded_count,
        dimensions,
    ) = _build_pool_scheduling_state(
        is_active=True,
        cooldown_reason="rate_limited_429",
        cooldown_ttl_seconds=180,
        circuit_breaker_open=False,
        cost_window_usage=0,
        cost_limit=None,
        cost_soft_threshold_percent=80,
        health_score=1.0,
    )

    assert reason == "cooldown"
    cooldown_reason = next(item for item in reasons if item.code == "cooldown")
    cooldown_dimension = next(item for item in dimensions if item.code == "cooldown")
    assert cooldown_reason.detail == "429 限流"
    assert cooldown_dimension.detail == "429 限流"


def test_pool_scheduling_state_cost_soft_is_degraded() -> None:
    (
        status,
        reason,
        _label,
        _reasons,
        _score,
        candidate_eligible,
        blocked_count,
        degraded_count,
        _dimensions,
    ) = _build_pool_scheduling_state(
        is_active=True,
        cooldown_reason=None,
        cooldown_ttl_seconds=None,
        circuit_breaker_open=False,
        cost_window_usage=85,
        cost_limit=100,
        cost_soft_threshold_percent=80,
        health_score=1.0,
    )

    assert status == "degraded"
    assert reason == "cost_soft"
    assert candidate_eligible is True
    assert blocked_count == 0
    assert degraded_count >= 1


def test_known_banned_reason_account_block_prefix() -> None:
    assert _is_known_banned_reason("[ACCOUNT_BLOCK] Google 要求验证账号") is True


def test_known_banned_key_detects_kiro_banned_metadata() -> None:
    key = SimpleNamespace(
        upstream_metadata={"kiro": {"is_banned": True}},
        oauth_invalid_reason=None,
    )
    assert _is_known_banned_key(key, "kiro") is True


def test_known_banned_key_detects_reason_keywords() -> None:
    key = SimpleNamespace(
        upstream_metadata={},
        oauth_invalid_reason="AWS account temporarily suspended",
    )
    assert _is_known_banned_key(key, "antigravity") is True


def test_known_banned_key_does_not_treat_token_expired_as_banned() -> None:
    key = SimpleNamespace(
        upstream_metadata={"kiro": {"is_banned": False}},
        oauth_invalid_reason="access token expired",
    )
    assert _is_known_banned_key(key, "kiro") is False
