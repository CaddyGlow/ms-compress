#!/usr/bin/env python3
"""Decode native LZX encoder output using the independently built original libwim."""
import ctypes
import argparse
import hashlib
import json
import pathlib
import random
import subprocess
import tempfile

ROOT = pathlib.Path(__file__).resolve().parents[1]
parser = argparse.ArgumentParser(description=__doc__)
parser.add_argument('--encoder', type=pathlib.Path, default=ROOT / 'target/debug/examples/encode_lzx')
parser.add_argument('--output', type=pathlib.Path)
args = parser.parse_args()
LIB = pathlib.Path('/tmp/wimlib-native-oracle/.libs/libwim.so')
lib = ctypes.CDLL(str(LIB))
lib.wimlib_create_decompressor.argtypes = [ctypes.c_int, ctypes.c_size_t, ctypes.POINTER(ctypes.c_void_p)]
lib.wimlib_decompress.argtypes = [ctypes.c_void_p, ctypes.c_size_t, ctypes.c_void_p, ctypes.c_size_t, ctypes.c_void_p]
lib.wimlib_free_decompressor.argtypes = [ctypes.c_void_p]
rng = random.Random(20261003)
cases = []
for size in [1, 2, 3, 9, 10, 11, 255, 256, 257, 32767, 32768, 32769, 65535, 65536, 131072, 2097152]:
    for kind in ['repeat', 'cycle', 'random', 'e8']:
        if kind == 'repeat':
            data = b'a' * size
        elif kind == 'cycle':
            data = bytes(i % 256 for i in range(size))
        elif kind == 'random':
            data = rng.randbytes(size)
        else:
            data = bytearray(b'\x90' * size)
            for pos in range(0, max(0, size - 10), 11):
                data[pos] = 0xe8
                relative = rng.choice([0, -pos, 11999999, -pos-1, 12000000])
                data[pos+1:pos+5] = relative.to_bytes(4, 'little', signed=True)
            data = bytes(data)
        cases.append((f'{kind}-{size}', data, max(32768, size)))
with tempfile.TemporaryDirectory(prefix='lzx-encode-') as tmp:
    folder = pathlib.Path(tmp)
    results = []
    for name, data, maximum in cases:
        source = folder / 'input'
        target = folder / 'compressed'
        source.write_bytes(data)
        subprocess.run([str(args.encoder), str(source), str(target), str(maximum)], check=True)
        compressed = target.read_bytes()
        context = ctypes.c_void_p()
        assert lib.wimlib_create_decompressor(2, maximum, ctypes.byref(context)) == 0
        output = ctypes.create_string_buffer(len(data))
        try:
            result = lib.wimlib_decompress(compressed, len(compressed), output, len(data), context)
        finally:
            lib.wimlib_free_decompressor(context)
        assert result == 0 and output.raw == data, (name, result)
        results.append({'case': name, 'input_bytes': len(data), 'encoded_bytes': len(compressed), 'input_sha256': hashlib.sha256(data).hexdigest(), 'decoded_equal': True})
record = {'oracle': str(LIB), 'source_commit': 'cd5e231c348c255ae5088873b5a66ee0eb96fa07', 'encoder_sha256': hashlib.sha256(args.encoder.read_bytes()).hexdigest(), 'cases': results}
if args.output:
    args.output.write_text(json.dumps(record, indent=2) + '\n')
    print(json.dumps({'original_decoder_roundtrips': len(results)}))
else:
    print(json.dumps(record, indent=2))
