# Native RTL codec benchmark, 2026-10-10

Windows 11 Pro (OS 10.0.26200, ntdll 10.0.26100.7920), QEMU/KVM guest with four vCPUs on an Intel Core i7-13700K. Rust 1.99.0, portable x86_64-pc-windows-msvc release build, default features, thin LTO. No CPU affinity or frequency pinning; results describe this VM run.

Five 100 ms samples per case, alternating Rust/RTL order. All 1,260 rows completed; every available Rust/native compressed stream was validated by both decoders before timing. Native formats use the standard compression engine and a 4 KiB chunk-size argument.

Compression timings include allocation and workspace construction. Decompression reuses output and native workspace. Rust Huffman compression stops at 64 KiB; larger buffers have native compression and paired decoding results. Compare decoder rows with matching `source`, size, and dataset.

Raw measurements: [CSV](rtl-codecs-2026-10-10.csv). See `benches/rtl_codecs.rs` and the root validation guide for reproduction.

## Compression at 32 KiB

Median MiB/s; higher is faster. Output bytes are shown separately because the encoders can choose different representations.

| Format | Dataset | Rust MiB/s | RTL MiB/s | RTL / Rust | Rust bytes | RTL bytes |
| --- | --- | ---: | ---: | ---: | ---: | ---: |
| LZNT1 | repeat | 787.6 | 1,467.0 | 1.86x | 48 | 48 |
| LZNT1 | pattern | 479.2 | 668.8 | 1.40x | 4864 | 4870 |
| LZNT1 | random | 96.2 | 97.6 | 1.02x | 32784 | 32784 |
| XPRESS | repeat | 924.8 | 3,148.2 | 3.40x | 11 | 11 |
| XPRESS | pattern | 882.5 | 2,996.5 | 3.40x | 289 | 289 |
| XPRESS | random | 103.1 | 799.8 | 7.76x | 36862 | 36861 |
| XPRESS Huffman | repeat | 487.3 | 2,429.0 | 4.98x | 263 | 263 |
| XPRESS Huffman | pattern | 460.7 | 1,967.6 | 4.27x | 515 | 515 |
| XPRESS Huffman | random | 100.1 | 307.9 | 3.08x | 33040 | 33040 |

## Huffman decoding of RTL-produced buffers at 32 KiB

| Dataset | Rust MiB/s | RTL MiB/s | Rust / RTL |
| --- | ---: | ---: | ---: |
| repeat | 5,007.4 | 5,453.5 | 0.92x |
| pattern | 3,943.9 | 4,533.5 | 0.87x |
| random | 308.5 | 310.8 | 0.99x |

## Validation

The Windows MSVC release cross-build and native run passed. Linux all-features
and no-default-features tests, strict all-targets Clippy, embedded compilation,
and fuzz regression tests passed. Strict Windows Clippy encounters 14 existing
`unnecessary_cast` diagnostics in vendored zlib code; the benchmark's Windows
Clippy check passes with that lint suppressed on the command line. No vendored
codec code was changed for this benchmark.
