"""Generate finite offline LZMS decoder cases with original C wimlib.

Usage: python3 generate_lzms_oracle.py /tmp/wimlib-native-oracle/.libs/libwim.so
Tests consume only the resulting bytes and do not load C.
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
MAX_SIZE = 65536
rng = random.Random(0x4c5a4d53)
decoder = c.c_void_p()
assert lib.wimlib_create_decompressor(3, MAX_SIZE, c.byref(decoder)) == 0
cases = []
valid = []


def oracle(name, data, size):
    output = c.create_string_buffer(max(1, size) + 64)
    result = lib.wimlib_decompress(data, len(data), output, size, decoder)
    decoded = output.raw[:size] if result == 0 else b""
    cases.append((name, data, size, result == 0, decoded))
    return result, decoded


for level in [1, 35, 60]:
    compressor = c.c_void_p()
    assert lib.wimlib_create_compressor(3, MAX_SIZE, level, c.byref(compressor)) == 0
    for size in [32, 64, 256, 1024, 4096, 16384, 65536]:
        patterns = {
            "zeros": b"\0",
            "phrase": b"Native Rust LZMS decoder adaptive Huffman range coder\n",
            "all-bytes": bytes(range(256)),
            "binary-period": bytes(rng.randrange(256) for _ in range(129)),
        }
        raw_patterns = {name: (seq * ((size + len(seq) - 1) // len(seq)))[:size]
                        for name, seq in patterns.items()}
        raw_patterns["delta-u32"] = b"".join(struct.pack("<I", i * 17) for i in range((size + 3) // 4))[:size]
        raw_patterns["delta-u8"] = bytes((i // 3 + i % 3) % 256 for i in range(size))
        raw_patterns["random"] = bytes(rng.randrange(256) for _ in range(size))
        code = bytearray(b"\x90" * size)
        for pos in range(1, size - 20, 8):
            opcode = rng.choice([b"\xe8", b"\x48\x8d\x05", b"\xff\x15", b"\xf0\x83\x05", b"\x4c\x8d\x15"])
            code[pos:pos + len(opcode)] = opcode
            # Repeated target activates the x86 identification heuristic.
            code[pos + len(opcode):pos + len(opcode) + 4] = struct.pack("<i", 1024 - pos)
        raw_patterns["x86-addresses"] = bytes(code)
        for name, raw in raw_patterns.items():
            compressed = c.create_string_buffer(2 * MAX_SIZE)
            length = lib.wimlib_compress(raw, size, compressed, len(compressed), compressor)
            if not length:
                continue
            encoded = compressed.raw[:length]
            label = f"encoded-level-{level}-size-{size}-{name}"
            result, decoded = oracle(label, encoded, size)
            assert result == 0 and decoded == raw, label
            valid.append((label, encoded, size))
    lib.wimlib_free_compressor(compressor)

# Every cutoff of representative small blocks; longer blocks have sampled cuts.
for name, encoded, size in valid[:16]:
    for cutoff in range(len(encoded) + 1):
        oracle(f"truncated-{name}-at-{cutoff}", encoded[:cutoff], size)
for case in range(600):
    name, encoded, size = rng.choice(valid)
    mutated = bytearray(encoded)
    for _ in range(1 + rng.randrange(4)):
        index = rng.randrange(len(mutated))
        mutated[index] ^= 1 << rng.randrange(8)
    oracle(f"mutated-{case}-{name}", bytes(mutated), size)
for case in range(150):
    name, encoded, size = rng.choice(valid)
    oracle(f"sampled-cut-{case}-{name}", encoded[:rng.randrange(len(encoded) + 1)], size)
for case in range(100):
    data = bytes(rng.randrange(256) for _ in range(rng.randrange(80)))
    oracle(f"random-{case}", data, rng.choice([32, 256, 1024]))
for data in [b"", b"\0", b"\0\0", b"\0\0\0", b"\0\0\0\0", b"\xff" * 4]:
    for size in [32]:
        oracle(f"boundary-{data.hex()}-output-{size}", data, size)
# The upstream corpus selector is not part of the compressed block.
# The original harness incorrectly creates a decoder smaller than its output;
# this oracle deliberately uses MAX_SIZE >= every requested output length.
corpus = (HERE / "lzms-libfuzzer").read_bytes()[1:]
for size in [32, 256, 1024, 4096, 16384, 32768, 3 * (len(corpus) + 1), 65536]:
    oracle(f"upstream-libfuzzer-output-{size}", corpus, size)
for case in range(200):
    cutoff = rng.randrange(len(corpus) + 1)
    data = bytearray(corpus[:cutoff])
    if data:
        data[rng.randrange(len(data))] ^= 1 << rng.randrange(8)
    oracle(f"upstream-cut-mutated-{case}", bytes(data), rng.choice([32, 256, 1024, 4096, 16384]))
lib.wimlib_free_decompressor(decoder)
with (HERE / "lzms-oracle.wlm").open("wb") as f:
    f.write(b"WLM1" + struct.pack("<I", len(cases)))
    for name, data, size, success, output in cases:
        encoded_name = name.encode()
        f.write(struct.pack("<H", len(encoded_name)) + encoded_name)
        f.write(struct.pack("<IIB", len(data), size, success))
        f.write(data + output)
print(f"Saved {len(cases)} cases: {sum(x[3] for x in cases)} accepted, {sum(not x[3] for x in cases)} rejected")
