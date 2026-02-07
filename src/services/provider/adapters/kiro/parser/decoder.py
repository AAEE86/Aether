"""Incremental AWS Event Stream decoder."""

from __future__ import annotations

from dataclasses import dataclass

from .error import BufferOverflowError, EventStreamParseError
from .frame import Frame, MAX_MESSAGE_SIZE, parse_frame


DEFAULT_MAX_BUFFER_SIZE = MAX_MESSAGE_SIZE
DEFAULT_MAX_ERRORS = 5


@dataclass(slots=True)
class DecoderStats:
    frames_decoded: int = 0
    bytes_skipped: int = 0
    error_count: int = 0


class EventStreamDecoder:
    def __init__(
        self,
        *,
        max_buffer_size: int = DEFAULT_MAX_BUFFER_SIZE,
        max_errors: int = DEFAULT_MAX_ERRORS,
    ) -> None:
        self._buffer = bytearray()
        self._max_buffer_size = int(max_buffer_size)
        self._max_errors = int(max_errors)
        self._stopped = False
        self.stats = DecoderStats()

    @property
    def stopped(self) -> bool:
        return self._stopped

    def feed(self, data: bytes) -> None:
        if self._stopped:
            return
        if not data:
            return
        new_size = len(self._buffer) + len(data)
        if new_size > self._max_buffer_size:
            self._stopped = True
            raise BufferOverflowError(size=new_size, max_size=self._max_buffer_size)
        self._buffer.extend(data)

    def decode_available(self) -> list[Frame]:
        """Decode all complete frames currently in buffer."""
        out: list[Frame] = []
        if self._stopped:
            return out

        while True:
            try:
                parsed = parse_frame(bytes(self._buffer))
            except EventStreamParseError:
                self.stats.error_count += 1
                if self.stats.error_count >= self._max_errors:
                    self._stopped = True
                    raise

                # Recovery: skip a byte and keep scanning.
                if self._buffer:
                    del self._buffer[0]
                    self.stats.bytes_skipped += 1
                else:
                    break
                continue

            if parsed is None:
                break

            frame, consumed = parsed
            if consumed <= 0:
                break

            out.append(frame)
            del self._buffer[:consumed]
            self.stats.frames_decoded += 1
            self.stats.error_count = 0

        return out


__all__ = [
    "DecoderStats",
    "EventStreamDecoder",
]
