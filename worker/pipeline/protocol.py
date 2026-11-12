"""Length-prefixed JSON frames. The body is UTF-8. The prefix is a little-endian u32."""

import json
import struct
from typing import Any, BinaryIO


def write_frame(stream: BinaryIO, value: Any) -> None:
    body = json.dumps(value, ensure_ascii=False).encode("utf-8")
    stream.write(struct.pack("<I", len(body)))
    stream.write(body)
    stream.flush()


def read_frame(stream: BinaryIO) -> Any | None:
    prefix = stream.read(4)
    if not prefix:
        return None
    if len(prefix) != 4:
        raise EOFError("short length prefix")
    (length,) = struct.unpack("<I", prefix)
    if length > 64 * 1024 * 1024:
        raise ValueError("worker frame is larger than 64MB")
    body = stream.read(length)
    if len(body) != length:
        raise EOFError("short frame body")
    return json.loads(body.decode("utf-8"))
