# Classic zlib golden streams

`zlib-classic.bin` was generated with CPython's native **zlib 1.3.2** binding by
`python3 tests/fixtures/generate_zlib_classic.py`. The generator checks the runtime
version and independently decompresses both outputs. Inputs are synthetic and
fully specified by the generator; no third-party corpus is embedded.

Each input has two native expected streams:

- Raw DEFLATE: level 1, window bits -15, memory level 9, default strategy;
  successive 32,768-byte `compress()` submissions, then `Z_FINISH`.
- Gzip: level 6, window bits 31, memory level 8, default strategy; one whole-input
  submission, then `Z_FINISH`. Only the gzip OS byte is set to 10 to match the
  documented Defender NTFS header produced by `gzip_level6`.

The seven cases cover empty and short input, the 262-byte lookahead boundary,
a 32 KiB input boundary, random and repetitive 64 KiB window boundaries, and
multiple windows mixing random, repeated, and patterned regions. Tests require
byte-for-byte equality, including headers, block choices, and trailers.

Format: `ZCL1`, u32 case count; each record contains u16 UTF-8 name length,
name bytes, u32 input length, u32 raw length, u32 gzip length, then input, raw,
and gzip bytes. All integers are little-endian. Preserve these original golden
streams when changing the implementation; regenerate only against the specified
independent oracle.

Original corpus SHA-256:
`b1a4c00a71c1bbfff5cffb6e66f5973f8a142b25f64d3379590824eb4c2b2ed9`.
