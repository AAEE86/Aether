"""Kiro header builders."""

from __future__ import annotations

import uuid

from src.services.provider.adapters.kiro.constants import (
    AWS_EVENTSTREAM_CONTENT_TYPE,
    AWS_SDK_JS_MAIN_VERSION,
    AWS_SDK_JS_USAGE_VERSION,
    CODEWHISPERER_OPTOUT,
    DEFAULT_KIRO_VERSION,
    DEFAULT_NODE_VERSION,
    DEFAULT_SYSTEM_VERSION,
    KIRO_AGENT_MODE,
)


def build_kiro_ide_tag(*, kiro_version: str, machine_id: str) -> str:
    version = (kiro_version or DEFAULT_KIRO_VERSION).strip() or DEFAULT_KIRO_VERSION
    mid = (machine_id or '').strip()
    return f"KiroIDE-{version}-{mid}" if mid else f"KiroIDE-{version}"


def build_x_amz_user_agent_main(*, kiro_version: str, machine_id: str) -> str:
    return f"aws-sdk-js/{AWS_SDK_JS_MAIN_VERSION} {build_kiro_ide_tag(kiro_version=kiro_version, machine_id=machine_id)}"


def build_user_agent_main(*, system_version: str, node_version: str, kiro_version: str, machine_id: str) -> str:
    os_tag = (system_version or DEFAULT_SYSTEM_VERSION).strip() or DEFAULT_SYSTEM_VERSION
    node_tag = (node_version or DEFAULT_NODE_VERSION).strip() or DEFAULT_NODE_VERSION
    ide = build_kiro_ide_tag(kiro_version=kiro_version, machine_id=machine_id)
    return (
        f"aws-sdk-js/{AWS_SDK_JS_MAIN_VERSION} ua/2.1 os/{os_tag} lang/js "
        f"md/nodejs#{node_tag} api/codewhispererstreaming#{AWS_SDK_JS_MAIN_VERSION} m/E {ide}"
    )


def build_x_amz_user_agent_usage(*, kiro_version: str, machine_id: str) -> str:
    ide = build_kiro_ide_tag(kiro_version=kiro_version, machine_id=machine_id)
    return f"aws-sdk-js/{AWS_SDK_JS_USAGE_VERSION} {ide}"


def build_user_agent_usage(*, kiro_version: str, machine_id: str) -> str:
    ide = build_kiro_ide_tag(kiro_version=kiro_version, machine_id=machine_id)
    # Match kiro.rs default UA for getUsageLimits.
    return (
        f"aws-sdk-js/{AWS_SDK_JS_USAGE_VERSION} ua/2.1 os/darwin#24.6.0 lang/js "
        f"md/nodejs#22.21.1 api/codewhispererruntime#1.0.0 m/N,E {ide}"
    )


def build_generate_assistant_headers(
    *,
    host: str,
    access_token: str,
    machine_id: str,
    kiro_version: str | None = None,
    system_version: str | None = None,
    node_version: str | None = None,
) -> dict[str, str]:
    version = (kiro_version or DEFAULT_KIRO_VERSION).strip() or DEFAULT_KIRO_VERSION
    sys_ver = (system_version or DEFAULT_SYSTEM_VERSION).strip() or DEFAULT_SYSTEM_VERSION
    node_ver = (node_version or DEFAULT_NODE_VERSION).strip() or DEFAULT_NODE_VERSION

    headers: dict[str, str] = {
        'Content-Type': 'application/json',
        'Accept': AWS_EVENTSTREAM_CONTENT_TYPE,
        'Authorization': f'Bearer {access_token}',
        'host': host,
        'Connection': 'close',
        'x-amzn-codewhisperer-optout': CODEWHISPERER_OPTOUT,
        'x-amzn-kiro-agent-mode': KIRO_AGENT_MODE,
        'x-amz-user-agent': build_x_amz_user_agent_main(kiro_version=version, machine_id=machine_id),
        'User-Agent': build_user_agent_main(
            system_version=sys_ver,
            node_version=node_ver,
            kiro_version=version,
            machine_id=machine_id,
        ),
        'amz-sdk-invocation-id': str(uuid.uuid4()),
        'amz-sdk-request': 'attempt=1; max=3',
    }

    return headers


__all__ = [
    'build_generate_assistant_headers',
    'build_kiro_ide_tag',
    'build_user_agent_main',
    'build_user_agent_usage',
    'build_x_amz_user_agent_main',
    'build_x_amz_user_agent_usage',
]
