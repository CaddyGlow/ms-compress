# Historical vendoring evidence

These files preserve provenance and license evidence for the former vendored
implementation. Executable sources were removed when ms-compress switched to
CaddyGlow/zstd-rs commit 468f57ac3de6779ab17191f8bc2fd2bebc0b5439.
The fork preserves the MIT license and implements the changes described below.

# Zstandard codec provenance

Adapted from ruzstd 0.9.0, https://github.com/KillingSpark/zstd-rs, author Moritz Borcherding. The upstream MIT text is preserved in LICENSE-MIT. Package sources came from the locally cached crates.io release.

Namespace paths were adapted to ms_compress::zstd. Hashing is always enabled through a dependency-free XXH64 implementation. The unsafe ring buffer was replaced with checked alloc::collections::VecDeque storage; all Zstandard code forbids unsafe code. Corpus/dev-dependency tests remain preserved but disabled; integration tests supply external-libzstd fixtures. Optional dictionary training and upstream fuzz exports are not exposed.

Retained-output mode has a per-frame emitted cursor. Copying hashes only newly emitted bytes, then evicts emitted history older than the negotiated window. Destructive Read/collect mode remains separate and must not be mixed with retained mode on a frame. Reset the cursor on every init/reset.
