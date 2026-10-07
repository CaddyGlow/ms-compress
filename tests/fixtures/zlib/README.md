# zlib interoperability fixtures

`hello-world.gz` was generated independently with Python's `gzip.compress`,
using `b"Hello World!\n"`, compression level 1, and `mtime=0`. The host zlib
version was 1.3.2. Its decoded bytes are the plaintext of upstream
zlib-rs's `deflate::test::hello_world_quick` regression.

The integration test also uses the exact upstream zlib expected vector and its
raw DEFLATE payload. Upstream revision and licensing are recorded in
`src/zlib/NOTICE.md`; the full upstream unit tests and decoder/encoder fixtures
remain alongside the imported code.
