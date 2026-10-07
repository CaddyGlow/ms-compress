# Independent LZMA fixtures

`hello.lzma` (.lzma header) and `hello.lzma2` (raw LZMA2) decode to
`Hello, world!`. Both were generated with Python 3 standard-library `lzma`
(backed by liblzma), using dictionary size 4096. No production C code is linked.

Generation: `lzma.compress(b"Hello, world!", format=lzma.FORMAT_ALONE,
filters=[{"id": lzma.FILTER_LZMA1, "dict_size": 4096}])`; for LZMA2, use
`FORMAT_RAW` and `FILTER_LZMA2`.

The raw LZMA1 Hello World vector in integration tests and imported unit tests
is retained from upstream lzma-rust2. See ../../../src/lzma/NOTICE.md for provenance.
