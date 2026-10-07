# Native LZX encoding evidence

The independent decoder oracle is the original wimlib source commit
`cd5e231c348c255ae5088873b5a66ee0eb96fa07`, built at
`/tmp/wimlib-native-oracle/.libs/libwim.so`. No production encoder code calls or
links that library. `lzx-encode-differential.py` invokes the Rust example and
passes its output to original `wimlib_decompress()`, comparing all output bytes.
The 64 deterministic cases cover tiny inputs, default/nondefault framing,
16/24-bit block sizes, 32 KiB through 2 MiB windows, random bytes, repeating
bytes, byte cycles, and E8 relative targets at boundary values. Results reside
in `docs/wimlib/evidence/native-lzx-encoding/differential.json`.

Six Rust contract tests cover native decoding, literal byte coverage,
repetition compression, E8 input preservation, exact output capacity, empty
input, invalid capacities, and far offsets. The initial six-test run passed;
this batch does not claim a recorded red-phase log. Existing native LZX decoder
fixtures and original-decoder observations are separate evidence.

The implementation uses a verbatim compressed block with greedy three-byte
hash matching (maximum match length 257) and frequency-ranked balanced complete
canonical Huffman codes. It serializes code lengths with the LZX precode and
performs the WIM E8 transformation on a copied buffer. It emits genuine matches
and Huffman-coded literals; repetitive 32 KiB input compresses below 2 KiB.

This evidence establishes interoperability for the tested chunk encoder, not
full compressor API parity. Compression level/context APIs, original allocation
size reporting, lazy/optimal parsing, aligned blocks, destructive mode,
original compression ratios, and exact encoded-byte identity remain pending.
Header length run coding is currently emitted as individual delta symbols.
All 64 cases are deterministic but do not establish exhaustive input coverage.

Format references: original `src/lzx_compress.c`, `src/lzx_common.c`, and
`include/wimlib/lzx_constants.h`; LGPL-2.1-or-later notice retained in
`src/LZX-NOTICE.md` and source-file SPDX header. Match selection and balanced
canonical-code generation are original Rust implementations.
