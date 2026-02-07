"""AWS Event Stream header parsing."""

from __future__ import annotations

from dataclasses import dataclass
from enum import IntEnum

from .error import HeaderParseError, IncompleteFrameError, InvalidHeaderTypeError


class HeaderValueType(IntEnum):
    BOOL_TRUE = 0
    BOOL_FALSE = 1
    BYTE = 2
    SHORT = 3
    INTEGER = 4
    LONG = 5
    BYTE_ARRAY = 6
    STRING = 7
    TIMESTAMP = 8
    UUID = 9


@dataclass(slots=True)
class Headers:
    values: dict[str, object]

    def get(self, name: str) -> object | None:
        return self.values.get(name)

    def get_string(self, name: str) -> str | None:
        v = self.values.get(name)
        return v if isinstance(v, str) else None

    def message_type(self) -> str | None:
        return self.get_string(":message-type")

    def event_type(self) -> str | None:
        return self.get_string(":event-type")

    def exception_type(self) -> str | None:
        return self.get_string(":exception-type")

    def error_code(self) -> str | None:
        return self.get_string(":error-code")


def _ensure_bytes(data: bytes, offset: int, needed: int) -> None:
    available = len(data) - offset
    if available < needed:
        raise IncompleteFrameError(needed=needed, available=available)


def parse_headers(data: bytes, header_length: int) -> Headers:
    if len(data) < header_length:
        raise IncompleteFrameError(needed=header_length, available=len(data))

    values: dict[str, object] = {}
    offset = 0

    while offset < header_length:
        _ensure_bytes(data, offset, 1)
        name_len = data[offset]
        offset += 1
        if name_len == 0:
            raise HeaderParseError("header name length cannot be 0")

        _ensure_bytes(data, offset, name_len)
        name = data[offset : offset + name_len].decode("utf-8", errors="replace")
        offset += name_len

        _ensure_bytes(data, offset, 1)
        type_id = data[offset]
        offset += 1
        try:
            value_type = HeaderValueType(type_id)
        except ValueError as e:
            raise InvalidHeaderTypeError(type_id) from e

        if value_type == HeaderValueType.BOOL_TRUE:
            values[name] = True
            continue
        if value_type == HeaderValueType.BOOL_FALSE:
            values[name] = False
            continue

        if value_type == HeaderValueType.BYTE:
            _ensure_bytes(data, offset, 1)
            values[name] = int.from_bytes(data[offset : offset + 1], "big", signed=True)
            offset += 1
            continue

        if value_type == HeaderValueType.SHORT:
            _ensure_bytes(data, offset, 2)
            values[name] = int.from_bytes(data[offset : offset + 2], "big", signed=True)
            offset += 2
            continue

        if value_type == HeaderValueType.INTEGER:
            _ensure_bytes(data, offset, 4)
            values[name] = int.from_bytes(data[offset : offset + 4], "big", signed=True)
            offset += 4
            continue

        if value_type in (HeaderValueType.LONG, HeaderValueType.TIMESTAMP):
            _ensure_bytes(data, offset, 8)
            values[name] = int.from_bytes(data[offset : offset + 8], "big", signed=True)
            offset += 8
            continue

        if value_type == HeaderValueType.BYTE_ARRAY:
            _ensure_bytes(data, offset, 2)
            length = int.from_bytes(data[offset : offset + 2], "big", signed=False)
            offset += 2
            _ensure_bytes(data, offset, length)
            values[name] = data[offset : offset + length]
            offset += length
            continue

        if value_type == HeaderValueType.STRING:
            _ensure_bytes(data, offset, 2)
            length = int.from_bytes(data[offset : offset + 2], "big", signed=False)
            offset += 2
            _ensure_bytes(data, offset, length)
            values[name] = data[offset : offset + length].decode("utf-8", errors="replace")
            offset += length
            continue

        if value_type == HeaderValueType.UUID:
            _ensure_bytes(data, offset, 16)
            values[name] = bytes(data[offset : offset + 16])
            offset += 16
            continue

        raise HeaderParseError(f"unhandled header type: {value_type}")

    return Headers(values=values)


__all__ = [
    "HeaderValueType",
    "Headers",
    "parse_headers",
]
