// SPDX-License-Identifier: LGPL-2.1-or-later
// Native safe Rust port of WIM LZX behavior from wimlib
// cd5e231c348c255ae5088873b5a66ee0eb96fa07: src/lzx_decompress.c and
// src/lzx_common.c, Copyright (C) 2012-2016 Eric Biggers, LGPL-2.1-or-later.
// See LICENSE-LGPL-2.1 and NOTICE.md in this directory. The MIT notice for
// the shared bitstream/canonical-code behavioral reference is also retained.

//! Native WIM LZX block decoding.

use alloc::vec::Vec;
mod cabinet;
mod delta;
pub use cabinet::{CabinetLzxDecoder, CabinetLzxError};
pub use delta::{LzxDeltaDecoder, LzxDeltaError, decompress_lzxd};

/// A malformed LZX block or unsupported configured block capacity.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LzxError {
    /// Maximum block size is zero or exceeds 2 MiB.
    InvalidMaxBlockSize,
    /// Output exceeds the configured maximum block size.
    OutputExceedsLimit,
    /// A block type is not verbatim, aligned, or uncompressed.
    InvalidBlockType,
    /// A block has zero length or exceeds the remaining output.
    InvalidBlockSize,
    /// Codeword lengths cannot describe a complete or empty canonical code.
    InvalidHuffmanCode,
    /// A repeated codeword-length run uses an invalid precode symbol.
    InvalidLengthRun,
    /// An uncompressed block initializes a recent offset to zero.
    InvalidRecentOffset,
    /// Uncompressed literal bytes extend past the available input.
    TruncatedUncompressedBlock,
    /// A match source or destination lies outside the block's output bounds.
    InvalidMatch,
}

impl core::fmt::Display for LzxError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.write_str(match self {
            Self::InvalidMaxBlockSize => "invalid LZX maximum block size",
            Self::OutputExceedsLimit => "LZX output exceeds configured capacity",
            Self::InvalidBlockType => "invalid LZX block type",
            Self::InvalidBlockSize => "invalid LZX block size",
            Self::InvalidHuffmanCode => "invalid LZX canonical code lengths",
            Self::InvalidLengthRun => "invalid LZX codeword-length run",
            Self::InvalidRecentOffset => "zero LZX recent match offset",
            Self::TruncatedUncompressedBlock => "truncated LZX uncompressed block",
            Self::InvalidMatch => "LZX match exceeds output bounds",
        })
    }
}

impl core::error::Error for LzxError {}

const FAST_BITS: usize = 11;
const MAIN_SYMBOLS: usize = 656;
const LENGTH_SYMBOLS: usize = 249;
const LENS_OVERRUN: usize = 50;
const OFFSET_BASE: [usize; 50] = [
    0, 0, 0, 1, 2, 4, 6, 10, 14, 22, 30, 46, 62, 94, 126, 190, 254, 382, 510, 766, 1022, 1534,
    2046, 3070, 4094, 6142, 8190, 12286, 16382, 24574, 32766, 49150, 65534, 98302, 131070, 196606,
    262142, 393214, 524286, 655358, 786430, 917502, 1048574, 1179646, 1310718, 1441790, 1572862,
    1703934, 1835006, 1966078,
];
const EXTRA_BITS: [usize; 50] = [
    0, 0, 0, 0, 1, 1, 2, 2, 3, 3, 4, 4, 5, 5, 6, 6, 7, 7, 8, 8, 9, 9, 10, 10, 11, 11, 12, 12, 13,
    13, 14, 14, 15, 15, 16, 16, 17, 17, 17, 17, 17, 17, 17, 17, 17, 17, 17, 17, 17, 17,
];

/// Decode a complete WIM LZX chunk into the caller's exact-sized output.
///
/// `max_block_size` configures the same window as wimlib: it must be between
/// 1 and 2097152 inclusive, and is rounded up to a power of two with a minimum
/// window of 32768. WIMGAPI interoperability normally requires that minimum
/// window. This implements the WIM variant, including the fixed-size inverse
/// E8 transform, not cabinet framing or LZX DELTA extensions.
///
/// Coding bits and embedded integers have wimlib's zero-padding behavior;
/// uncompressed literal bytes require real input. Empty canonical codes emit
/// symbol zero. Up to 50 overrun codeword lengths are retained in scratch
/// storage, matching upstream delta-length decoding semantics for malformed
/// blocks. Resource checksums must detect otherwise accepted corruption.
///
/// The decoder allocates no memory and uses only safe Rust. On error, output
/// may contain a decoded prefix and must not be considered a valid resource.
/// Trailing compressed input is ignored after the requested output is full.
///
/// # Errors
///
/// Returns [`LzxError`] for invalid capacity, block type/size, Huffman codes,
/// recent offsets, repeated length runs, missing uncompressed bytes, or a
/// match that crosses output bounds. Match sources can reference earlier
/// blocks in the same chunk, but match destinations cannot cross block ends.
pub fn decompress_lzx(
    input: &[u8],
    output: &mut [u8],
    max_block_size: usize,
) -> Result<(), LzxError> {
    decompress_using(input, output, max_block_size, &mut Workspace::empty())
}
#[derive(Clone, Copy, Debug)]
struct Workspace {
    main_lengths: [u8; MAIN_SYMBOLS + LENS_OVERRUN],
    length_lengths: [u8; LENGTH_SYMBOLS + LENS_OVERRUN],
    main: Code<MAIN_SYMBOLS>,
    lengths: Code<LENGTH_SYMBOLS>,
    aligned: Code<8>,
}
impl Workspace {
    fn empty() -> Self {
        Self {
            main_lengths: [0; MAIN_SYMBOLS + LENS_OVERRUN],
            length_lengths: [0; LENGTH_SYMBOLS + LENS_OVERRUN],
            main: Code::empty(16),
            lengths: Code::empty(16),
            aligned: Code::empty(7),
        }
    }
}
#[derive(Debug)]
pub(crate) struct LzxDecoder {
    workspace: Vec<Workspace>,
}
impl LzxDecoder {
    pub(crate) fn new() -> Result<Self, alloc::collections::TryReserveError> {
        Ok(Self {
            workspace: crate::memory::filled_vec(1, Workspace::empty())?,
        })
    }
    pub(crate) fn decompress(
        &mut self,
        input: &[u8],
        output: &mut [u8],
        maximum: usize,
    ) -> Result<(), LzxError> {
        decompress_using(input, output, maximum, &mut self.workspace[0])
    }
}
fn decompress_using(
    input: &[u8],
    output: &mut [u8],
    max_block_size: usize,
    workspace: &mut Workspace,
) -> Result<(), LzxError> {
    if max_block_size == 0 || max_block_size > (1 << 21) {
        return Err(LzxError::InvalidMaxBlockSize);
    }
    if output.len() > max_block_size {
        return Err(LzxError::OutputExceedsLimit);
    }
    let window_order = (usize::BITS - (max_block_size - 1).leading_zeros()).max(15) as usize;
    let max_offset = (1usize << window_order) - 3;
    let mut offset_slots = 30;
    while offset_slots < OFFSET_BASE.len() && max_offset >= OFFSET_BASE[offset_slots] {
        offset_slots += 1;
    }
    let num_main_symbols = 256 + offset_slots * 8;
    let Workspace {
        main_lengths,
        length_lengths,
        main,
        lengths,
        aligned,
    } = workspace;
    main_lengths.fill(0);
    length_lengths.fill(0);
    let mut recent_offsets = [1usize; 3];
    let mut bits = Bits::new(input);
    let mut produced = 0;
    let mut may_have_e8 = false;

    while produced < output.len() {
        bits.ensure(4);
        let block_type = bits.pop(3);
        let default_size = bits.pop(1) != 0;
        let block_size = if default_size {
            32768
        } else {
            let mut size = bits.read(16) as usize;
            if window_order >= 16 {
                size = (size << 8) | bits.read(8) as usize;
            }
            size
        };
        let mut aligned_lengths = [0u8; 8];
        match block_type {
            1 | 2 => {
                if block_type == 2 {
                    for length in &mut aligned_lengths {
                        *length = bits.read(3) as u8;
                    }
                }
                read_lengths(&mut bits, main_lengths, 256)?;
                read_lengths(&mut bits, &mut main_lengths[256..], num_main_symbols - 256)?;
                read_lengths(&mut bits, length_lengths, LENGTH_SYMBOLS)?;
            }
            3 => {
                // If already aligned, ensure consumes a whole extra unit;
                // this seemingly redundant discard is part of the format.
                bits.ensure(1);
                bits.align();
                for offset in &mut recent_offsets {
                    *offset = bits.literal_u32() as usize;
                }
                if recent_offsets.contains(&0) {
                    return Err(LzxError::InvalidRecentOffset);
                }
            }
            _ => return Err(LzxError::InvalidBlockType),
        }
        if block_size == 0 || block_size > output.len() - produced {
            return Err(LzxError::InvalidBlockSize);
        }
        let block_end = produced + block_size;
        if block_type == 3 {
            output[produced..block_end].copy_from_slice(bits.literal_bytes(block_size)?);
            if block_size & 1 != 0 {
                bits.literal_byte();
            }
            produced = block_end;
            may_have_e8 = true;
            continue;
        }

        *main = Code::<MAIN_SYMBOLS>::new(&main_lengths[..num_main_symbols], 16)?;
        *lengths = Code::<LENGTH_SYMBOLS>::new(&length_lengths[..LENGTH_SYMBOLS], 16)?;
        if block_type == 2 {
            *aligned = Code::<8>::new(&aligned_lengths, 7)?;
        }
        while produced < block_end {
            let symbol = main.read(&mut bits)?;
            if symbol < 256 {
                output[produced] = symbol as u8;
                produced += 1;
                continue;
            }
            let mut length = symbol & 7;
            let slot = (symbol - 256) / 8;
            if length == 7 {
                length += lengths.read(&mut bits)?;
            }
            length += 2;
            let offset = if slot < 3 {
                let offset = recent_offsets[slot];
                // R2 does not demote R1: this deliberately is not true LRU.
                recent_offsets[slot] = recent_offsets[0];
                offset
            } else {
                let aligned_slot = block_type == 2 && slot >= 8;
                let mut extra =
                    bits.read(EXTRA_BITS[slot] - if aligned_slot { 3 } else { 0 }) as usize;
                if aligned_slot {
                    extra = (extra << 3) | aligned.read(&mut bits)?;
                }
                recent_offsets[2] = recent_offsets[1];
                recent_offsets[1] = recent_offsets[0];
                OFFSET_BASE[slot] + extra
            };
            recent_offsets[0] = offset;
            if offset == 0 || offset > produced || length > block_end - produced {
                return Err(LzxError::InvalidMatch);
            }
            crate::copy_lz_match(output, produced, offset, length);
            produced += length;
        }
        may_have_e8 |= main_lengths[0xe8] != 0;
    }
    if may_have_e8 {
        inverse_e8(output);
    }
    Ok(())
}

fn read_lengths(
    bits: &mut impl HuffmanBits,
    lengths: &mut [u8],
    count: usize,
) -> Result<(), LzxError> {
    let mut pre_lengths = [0u8; 20];
    for length in &mut pre_lengths {
        *length = bits.read(4) as u8;
    }
    let pre = Code::<20>::new(&pre_lengths, 15)?;
    let mut position = 0;
    while position < count {
        let symbol = pre.read(bits)?;
        let (run, length) = match symbol {
            0..=16 => (1, delta_length(lengths[position], symbol)),
            17 => (4 + bits.read(4) as usize, 0),
            18 => (20 + bits.read(5) as usize, 0),
            _ => {
                let run = 4 + bits.read(1) as usize;
                let delta = pre.read(bits)?;
                if delta > 17 {
                    return Err(LzxError::InvalidLengthRun);
                }
                (run, delta_length(lengths[position], delta))
            }
        };
        let end = position + run;
        let slice = lengths
            .get_mut(position..end)
            .ok_or(LzxError::InvalidLengthRun)?;
        slice.fill(length);
        position = end;
    }
    Ok(())
}

fn delta_length(previous: u8, delta: usize) -> u8 {
    let value = i16::from(previous) - delta as i16;
    (if value < 0 { value + 17 } else { value }) as u8
}

fn inverse_e8(output: &mut [u8]) {
    if output.len() <= 10 {
        return;
    }
    let mut position = 0;
    while position < output.len() - 10 {
        if output[position] != 0xe8 {
            position += 1;
            continue;
        }
        let bytes = &output[position + 1..position + 5];
        let absolute = i32::from_le_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]);
        let relative = if (0..12000000).contains(&absolute) {
            Some(absolute - position as i32)
        } else if absolute < 0 && absolute >= -(position as i32) {
            Some(absolute + 12000000)
        } else {
            None
        };
        if let Some(value) = relative {
            output[position + 1..position + 5].copy_from_slice(&value.to_le_bytes());
        }
        position += 5;
    }
}

#[derive(Clone, Copy, Debug)]
struct Code<const N: usize> {
    // Packed symbol/length entries; long codewords retain canonical fallback.
    fast: [u32; 1 << FAST_BITS],
    count: [usize; 17],
    first_code: [u32; 17],
    first_symbol: [usize; 17],
    symbols: [usize; N],
    max_length: usize,
    empty: bool,
}

impl<const N: usize> Code<N> {
    fn empty(max_length: usize) -> Self {
        Self {
            fast: [0; 1 << FAST_BITS],
            count: [0; 17],
            first_code: [0; 17],
            first_symbol: [0; 17],
            symbols: [0; N],
            max_length,
            empty: true,
        }
    }
    fn new(lengths: &[u8], max_length: usize) -> Result<Self, LzxError> {
        let mut code = Self {
            fast: [0; 1 << FAST_BITS],
            count: [0; 17],
            first_code: [0; 17],
            first_symbol: [0; 17],
            symbols: [0; N],
            max_length,
            empty: true,
        };
        if lengths.len() > N {
            return Err(LzxError::InvalidHuffmanCode);
        }
        for &length in lengths {
            if length as usize > max_length {
                return Err(LzxError::InvalidHuffmanCode);
            }
            if length != 0 {
                code.count[length as usize] += 1;
                code.empty = false;
            }
        }
        let mut remaining = 1i32;
        for length in 1..=max_length {
            remaining = (remaining << 1) - code.count[length] as i32;
            if remaining < 0 {
                return Err(LzxError::InvalidHuffmanCode);
            }
        }
        if remaining != 0 && !code.empty {
            return Err(LzxError::InvalidHuffmanCode);
        }
        let mut value = 0;
        let mut index = 0;
        for length in 1..=max_length {
            value = (value + code.count[length - 1] as u32) << 1;
            code.first_code[length] = value;
            code.first_symbol[length] = index;
            for (symbol, &symbol_length) in lengths.iter().enumerate() {
                if usize::from(symbol_length) == length {
                    code.symbols[index] = symbol;
                    index += 1;
                }
            }
        }
        for length in 1..=max_length.min(FAST_BITS) {
            for ordinal in 0..code.count[length] {
                let start = ((code.first_code[length] as usize) + ordinal) << (FAST_BITS - length);
                let symbol = code.symbols[code.first_symbol[length] + ordinal];
                let packed = ((symbol << 5) | length) as u32;
                code.fast[start..start + (1 << (FAST_BITS - length))].fill(packed);
            }
        }
        Ok(code)
    }

    #[inline]
    fn read(&self, bits: &mut impl HuffmanBits) -> Result<usize, LzxError> {
        bits.ensure(self.max_length);
        if self.empty {
            return Ok(0);
        }
        self.read_ready(bits, self.max_length)
    }

    // CAB callers validate nonempty trees before decoding their symbols.
    #[inline]
    fn read_nonempty(
        &self,
        bits: &mut impl HuffmanBits,
        max_length: usize,
    ) -> Result<usize, LzxError> {
        debug_assert!(!self.empty);
        debug_assert_eq!(self.max_length, max_length);
        bits.ensure(max_length);
        self.read_ready(bits, max_length)
    }

    #[inline]
    fn read_ready(
        &self,
        bits: &mut impl HuffmanBits,
        max_length: usize,
    ) -> Result<usize, LzxError> {
        // Replicated short entries make unused lookahead bits irrelevant, even
        // when fewer than FAST_BITS remain at the end of a strict CAB frame.
        let entry = self.fast[bits.peek(FAST_BITS) as usize];
        if entry != 0 {
            bits.remove((entry & 31) as usize);
            return Ok((entry >> 5) as usize);
        }
        self.read_long(bits.peek(max_length), bits)
    }

    #[inline(never)]
    fn read_long(&self, prefix: u32, bits: &mut impl HuffmanBits) -> Result<usize, LzxError> {
        for length in FAST_BITS + 1..=self.max_length {
            let value = prefix >> (self.max_length - length);
            if value >= self.first_code[length] {
                let ordinal = (value - self.first_code[length]) as usize;
                if ordinal < self.count[length] {
                    bits.remove(length);
                    return Ok(self.symbols[self.first_symbol[length] + ordinal]);
                }
            }
        }
        Err(LzxError::InvalidHuffmanCode)
    }
}

// CAB and WIM share canonical codes, but CAB requires strict input bounds.
trait HuffmanBits {
    fn ensure(&mut self, count: usize);
    fn peek(&self, count: usize) -> u32;
    fn remove(&mut self, count: usize);

    fn read(&mut self, count: usize) -> u32 {
        self.ensure(count);
        let value = self.peek(count);
        self.remove(count);
        value
    }
}

struct Bits<'a> {
    input: &'a [u8],
    next: usize,
    buffer: u32,
    remaining: usize,
}

impl HuffmanBits for Bits<'_> {
    fn ensure(&mut self, count: usize) {
        self.ensure(count);
    }

    fn peek(&self, count: usize) -> u32 {
        self.peek(count)
    }

    fn remove(&mut self, count: usize) {
        self.remove(count);
    }
}

impl<'a> Bits<'a> {
    fn new(input: &'a [u8]) -> Self {
        Self {
            input,
            next: 0,
            buffer: 0,
            remaining: 0,
        }
    }

    fn ensure(&mut self, count: usize) {
        if self.remaining >= count {
            return;
        }
        if let Some(word) = self.input[self.next..].get(..2) {
            self.buffer |=
                u32::from(u16::from_le_bytes([word[0], word[1]])) << (16 - self.remaining);
            self.next += 2;
            self.remaining += 16;
        } else {
            self.remaining = 32;
            return;
        }
        if count == 17 && self.remaining == 16 {
            if let Some(word) = self.input[self.next..].get(..2) {
                self.buffer |= u32::from(u16::from_le_bytes([word[0], word[1]]));
                self.next += 2;
            }
            self.remaining = 32;
        }
    }

    fn peek(&self, count: usize) -> u32 {
        if count == 0 {
            0
        } else {
            self.buffer >> (32 - count)
        }
    }
    fn remove(&mut self, count: usize) {
        self.buffer <<= count;
        self.remaining -= count;
    }
    fn pop(&mut self, count: usize) -> u32 {
        let value = self.peek(count);
        self.remove(count);
        value
    }
    fn read(&mut self, count: usize) -> u32 {
        self.ensure(count);
        self.pop(count)
    }
    fn align(&mut self) {
        self.buffer = 0;
        self.remaining = 0;
    }
    fn literal_byte(&mut self) -> u8 {
        if let Some(&byte) = self.input.get(self.next) {
            self.next += 1;
            byte
        } else {
            0
        }
    }
    fn literal_u32(&mut self) -> u32 {
        if let Some(bytes) = self.input[self.next..].get(..4) {
            self.next += 4;
            u32::from_le_bytes([bytes[0], bytes[1], bytes[2], bytes[3]])
        } else {
            0
        }
    }
    fn literal_bytes(&mut self, count: usize) -> Result<&'a [u8], LzxError> {
        let bytes = self.input[self.next..]
            .get(..count)
            .ok_or(LzxError::TruncatedUncompressedBlock)?;
        self.next += count;
        Ok(bytes)
    }
}

#[cfg(test)]
mod lookup_regression {
    use super::*;
    use alloc::vec::Vec;

    struct Codeword {
        bits: u32,
        remaining: usize,
    }
    impl HuffmanBits for Codeword {
        fn ensure(&mut self, _: usize) {}
        fn peek(&self, count: usize) -> u32 {
            self.bits >> (32 - count)
        }
        fn remove(&mut self, count: usize) {
            self.bits <<= count;
            self.remaining -= count;
        }
    }

    #[test]
    fn short_lookup_replicates_every_unused_suffix_without_consuming_it() {
        let code = Code::<2>::new(&[1, 1], 1).unwrap();
        for symbol in 0..2 {
            for suffix in 0..1 << (FAST_BITS - 1) {
                let mut bits = Codeword {
                    bits: ((symbol as u32) << 31) | ((suffix as u32) << (32 - FAST_BITS)),
                    remaining: 1,
                };
                assert_eq!(code.read(&mut bits).unwrap(), symbol);
                assert_eq!(bits.remaining, 0);
            }
        }
    }

    #[test]
    fn fast_and_long_canonical_codewords_consume_exact_lengths() {
        // Complete canonical tree with one code at each length 1..15 and
        // two at length 16. This crosses the lookup/fallback boundary and
        // has independently known codewords: 0, 10, 110, ..., 111...110/1.
        let lengths: Vec<u8> = (1..=16).chain([16]).collect();
        let code = Code::<17>::new(&lengths, 16).unwrap();
        for (symbol, &length) in lengths.iter().enumerate() {
            let length = usize::from(length);
            let word = if symbol == 16 {
                65535
            } else {
                (1u32 << length) - 2
            };
            let mut bits = Codeword {
                bits: word << (32 - length),
                remaining: 16,
            };
            assert_eq!(code.read(&mut bits).unwrap(), symbol);
            assert_eq!(bits.remaining, 16 - length);
            let mut fixed_bits = Codeword {
                bits: word << (32 - length),
                remaining: 16,
            };
            assert_eq!(code.read_nonempty(&mut fixed_bits, 16).unwrap(), symbol);
            assert_eq!(fixed_bits.remaining, 16 - length);
        }
    }
}
