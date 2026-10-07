# ms-compress

Pure Rust Microsoft compression codecs shared by WIM, CAB, Windows buffer,
and delta consumers, with DEFLATE, zlib, gzip, LZMA1, LZMA2, and Zstandard support. The Microsoft codecs
deny unsafe code; upstream SIMD and allocation code is confined to the vendored
`zlib` and `lzma` modules. Its library uses `core` and `alloc` directly and depends on the patched `ms-compress-ruzstd` crate pinned to version 0.9.1.
Licenses and codec attribution notices are retained alongside the implementation.

Select the API for the encoded format:

| Format | Rust API |
| --- | --- |
| LZNT1 / Windows format 2 | `ms_compress::lznt1::{compress, decompress}` |
| Plain XPRESS / Windows format 3 | `ms_compress::xpress_plain::{compress, decompress}` |
| WIM XPRESS Huffman | `ms_compress::decompress_xpress`, `ms_compress::xpress_encode::compress_xpress` |
| WIM LZX | `ms_compress::lzx::decompress_lzx`, `ms_compress::lzx_encode::compress_lzx` |
| CAB LZX | `ms_compress::lzx::CabinetLzxDecoder`, `ms_compress::lzx_encode::CabinetLzxEncoder` |
| LZX Delta | `ms_compress::lzx::{decompress_lzxd, LzxDeltaDecoder}` |
| LZMS | `ms_compress::lzms::decompress_lzms`, `ms_compress::lzms::encode::compress_lzms` |
| DEFLATE / zlib / gzip | `ms_compress::zlib::{Deflate, Inflate}` |
| LZMA1 (.lzma) | `ms_compress::lzma::{compress, decompress}` |
| Raw LZMA2 | `ms_compress::lzma::{compress_lzma2, decompress_lzma2}` |
| Zstandard | `ms_compress::zstd::{decoding, encoding}` |
| CRC32 / Adler32 | `ms_compress::zlib::{crc32, adler32}` |
| CAB Quantum | `ms_compress::quantum::{QuantumDecoder, QuantumEncoder}` |

The `context` module retains the WIM codec selection and reusable context APIs.
Windows compression format numbers and WIM codec identifiers are separate
namespaces; choose the API matching the buffer's format.

From the repository root:

```sh
cargo test -p ms-compress --locked
cargo clippy -p ms-compress --all-targets --all-features --locked -- -D warnings
```

For Windows buffer validation details, see
[the codec validation document](../wim-rs/docs/wimlib/nt-buffer-codecs.md).

See [VALIDATION.md](VALIDATION.md) for test and benchmark coverage and
[fuzz/README.md](fuzz/README.md) for the crate-owned fuzz runners and replay.

## `no_std` support

Disable default features to use every codec with `core` and `alloc`:

```toml
ms-compress = { path = "path/to/ms-compress", default-features = false }
```

A global allocator is required for heap-backed APIs, including returned `Vec`s
and `Box`-owned workspaces. Microsoft codec contexts allocate their retained `Vec` storage
fallibly at construction and reuse its capacity when processing blocks.
This configuration is not heap-free. The default `std` feature preserves normal
host builds, and `cli` enables `std` automatically.

Zstandard also supports `no_std + alloc`, including dictionaries and checksums,
with no external hashing dependency. Its checked decoder storage forbids unsafe
code. Use `zstd::io` for the selected std or no_std I/O traits. Retained-output
decoding must not be mixed with destructive reads within one frame; see
[ms-compress-ruzstd](https://crates.io/crates/ms-compress-ruzstd/0.9.1)
for the implementation and [historical provenance](docs/zstd-vendoring/PROVENANCE.md).

The library has no dependencies on `wim-memory` or a separate allocation crate.
Custom per-context allocator callbacks and `with_allocator` constructors have
been removed; use the ordinary `new` constructors. WIM's registered C allocators
no longer control codec scratch storage.

```sh
cargo test -p ms-compress --no-default-features --locked
cargo check -p ms-compress --lib --no-default-features --target thumbv7em-none-eabi --locked
```

The second command requires `rustup target add thumbv7em-none-eabi`.

## Optional CLI

```sh
cargo build -p ms-compress --features cli --release --locked
target/release/ms-compress rtlcompress --format lznt1 -i input.bin -o input.lznt1
target/release/ms-compress rtldecompress --format lznt1 -i input.lznt1 -o restored.bin --output-size 12345
cat input.bin | target/release/ms-compress rtlcompress --format xpress > input.xpress
cat input.xpress | target/release/ms-compress rtldecompress --format xpress --output-size 12345 > restored.bin
```

Replace `12345` with the exact original byte count. `--format` accepts `lznt1`
(default), `xpress` (plain), and `xpress-huffman` (single WIM block, 25–65536
input bytes for compression). These commands use the Rust codecs and produce
raw buffers without a size header. They do not call the Windows RTL functions.
WIM Huffman decoding retains its existing permissive padding behavior.

Input/output default to `-` for stdin/stdout. The CLI buffers the complete input
and output, with `--max-input` and `--max-output` defaulting to 256 MiB each.
Compression checks the worst-case encoded size against the output limit before
encoding. Decompression requires `--output-size` and checks it against the limit.
File output is published only after successful conversion; use `--force` to
replace an existing file. Binary stdout contains only the result, with errors
written to stderr. Shell redirection opens its destination before the CLI runs;
use `-o` when you need the CLI's file-preservation behavior.

## CPU acceleration

The XPRESS, LZNT1, LZX, LZMS, and Quantum encoders share the vendored SIMD
match-prefix routine. Standard builds detect AVX2 or NEON at runtime; scalar
fallbacks handle unsupported CPUs and short matches. `no_std` builds use only
CPU features enabled at compile time (for example, `-C target-cpu=native`).

`vpclmulqdq` enables the AVX-512 CRC32 implementation, and `avx512` also enables
AVX-512 Adler32 and match comparison. These features require Rust 1.89 or later
and appropriate target features; enabling a Cargo feature alone does not force
unsupported instructions. `lsx` enables LoongArch LSX support and requires
nightly Rust on LoongArch because its intrinsics remain unstable.

For a build optimized for the machine that runs Cargo, enable the compiler's
host CPU selection:

```sh
RUSTFLAGS="-C target-cpu=native" cargo build -p ms-compress --release --locked
RUSTFLAGS="-C target-cpu=native" cargo bench -p ms-compress --bench codecs --locked
```

This enables the host's supported instructions for compiler-generated code as
well as the codec's static dispatch. The resulting binaries require a compatible
CPU. For a known x86 deployment baseline, individual instructions can be selected:

```sh
RUSTFLAGS="-C target-feature=+avx2,+bmi1,+bmi2,+pclmulqdq,+sse4.1,+sse4.2" cargo build -p ms-compress --release --locked
```

These compiler flags also work with `--no-default-features`. Standard portable
builds already detect these codec instructions at runtime; extra Cargo features
are unnecessary for AVX2, BMI, SSE, and PCLMUL. AVX-VNNI, AES, and SHA do not have
additional dedicated codec implementations in this module.

Compiler flag reference: [rustc CPU and target features](https://doc.rust-lang.org/rustc/codegen-options/index.html#target-cpu).

See [zlib provenance](src/zlib/NOTICE.md) for the upstream revision and changes.

## LZMA and parallel decoding

The vendored `lzma` module provides LZMA1 and raw LZMA2 readers, writers, and
incremental decoders. Slice helpers accept presets 0–9, cap their encoder
dictionary at the input size (minimum 4 KiB), and require an explicit decoder
workspace limit in KiB. Output cannot grow beyond the supplied slice. Raw LZMA2
encoding returns its dictionary size separately; it has no dictionary header.
XZ and 7z containers are not included in this module.

```rust
let input = b"Hello, world!";
let encoded = ms_compress::lzma::compress(input, 6).unwrap();
let mut output = [0; 13];
let size = ms_compress::lzma::decompress(&encoded, &mut output, 1024).unwrap();
assert_eq!(&output[..size], input);
```

With `std`, `Lzma2ReaderMt` decodes independent LZMA2 blocks using multiple workers
and returns their output in stream order. `new_bounded` accepts worker count,
per-worker workspace limit, and per-block compressed/decoded byte limits. These
limits do not impose a total memory budget on queued and reordered results.
Workers are joined when the reader is dropped. Single LZMA1 streams and LZMA2
blocks that preserve previous dictionary history remain sequential. Separate
LZMA1 streams can be processed concurrently by caller-managed threads.

`Lzma2Writer` can produce independent blocks with `Lzma2Options::set_chunk_size`;
flush at the desired boundaries when feeding smaller chunks. Plain LZMA2 packet
boundaries do not imply independence: dictionary-reset boundaries determine
whether the decoder can distribute work.

```rust
# #[cfg(feature = "std")]
# {
use ms_compress::lzma::{Lzma2ReaderMt, Read};
let (encoded, dictionary) = ms_compress::lzma::compress_lzma2(b"hello", 1).unwrap();
let mut reader = Lzma2ReaderMt::new_bounded(&encoded[..], dictionary, 4, 1024, 65536).unwrap();
let mut output = Vec::new();
reader.read_to_end(&mut output).unwrap();
assert_eq!(output, b"hello");
# }
```

`lzma-optimization` is enabled by default. It enables upstream optimized range
coding and match-finder paths, with runtime CPU detection under `std` and static
CPU selection under `no_std`. Disable default features for the safe Rust LZMA
implementation, or explicitly enable `lzma-optimization` for optimized `no_std`.
All paths use Rust global allocation; low-level writer options control workspace
size and can require much more memory than the bounded slice helpers.

See [LZMA provenance](src/lzma/NOTICE.md) and [validation](VALIDATION.md).

## Standalone repository

Build and test directly from this directory; no sibling checkout is required.
The CLI is optional (`cargo run --features cli -- --help`). `windows-uup` and
`wim-rs` consume this package through `../ms-compress` in the sibling checkout
layout. Historical evidence in wim-rs remains there and is not a build input.
All codec licensing and attribution files remain alongside their implementations.

## Releases

Push a tag matching the package version in `Cargo.toml`, such as `v0.1.0`, to
publish a release. The GitHub Actions workflow runs the CI gates, verifies the
tag/version match and crate packaging, and builds the optional CLI for Linux
x86-64, Windows x86-64, and macOS ARM64. It then publishes the Rust package to
crates.io and creates a GitHub Release with binary tarballs, license texts, and
SHA-256 checksums. Tags with a prerelease suffix create GitHub prereleases.

Configure a repository secret named `CARGO_REGISTRY_TOKEN` with a crates.io token
authorized to publish `ms-compress` before pushing the first release tag.

Release binaries embed the Git tag at build time and report it with `--version`.
Local builds use the Cargo package version unless `MS_COMPRESS_BUILD_TAG` is set
when compiling.
