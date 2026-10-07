# Codec validation

## Zstandard fork dependency (2026-10-07)

Zstandard now uses CaddyGlow/zstd-rs commit
`468f57ac3de6779ab17191f8bc2fd2bebc0b5439` through a pinned Git dependency.
The `ms_compress::zstd` API re-exports this dependency, and the crate's `std`
feature forwards to `ruzstd/std`. The fork retains safe decoder storage,
internal XXH64, and retained-output support; fixtures and local integration
tests remain. Former vendoring license, attribution, and source hashes are
preserved in `docs/zstd-vendoring`. Earlier statements below about having no
library dependencies describe the configuration at the time of those checks.

Local Linux validation passed after the dependency switch: all-feature tests,
strict all-target/all-feature Clippy, no-default-feature tests, embedded library
compilation for thumbv7em-none-eabi, and the fuzz regression test suite, all with
locked dependencies. Native Windows interoperability was not run for this switch.


The crate has format-specific regression tests for XPRESS Huffman, LZX, CAB LZX,
LZX Delta, LZMS, Quantum, LZNT1, and plain XPRESS, plus context-reuse and allocator
failure tests. Checked-in independent fixtures exercise XPRESS/LZX/LZMS decoding,
and published MS-XCA vectors exercise LZNT1/plain XPRESS. The LZX encoder has a
separate external-oracle harness. Host tests do not prove every output is accepted
by native Windows or independent implementations.

```sh
cargo test -p ms-compress --all-features --locked
cargo clippy -p ms-compress --all-targets --all-features --locked -- -D warnings
cargo fmt -p ms-compress -- --check
```

The ignored `nt_buffers_windows` test checks LZNT1/plain XPRESS against ntdll in
both directions. The codec CI workflow runs it on Windows. Its native execution
has not been verified on the current Linux host. Other external-oracle gates
retain their documented tooling requirements.

## `no_std` and Rust allocation (2026-10-05)

The codec library uses `core` and `alloc` directly, has no library dependencies,
and denies unsafe code outside the vendored zlib module. Microsoft codec
workspaces are ordinary `Vec`s,
allocated fallibly at construction. Capacity checks prevent workspace growth
while processing blocks. Custom per-context allocators are no longer supported.

WIM's C allocator hooks still control outer codec handles and WIM-owned storage,
but codec scratch memory uses Rust's global allocator. Earlier evidence of C hook
coverage for codec scratch is historical and does not describe this behavior.
Factory tests check exactly one C allocation per handle, nonzero Rust workspace
allocations, recoverable failure at every allocation, and allocation-free reuse.

Local Linux checks passed: codec tests with and without default features, CLI
tests, workspace Clippy with all targets/features and warnings denied, codec
Clippy without default features, and the bare-metal library build. WIM compression,
compressor/decompressor allocation-failure, XML allocation, and pipable-reader
tests passed, as did codec fuzz-harness tests and archive fuzz compilation.
Formatting and dependency-tree checks passed; the codec library has no normal
dependencies when optional CLI features are disabled.

CI includes codec tests and Clippy with and without default features, plus a
`thumbv7em-none-eabi` library build check. The bare-metal check proves compilation
without `std`; it does not execute codecs on hardware or establish heap-free
operation. Native Windows interoperability was not rerun for this change.

## Benchmarks

```sh
cargo bench -p ms-compress --bench codecs --locked > target/ms-compress-bench.csv
```

The dependency-free release benchmark covers all twelve encoder/decoder formats
and LZX Delta decoding. Workloads are deterministic repetitive, patterned, and
pseudorandom data at 4 KiB and 32 KiB, with three time-bounded samples per
operation. LZMA uses preset 6 and dictionaries capped at input size; DEFLATE
uses default level 6. Raw LZMA2 benchmark buffers include a four-byte dictionary
size prefix for the decoder. Every workload is decoded and checked before timing; CSV records
compressed size, iterations, elapsed time, and throughput in MiB/s.
`MS_COMPRESS_BENCH_MS` sets the minimum time per sample (default 100 ms).
CI uses 10 ms to smoke-test the runner, not to enforce performance thresholds.

Measurements include context construction and codec output allocation, but
exclude input generation and output-buffer allocation for decoding. They measure
end-to-end one-shot API costs, not steady-state reusable context throughput.
Compare runs on the same host, Rust toolchain, CPU placement, and power settings;
retain CSV and environment information for performance claims. No statistical
regression threshold or independently calibrated speed claim is established.

## Fuzzing

[`fuzz/`](fuzz/README.md) owns bounded mutation and roundtrip harnesses, valid
seeds, replay, and tests. Every format has direct decoder coverage; every encoder
has roundtrip coverage. LZX Delta has no encoder in this crate.
Fuzz harness tests and 1,000-iteration instrumented smoke campaigns are wired
into codec CI. Short campaigns verify harness operation;
they do not establish absence of bugs or replace longer sanitizer campaigns.

## Local verification (2026-10-04)

On Linux, all 148 codec tests and both crate-owned fuzz harness regression tests
passed. The archive fuzz package's compatibility tests also passed. Clippy with
warnings denied and formatting checks passed for the codec and both fuzz
packages. The release benchmark completed all 255 measurement rows (10 ms per
sample); this establishes runner operation, not stable performance estimates.
Crate-local instrumented honggfuzz campaigns each completed 1,001 iterations
with zero crashes and zero timeouts for `decompress` and `roundtrip`.
The standalone decoder corpus also replayed without panics. These campaigns did
not use sanitizers. CI configuration is added; remote CI execution and native
Windows interoperability remain unverified in this session.

## Optional CLI validation

With `--features cli`, integration tests exercise stdin/stdout and file roundtrips
for LZNT1, plain XPRESS, and single-block WIM XPRESS Huffman. They also check
empty NT buffers, malformed input, declared-size mismatches, input/output limits,
short Huffman input rejection, existing-file protection, forced replacement, and
preservation of destinations after conversion failure. Codec CI enables all
features so these tests run on both Linux and Windows.

## Vendored DEFLATE and shared CPU matching (2026-10-05)

The zlib module retains upstream checksum, streaming, dictionary, match, and
allocation unit tests and fixtures; C allocator tests are removed with that
backend. Public integration tests cover the original Hello World zlib vector,
raw DEFLATE, an independently generated gzip fixture, truncation, corrupt
checksums, output limits, streaming reset, and preset dictionaries.

XPRESS, LZNT1, LZX, LZMS, and Quantum encoders share a bounded prefix comparator
with upstream CPU dispatch. A regression compares every mismatch position and
unaligned slice boundary against scalar results. Arithmetic LZMS delta matches
retain their specialized comparisons. Cargo flags do not imply AVX-512 hardware
availability; no AVX-512 or LoongArch LSX execution is established locally.

The standalone fuzz harness includes malformed and streaming raw DEFLATE, zlib,
and gzip decoding, plus all ten compression levels and five strategies for each
wrapper. The imported code is pinned and documented in
[src/zlib/NOTICE.md](src/zlib/NOTICE.md).

Local checks passed: 237 codec tests with all features and 234 without default
features (including doctests); 80 zlib tests compiled for the host CPU; all three
standalone fuzz regression tests, 44 CAB tests, and seven archive fuzz tests.
Strict workspace and standalone fuzz Clippy, no_std Clippy, formatting, and a
thumbv7em-none-eabi library check with avx512/lsx flags passed. Both instrumented
fuzz campaigns completed 1,001 iterations with zero crashes or timeouts, and
both seed corpora replayed successfully. The release benchmark completed its
255 rows at 10 ms per sample. These results do not establish a speedup or native
Windows compatibility for the new implementation.

### Explicit compiler CPU selection

A full `RUSTFLAGS="-C target-cpu=native"` codec test run passed 241 tests on the
i7-13700K host. A native release library build and an explicit AVX2/BMI1/BMI2/
PCLMUL/SSE4.1/SSE4.2 no_std check also passed. Commands are documented in README.md.

An exploratory comparison against the normal runtime-dispatched release build
used three 100 ms samples per cell, CPU affinity 0, and the same workloads/profile
as the codec benchmark, extended with DEFLATE wrappers and checksums. Native
compiler flags gave mixed results: geometric mean CAB LZX decode 1.13x, XPRESS
Huffman decode 1.09x, and Adler32 0.93x; most other ratios were near 1. Both builds
passed all codec workload roundtrips and agreed on compressed sizes. The shared
host and short samples limit these observations; native CPU selection is not
a demonstrated improvement for all codecs. CPU-specific builds require a
compatible deployment CPU and are not the default for distributed binaries.

Crate-scoped strict Clippy passed. That compiler-flag check encountered
unrelated `collapsible_if` diagnostics in wsus-client `install/executor.rs:717`
and `install/servicing/os_installer.rs:202`. The subsequent LZMA integration
validation below passed workspace-wide strict Clippy.

## LZMA1, LZMA2, and parallel decoding (2026-10-05)

The Apache-2.0 lzma-rust2 core is vendored at a pinned revision, with its license
and adaptation notice in [src/lzma/NOTICE.md](src/lzma/NOTICE.md). The library
remains dependency-free with CLI disabled. Both codecs work with no_std and alloc.
Optimized LZMA internals are enabled by default via `lzma-optimization`; disabling
default features also selects its safe Rust implementation. Static CPU detection
supports optimized no_std builds, and std builds use runtime detection.

One-shot helpers bound output to caller slices, cap encoder dictionaries at input
size (minimum 4 KiB), and require decoder workspace limits. Low-level streaming
APIs retain explicit dictionaries and encoder options. XZ/LZIP containers and
multithreaded encoding are not part of this integration.

The std-only LZMA2 worker decoder distributes independent dictionary-reset blocks
and returns results in order. LZMA1 and dependent LZMA2 blocks remain sequential.
The bounded constructor limits workspace per worker and compressed/decoded bytes
per work unit; queued and reordered results also contribute to memory use, so
these are not a total memory bound. EOF without an LZMA2 end marker is rejected,
result sends observe shutdown, and Drop joins the workers.

Tests retain upstream core, allocation-estimate, BCJ/delta, and work-queue cases.
Independent Python/liblzma fixtures decode correctly, and Python/liblzma accepts
both Rust encoder formats. Tests cover every preset, empty/repeated/varied input,
truncation at every fixture boundary, output/workspace limits, raw dictionaries,
and byte-at-a-time streaming with trailing-data recovery. Parallel tests check
64 independent blocks with 1/2/4 workers, output order, empty streams, malformed
input, bounds, and early drop. CPU normalization matches scalar results across
unaligned tails and signed values, including in a native-CPU test build.

Fuzz selectors include malformed .lzma and raw LZMA2, incremental decoders, and
a bounded two-worker LZMA2 reader. Preset roundtrips and independent-block seeds
exercise both normal and threaded paths. Benchmark workloads now include LZMA1,
raw LZMA2, raw DEFLATE, zlib, and gzip. The benchmark validates outputs before timing;
its short smoke run is not a threading speedup measurement.

Local validation passed: 270 all-feature codec tests and 250 tests without
default features, counting the current eight doctests; optimized no_std LZMA
API and unit tests; 20 LZMA unit tests with native CPU compiler flags; all five
standalone fuzz regressions and seven archive fuzz tests. Strict workspace
Clippy, no_std Clippy, standalone fuzz Clippy, formatting, dependency-tree, and
thumbv7em-none-eabi checks passed. Python/liblzma interoperability passed.
Both final instrumented campaigns completed 1,001 iterations with zero crashes
and timeouts, and the generated corpora replayed successfully. The extended
release benchmark completed 435 measurement rows at 10 ms per sample. Native
Windows execution and remote CI were not performed in this session.

## Standalone extraction (2026-10-06)

The package was validated outside the windows-uup workspace before installation
as the sibling ms-compress repository. All-feature and no-default-feature host
tests and Clippy passed, as did the standalone fuzz regression tests, formatting,
workflow linting, and the thumbv7em-none-eabi no_std library check. The Quantum
fixture is now retained locally with its provenance and LGPL license, so the
codec test suite no longer reads CAB fixtures from another checkout. Native
Windows interoperability and sustained fuzz campaigns were not rerun.
