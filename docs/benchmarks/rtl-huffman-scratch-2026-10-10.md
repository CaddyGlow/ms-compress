# Huffman scratch initialization optimization, 2026-10-10

The reusable XPRESS compressor now reserves chain-link, token, and output capacity without initializing the entire maximum-sized scratch area. Parsing appends only the links/tokens it needs, and output is initialized to the exact size calculated from the parse and Huffman codes. Head-table initialization, parser search depth, match selection, canonical codes, and public APIs are unchanged. Capacity remains reserved up front with fallible allocation; per-block processing does not grow or reallocate these vectors.

A regression test exercises small/large and repeated/random blocks after capacity rejection, checks roundtrips, and verifies that scratch pointers and capacities remain unchanged. Existing tests cover stale output, reused chains, and generation wrap.

## Native comparison

The previous and updated executables ran sequentially in the same Windows 11 KVM guest, using the [earlier environment](rtl-codecs-2026-10-10.md), five alternating-order 100 ms samples per case, and `MS_COMPRESS_RTL_FORMAT=4`. Both runs completed all 360 measurements and cross-validated every available Rust/native stream through both decoders. Every corresponding compressed size was unchanged. Native controls remained close for the 32 KiB compression cases.

Median compression throughput in MiB/s at 32 KiB:

| Dataset | Rust before | Rust after | Gain | RTL before | RTL after | Rust after / RTL after |
| --- | ---: | ---: | ---: | ---: | ---: | ---: |
| repeat | 1,071.0 | 2,678.2 | 2.50x | 2,459.8 | 2,448.9 | 1.09x |
| pattern | 939.2 | 2,044.8 | 2.18x | 2,022.3 | 2,027.6 | 1.01x |
| random | 100.9 | 100.6 | 1.00x | 308.1 | 309.2 | 0.33x |

Repeated-data compression is about 9% faster than RTL in this run; patterned data is effectively tied. Random data remains much slower than RTL and gains nothing from avoiding unused initialization, because its parser touches most scratch entries anyway. No affinity/frequency control was used, so small differences should not be treated as a general performance guarantee.

Raw measurements: [before](rtl-huffman-scratch-before-2026-10-10.csv), [after](rtl-huffman-scratch-after-2026-10-10.csv).

All required all-features/no-default-features tests, strict Linux Clippy, embedded compilation, and fuzz regressions passed. The native benchmark provides Windows interoperability validation. No codec attribution or historical evidence was replaced.
