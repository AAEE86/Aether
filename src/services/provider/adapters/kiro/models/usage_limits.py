"""Kiro getUsageLimits response models (best-effort).

The AWS API uses camelCase fields; we parse defensively.
"""

from __future__ import annotations

from dataclasses import dataclass, field
from typing import Any


@dataclass(slots=True)
class SubscriptionInfo:
    subscription_title: str | None = None

    @classmethod
    def from_dict(cls, raw: Any) -> 'SubscriptionInfo | None':
        if not isinstance(raw, dict):
            return None
        title = raw.get('subscriptionTitle')
        if isinstance(title, str) and title.strip():
            return cls(subscription_title=title.strip())
        return cls(subscription_title=None)


@dataclass(slots=True)
class Bonus:
    current_usage: float = 0.0
    usage_limit: float = 0.0
    status: str | None = None

    @classmethod
    def from_dict(cls, raw: Any) -> 'Bonus | None':
        if not isinstance(raw, dict):
            return None
        status = raw.get('status')
        status_val = status.strip() if isinstance(status, str) and status.strip() else None
        cu = raw.get('currentUsage')
        ul = raw.get('usageLimit')
        try:
            cu_f = float(cu) if cu is not None else 0.0
        except Exception:
            cu_f = 0.0
        try:
            ul_f = float(ul) if ul is not None else 0.0
        except Exception:
            ul_f = 0.0
        return cls(current_usage=cu_f, usage_limit=ul_f, status=status_val)


@dataclass(slots=True)
class FreeTrialInfo:
    current_usage: int = 0
    current_usage_with_precision: float = 0.0
    usage_limit: int = 0
    usage_limit_with_precision: float = 0.0
    free_trial_expiry: float | None = None
    free_trial_status: str | None = None

    @classmethod
    def from_dict(cls, raw: Any) -> 'FreeTrialInfo | None':
        if not isinstance(raw, dict):
            return None

        def _int(v: Any) -> int:
            try:
                return int(v)
            except Exception:
                return 0

        def _float(v: Any) -> float:
            try:
                return float(v)
            except Exception:
                return 0.0

        expiry = raw.get('freeTrialExpiry')
        try:
            expiry_f = float(expiry) if expiry is not None else None
        except Exception:
            expiry_f = None

        status = raw.get('freeTrialStatus')
        status_val = status.strip() if isinstance(status, str) and status.strip() else None

        return cls(
            current_usage=_int(raw.get('currentUsage')),
            current_usage_with_precision=_float(raw.get('currentUsageWithPrecision')),
            usage_limit=_int(raw.get('usageLimit')),
            usage_limit_with_precision=_float(raw.get('usageLimitWithPrecision')),
            free_trial_expiry=expiry_f,
            free_trial_status=status_val,
        )


@dataclass(slots=True)
class UsageBreakdown:
    current_usage: int = 0
    current_usage_with_precision: float = 0.0
    usage_limit: int = 0
    usage_limit_with_precision: float = 0.0
    next_date_reset: float | None = None
    bonuses: list[Bonus] = field(default_factory=list)
    free_trial_info: FreeTrialInfo | None = None

    @classmethod
    def from_dict(cls, raw: Any) -> 'UsageBreakdown | None':
        if not isinstance(raw, dict):
            return None

        def _int(v: Any) -> int:
            try:
                return int(v)
            except Exception:
                return 0

        def _float(v: Any) -> float:
            try:
                return float(v)
            except Exception:
                return 0.0

        next_reset = raw.get('nextDateReset')
        try:
            next_reset_f = float(next_reset) if next_reset is not None else None
        except Exception:
            next_reset_f = None

        bonuses_raw = raw.get('bonuses')
        bonuses: list[Bonus] = []
        if isinstance(bonuses_raw, list):
            for b in bonuses_raw:
                parsed = Bonus.from_dict(b)
                if parsed is not None:
                    bonuses.append(parsed)

        return cls(
            current_usage=_int(raw.get('currentUsage')),
            current_usage_with_precision=_float(raw.get('currentUsageWithPrecision')),
            usage_limit=_int(raw.get('usageLimit')),
            usage_limit_with_precision=_float(raw.get('usageLimitWithPrecision')),
            next_date_reset=next_reset_f,
            bonuses=bonuses,
            free_trial_info=FreeTrialInfo.from_dict(raw.get('freeTrialInfo')),
        )


@dataclass(slots=True)
class UsageLimitsResponse:
    next_date_reset: float | None = None
    subscription_info: SubscriptionInfo | None = None
    usage_breakdown_list: list[UsageBreakdown] = field(default_factory=list)

    @classmethod
    def from_dict(cls, raw: Any) -> 'UsageLimitsResponse':
        if not isinstance(raw, dict):
            raw = {}

        next_reset = raw.get('nextDateReset')
        try:
            next_reset_f = float(next_reset) if next_reset is not None else None
        except Exception:
            next_reset_f = None

        breakdown_raw = raw.get('usageBreakdownList')
        breakdowns: list[UsageBreakdown] = []
        if isinstance(breakdown_raw, list):
            for b in breakdown_raw:
                parsed = UsageBreakdown.from_dict(b)
                if parsed is not None:
                    breakdowns.append(parsed)

        return cls(
            next_date_reset=next_reset_f,
            subscription_info=SubscriptionInfo.from_dict(raw.get('subscriptionInfo')),
            usage_breakdown_list=breakdowns,
        )


def calculate_total_usage_limit(response: UsageLimitsResponse) -> float:
    if not response.usage_breakdown_list:
        return 0.0

    breakdown = response.usage_breakdown_list[0]
    total = breakdown.usage_limit_with_precision

    if breakdown.free_trial_info and breakdown.free_trial_info.free_trial_status == 'ACTIVE':
        total += breakdown.free_trial_info.usage_limit_with_precision

    for bonus in breakdown.bonuses:
        if bonus.status == 'ACTIVE':
            total += bonus.usage_limit

    return total


def calculate_current_usage(response: UsageLimitsResponse) -> float:
    if not response.usage_breakdown_list:
        return 0.0

    breakdown = response.usage_breakdown_list[0]
    total = breakdown.current_usage_with_precision

    if breakdown.free_trial_info and breakdown.free_trial_info.free_trial_status == 'ACTIVE':
        total += breakdown.free_trial_info.current_usage_with_precision

    for bonus in breakdown.bonuses:
        if bonus.status == 'ACTIVE':
            total += bonus.current_usage

    return total


__all__ = [
    'Bonus',
    'FreeTrialInfo',
    'SubscriptionInfo',
    'UsageBreakdown',
    'UsageLimitsResponse',
    'calculate_current_usage',
    'calculate_total_usage_limit',
]
