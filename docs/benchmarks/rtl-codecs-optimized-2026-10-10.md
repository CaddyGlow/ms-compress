# First RTL-inspired optimization results, 2026-10-10

See [binary analysis](rtl-compression-analysis-2026-10-10.md) for the inspected native routines and remaining opportunities. The change skips unused final-token dictionary updates in LZNT1/plain XPRESS/Huffman and rejects candidates with unequal three-byte prefixes before long-match comparison. Match selection and output bytes are preserved by construction.

Reused the Windows 11 KVM guest and benchmark configuration from the [baseline](rtl-codecs-2026-10-10.md): portable Rust 1.99.0 release build, five alternating-order 100 ms samples. All 1,260 measurements completed, every available encoded stream was validated through both Rust and RTL decoders, and compressed sizes matched the baseline for every row.

Raw measurements: [optimized CSV](rtl-codecs-optimized-2026-10-10.csv).

## Compression at 32 KiB

Median MiB/s; higher is faster. Rust / RTL greater than 1 means Rust won in this run.

| Format | Dataset | Rust MiB/s | RTL MiB/s | Rust / RTL |
| --- | --- | ---: | ---: | ---: |
| LZNT1 | repeat | 1,865.7 | 446.2 | 4.18x |
| LZNT1 | pattern | 250.8 | 257.7 | 0.97x |
| LZNT1 | random | 30.7 | 42.5 | 0.72x |
| XPRESS | repeat | 6,231.7 | 3,171.5 | 1.96x |
| XPRESS | pattern | 5,164.7 | 3,028.2 | 1.71x |
| XPRESS | random | 154.5 | 809.4 | 0.19x |
| XPRESS Huffman | repeat | 1,064.6 | 2,430.3 | 0.44x |
| XPRESS Huffman | pattern | 935.6 | 2,003.1 | 0.47x |
| XPRESS Huffman | random | 98.2 | 300.5 | 0.33x |

Plain XPRESS wins on the repeated and patterned workloads; its random-data path still trails RTL. Huffman remains behind RTL, with large eagerly initialized one-shot workspaces and the existing deeper hash-chain parser still in place.

Absolute speeds varied between runs, especially during the LZNT1 phase: its native measurements were substantially slower than the earlier baseline. Treat those figures as this run’s paired observations, not as a reliable before/after speedup. No CPU affinity/frequency control was used.

All required all-features/no-default-features tests, strict Linux Clippy, embedded compilation, and fuzz regressions passed. Native validation is provided by the complete RTL benchmark cross-decoding run.
