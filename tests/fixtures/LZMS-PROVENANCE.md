# LZMS offline differential oracle

`lzms-oracle.wlm` contains 1,938 original C decoder observations: 721
successful decodes with exact output bytes and 1,217 rejected blocks. Rust
runtime tests load this fixture and never load or invoke the original library.

The bounded generator `generate_lzms_oracle.py` uses a deterministic
`random.Random(0x4c5a4d53)` seed, compression levels 1/35/60, and output sizes
32 through 65,536 bytes. Cases cover zeros, phrases, all byte values, repeated
binary data, increasing byte and 32-bit integer delta patterns, pseudorandom
literals, and repeated x86 relative addresses. Additional cases include every
byte cutoff of 16 small blocks, 600 bit mutations, 150 sampled cuts, 100 random
word streams, boundary input lengths, and the shipped libFuzzer LZMS corpus
with 200 seeded truncations/mutations. Both successful corrupted streams and
rejected streams are retained; mutation does not imply an expected rejection.

Generate with:

```sh
python3 tests/fixtures/generate_lzms_oracle.py /tmp/wimlib-native-oracle/.libs/libwim.so
```

The oracle decoder's maximum block size is 65,536, covering every requested
output size. The upstream libFuzzer harness instead creates its decoder with
compressed input size while requesting output of three times that size; its
rejection does not establish codec behavior. `lzms-libfuzzer` preserves that
upstream corpus byte-for-byte, including the first selector byte (3), which
must be removed before decoding.

Output allocation includes 64 guard bytes for the original C optimized x86
filter's SIMD tail reads. The native Rust decoder implements the scalar filter
with slice bounds and needs no guard padding. C observations for output sizes
0 and 1 are excluded because that implementation crashes constructing the
zero-symbol offset alphabet before its decode loop. Rust has separate tests
for those boundary sizes. No C failure or crash is presented as a valid oracle
answer.

The fixture binary uses magic `WLM1`, a little-endian u32 count, then records:
u16 UTF-8 name length; name bytes; u32 compressed length; u32 output length;
u8 success flag; compressed bytes; and exact output bytes only for success.
The reader checks the record count and rejects trailing unparsed fixture data.

Source version: wimlib 1.14.5, commit
`cd5e231c348c255ae5088873b5a66ee0eb96fa07`.
The native translation preserves Eric Biggers' copyright notice and uses
`LGPL-2.1-or-later`, matching the translated implementation. The upstream
libFuzzer corpus is distributed with the original tree under GPL-3.0-or-later;
`LICENSE-GPL-3.0` is present alongside it.

Source hashes (SHA-256):

- `src/lzms_decompress.c`: `c2b521c787101cff8102341c1407517e8ac83401b5c2bd8050068fe9b754c00e`
- `src/lzms_common.c`: `4c33aad108d2ec5f50323a3a5c82028ff339476081e7c098ed2ee2b1f21c7a24`
- `src/compress_common.c`: `a588cccd434c0b7c3849601502b45c6b0a5034315b6e08809fb3272cde51ddae`
- `include/wimlib/lzms_constants.h`: `2cf96fe8040f80f75ca61c65294861aafd38cf40f18ef8082c8d6b59fdd74ed6`
- Original oracle `libwim.so`: `62eb9c97f9ec6e3dae6c7e995b0a65a2e94a4d9b41491666947ba788a15bb74a`

These are raw-codec gates. They do not establish native encoder parity,
Microsoft interoperability, archive roundtrips, Windows servicing correctness,
or complete drop-in replacement readiness.

## Native TDD and branch evidence

Before the decoder existed, the public-contract tests failed to compile because
`lzms.rs` and its API were missing. Six focused delta-copy assertions then
failed to compile against the absent `copy_delta` helper, which was extracted
from the decoder and implemented with checked source/output bounds.

Current native gates passed:

```sh
cargo test -p ms-compress --lib lzms::tests
cargo test -p ms-compress --test lzms --test lzms_differential
cargo clippy -p ms-compress --lib --test lzms --test lzms_differential --all-features --locked -- -D warnings
python3 tests/fixtures/audit_lzms_branches.py
```

There are eight private tests and four public API tests, plus the differential
test covering all 1,938 records. The branch audit compiles an instrumented
temporary Rust source copy and checks the same statuses and bytes; it changes
no production files and invokes no C library. Its thirteen nonzero counters
establish that the corpus actually exercises each LZ/delta match encoding,
rebuilds, x86 translation, and both probability clamps:

| Native branch | Count |
|---|---:|
| Literal | 1,400,268 |
| Explicit LZ | 91,200 |
| Repeat LZ 0 / 1 / 2 | 63,895 / 3,429 / 2,414 |
| Explicit delta | 311 |
| Repeat delta 0 / 1 / 2 | 1,238 / 47 / 81 |
| Adaptive Huffman rebuild | 1,616 |
| x86 address translation | 43,108 |
| Probability numerator 0 / 64 | 75,684 / 1,371,026 |

The Huffman builder preserves the upstream leaf-first tie ordering and assigns
lengths in decreasing order to symbols sorted by increasing frequency then
symbol. It also retains the exact upstream bounded-length count adjustment,
rather than rejecting an unbounded tree deeper than fifteen bits. The separate
`lzms_huffman_oracle.c` generates a deliberately deep Fibonacci-frequency tree;
its C output is asserted in a native test. Regenerate that observation with:

```sh
cc tests/fixtures/lzms_huffman_oracle.c /tmp/wimlib-native-oracle/.libs/libwim.a -o /tmp/lzms-huffman-oracle
/tmp/lzms-huffman-oracle
```

Expected code lengths are eight occurrences of 15 followed by
`12, 11, 10, 9, 8, 7, 6, 5, 4, 3, 2, 1`.

Decoder-owned vectors and the Huffman priority queue allocate through
`try_reserve_exact`; reservation failures return `LzmsError::OutOfMemory`.
The x86 filter propagates this error as well. A capacity-overflow regression
verifies that the error remains recoverable. This does not yet establish
custom wimlib allocator hooks, or an injected real-OOM allocation-failure
matrix; those belong to later ABI/context implementation gates.

Fixture SHA-256: `ad818e8f6b49fbb319bdd456419a05f4adb7b8aa62424a0696cb247b21708f5e`.
Original copied corpus SHA-256: `c8fabe93ca9d9d0c63379ce6f0802beb4380c02064bf312f1d02473e9bf7fe89`.
