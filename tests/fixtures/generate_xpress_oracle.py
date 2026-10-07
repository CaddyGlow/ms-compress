"""Build offline XPRESS differential cases using the pinned original C library.

Usage: python3 generate_xpress_oracle.py /tmp/wimlib-native-oracle/.libs/libwim.so
Output is the adjacent xpress-oracle.wxp; runtime Rust tests never load C.
"""
import ctypes as c
from pathlib import Path
import random
import struct
import sys

HERE = Path(__file__).resolve().parent
lib = c.CDLL(sys.argv[1])
lib.wimlib_create_compressor.argtypes = [c.c_int, c.c_size_t, c.c_uint, c.POINTER(c.c_void_p)]
lib.wimlib_compress.argtypes = [c.c_void_p, c.c_size_t, c.c_void_p, c.c_size_t, c.c_void_p]
lib.wimlib_compress.restype = c.c_size_t
lib.wimlib_free_compressor.argtypes = [c.c_void_p]
lib.wimlib_create_decompressor.argtypes = [c.c_int, c.c_size_t, c.POINTER(c.c_void_p)]
lib.wimlib_decompress.argtypes = [c.c_void_p, c.c_size_t, c.c_void_p, c.c_size_t, c.c_void_p]
lib.wimlib_free_decompressor.argtypes = [c.c_void_p]

decoder = c.c_void_p()
assert lib.wimlib_create_decompressor(1, 65536, c.byref(decoder)) == 0
cases = []
valid = []
rng = random.Random(0x585052455353)


def oracle(name, data, out_len):
    output = c.create_string_buffer(max(1, out_len))
    status = lib.wimlib_decompress(data, len(data), output, out_len, decoder)
    result = output.raw[:out_len] if status == 0 else b""
    cases.append((name, data, out_len, status == 0, result))
    return status, result


# Real upstream encoder output, including large blocks and all byte values.
for level in [1, 50, 100]:
    compressor = c.c_void_p()
    assert lib.wimlib_create_compressor(1, 65536, level, c.byref(compressor)) == 0
    for size in [256, 512, 4096, 32768, 65536]:
        patterns = {
            "zeros": b"\0",
            "phrase": b"Native Rust XPRESS Huffman decoder\n",
            "all-bytes": bytes(range(256)),
            "binary-period": bytes(rng.randrange(256) for _ in range(129)),
        }
        for pattern, sequence in patterns.items():
            raw = (sequence * ((size + len(sequence) - 1) // len(sequence)))[:size]
            compressed = c.create_string_buffer(131072)
            compressed_len = lib.wimlib_compress(raw, size, compressed, len(compressed), compressor)
            if not compressed_len:
                continue  # upstream cannot compress the small high-entropy case
            encoded = compressed.raw[:compressed_len]
            name = f"encoded-level-{level}-size-{size}-{pattern}"
            status, result = oracle(name, encoded, size)
            assert status == 0 and result == raw, name
            valid.append((name, encoded, size))
    lib.wimlib_free_compressor(compressor)

# Shipped libFuzzer file includes the codec selector byte; it is not block data.
corpus = (HERE / "xpress-libfuzzer").read_bytes()[1:]
for size in [0, 1, 16, 256, 1024, 4096, 16384, 32768, 65536]:
    oracle(f"libfuzzer-original-output-{size}", corpus, size)

# Exhaustively truncate representative compressed blocks through every byte.
for name, encoded, size in valid[:8]:
    for cutoff in range(len(encoded) + 1):
        oracle(f"truncated-{name}-at-{cutoff}", encoded[:cutoff], size)

# Mutate both code lengths and bitstream, including the long shipped corpus.
for case in range(800):
    name, encoded, size = valid[rng.randrange(len(valid))]
    mutated = bytearray(encoded)
    for _ in range(1 + rng.randrange(4)):
        index = rng.randrange(len(mutated))
        mutated[index] ^= 1 << rng.randrange(8)
    oracle(f"mutated-{case}-{name}", bytes(mutated), size)
for case in range(200):
    cutoff = rng.randrange(len(corpus) + 1)
    mutated = bytearray(corpus[:cutoff])
    if mutated:
        mutated[rng.randrange(len(mutated))] ^= 1 << rng.randrange(8)
    oracle(f"corpus-cut-mutated-{case}", bytes(mutated), rng.choice([1, 256, 4096, 32768, 65536]))

# Random complete/incomplete tables and payloads exercise canonical validation.
for case in range(200):
    if case % 3 == 0:
        data = bytes(rng.randrange(256) for _ in range(256 + rng.randrange(80)))
    else:
        header = bytearray(256)
        a, b = sorted(rng.sample(range(512), 2))
        for symbol in [a, b]:
            header[symbol // 2] |= 1 << ((symbol % 2) * 4)
        data = bytes(header) + bytes(rng.randrange(256) for _ in range(rng.randrange(60)))
    oracle(f"table-and-payload-{case}", data, rng.choice([0, 1, 32, 1024]))

lib.wimlib_free_decompressor(decoder)
# WXP1: u32 case count; each record has u16 UTF-8 name size, name, u32 input
# size, u32 requested output size, u8 success, input, output only on success.
with (HERE / "xpress-oracle.wxp").open("wb") as f:
    f.write(b"WXP1" + struct.pack("<I", len(cases)))
    for name, data, size, success, output in cases:
        name = name.encode()
        f.write(struct.pack("<H", len(name)) + name)
        f.write(struct.pack("<IIB", len(data), size, success))
        f.write(data + output)
print(f"Saved {len(cases)} cases: {sum(x[3] for x in cases)} successful, {sum(not x[3] for x in cases)} rejected")
