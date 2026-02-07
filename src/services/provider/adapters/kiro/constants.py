"""Kiro adapter constants.

Kiro upstream uses AWS Event Stream (binary frames) for streaming responses.
"""

from __future__ import annotations

import platform


AWS_EVENTSTREAM_CONTENT_TYPE = "application/vnd.amazon.eventstream"

# Kiro API endpoints
KIRO_GENERATE_ASSISTANT_PATH = "/generateAssistantResponse"
KIRO_USAGE_LIMITS_PATH = "/getUsageLimits"

# Default client fingerprints used in headers (best-effort)
DEFAULT_KIRO_VERSION = "0.8.0"
DEFAULT_NODE_VERSION = "22.21.1"


def _detect_system_version() -> str:
    system = platform.system().lower() or "other"
    release = platform.release() or "unknown"
    # Match KiroIDE style: darwin#24.6.0, windows#10.0.22631, linux#6.8.0-...
    return f"{system}#{release}"


DEFAULT_SYSTEM_VERSION = _detect_system_version()

# Header constants
KIRO_AGENT_MODE = "vibe"
CODEWHISPERER_OPTOUT = "true"

# aws-sdk-js versions observed in kiro.rs
AWS_SDK_JS_MAIN_VERSION = "1.0.27"
AWS_SDK_JS_USAGE_VERSION = "1.0.0"

# Claude model context window used by kiro.rs to convert contextUsage percentage -> tokens
CONTEXT_WINDOW_TOKENS = 200_000

__all__ = [
    "AWS_EVENTSTREAM_CONTENT_TYPE",
    "AWS_SDK_JS_MAIN_VERSION",
    "AWS_SDK_JS_USAGE_VERSION",
    "CODEWHISPERER_OPTOUT",
    "CONTEXT_WINDOW_TOKENS",
    "DEFAULT_KIRO_VERSION",
    "DEFAULT_NODE_VERSION",
    "DEFAULT_SYSTEM_VERSION",
    "KIRO_AGENT_MODE",
    "KIRO_GENERATE_ASSISTANT_PATH",
    "KIRO_USAGE_LIMITS_PATH",
]
