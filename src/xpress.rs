// XPRESS format and compatibility behavior based on wimlib at
// cd5e231c348c255ae5088873b5a66ee0eb96fa07, src/xpress_decompress.c:
// Copyright (C) 2012-2016 Eric Biggers, LGPL-2.1-or-later.
// This Rust implementation is distributed under LGPL-2.1-or-later.
// The shared bitstream/Huffman behavioral reference has the MIT notice
// preserved in NOTICE.md. No C code or library is linked by this crate.

use crate::DecodeError;
use alloc::vec::Vec;

const HEADER_BYTES: usize = 256;
const SYMBOLS: usize = 512;
const MAX_CODE_LENGTH: usize = 15;
const LOOKUP_BITS: usize = 11;

/// Decode one WIM XPRESS Huffman block into the exact-sized `output` slice.
///
/// This follows wimlib's compatibility behavior: missing coding units and
/// embedded length bytes are zero-padded, an empty Huffman code emits zeroes,
/// and decoding stops when output is full without requiring an end marker.
/// WIM callers must check the decompressed resource's digest to detect corrupt
/// input that these permissive rules cannot reject. Trailing input is ignored.
///
/// This function allocates no memory and contains no unsafe code. It places no
/// independent block-size limit on output; the WIM resource layer must enforce
/// its configured chunk size (XPRESS normally uses at most 65536 bytes).
/// On failure the output may contain a partially decoded prefix; its contents
/// are otherwise unspecified. Input and output must be separate slices.
///
/// # Errors
///
/// Returns [`DecodeError`] if the header is missing, a nonempty Huffman code is
/// incomplete/oversubscribed, or a match extends outside the output bounds.
pub fn decompress_xpress(input: &[u8], output: &mut [u8]) -> Result<(), DecodeError> {
    decompress_using(input, output, &mut HuffmanCode::empty(), false)
}

/// Decode a Windows Compression API XPRESS Huffman chunk.
/// Supports the 32-bit match-length escape used for chunks larger than 64 KiB.
/// The escape is specified by [MS-XCA section 2.2.4](https://learn.microsoft.com/en-us/openspecs/windows_protocols/ms-xca/26db8e62-bbd8-472c-a09e-623f6de10f0b).
/// The caller must verify the output digest because truncated coding units use
/// the compatibility padding behavior described by [`decompress_xpress`].
pub fn decompress_xpress_windows(input: &[u8], output: &mut [u8]) -> Result<(), DecodeError> {
    decompress_using(input, output, &mut HuffmanCode::empty(), true)
}
#[derive(Debug)]
pub(crate) struct XpressDecoder {
    code: Vec<HuffmanCode>,
}
impl XpressDecoder {
    pub(crate) fn new() -> Result<Self, alloc::collections::TryReserveError> {
        Ok(Self {
            code: crate::memory::filled_vec(1, HuffmanCode::empty())?,
        })
    }
    pub(crate) fn decompress(
        &mut self,
        input: &[u8],
        output: &mut [u8],
    ) -> Result<(), DecodeError> {
        decompress_using(input, output, &mut self.code[0], false)
    }
}
fn decompress_using(
    input: &[u8],
    output: &mut [u8],
    huffman: &mut HuffmanCode,
    windows_lengths: bool,
) -> Result<(), DecodeError> {
    let header = input
        .get(..HEADER_BYTES)
        .ok_or(DecodeError::HeaderTooShort)?;
    *huffman = HuffmanCode::from_header(header)?;
    let mut bits = InputBitstream::new(&input[HEADER_BYTES..]);
    let mut produced = 0;

    while produced < output.len() {
        let (symbol, code_length) = huffman.read_symbol(&mut bits)?;
        bits.remove_bits(code_length);
        if symbol < 256 {
            output[produced] = symbol as u8;
            produced += 1;
            continue;
        }

        let log_offset = usize::from((symbol >> 4) & 15);
        // This prefetch is essential: length bytes are interleaved after the
        // coding words consumed by the bitreader, not at a bit-aligned cursor.
        bits.ensure_bits(16);
        let offset = (1usize << log_offset) | bits.pop_bits(log_offset) as usize;
        let mut length = usize::from(symbol & 15);
        if length == 15 {
            length += usize::from(bits.read_byte());
            if length == 270 {
                length = usize::from(bits.read_u16());
                if windows_lengths && length == 0 {
                    length = bits.read_u32() as usize;
                }
            }
        }
        length = length
            .checked_add(3)
            .ok_or(DecodeError::MatchExceedsOutput {
                length,
                remaining: output.len() - produced,
            })?;

        if offset > produced {
            return Err(DecodeError::InvalidMatchOffset { offset, produced });
        }
        let remaining = output.len() - produced;
        if length > remaining {
            return Err(DecodeError::MatchExceedsOutput { length, remaining });
        }
        crate::copy_lz_match(output, produced, offset, length);
        produced += length;
    }
    Ok(())
}

#[derive(Clone, Copy, Debug)]
struct HuffmanCode {
    fast: [u16; 1 << LOOKUP_BITS],
    counts: [u16; MAX_CODE_LENGTH + 1],
    first_codes: [u16; MAX_CODE_LENGTH + 1],
    first_symbols: [u16; MAX_CODE_LENGTH + 1],
    symbols: [u16; SYMBOLS],
    empty: bool,
}

impl HuffmanCode {
    fn empty() -> Self {
        Self {
            fast: [0; 1 << LOOKUP_BITS],
            counts: [0; MAX_CODE_LENGTH + 1],
            first_codes: [0; MAX_CODE_LENGTH + 1],
            first_symbols: [0; MAX_CODE_LENGTH + 1],
            symbols: [0; SYMBOLS],
            empty: true,
        }
    }
    fn from_header(header: &[u8]) -> Result<Self, DecodeError> {
        let mut lengths = [0u8; SYMBOLS];
        let mut code = Self {
            fast: [0; 1 << LOOKUP_BITS],
            counts: [0; MAX_CODE_LENGTH + 1],
            first_codes: [0; MAX_CODE_LENGTH + 1],
            first_symbols: [0; MAX_CODE_LENGTH + 1],
            symbols: [0; SYMBOLS],
            empty: true,
        };
        for (index, &byte) in header.iter().enumerate() {
            lengths[2 * index] = byte & 15;
            lengths[2 * index + 1] = byte >> 4;
        }
        for &length in &lengths {
            if length != 0 {
                code.counts[usize::from(length)] += 1;
                code.empty = false;
            }
        }

        let mut remaining = 1i32;
        for length in 1..=MAX_CODE_LENGTH {
            remaining = (remaining << 1) - i32::from(code.counts[length]);
            if remaining < 0 {
                return Err(DecodeError::InvalidHuffmanCode);
            }
        }
        if remaining != 0 && !code.empty {
            return Err(DecodeError::InvalidHuffmanCode);
        }

        let mut first_code = 0u16;
        let mut index = 0usize;
        for length in 1..=MAX_CODE_LENGTH {
            first_code = (first_code + code.counts[length - 1]) << 1;
            code.first_codes[length] = first_code;
            code.first_symbols[length] = index as u16;
            for (symbol, &symbol_length) in lengths.iter().enumerate() {
                if usize::from(symbol_length) == length {
                    code.symbols[index] = symbol as u16;
                    index += 1;
                }
            }
        }
        for length in 1..=LOOKUP_BITS {
            for ordinal in 0..usize::from(code.counts[length]) {
                let start =
                    (usize::from(code.first_codes[length]) + ordinal) << (LOOKUP_BITS - length);
                let symbol = code.symbols[usize::from(code.first_symbols[length]) + ordinal];
                code.fast[start..start + (1 << (LOOKUP_BITS - length))]
                    .fill((symbol << 4) | length as u16);
            }
        }
        Ok(code)
    }

    #[inline]
    fn read_symbol(&self, bits: &mut InputBitstream<'_>) -> Result<(u16, usize), DecodeError> {
        bits.ensure_bits(MAX_CODE_LENGTH);
        if self.empty {
            return Ok((0, 0));
        }
        let entry = self.fast[bits.peek_bits(LOOKUP_BITS) as usize];
        if entry != 0 {
            return Ok((entry >> 4, usize::from(entry & 15)));
        }
        self.read_long(bits.peek_bits(MAX_CODE_LENGTH) as u16)
    }

    #[inline(never)]
    fn read_long(&self, prefix: u16) -> Result<(u16, usize), DecodeError> {
        for length in LOOKUP_BITS + 1..=MAX_CODE_LENGTH {
            let codeword = prefix >> (MAX_CODE_LENGTH - length);
            let first = self.first_codes[length];
            if codeword >= first {
                let ordinal = codeword - first;
                if ordinal < self.counts[length] {
                    let index = usize::from(self.first_symbols[length] + ordinal);
                    return Ok((self.symbols[index], length));
                }
            }
        }
        Err(DecodeError::InvalidHuffmanCode)
    }
}

// Models wimlib's 32-bit left-aligned bit buffer and 16-bit little-endian
// coding units. Refill positions must agree even if fewer bits would suffice.
struct InputBitstream<'a> {
    input: &'a [u8],
    next: usize,
    buffer: u32,
    bits_left: usize,
}

impl<'a> InputBitstream<'a> {
    fn new(input: &'a [u8]) -> Self {
        Self {
            input,
            next: 0,
            buffer: 0,
            bits_left: 0,
        }
    }

    fn ensure_bits(&mut self, required: usize) {
        if self.bits_left >= required {
            return;
        }
        if let Some(word) = self.input[self.next..].get(..2) {
            self.buffer |=
                u32::from(u16::from_le_bytes([word[0], word[1]])) << (16 - self.bits_left);
            self.next += 2;
            self.bits_left += 16;
        } else {
            // Padding does not consume a lone remaining input byte; that byte
            // can still be read as a literal match-length extension.
            self.bits_left = 32;
        }
    }

    fn peek_bits(&self, count: usize) -> u32 {
        if count == 0 {
            0
        } else {
            self.buffer >> (32 - count)
        }
    }

    fn remove_bits(&mut self, count: usize) {
        self.buffer <<= count;
        self.bits_left -= count;
    }

    fn pop_bits(&mut self, count: usize) -> u32 {
        let value = self.peek_bits(count);
        self.remove_bits(count);
        value
    }

    fn read_byte(&mut self) -> u8 {
        if let Some(&byte) = self.input.get(self.next) {
            self.next += 1;
            byte
        } else {
            0
        }
    }

    fn read_u32(&mut self) -> u32 {
        if let Some(word) = self.input[self.next..].get(..4) {
            self.next += 4;
            u32::from_le_bytes([word[0], word[1], word[2], word[3]])
        } else {
            0
        }
    }

    fn read_u16(&mut self) -> u16 {
        if let Some(word) = self.input[self.next..].get(..2) {
            self.next += 2;
            u16::from_le_bytes([word[0], word[1]])
        } else {
            0
        }
    }
}
