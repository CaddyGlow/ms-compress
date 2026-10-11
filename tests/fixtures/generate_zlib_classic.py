"""Regenerate zlib-classic.bin with independent native zlib 1.3.2."""
from pathlib import Path
import struct
import zlib

assert zlib.ZLIB_RUNTIME_VERSION == "1.3.2", zlib.ZLIB_RUNTIME_VERSION


def noise(length):
    state = 0x12345678
    result = bytearray()
    for _ in range(length):
        state ^= (state << 13) & 0xFFFFFFFF
        state ^= state >> 17
        state ^= (state << 5) & 0xFFFFFFFF
        result.append(state & 255)
    return bytes(result)


def pattern(length):
    return bytes((i * 37 + i // 11) & 255 for i in range(length))


cases = [
    ("empty", b""),
    ("tiny", b"abcabcabcabcd"),
    ("lookahead", pattern(262)),
    ("chunk-boundary", pattern(32769)),
    ("random-window-boundary", noise(65537)),
    ("repeated-window-boundary", b"A" * 65537),
    ("mixed-multiple-windows", noise(32768) + b"A" * 32768 + pattern(65536)),
]
result = bytearray(b"ZCL1" + struct.pack("<I", len(cases)))
for name, data in cases:
    raw = zlib.compressobj(1, zlib.DEFLATED, -15, 9, zlib.Z_DEFAULT_STRATEGY)
    encoded = b"".join(raw.compress(data[i:i + 32768]) for i in range(0, len(data), 32768))
    encoded += raw.flush(zlib.Z_FINISH)
    gzip = zlib.compressobj(6, zlib.DEFLATED, 31, 8, zlib.Z_DEFAULT_STRATEGY)
    wrapped = bytearray(gzip.compress(data) + gzip.flush(zlib.Z_FINISH))
    wrapped[9] = 10  # Defender's documented NTFS OS byte.
    assert zlib.decompress(encoded, -15) == data
    assert zlib.decompress(wrapped, 31) == data
    name = name.encode()
    result += struct.pack("<H", len(name)) + name
    result += struct.pack("<III", len(data), len(encoded), len(wrapped))
    result += data + encoded + wrapped
path = Path(__file__).with_name("zlib-classic.bin")
path.write_bytes(result)
print(f"{path.name}: {len(cases)} cases, {len(result)} bytes, zlib {zlib.ZLIB_RUNTIME_VERSION}")
