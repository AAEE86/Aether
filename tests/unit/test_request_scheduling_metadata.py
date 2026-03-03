from src.api.handlers.base.base_handler import BaseMessageHandler
from src.services.candidate.schema import CandidateKey


def _handler() -> BaseMessageHandler:
    return BaseMessageHandler.__new__(BaseMessageHandler)


def test_scheduling_audit_detects_internal_failover() -> None:
    handler = _handler()
    metadata = handler._merge_scheduling_metadata(
        {},
        candidate_keys=[
            {
                "candidate_index": 0,
                "retry_index": 0,
                "provider_id": "p1",
                "provider_name": "provider-a",
                "key_id": "k1",
                "key_name": "account-a",
                "status": "failed",
                "status_code": 429,
            },
            {
                "candidate_index": 1,
                "retry_index": 0,
                "provider_id": "p1",
                "provider_name": "provider-a",
                "key_id": "k2",
                "key_name": "account-b",
                "status": "success",
                "status_code": 200,
            },
        ],
        selected_key_id="k2",
        fallback_from_request=False,
    )
    assert metadata is not None
    audit = metadata.get("scheduling_audit")
    assert isinstance(audit, dict)
    assert audit.get("attempted_count") == 2
    assert audit.get("account_count") == 2
    assert audit.get("retry_occurred") is True
    assert audit.get("failover_occurred") is True
    assert audit.get("selected_key_id") == "k2"

    accounts = audit.get("accounts")
    assert isinstance(accounts, list)
    assert any(a.get("key_id") == "k2" and a.get("selected") for a in accounts)


def test_scheduling_audit_distinguishes_retry_from_failover() -> None:
    handler = _handler()
    metadata = handler._merge_scheduling_metadata(
        {},
        candidate_keys=[
            {
                "candidate_index": 0,
                "retry_index": 0,
                "provider_id": "p1",
                "provider_name": "provider-a",
                "key_id": "k1",
                "key_name": "account-a",
                "status": "failed",
                "status_code": 503,
            },
            {
                "candidate_index": 0,
                "retry_index": 1,
                "provider_id": "p1",
                "provider_name": "provider-a",
                "key_id": "k1",
                "key_name": "account-a",
                "status": "success",
                "status_code": 200,
            },
        ],
        selected_key_id="k1",
        fallback_from_request=False,
    )
    assert metadata is not None
    audit = metadata.get("scheduling_audit")
    assert isinstance(audit, dict)
    assert audit.get("attempted_count") == 2
    assert audit.get("account_count") == 1
    assert audit.get("retry_occurred") is True
    assert audit.get("failover_occurred") is False


def test_scheduling_metadata_supports_candidate_key_dataclass() -> None:
    handler = _handler()
    metadata = handler._merge_scheduling_metadata(
        {},
        candidate_keys=[
            CandidateKey(
                candidate_index=0,
                retry_index=0,
                provider_id="p1",
                provider_name="provider-a",
                endpoint_id="e1",
                key_id="k1",
                key_name="account-a",
                status="success",
                status_code=200,
            )
        ],
        selected_key_id="k1",
        fallback_from_request=False,
    )
    assert metadata is not None
    snapshots = metadata.get("candidate_keys")
    assert isinstance(snapshots, list)
    assert snapshots[0]["status"] == "success"
    assert snapshots[0]["key_id"] == "k1"


def test_scheduling_audit_excludes_unused_candidates() -> None:
    handler = _handler()
    metadata = handler._merge_scheduling_metadata(
        {},
        candidate_keys=[
            {
                "candidate_index": 0,
                "retry_index": 0,
                "provider_id": "p1",
                "provider_name": "provider-a",
                "key_id": "k1",
                "key_name": "account-a",
                "status": "success",
                "status_code": 200,
            },
            {
                "candidate_index": 1,
                "retry_index": 0,
                "provider_id": "p1",
                "provider_name": "provider-a",
                "key_id": "k2",
                "key_name": "account-b",
                "status": "unused",
            },
        ],
        selected_key_id="k1",
        fallback_from_request=False,
    )

    assert metadata is not None
    audit = metadata.get("scheduling_audit")
    assert isinstance(audit, dict)
    assert audit.get("attempted_count") == 1
    assert audit.get("account_count") == 1
    attempts = audit.get("attempts")
    assert isinstance(attempts, list)
    assert len(attempts) == 1
    assert attempts[0].get("key_id") == "k1"
