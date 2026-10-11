# DEFLATE comparison — 2026-10-10

[Full CSV](zlib-comparison-2026-10-10.csv), 1,620 measurement rows; all 432
encoder/decoder interoperability checks passed. Three 100 ms samples per case.

Environment: Linux x86-64, Intel Core i7-13700K, Rust 1.99.0, release profile
with thin LTO and the default CPU target. The final run followed all test/build
checks, without concurrent validation commands. No CPU affinity or fixed clock
was imposed. These are local synthetic workload results, not general rankings.

Libraries: ms-compress 0.1.3 plus the uncommitted classic-level-1 work;
C zlib 1.3.2 through flate2 1.1.10/libz-sys 1.1.29 (statically linked in this run),
miniz_oxide 0.9.1, and libdeflate 1.26.1 through libdeflater.
See the [benchmark README](../../benches/zlib-comparison/README.md) for complete
parameters, timing scope, and reproduction commands. All streams are raw DEFLATE.

Compression includes creation/destruction and allocations. Decompression reuses
output storage but includes decoder creation/destruction. Classic has a fixed
memory level of 9 and 32 KiB submissions; regular ms-compress and native zlib
use memory level 8 and whole-buffer submissions. Level numbers are library-specific
speed/ratio settings. Compare compressed size alongside speed.

## 32 KiB compression

Cells contain **median MiB/s / compressed bytes**; lower byte counts are better.

### Level 1

| Implementation | Repeated | Patterned | Random |
|---|---:|---:|---:|
| ms-compress | 3,250.2 / 320 | 2,793.2 / 768 | 187.7 / 34,571 |
| ms-compress-classic | 1,795.9 / 158 | 1,056.9 / 675 | 110.2 / 32,773 |
| zlib-c | 2,726.5 / 158 | 1,441.7 / 675 | 83.7 / 32,778 |
| miniz_oxide | 3,125.7 / 167 | 2,071.0 / 752 | 142.3 / 32,773 |
| libdeflate | 1,323.3 / 47 | 1,151.6 / 587 | 157.4 / 32,773 |

### Level 6

| Implementation | Repeated | Patterned | Random |
|---|---:|---:|---:|
| ms-compress | 2,342.9 / 49 | 1,195.6 / 710 | 94.4 / 32,780 |
| zlib-c | 621.8 / 47 | 521.3 / 543 | 82.3 / 32,778 |
| miniz_oxide | 684.1 / 47 | 593.2 / 517 | 66.4 / 32,778 |
| libdeflate | 773.9 / 47 | 530.2 / 517 | 130.2 / 32,773 |

## 32 KiB decompression of native-zlib level-1 streams

Every decoder receives the same compressed bytes in each column. Cells contain
median MiB/s. Results for every other encoder's streams are retained in the CSV.

| Implementation | Repeated | Patterned | Random |
|---|---:|---:|---:|
| ms-compress | 13,174.4 | 5,860.4 | 16,323.5 |
| zlib-c | 13,216.7 | 6,226.5 | 37,868.6 |
| miniz_oxide | 8,595.1 | 5,983.8 | 31,845.0 |
| libdeflate | 13,950.4 | 8,516.6 | 30,849.2 |

At 32 KiB and level 1, the regular ms-compress encoder is fastest on all three
synthetic workloads, while producing larger streams. Classic matches native
zlib's output sizes for repeated/patterned inputs here but is slower on those
inputs; its random-input result is faster and slightly smaller. The differing
memory levels mean these are API workload comparisons, not identical-setting
algorithm comparisons. Larger sizes, level 6, and compression ratios change the
ranking; consult the full CSV rather than extrapolating this table.

Validation: all-features tests, strict all-targets Clippy, no-default-features
tests, thumbv7em-none-eabi compilation, and fuzz regressions passed. The standalone
benchmark also passed strict Clippy. The six classic-zlib tests include seven
independent golden cases for both raw level 1 and gzip level 6, plus buffer limits,
guards, unaligned scratch, and reuse after rejection. A 1 ms benchmark smoke run
is configured in Linux CI; CI itself has not been triggered for these local edits.
