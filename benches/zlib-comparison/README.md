# DEFLATE library comparison

Run from the repository root:

```sh
cargo bench --manifest-path benches/zlib-comparison/Cargo.toml --bench zlib --locked > zlib-comparison.csv
```

This separate package keeps native build dependencies out of the library and its
normal test dependency graph. Its lockfile pins flate2 1.1.10 with the explicit
C zlib backend, miniz_oxide 0.9.1, and libdeflater/libdeflate 1.26.1. A C compiler
is required. flate2 defaults are disabled to avoid silently benchmarking its
Rust backend in the native-zlib row.

The benchmark includes ms-compress's normal DEFLATE implementation and the new
classic `raw_level1` encoder. All streams use **raw DEFLATE**, without zlib/gzip
headers or checksums, so decoder comparisons use identical encoded bytes.
Levels 1 and 6 are measured; classic is included only at its supported level 1.
Level numbers represent each library's own tuning, not equal compression ratios.
ms-compress and native zlib use memory level 8, while classic's fixed memory level
is 9. `source_mem_level` records that difference (`na` for libraries without that
setting). Classic uses 32 KiB submissions; the other APIs receive the whole input.

Workloads are deterministic repeated, patterned, and xorshift-random buffers of
4 KiB, 32 KiB, 128 KiB, and 1 MiB. The seed is `0x12345678`; the pattern byte at
index `i` is `(i * 37 + i / 11) mod 256`. These synthetic workloads are useful for
regression comparisons, but are not a representative application corpus.

Compression timing includes fresh context/scratch/output allocation, encoding,
and destruction. miniz_oxide uses its vector-returning API; other encoders use
an output buffer sized to `n + n / 8 + 4096` bytes. Classic's fresh 400,000-byte
scratch buffer and initialization are included. Decompression reuses the output
buffer but creates and destroys decoder state each call. This measures one-shot
API costs rather than retained compressor/decoder contexts.

Before timing, all four decoders must reproduce the input from **every encoder's
stream**, including classic. The output buffer is poisoned between checks.
Decompression rows name the stream's encoder in `source`, allowing comparisons
on identical compressed data. `level` and `source_mem_level` describe that encoder.
Sizes and status checks remain in timed operations; full equality checks run
outside timing. Encoders and decoders rotate order across three samples.

The default budget is 100 ms per row; `MS_COMPRESS_BENCH_MS=1` is a CI smoke run.
A full run emits 1,620 rows plus the CSV header. Compare medians within the same
workload, size, level, source, and operation, and inspect `compressed_bytes`
alongside throughput. Run measurements without concurrent tests or builds.

API references: [flate2](https://docs.rs/flate2/1.1.10/flate2/),
[miniz_oxide](https://docs.rs/miniz_oxide/0.9.1/miniz_oxide/), and
[libdeflater](https://docs.rs/libdeflater/1.26.1/libdeflater/).
