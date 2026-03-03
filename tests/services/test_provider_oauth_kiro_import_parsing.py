from __future__ import annotations

import json

from src.api.admin.provider_oauth import _parse_kiro_import_input
from src.services.provider.adapters.kiro.models.credentials import KiroAuthConfig


def test_parse_kiro_import_input_array_unwraps_nested_auth_config() -> None:
    raw = json.dumps(
        [
            {
                "name": "acc-1",
                "auth_config": {
                    "refresh_token": "rt-1",
                    "auth_method": "identity_center",
                    "client_id": "cid-1",
                    "client_secret": "csec-1",
                },
            }
        ]
    )

    parsed = _parse_kiro_import_input(raw)
    assert len(parsed) == 1
    assert parsed[0]["refresh_token"] == "rt-1"
    assert parsed[0]["auth_method"] == "identity_center"
    assert parsed[0]["client_id"] == "cid-1"


def test_parse_kiro_import_input_single_object_unwraps_auth_config() -> None:
    raw = json.dumps(
        {
            "name": "acc-1",
            "authConfig": {
                "refreshToken": "rt-1",
                "authType": "builder_id",
                "clientId": "cid-1",
                "clientSecret": "csec-1",
            },
        }
    )

    parsed = _parse_kiro_import_input(raw)
    assert len(parsed) == 1
    assert parsed[0]["refreshToken"] == "rt-1"
    assert parsed[0]["authType"] == "builder_id"


def test_kiro_auth_config_from_dict_maps_device_alias_to_idc() -> None:
    cfg = KiroAuthConfig.from_dict(
        {
            "refreshToken": "rt-1",
            "auth_type": "identity_center",
            "clientId": "cid-1",
            "clientSecret": "csec-1",
        }
    )
    assert cfg.auth_method == "idc"


def test_kiro_auth_config_validate_requires_idc_client_fields_when_explicit() -> None:
    is_valid, message = KiroAuthConfig.validate_required_fields(
        {
            "refreshToken": "rt-1",
            "auth_type": "builder_id",
        }
    )
    assert is_valid is False
    assert "clientId" in message
