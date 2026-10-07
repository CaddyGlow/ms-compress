# Quantum decoder provenance

`quantum.rs` is a native Rust CAB Quantum decoder added in 2026. It follows the
arithmetic/LZ format and adaptive-model behavior documented by these references:

- libmspack `mspack/qtmd.c` and `qtm.h`, pinned commit
  `55d501976171397ccd5d5a7a1ca7da065b1d9a06`:
  <https://github.com/kyz/libmspack/blob/55d501976171397ccd5d5a7a1ca7da065b1d9a06/libmspack/mspack/qtmd.c>.
  (C) 2003–2023 Stuart Caie. The decompressor was researched and implemented
  by Matthew Russotto and adapted with permission. The Quantum method was
  created by David Stafford and adapted by Microsoft Corporation. Upstream
  states GNU LGPL version 2.1.
- 7-Zip `CPP/7zip/Compress/QuantumDecoder.cpp`:
  <https://github.com/ip7z/7zip/blob/master/CPP/7zip/Compress/QuantumDecoder.cpp>.
  7-Zip Copyright (C) 1999–2026 Igor Pavlov. LGPL-2.1-or-later; Quantum is outside
  the RAR-specific files and restrictions. Reference source and license were
  consulted on 2026-10-03.

The Rust implementation retains these attributions and is distributed under
LGPL-2.1-only, conservatively preserving libmspack's stated version. The complete
license text is in `LICENSE-LGPL-2.1` in this directory. Other codec sources retain
their existing LGPL-2.1-or-later licenses; the crate metadata records both terms.
No C/C++ code is compiled or invoked by the native extraction path.

The implementation uses checked MSB-first bit reads, bounded frame and dictionary
sizes, persistent nine-model state and a circular dictionary, exact model reorder
semantics, and per-frame arithmetic restart/termination. It rejects unavailable
history, matches that cross frame boundaries, and implicit missing input bytes.
Zero through four trailing zero bytes are accepted. A decoding error invalidates
that folder decoder.
