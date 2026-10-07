# ms-compress fuzzing

This standalone package owns the codec fuzz harnesses, honggfuzz runners,
deterministic seed generator, replay tool, and harness regression tests.
Fuzz dependencies stay outside production builds. The sibling windows-uup repository's archive
fuzz package re-exports these harnesses to preserve existing Task commands.

| Runner | Input layout and checks |
| --- | --- |
| `decompress` | Selector modulo 13, LE u16 output capacity modulo 32769, encoded bytes. Selectors: WIM XPRESS Huffman, WIM LZX, LZMS, CAB LZX, Quantum, LZNT1, plain XPRESS, LZX Delta, raw DEFLATE, zlib, gzip, .lzma, raw LZMA2. Raw LZMA2 payload starts with a LE u32 dictionary size. |
| `roundtrip` | Selector modulo 12, LE u16 compressed capacity, plaintext up to 32 KiB. Selectors: WIM XPRESS Huffman, WIM LZX, LZMS, LZNT1, plain XPRESS, CAB LZX, Quantum, raw DEFLATE, zlib, gzip, LZMA1, LZMA2. |

DEFLATE roundtrips cover all compression levels and strategies, streaming,
reset, and unequal consecutive inputs.

LZMA decode workspace is capped at 1024 KiB. Threaded LZMA2 coverage uses two
workers and caps work-unit input and decoded output at 64 KiB. All presets
roundtrip with bounded one-shot decoders; LZMA2 also uses the threaded reader.

Malformed input must return an error without panicking. WIM contexts must reset
after failed decodes. Successful encodings must decode identically; CAB encoders
and decoders are reused across unequal consecutive frames. LZX Delta reserves
the first up-to-256 payload bytes as reference history and uses window order 17.
Encoded inputs are capped at 1 MiB and decoded output at 32 KiB. These bounds
leave 32-bit XPRESS extended-length coverage to the crate's larger host tests.
Roundtrip checks are a consistency oracle; independent fixtures and native
Windows interoperability tests supply separate compatibility checks.

From the repository root, using the development shell:

```sh
nix develop
cargo test --manifest-path fuzz/Cargo.toml --locked
cargo run --manifest-path fuzz/Cargo.toml --locked --bin seed-codecs -- fuzz/corpus/decompress
export CARGO_TARGET_DIR="$PWD/target/ms-compress-honggfuzz"
export HFUZZ_WORKSPACE="$PWD/target/ms-compress-hfuzz-workspace"
cd fuzz
export RUSTC_WRAPPER="" CARGO_INCREMENTAL=0 CC=gcc NIX_HARDENING_ENABLE=""
export HFUZZ_BUILD_ARGS="--locked"
HFUZZ_INPUT=corpus/decompress HFUZZ_RUN_ARGS="-n 1 -t 5 -N 1000 --exit_upon_crash" cargo hfuzz run decompress
HFUZZ_INPUT=corpus/roundtrip HFUZZ_RUN_ARGS="-n 1 -t 5 -N 1000 --exit_upon_crash" cargo hfuzz run roundtrip
```

Install `honggfuzz` version 0.5.62 if `cargo hfuzz` is unavailable. Native
dependencies are supplied by the repository's Nix shell. Remove `-N` for a
long-running campaign. Corpora and crash workspaces are ignored; retain discovered
regressions as descriptive checked-in fixtures and tests.

Replay without instrumentation from the repository root:

```sh
cargo run --manifest-path fuzz/Cargo.toml --locked --bin replay -- decompress path/to/crash.fuzz
```
