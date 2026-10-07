DEFLATE, zlib, gzip, CRC32, and Adler32 support adapted from zlib-rs.

This module works with `core` and `alloc`, using Rust's global allocator.
`Deflate` and `Inflate` provide reusable streaming codecs. Window bits -15, 15,
and 31 select raw DEFLATE, zlib, and gzip respectively. One-shot slice helpers
use `DeflateConfig` and `InflateConfig`. See the module's NOTICE.md for source
provenance and local modifications.

## Example

```rust
use ms_compress::zlib::ReturnCode;
use ms_compress::zlib::{DeflateConfig, compress_bound, compress_slice};
use ms_compress::zlib::{InflateConfig, decompress_slice};

let input = b"Hello World";

// --- compress ---
let mut compressed_buf = vec![0u8; compress_bound(input.len())];
let (compressed, rc) =
    compress_slice(&mut compressed_buf, input, DeflateConfig::default());
assert_eq!(rc, ReturnCode::Ok);

// --- decompress ---
let mut decompressed_buf = vec![0u8; input.len()];
let (decompressed, rc) =
    decompress_slice(&mut decompressed_buf, compressed, InflateConfig::default());
assert_eq!(rc, ReturnCode::Ok);

assert_eq!(decompressed, input);
```
