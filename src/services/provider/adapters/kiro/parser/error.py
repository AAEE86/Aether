"""AWS Event Stream parsing errors."""

from __future__ import annotations


class EventStreamParseError(Exception):
    """Base error for AWS Event Stream decoding."""


class IncompleteFrameError(EventStreamParseError):
    def __init__(self, *, needed: int, available: int) -> None:
        super().__init__(f"incomplete frame: needed={needed} available={available}")
        self.needed = needed
        self.available = available


class MessageTooSmallError(EventStreamParseError):
    def __init__(self, *, length: int, min_length: int) -> None:
        super().__init__(f"message too small: length={length} min={min_length}")
        self.length = length
        self.min_length = min_length


class MessageTooLargeError(EventStreamParseError):
    def __init__(self, *, length: int, max_length: int) -> None:
        super().__init__(f"message too large: length={length} max={max_length}")
        self.length = length
        self.max_length = max_length


class PreludeCrcMismatchError(EventStreamParseError):
    def __init__(self, *, expected: int, actual: int) -> None:
        super().__init__(f"prelude crc mismatch: expected={expected} actual={actual}")
        self.expected = expected
        self.actual = actual


class MessageCrcMismatchError(EventStreamParseError):
    def __init__(self, *, expected: int, actual: int) -> None:
        super().__init__(f"message crc mismatch: expected={expected} actual={actual}")
        self.expected = expected
        self.actual = actual


class InvalidHeaderTypeError(EventStreamParseError):
    def __init__(self, type_id: int) -> None:
        super().__init__(f"invalid header type: {type_id}")
        self.type_id = type_id


class HeaderParseError(EventStreamParseError):
    pass


class BufferOverflowError(EventStreamParseError):
    def __init__(self, *, size: int, max_size: int) -> None:
        super().__init__(f"buffer overflow: size={size} max={max_size}")
        self.size = size
        self.max_size = max_size


__all__ = [
    "BufferOverflowError",
    "EventStreamParseError",
    "HeaderParseError",
    "IncompleteFrameError",
    "InvalidHeaderTypeError",
    "MessageCrcMismatchError",
    "MessageTooLargeError",
    "MessageTooSmallError",
    "PreludeCrcMismatchError",
]
