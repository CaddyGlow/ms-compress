# Vendored zlib-rs

Source: https://github.com/trifectatechfoundation/zlib-rs/tree/7909c0fc48f5d29f6610770e31d6f0c924f9ec3b/zlib-rs

Upstream version: 0.6.8. Copyright 2024 Trifecta Tech Foundation.
The original Zlib license is retained in LICENSE. This copy is modified.

The crate is embedded as `ms_compress::zlib`, with paths and examples adapted.
Rust global allocation is the only backend; the libc allocator and its tests
are removed. Internal C-shaped stream structures remain private implementation
details. There is no public C allocator interface or dependency on libc.
Allocation size overflow is checked. Upstream optional debugging, fuzzing, and
checksum-disabling features are not exposed.

Upstream unit tests and fixtures are retained, with explicit alloc imports for
no_std testing and a CPU-conditional CRC panic assertion. A bounded shared
match-prefix adapter extends SIMD comparison to the Microsoft encoders. Static
CPU feature detection is also available without std. Public API integration
tests and bounded fuzz harnesses live in the containing crate.
