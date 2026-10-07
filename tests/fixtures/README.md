# XPRESS baseline and TDD evidence

Upstream revision: cd5e231c348c255ae5088873b5a66ee0eb96fa07.
The crate source is LGPL-2.1-or-later, with source notices in src/NOTICE.md.
The copied upstream libFuzzer corpus is project test material covered by the
upstream GPL-3.0-or-later option; the license text is supplied here.

`xpress-libfuzzer` is an exact copy of tools/libFuzzer/decompress/corpus/xpress.
Its first byte selects XPRESS; it must be removed before block decoding.
Upstream fuzz.c creates a decoder with max input_size-1 but requests output
of 3*input_size. The public C wrapper returns -2 before decoding. Our oracle
uses maximum block size 65536, so these cases actually exercise XPRESS.

`generate_xpress_oracle.py` uses Python standard-library ctypes to load the
original C oracle and create `xpress-oracle.wxp`. Test runtime needs neither
Python nor C nor access to /tmp. The file contains 4150 differential records:
1105 successful outputs and 3045 rejected inputs. Coverage includes upstream
compressed blocks at levels 1/50/100 and sizes 256/512/4096/32768/65536;
repeated zeroes, text, all byte values and random binary periods; exhaustive
truncations of eight representative blocks; 800 bit mutations; 200 truncated
and mutated libFuzzer blocks; 200 random/complete canonical table cases.

Regenerate against the pinned isolated build with:

    python3 generate_xpress_oracle.py /tmp/wimlib-native-oracle/.libs/libwim.so

Binary record format: ASCII WXP1; little-endian u32 case count; each case:
u16 UTF-8 name length; name; u32 input size; u32 requested output size; u8
success; input; exact output bytes only when success is 1. Failed output
contents are unspecified by both APIs and therefore not compared.

TDD evidence from this session:

    cargo test -p ms-compress --offline

Before implementation, the public function returned HeaderTooShort for all
inputs: 20 hand-encoded tests compiled, 19 failed and 1 passed (the actual
short-header rejection). After the native decoder was written, 20/20 passed.
The independent differential test then compared all 4150 C records and passed.

The raw decode function has no configured maximum-block-size object. The
65539-byte hand-encoded maximal match test exercises the format's maximum
u16 match extension plus its preceding literal. C's public configured decoder
rejects such a request because its configured maximum is 65536; future C ABI
wrapper tests must retain that separate capacity rejection. Do not infer API
wrapper parity from this raw codec test.

These tests establish XPRESS decompression progress only. They do not prove
compression, other codecs, WIM parsing/writing, extraction, platform metadata,
servicing or complete drop-in ABI compatibility.
