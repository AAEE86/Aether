"""CRC helpers for AWS Event Stream frames."""

from __future__ import annotations

import binascii


def crc32(data: bytes) -> int:
    """Compute unsigned CRC32 (IEEE)."""
    return binascii.crc32(data) & 0xFFFFFFFF


__all__ = ["crc32"]
