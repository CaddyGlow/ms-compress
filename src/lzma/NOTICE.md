# Vendored lzma-rust2

Source: https://github.com/hasenbanck/lzma-rust2/tree/8fdb428469c3a11564583941f4621883c2f242f4

Upstream version: 0.21.0. License: Apache-2.0, retained in LICENSE.
Upstream describes this as a port of Tukaani XZ for Java and a maintained fork
of lzma-rust. This copy is modified.

Included: LZMA1 and raw LZMA2 encoders, blocking and incremental decoders,
filters, LZMA2 worker decoder, and its work queue. XZ/LZIP containers and
multithreaded encoding are omitted, preserving a dependency-free library.
Crate paths and examples are adapted to `ms_compress::lzma`. The encoder is
always available; upstream's optimization feature is named `lzma-optimization`.
CPU detection is shared with zlib where possible, including no_std static
selection for normalization. No C allocator is used.

Local changes include bounded slice helpers, per-worker and per-work-unit
limits for the threaded decoder, rejection of EOF without an LZMA2 end marker,
shutdown-aware result sending and worker joins on drop, validation of minimum
raw LZMA2 dictionary size, core error traits without std, and Clippy style fixes.

Original core, allocation-estimate, filter, and work-queue unit tests are retained.
BCJ roundtrips use deterministic synthetic pseudorandom input instead of upstream
wget executables. New integration tests retain the original Hello World raw
LZMA vector and include independently generated liblzma fixtures, all presets,
streaming/trailing input, limits, truncation, parallel ordering, and early drop.
The fuzz harness and benchmark cover both LZMA variants.
