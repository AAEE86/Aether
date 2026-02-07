"""AWS Event Stream message frame parsing."""

from __future__ import annotations

from dataclasses import dataclass

from .crc import crc32
from .error import (
    HeaderParseError,
    IncompleteFrameError,
    MessageCrcMismatchError,
    MessageTooLargeError,
    MessageTooSmallError,
    PreludeCrcMismatchError,
)
from .header import Headers, parse_headers


PRELUDE_SIZE = 12
MIN_MESSAGE_SIZE = PRELUDE_SIZE + 4
MAX_MESSAGE_SIZE = 16 * 1024 * 1024


@dataclass(slots=True)
class Frame:
    headers: Headers
    payload: bytes

    def message_type(self) -> str | None:
        return self.headers.message_type()

    def event_type(self) -> str | None:
        return self.headers.event_type()

    def payload_as_text(self) -> str:
        return self.payload.decode("utf-8", errors="replace")


def parse_frame(buffer: bytes) -> tuple[Frame, int] | None:
    """Parse a single frame from the front of buffer.

    Returns:
        (frame, consumed_bytes) if a full frame is available, otherwise None.

    Raises:
        EventStreamParseError subclasses on validation errors.
    """
    if len(buffer) < PRELUDE_SIZE:
        return None

    total_length = int.from_bytes(buffer[0:4], "big", signed=False)
    header_length = int.from_bytes(buffer[4:8], "big", signed=False)
    prelude_crc = int.from_bytes(buffer[8:12], "big", signed=False)

    if total_length < MIN_MESSAGE_SIZE:
        raise MessageTooSmallError(length=total_length, min_length=MIN_MESSAGE_SIZE)
    if total_length > MAX_MESSAGE_SIZE:
        raise MessageTooLargeError(length=total_length, max_length=MAX_MESSAGE_SIZE)

    if len(buffer) < total_length:
        return None

    actual_prelude_crc = crc32(buffer[0:8])
    if actual_prelude_crc != prelude_crc:
        raise PreludeCrcMismatchError(expected=prelude_crc, actual=actual_prelude_crc)

    message_crc = int.from_bytes(buffer[total_length - 4 : total_length], "big", signed=False)
    actual_message_crc = crc32(buffer[0 : total_length - 4])
    if actual_message_crc != message_crc:
        raise MessageCrcMismatchError(expected=message_crc, actual=actual_message_crc)

    headers_start = PRELUDE_SIZE
    headers_end = headers_start + header_length

    if headers_end > total_length - 4:
        raise HeaderParseError("header length exceeds frame boundary")

    headers = parse_headers(buffer[headers_start:headers_end], header_length)

    payload_start = headers_end
    payload_end = total_length - 4
    if payload_end < payload_start:
        raise IncompleteFrameError(needed=payload_start, available=payload_end)

    payload = bytes(buffer[payload_start:payload_end])

    return Frame(headers=headers, payload=payload), total_length


__all__ = [
    "Frame",
    "MAX_MESSAGE_SIZE",
    "MIN_MESSAGE_SIZE",
    "PRELUDE_SIZE",
    "parse_frame",
]
