# LZX decoder provenance

The native safe Rust implementation in lzx.rs is a behavioral port of wimlib
revision cd5e231c348c255ae5088873b5a66ee0eb96fa07, src/lzx_decompress.c,
src/lzx_common.c, include/wimlib/lzx_constants.h and include/wimlib/lzx_common.h.
The decoder and common preprocessing code carry Copyright (C) 2012-2016
Eric Biggers and LGPL-2.1-or-later. This Rust implementation retains that
license. See LICENSE-LGPL-2.1 in this directory.

The canonical-code and input-bitstream semantics also follow
include/wimlib/decompress_common.h and src/decompress_common.c, Copyright
2022 Eric Biggers, MIT. The complete MIT notice is preserved in NOTICE.md
in this directory. No upstream C code is compiled, loaded or executed by
this module or its Rust tests.

The libFuzzer corpus copied into tests/fixtures/lzx-libfuzzer is test material
from the upstream project, distributed under its GPL-3.0-or-later option;
see tests/fixtures/LICENSE-GPL-3.0. Generated original-C snapshots and their
standalone standard-library Python generator retain their explicit provenance.
