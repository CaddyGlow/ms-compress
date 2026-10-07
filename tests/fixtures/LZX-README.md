# Native LZX fixture provenance and validation

Upstream revision: `cd5e231c348c255ae5088873b5a66ee0eb96fa07`.
Codec: WIM LZX, original public compression type 2. Production source is
LGPL-2.1-or-later; see `src/LZX-NOTICE.md`, `src/NOTICE.md` and the license
text. The copied upstream fuzz corpus is GPL-3.0-or-later test material,
with its license text in this directory.

`lzx-libfuzzer` is the unmodified upstream file
`tools/libFuzzer/decompress/corpus/lzx`. The first byte selects LZX and is
removed before passing its payload to the decoder. Original fuzz.c requests
three times the input length while creating a smaller-capacity decoder,
causing its public wrapper to reject before decoding. Our oracle uses adequate
capacity and exercises the payload directly; this is not claimed as coverage
from the uncorrected upstream fuzz harness.

`generate_lzx_oracle.py` loads the original C library with standard-library
ctypes and creates an offline binary snapshot. It uses deterministic Python
random seed `0x4C5A584E4154495645`. Recreate it with:

    python3 generate_lzx_oracle.py /tmp/wimlib-native-oracle/.libs/libwim.so

The snapshot has 3355 records: 637 successful decoded outputs and 2718
rejections. It covers configured window orders 15 through 21, levels 1 and 50,
4096-byte and default 32768-byte blocks, and full configured maximum blocks
up to 2 MiB. Profiles include text, every byte value, long random periods,
and E8 instruction patterns. The original encoder's first blocks include
156 verbatim and four aligned blocks. Multi-block large streams exercise
shared code lengths and repeated offsets beyond the first block.

There are exhaustive byte truncations for eight compact encoded vectors,
900 encoded-block bit mutations, 200 mutated/truncated upstream corpus
vectors, and 200 arbitrary inputs. Eight independent hand-encoded C-checked
records additionally exercise aligned offsets, the mandatory discarded coding
word when an uncompressed header is already aligned, 17-bit offsets, R2's
non-LRU queue behavior, code-length deltas across blocks, and code-length run
symbols 17/18/19 with preserved scratch overruns. These hand records include
uncompressed blocks even though the recorded encoder first-block types do not.

The raw-block API is:

    decompress_lzx(input: &[u8], output: &mut [u8], max_block_size: usize)
        -> Result<(), LzxError>

Capacity is validated before decoding and chooses the window size. This
matches the original configured public decompressor. Output must have the
exact decompressed size. Window sizes beyond 32768 are original-library
extensions and do not establish WIMGAPI compatibility. Cabinet framing,
sliding dictionaries between calls and LZX DELTA extensions are outside the
original WIM decoder's supported scope.

`lzx-oracle.bin` uses ASCII LZX1 then little-endian u32 record count. A record
has: u16 UTF-8 name size; name; u32 input size; u32 requested output size;
u32 configured maximum; u8 success; u8 expectation kind; u32 expectation
storage size; input; expectation bytes. Kind 0 stores exact output. Kind 1
stores an exact repeating byte pattern and reconstructs requested output
by repetition modulo pattern length. Periodic storage changes only fixture
representation; the test still compares every decoded byte. Failure output
contents are unspecified and are not compared. Hashes and counts are recorded
in `lzx-oracle-metadata.json`.

TDD evidence: 22 initial public-contract tests compiled against a function
returning `InvalidBlockType` for all inputs: 21 failed and one passed (actual
invalid block type). This red run used an isolated rustc wrapper because a
concurrently added shared LZMS module initially had no source file. After
native implementation, all 22 passed. Ten additional edge contracts then
passed, and the final 3355-record original-C differential replay passed.

Final checks:

    cargo test \
        --target-dir target -p ms-compress \
        --test lzx --test lzx_differential --locked --offline
    cargo clippy \
        --target-dir target -p ms-compress \
        --all-targets --all-features --locked --offline -- -D warnings

The native module uses safe Rust and fixed stack storage, with no allocation,
FFI or fallback to C. This establishes LZX decompression evidence, not LZX
compression, complete WIM functionality, platform extraction or drop-in ABI
acceptance.
