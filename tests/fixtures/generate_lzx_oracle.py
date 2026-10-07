"""Create pinned original-C LZX differential records with compact output snapshots.

Run with the original libwim.so as the sole argument. Rust tests need no C.
"""
from collections import Counter
import ctypes as c
import hashlib
import json
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
rng = random.Random(0x4C5A584E4154495645)
cases = []
valid = []
decoders = {}


def oracle(name, data, size, maximum):
    if maximum not in decoders:
        decoder = c.c_void_p()
        assert lib.wimlib_create_decompressor(2, maximum, c.byref(decoder)) == 0
        decoders[maximum] = decoder
    output = c.create_string_buffer(max(1, size))
    status = lib.wimlib_decompress(data, len(data), output, size, decoders[maximum])
    raw = output.raw[:size] if status == 0 else b""
    cases.append((name, data, size, maximum, status == 0, raw))
    return status, raw


def encode_expectation(output):
    if len(output) <= 32:
        return 0, output
    probe = output[:32]
    period = output.find(probe, 1)
    for _ in range(16):
        if period < 0:
            break
        pattern = output[:period]
        if output == (pattern * ((len(output) + period - 1) // period))[:len(output)]:
            return 1, pattern
        period = output.find(probe, period + 1)
    return 0, output


for order in range(15, 22):
    maximum = 1 << order
    for level in [1, 50]:
        compressor = c.c_void_p()
        assert lib.wimlib_create_compressor(2, maximum, level, c.byref(compressor)) == 0
        for size in sorted(set([4096, 32768, maximum])):
            long_period = bytes(rng.randrange(256) for _ in range(min(size // 2, 131075)))
            # Include x86 targets and boundary values: original preprocessing
            # must be inverted exactly to recover this period.
            patterns = {
                "phrase": b"native WIM LZX compressed block\n",
                "all-bytes": bytes(range(256)),
                "long-offset": long_period,
                "e8-targets": b"\xe8\x01\0\0\0\xe8\xff\xff\xff\xff\xe8\x00\x1b\xb7\0\0\0\0",
            }
            for profile, period in patterns.items():
                raw = (period * ((size + len(period) - 1) // len(period)))[:size]
                encoded = c.create_string_buffer(size + 8192)
                count = lib.wimlib_compress(raw, size, encoded, len(encoded), compressor)
                if not count:
                    continue
                data = encoded.raw[:count]
                name = f"encoded-window{order}-level{level}-size{size}-{profile}"
                status, actual = oracle(name, data, size, maximum)
                assert status == 0 and actual == raw, name
                valid.append((name, data, size, maximum))
        lib.wimlib_free_compressor(compressor)

corpus = (HERE / "lzx-libfuzzer").read_bytes()[1:]
for maximum in [32768, 65536, 2097152]:
    for size in [0, 1, 100, 1024, 32768]:
        oracle(f"libfuzzer-max{maximum}-output{size}", corpus, size, maximum)

# Exhaustive byte truncation of compact actual compressed blocks.
for name, data, size, maximum in [v for v in valid if len(v[1]) < 512][:8]:
    for cutoff in range(len(data) + 1):
        oracle(f"truncated-{name}-at{cutoff}", data[:cutoff], size, maximum)

small_valid = [v for v in valid if v[2] == 4096]
for index in range(900):
    name, data, size, maximum = rng.choice(small_valid)
    data = bytearray(data)
    for _ in range(1 + rng.randrange(3)):
        position = rng.randrange(len(data))
        data[position] ^= 1 << rng.randrange(8)
    oracle(f"mutated{index}-{name}", bytes(data), size, maximum)
for index in range(200):
    data = bytearray(corpus[:rng.randrange(len(corpus) + 1)])
    if data:
        data[rng.randrange(len(data))] ^= 1 << rng.randrange(8)
    oracle(f"corpus-truncated-mutated{index}", bytes(data), rng.choice([1, 1024, 32768]), 32768)
for index in range(200):
    data = bytes(rng.randrange(256) for _ in range(rng.randrange(1, 2048)))
    oracle(f"arbitrary{index}", data, rng.choice([1, 16, 4096]), 32768)

# Independently hand-encoded semantic edge cases, also checked against C.
class Bits:
    def __init__(self):
        self.values = []

    def push(self, value, count):
        self.values.extend((value >> bit) & 1 for bit in reversed(range(count)))

    def bytes(self):
        result = bytearray()
        for start in range(0, len(self.values), 16):
            word = sum(value << (15 - i) for i, value in enumerate(self.values[start:start+16]))
            result.extend(struct.pack("<H", word))
        return bytes(result)


def hand_header(bits, block_type, size, order=15):
    bits.push(block_type, 3)
    bits.push(0, 1)
    bits.push(size, 16 if order == 15 else 24)


def hand_lens(bits, lengths):
    for symbol in range(20):
        bits.push(int(symbol in [0, 16]), 4)
    for length in lengths:
        assert length in [0, 1]
        bits.push(length, 1)


def hand_uncompressed(raw, offsets=(1, 1, 1), order=15):
    bits = Bits()
    hand_header(bits, 3, len(raw), order)
    return bits.bytes() + struct.pack("<III", *offsets) + raw + (b"\0" if len(raw) % 2 else b"")


for run_symbol in [17, 18, 19]:
    bits = Bits()
    hand_header(bits, 1, 1)
    run = {17: 19, 18: 51, 19: 5}[run_symbol]
    for count in [256, 240, 249]:
        for symbol in range(20):
            bits.push(int(symbol in [0, run_symbol]), 4)
        for _ in range((count + run - 1) // run):
            bits.push(1, 1)
            if run_symbol == 17:
                bits.push(15, 4)
            elif run_symbol == 18:
                bits.push(31, 5)
            else:
                bits.push(1, 1)
                bits.push(0, 1)
    status, raw = oracle(f"hand-code-length-overrun-symbol{run_symbol}", bits.bytes(), 1, 32768)
    assert status == 0 and raw == b"\0"

bits = Bits()
hand_header(bits, 2, 2)
for _ in range(8):
    bits.push(3, 3)
lengths = [0] * 496
lengths[65] = lengths[320] = 1
hand_lens(bits, lengths[:256])
hand_lens(bits, lengths[256:])
hand_lens(bits, [0] * 249)
bits.push(1, 1)
bits.push(0, 3)
status, raw = oracle("hand-aligned-low-three-bits", hand_uncompressed(b"ABCDEFGHIJKLMN") + bits.bytes(), 16, 32768)
assert status == 0 and raw == b"ABCDEFGHIJKLMNAB"

bits = Bits()
hand_header(bits, 1, 6)
lengths = [0] * 496
lengths[264] = lengths[272] = 1
hand_lens(bits, lengths[:256])
hand_lens(bits, lengths[256:])
hand_lens(bits, [0] * 249)
bits.push(0b101, 3)
status, raw = oracle("hand-recent-r2-not-real-lru", hand_uncompressed(b"ABCDE", (2, 3, 4)) + bits.bytes(), 11, 32768)
assert status == 0 and raw == b"ABCDEBCEBEB"

bits = Bits()
hand_header(bits, 1, 15)
lengths = [0] * 496
lengths[65] = lengths[66] = 1
hand_lens(bits, lengths[:256])
hand_lens(bits, lengths[256:])
hand_lens(bits, [0] * 249)
bits.push(0, 15)
hand_header(bits, 3, 1)
assert len(bits.values) % 16 == 0
input_block = bits.bytes() + b"\xff\xff" + struct.pack("<III", 1, 1, 1) + b"B\0"
status, raw = oracle("hand-aligned-uncompressed-extra-word", input_block, 16, 32768)
assert status == 0 and raw == b"A" * 15 + b"B"

bits = Bits()
hand_header(bits, 1, 1)
lengths = [0] * 496
lengths[65] = lengths[66] = 1
hand_lens(bits, lengths[:256])
hand_lens(bits, lengths[256:])
hand_lens(bits, [0] * 249)
bits.push(0, 1)
hand_header(bits, 1, 1)
hand_lens(bits, [0] * 256)
hand_lens(bits, [0] * 240)
hand_lens(bits, [0] * 249)
bits.push(1, 1)
status, raw = oracle("hand-cross-block-code-length-delta", bits.bytes(), 2, 32768)
assert status == 0 and raw == b"AB"

bits = Bits()
hand_header(bits, 1, 2, 19)
lengths = [0] * 560
lengths[0] = lengths[544] = 1
hand_lens(bits, lengths[:256])
hand_lens(bits, lengths[256:])
hand_lens(bits, [0] * 249)
bits.push(1, 1)
bits.push(0, 17)
status, raw = oracle("hand-seventeen-bit-offset", hand_uncompressed(b"\0" * 262142, order=19) + bits.bytes(), 262144, 1 << 19)
assert status == 0 and raw == b"\0" * 262144

for decoder in decoders.values():
    lib.wimlib_free_decompressor(decoder)

# LZX1 records: name, input_size, output_size, maximum, success, expected_kind,
# expected_storage_size, input, expectation. kind1 repeats expectation modulo
# its length to produce the requested output; kind0 contains exact bytes.
with (HERE / "lzx-oracle.bin").open("wb") as stream:
    stream.write(b"LZX1" + struct.pack("<I", len(cases)))
    for name, data, size, maximum, success, output in cases:
        name = name.encode()
        kind, expectation = encode_expectation(output) if success else (0, b"")
        stream.write(struct.pack("<H", len(name)) + name)
        stream.write(struct.pack("<IIIBBI", len(data), size, maximum, success, kind, len(expectation)))
        stream.write(data + expectation)
print(f"{len(cases)} cases, {sum(case[4] for case in cases)} successful, {sum(not case[4] for case in cases)} rejected")

metadata = {
    "upstream_revision": "cd5e231c348c255ae5088873b5a66ee0eb96fa07",
    "codec": "LZX (2)",
    "window_orders": list(range(15, 22)),
    "levels": [1, 50],
    "seed": "0x4C5A584E4154495645",
    "case_count": len(cases),
    "successful": sum(case[4] for case in cases),
    "rejected": sum(not case[4] for case in cases),
    "encoder_first_block_types": dict(Counter((data[1] >> 5) & 7 for _, data, _, _ in valid)),
    "fixture_sha256": hashlib.sha256((HERE / "lzx-oracle.bin").read_bytes()).hexdigest(),
    "corpus_sha256": hashlib.sha256((HERE / "lzx-libfuzzer").read_bytes()).hexdigest(),
}
(HERE / "lzx-oracle-metadata.json").write_text(json.dumps(metadata, indent=2) + "\n")
