# Codec provenance

This crate is LGPL-2.1-or-later. See LICENSE-LGPL-2.1. The implementation is
native safe Rust and does not link or invoke the upstream C decoder.

Behavioral reference: wimlib cd5e231c348c255ae5088873b5a66ee0eb96fa07:
- src/xpress_decompress.c: Copyright (C) 2012-2016 Eric Biggers,
  LGPL-2.1-or-later.
- include/wimlib/xpress_constants.h: format constants.
- include/wimlib/decompress_common.h and src/decompress_common.c:
  Copyright 2022 Eric Biggers, MIT. Their notice is retained below because
  the bitstream and canonical-code semantics informed the Rust port.

Copyright 2022 Eric Biggers

Permission is hereby granted, free of charge, to any person
obtaining a copy of this software and associated documentation
files (the "Software"), to deal in the Software without
restriction, including without limitation the rights to use,
copy, modify, merge, publish, distribute, sublicense, and/or sell
copies of the Software, and to permit persons to whom the
Software is furnished to do so, subject to the following
conditions:

The above copyright notice and this permission notice shall be
included in all copies or substantial portions of the Software.

THE SOFTWARE IS PROVIDED "AS IS", WITHOUT WARRANTY OF ANY KIND,
EXPRESS OR IMPLIED, INCLUDING BUT NOT LIMITED TO THE WARRANTIES
OF MERCHANTABILITY, FITNESS FOR A PARTICULAR PURPOSE AND
NONINFRINGEMENT. IN NO EVENT SHALL THE AUTHORS OR COPYRIGHT
HOLDERS BE LIABLE FOR ANY CLAIM, DAMAGES OR OTHER LIABILITY,
WHETHER IN AN ACTION OF CONTRACT, TORT OR OTHERWISE, ARISING
FROM, OUT OF OR IN CONNECTION WITH THE SOFTWARE OR THE USE OR
OTHER DEALINGS IN THE SOFTWARE.

