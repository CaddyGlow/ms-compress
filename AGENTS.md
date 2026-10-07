# Repository guidelines

This standalone Rust 2024 package provides Microsoft compression codecs and
DEFLATE/LZMA implementations. The library supports `no_std + alloc` and uses a
pinned patched `ms-compress-ruzstd` dependency for Zstandard; additional normal dependencies
are enabled by the CLI feature. Preserve codec license
texts, attribution notices, fixtures, and historical validation evidence.

Use rustfmt defaults. Validate changes with `cargo test --all-features --locked`,
`cargo clippy --all-targets --all-features --locked -- -D warnings`, and
`cargo test --no-default-features --locked`. Check embedded compilation with
`cargo check --lib --no-default-features --target thumbv7em-none-eabi --locked`.
Run fuzz regression tests with `cargo test --manifest-path fuzz/Cargo.toml --locked`.
Native Windows and external-oracle interoperability require separate gates.
