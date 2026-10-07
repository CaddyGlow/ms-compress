// SPDX-License-Identifier: LGPL-2.1-or-later
// Copyright (C) 2013-2016 Eric Biggers
// Rust translation of wimlib src/lzms_decompress.c, src/lzms_common.c,
// src/compress_common.c and include/wimlib/lzms_constants.h (wimlib 1.14.5).
//! Safe raw-block LZMS decoding with adaptive Huffman and binary range coding.

use crate::memory::resize_within_capacity;
use alloc::vec::Vec;

#[path = "lzms_encode.rs"]
pub mod encode;

/// A malformed or unsupported raw LZMS block.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LzmsError {
    /// Input must contain at least two little-endian 16-bit words.
    InvalidInputSize,
    /// Decoder-owned storage could not be allocated.
    OutOfMemory,
    /// LZMS limits an uncompressed block to 2^30 bytes.
    OutputTooLarge,
    /// An adaptive Huffman alphabet or code is invalid.
    InvalidHuffmanCode,
    /// A match source lies before the produced output.
    InvalidMatchOffset,
    /// A match would extend beyond the requested output slice.
    MatchExceedsOutput,
}
impl core::fmt::Display for LzmsError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        write!(f, "invalid LZMS block: {self:?}")
    }
}
impl core::error::Error for LzmsError {}

fn filled_buffer<T: Clone>(size: usize, value: T) -> Result<Vec<T>, LzmsError> {
    crate::memory::filled_vec(size, value).map_err(|_| LzmsError::OutOfMemory)
}

struct ReverseBits<'a> {
    input: &'a [u8],
    next: usize,
    buffer: u64,
    available: u32,
}
impl<'a> ReverseBits<'a> {
    fn new(input: &'a [u8]) -> Self {
        Self {
            input,
            next: input.len(),
            buffer: 0,
            available: 0,
        }
    }
    fn ensure(&mut self, count: u32) {
        while self.available < count {
            let word = if self.next >= 2 {
                self.next -= 2;
                u16::from_le_bytes([self.input[self.next], self.input[self.next + 1]])
            } else {
                0
            };
            self.buffer |= u64::from(word) << (48 - self.available);
            self.available += 16;
        }
    }
    fn peek(&mut self, count: u32) -> u32 {
        if count == 0 {
            return 0;
        }
        self.ensure(count);
        (self.buffer >> (64 - count)) as u32
    }
    fn consume(&mut self, count: u32) {
        self.buffer <<= count;
        self.available -= count;
    }
    fn read(&mut self, count: u32) -> u32 {
        let value = self.peek(count);
        self.consume(count);
        value
    }
}

#[derive(Debug)]
struct X86FilterState {
    targets: Vec<i32>,
    next_base: i32,
}
impl X86FilterState {
    fn new() -> Result<Self, LzmsError> {
        Ok(Self {
            targets: filled_buffer(65536, -65536)?,
            next_base: 0,
        })
    }
    fn begin(&mut self, length: usize) -> i32 {
        // A gap of 65536 makes every prior-block timestamp ineligible.
        // Clear only before the signed counter would wrap.
        let span = length as i32 + 65536;
        if self.next_base > i32::MAX - span {
            self.targets.fill(-65536);
            self.next_base = 0;
        }
        let base = self.next_base;
        self.next_base += span;
        base
    }
}

fn x86_opcode_in_word(bytes: &[u8]) -> bool {
    let word = u64::from_ne_bytes([
        bytes[0], bytes[1], bytes[2], bytes[3], bytes[4], bytes[5], bytes[6], bytes[7],
    ]);
    let zero =
        |value: u64| value.wrapping_sub(0x0101_0101_0101_0101) & !value & 0x8080_8080_8080_8080;
    // Masked pairs recognize 48/4c and e8/e9. False positives only cause a
    // scalar scan; the word probe never skips a possible instruction.
    zero(word ^ 0xffff_ffff_ffff_ffff)
        | zero(word ^ 0xf0f0_f0f0_f0f0_f0f0)
        | zero((word & 0xfbfb_fbfb_fbfb_fbfb) ^ 0x4848_4848_4848_4848)
        | zero((word & 0xfefe_fefe_fefe_fefe) ^ 0xe8e8_e8e8_e8e8_e8e8)
        != 0
}

#[derive(Clone, Copy, Debug)]
struct Probability {
    zeros: u32,
    recent: u64,
}
impl Default for Probability {
    fn default() -> Self {
        Self {
            zeros: 48,
            recent: 0x55555555,
        }
    }
}
#[derive(Debug)]
struct Decision {
    state: usize,
    probabilities: Vec<Probability>,
}
impl Decision {
    fn new(states: usize) -> Result<Self, LzmsError> {
        Ok(Self {
            state: 0,
            probabilities: filled_buffer(states, Probability::default())?,
        })
    }
}
struct RangeDecoder<'a> {
    input: &'a [u8],
    next: usize,
    range: u32,
    code: u32,
}
impl<'a> RangeDecoder<'a> {
    fn new(input: &'a [u8]) -> Self {
        let code = (u32::from(u16::from_le_bytes([input[0], input[1]])) << 16)
            | u32::from(u16::from_le_bytes([input[2], input[3]]));
        Self {
            input,
            next: 4,
            range: u32::MAX,
            code,
        }
    }
    fn bit(&mut self, decision: &mut Decision) -> bool {
        let entry = &mut decision.probabilities[decision.state];
        let prob = entry.zeros.clamp(1, 63);
        if self.range >> 16 == 0 {
            self.range <<= 16;
            self.code <<= 16;
            if self.next < self.input.len() {
                self.code |= u32::from(u16::from_le_bytes([
                    self.input[self.next],
                    self.input[self.next + 1],
                ]));
                self.next += 2;
            }
        }
        let bound = (self.range >> 6) * prob;
        let bit = self.code >= bound;
        if bit {
            self.range -= bound;
            self.code -= bound;
        } else {
            self.range = bound;
        }
        entry.zeros = entry.zeros + (entry.recent >> 63) as u32 - u32::from(bit);
        entry.recent = (entry.recent << 1) | u64::from(bit);
        decision.state =
            ((decision.state << 1) | usize::from(bit)) & (decision.probabilities.len() - 1);
        bit
    }
}

#[derive(Debug)]
struct Huffman {
    frequencies: Vec<u32>,
    table: Vec<u16>,
    remaining: usize,
    period: usize,
    internal_weights: Vec<u32>,
    parents: Vec<usize>,
    sorted: Vec<usize>,
    lengths: Vec<u32>,
}
impl Huffman {
    fn new(symbols: usize, period: usize) -> Result<Self, LzmsError> {
        Self::with_tables(symbols, period, true)
    }
    fn for_encoding(symbols: usize, period: usize) -> Result<Self, LzmsError> {
        Self::with_tables(symbols, period, false)
    }
    fn with_tables(symbols: usize, period: usize, decoding: bool) -> Result<Self, LzmsError> {
        let mut code = Self {
            frequencies: filled_buffer(symbols, 1)?,
            table: filled_buffer(if decoding { 1 << 15 } else { 0 }, 0)?,
            remaining: period,
            period,
            internal_weights: filled_buffer(symbols, 0)?,
            parents: filled_buffer(symbols.saturating_mul(2).saturating_sub(1), usize::MAX)?,
            sorted: filled_buffer(symbols, 0)?,
            lengths: filled_buffer(symbols, 0)?,
        };
        code.rebuild()?;
        Ok(code)
    }
    fn reset(&mut self, symbols: usize) -> Result<(), LzmsError> {
        resize_within_capacity(&mut self.frequencies, symbols, 1)
            .map_err(|_| LzmsError::OutOfMemory)?;
        self.frequencies.fill(1);
        self.remaining = self.period;
        self.rebuild()
    }
    fn rebuild(&mut self) -> Result<(), LzmsError> {
        let symbols = self.frequencies.len();
        if symbols == 0 {
            self.table.fill(0);
            return Ok(());
        }
        if symbols == 1 {
            // Upstream synthesizes a second 1-bit symbol for a one-symbol code.
            if !self.table.is_empty() {
                self.table[..1 << 14].fill(1);
                self.table[1 << 14..].fill(17);
            }
            resize_within_capacity(&mut self.lengths, 1, 1).map_err(|_| LzmsError::OutOfMemory)?;
            self.lengths[0] = 1;
            self.remaining = self.period;
            return Ok(());
        }
        resize_within_capacity(&mut self.sorted, symbols, 0).map_err(|_| LzmsError::OutOfMemory)?;
        for (symbol, value) in self.sorted.iter_mut().enumerate() {
            *value = symbol;
        }
        let uniform = self
            .frequencies
            .iter()
            .all(|&frequency| frequency == self.frequencies[0]);
        let mut counts = [0u32; 16];
        if uniform {
            // Equal weights form a balanced tree; avoid rebuilding it for every
            // independent block reset. Low symbols retain the longer codes.
            let short = symbols.ilog2() as usize;
            let shorter = (1usize << (short + 1)) - symbols;
            counts[short] = shorter as u32;
            counts[short + 1] = (symbols - shorter) as u32;
        } else {
            // Leaf/internal ties prefer leaves; equal leaves prefer smaller symbols;
            // equal internal nodes prefer their creation order.
            self.sorted
                .sort_unstable_by_key(|&symbol| (self.frequencies[symbol], symbol));
            resize_within_capacity(&mut self.parents, symbols * 2 - 1, usize::MAX)
                .map_err(|_| LzmsError::OutOfMemory)?;
            resize_within_capacity(&mut self.internal_weights, symbols, 0)
                .map_err(|_| LzmsError::OutOfMemory)?;
            let mut leaf = 0;
            let mut internal = 0;
            for node in symbols..self.parents.len() {
                let built = node - symbols;
                let mut take = || {
                    if leaf < symbols
                        && (internal == built
                            || self.frequencies[self.sorted[leaf]]
                                <= self.internal_weights[internal])
                    {
                        let symbol = self.sorted[leaf];
                        leaf += 1;
                        (self.frequencies[symbol], symbol)
                    } else {
                        let index = internal;
                        internal += 1;
                        (self.internal_weights[index], symbols + index)
                    }
                };
                let (left_weight, left) = take();
                let (right_weight, right) = take();
                self.parents[left] = node;
                self.parents[right] = node;
                self.internal_weights[built] = left_weight + right_weight;
            }
            // Exact upstream compute_length_counts() algorithm. Valid LZMS
            // frequency schedules already limit depth to 15; retaining its bounded
            // length construction also handles arbitrary frequencies identically.
            let parents = &mut self.parents;
            counts[1] = 2;
            let root = parents.len() - 1;
            parents[root] = 0;
            for node in (symbols..root).rev() {
                let depth = parents[parents[node]] + 1;
                parents[node] = depth;
                let mut length = depth;
                if length >= 15 {
                    length = 14;
                    while counts[length] == 0 && length > 0 {
                        length -= 1;
                    }
                }
                if length == 0 || counts[length] == 0 {
                    return Err(LzmsError::InvalidHuffmanCode);
                }
                counts[length] -= 1;
                counts[length + 1] += 2;
            }
        }
        // wimlib assigns decreasing lengths to ascending (frequency,symbol),
        // using the tree solely to derive counts of each length.
        resize_within_capacity(&mut self.lengths, symbols, 0)
            .map_err(|_| LzmsError::OutOfMemory)?;
        self.lengths.fill(0);
        let sorted = &self.sorted;
        let lengths = &mut self.lengths;
        let mut index = 0;
        for length in (1..=15).rev() {
            for _ in 0..counts[length] {
                lengths[sorted[index]] = length as u32;
                index += 1;
            }
        }
        // Encoders consume canonical lengths, never the expanded decode table.
        if self.table.is_empty() {
            self.remaining = self.period;
            return Ok(());
        }
        let mut next = [0u32; 16];
        for length in 2..=15 {
            next[length] = (next[length - 1] + counts[length - 1]) << 1;
        }
        for (symbol, &length) in lengths.iter().enumerate() {
            let start = (next[length as usize] << (15 - length)) as usize;
            let end = start + (1 << (15 - length));
            if end > self.table.len() {
                return Err(LzmsError::InvalidHuffmanCode);
            }
            self.table[start..end].fill(((symbol as u16) << 4) | length as u16);
            next[length as usize] += 1;
        }
        self.remaining = self.period;
        Ok(())
    }
    fn symbol(&mut self, bits: &mut ReverseBits<'_>) -> Result<usize, LzmsError> {
        let entry = self.table[bits.peek(15) as usize];
        let length = u32::from(entry & 15);
        let symbol = usize::from(entry >> 4);
        if length == 0 || symbol >= self.frequencies.len() {
            return Err(LzmsError::InvalidHuffmanCode);
        }
        bits.consume(length);
        self.frequencies[symbol] += 1;
        self.remaining -= 1;
        if self.remaining == 0 {
            self.rebuild()?;
            for frequency in self.frequencies.iter_mut() {
                *frequency = (*frequency >> 1) + 1;
            }
        }
        Ok(symbol)
    }
    fn value(
        &mut self,
        bits: &mut ReverseBits<'_>,
        bases: &[u32],
        extra: &[u32],
    ) -> Result<u32, LzmsError> {
        let slot = self.symbol(bits)?;
        Ok(bases[slot] + bits.read(extra[slot]))
    }
}

fn repeat<T: Copy>(queue: &mut [T; 4], index: usize, previous_same: bool) -> T {
    let adjusted = index + usize::from(previous_same);
    let value = queue[adjusted];
    queue[adjusted] = queue[index];
    for position in (1..=index).rev() {
        queue[position] = queue[position - 1];
    }
    value
}
fn insert<T: Copy>(queue: &mut [T; 4]) {
    for index in (1..4).rev() {
        queue[index] = queue[index - 1];
    }
}
fn rep_index(range: &mut RangeDecoder<'_>, first: &mut Decision, second: &mut Decision) -> usize {
    if !range.bit(first) {
        0
    } else if !range.bit(second) {
        1
    } else {
        2
    }
}

/// Decompresses a single raw LZMS block into the caller's exact output size.
///
/// The offset alphabet derives from `output.len()`, independently of any WIM
/// chunk setting. Input word exhaustion supplies zero bits, as in upstream.
/// Output may be partially modified if decoding fails.
///
/// # Errors
/// Returns [`LzmsError`] for odd/short input, oversized output, invalid matches,
/// or decoder storage allocation failure.
pub fn decompress_lzms(input: &[u8], output: &mut [u8]) -> Result<(), LzmsError> {
    // Reject invalid input before allocating, preserving standalone behavior.
    if input.len() < 4 || input.len() & 1 != 0 {
        return Err(LzmsError::InvalidInputSize);
    }
    if output.len() > 1 << 30 {
        return Err(LzmsError::OutputTooLarge);
    }
    LzmsDecoder::new()?.decompress(input, output)
}

/// Reusable LZMS scratch storage. All heap allocation happens in [`Self::new`].
/// Independent blocks reset adaptive codes, probabilities, and x86 history.
#[derive(Debug)]
pub struct LzmsDecoder {
    literals: Huffman,
    lengths: Huffman,
    offsets: Huffman,
    delta_offsets: Huffman,
    powers: Huffman,
    decisions: [Decision; 8],
    last_target: X86FilterState,
}
impl LzmsDecoder {
    /// Allocate every bounded decoder workspace, returning an error on failure.
    pub fn new() -> Result<Self, LzmsError> {
        Ok(Self {
            literals: Huffman::new(256, 1024)?,
            lengths: Huffman::new(54, 512)?,
            offsets: Huffman::new(OFFSET_BASES.len(), 1024)?,
            delta_offsets: Huffman::new(OFFSET_BASES.len(), 1024)?,
            powers: Huffman::new(8, 512)?,
            decisions: [
                Decision::new(16)?,
                Decision::new(32)?,
                Decision::new(64)?,
                Decision::new(64)?,
                Decision::new(64)?,
                Decision::new(64)?,
                Decision::new(64)?,
                Decision::new(64)?,
            ],
            last_target: X86FilterState::new()?,
        })
    }
    /// Decode one independent block without allocating, even on malformed input.
    /// Output can be partially modified on failure.
    pub fn decompress(&mut self, input: &[u8], output: &mut [u8]) -> Result<(), LzmsError> {
        if input.len() < 4 || input.len() & 1 != 0 {
            return Err(LzmsError::InvalidInputSize);
        }
        if output.len() > 1 << 30 {
            return Err(LzmsError::OutputTooLarge);
        }
        let slots = if output.len() < 2 {
            0
        } else {
            OFFSET_BASES.partition_point(|&base| base <= (output.len() - 1) as u32)
        };
        self.literals.reset(256)?;
        self.lengths.reset(54)?;
        self.offsets.reset(slots)?;
        self.delta_offsets.reset(slots)?;
        self.powers.reset(8)?;
        for decision in &mut self.decisions {
            decision.state = 0;
            decision.probabilities.fill(Probability::default());
        }
        let Self {
            literals,
            lengths,
            offsets,
            delta_offsets,
            powers,
            decisions,
            last_target,
        } = self;
        let [
            main,
            matched,
            lz,
            delta,
            lz_rep0,
            lz_rep1,
            delta_rep0,
            delta_rep1,
        ] = decisions;
        let mut bits = ReverseBits::new(input);
        let mut range = RangeDecoder::new(input);
        let mut recent_lz = [1u32, 2, 3, 4];
        let mut recent_delta = [(0u32, 1u32), (0, 2), (0, 3), (0, 4)];
        let mut previous = 0;
        let mut produced = 0;
        while produced < output.len() {
            if !range.bit(main) {
                output[produced] = literals.symbol(&mut bits)? as u8;
                produced += 1;
                previous = 0;
            } else if !range.bit(matched) {
                let offset = if !range.bit(lz) {
                    let value = offsets.value(&mut bits, OFFSET_BASES, EXTRA_OFFSET_BITS)?;
                    insert(&mut recent_lz);
                    value
                } else {
                    let index = rep_index(&mut range, lz_rep0, lz_rep1);
                    repeat(&mut recent_lz, index, previous == 1)
                };
                recent_lz[0] = offset;
                previous = 1;
                let length = lengths.value(&mut bits, LENGTH_BASES, EXTRA_LENGTH_BITS)? as usize;
                let offset = offset as usize;
                if offset == 0 || offset > produced {
                    return Err(LzmsError::InvalidMatchOffset);
                }
                if length > output.len() - produced {
                    return Err(LzmsError::MatchExceedsOutput);
                }
                crate::copy_lz_match(output, produced, offset, length);
                produced += length;
            } else {
                let pair = if !range.bit(delta) {
                    let power = powers.symbol(&mut bits)? as u32;
                    let offset = delta_offsets.value(&mut bits, OFFSET_BASES, EXTRA_OFFSET_BITS)?;
                    insert(&mut recent_delta);
                    (power, offset)
                } else {
                    let index = rep_index(&mut range, delta_rep0, delta_rep1);
                    repeat(&mut recent_delta, index, previous == 2)
                };
                recent_delta[0] = pair;
                previous = 2;
                let length = lengths.value(&mut bits, LENGTH_BASES, EXTRA_LENGTH_BITS)? as usize;
                let (power, raw_offset) = pair;
                copy_delta(output, produced, length, power, raw_offset)?;
                produced += length;
            }
        }
        undo_x86_filter_with_workspace(output, last_target);
        Ok(())
    }
}

fn copy_delta(
    output: &mut [u8],
    mut produced: usize,
    length: usize,
    power: u32,
    raw_offset: u32,
) -> Result<(), LzmsError> {
    let span = 1u32
        .checked_shl(power)
        .ok_or(LzmsError::InvalidMatchOffset)?;
    let offset = raw_offset
        .checked_mul(span)
        .ok_or(LzmsError::InvalidMatchOffset)?;
    let required = offset
        .checked_add(span)
        .ok_or(LzmsError::InvalidMatchOffset)?;
    if raw_offset == 0 || required as usize > produced {
        return Err(LzmsError::InvalidMatchOffset);
    }
    let remaining = output
        .len()
        .checked_sub(produced)
        .ok_or(LzmsError::MatchExceedsOutput)?;
    if length > remaining {
        return Err(LzmsError::MatchExceedsOutput);
    }
    let offset = offset as usize;
    let span = span as usize;
    for _ in 0..length {
        output[produced] = output[produced - offset]
            .wrapping_add(output[produced - span])
            .wrapping_sub(output[produced - offset - span]);
        produced += 1;
    }
    Ok(())
}

#[cfg(test)]
fn undo_x86_filter(data: &mut [u8]) -> Result<(), LzmsError> {
    let mut last_target = X86FilterState::new()?;
    undo_x86_filter_with_workspace(data, &mut last_target);
    Ok(())
}
fn undo_x86_filter_with_workspace(data: &mut [u8], state: &mut X86FilterState) {
    if data.len() <= 17 {
        return;
    }
    let base = state.begin(data.len());
    let last_target = &mut state.targets;
    let mut last_x86 = -1024i32;
    let mut position = 1usize;
    while position < data.len() - 16 {
        if !x86_opcode_in_word(&data[position..position + 8]) {
            position += 8;
            continue;
        }
        let byte = data[position];
        let mut max_offset = 1023;
        let opcode_length = match byte {
            0xff if data[position + 1] == 0x15 => 2,
            0xf0 if data[position + 1] == 0x83 && data[position + 2] == 0x05 => 3,
            0x48 | 0x4c
                if data[position + 2] & 7 == 5
                    && (data[position + 1] == 0x8d
                        || (data[position + 1] == 0x8b
                            && byte & 4 == 0
                            && data[position + 2] & 0xf0 == 0)) =>
            {
                3
            }
            0xe8 => {
                max_offset >>= 1;
                1
            }
            0xe9 => {
                position += 5;
                continue;
            }
            _ => {
                position += 1;
                continue;
            }
        };
        let index = position as i32;
        let start = position + opcode_length;
        if index - last_x86 <= max_offset {
            let value = u32::from_le_bytes([
                data[start],
                data[start + 1],
                data[start + 2],
                data[start + 3],
            ]);
            data[start..start + 4].copy_from_slice(&value.wrapping_sub(index as u32).to_le_bytes());
        }
        let target = ((index as u32).wrapping_add(u32::from(u16::from_le_bytes([
            data[start],
            data[start + 1],
        ]))) & 0xffff) as usize;
        let instruction_end = index + opcode_length as i32 + 3;
        let stamp = base + instruction_end;
        if last_target[target] >= base && stamp - last_target[target] <= 65535 {
            last_x86 = instruction_end;
        }
        last_target[target] = stamp;
        position = start + 4;
    }
}

const OFFSET_BASES: &[u32] = &[
    0x00000001, 0x00000002, 0x00000003, 0x00000004, 0x00000005, 0x00000006, 0x00000007, 0x00000008,
    0x00000009, 0x0000000d, 0x00000011, 0x00000015, 0x00000019, 0x0000001d, 0x00000021, 0x00000025,
    0x00000029, 0x0000002d, 0x00000035, 0x0000003d, 0x00000045, 0x0000004d, 0x00000055, 0x0000005d,
    0x00000065, 0x00000075, 0x00000085, 0x00000095, 0x000000a5, 0x000000b5, 0x000000c5, 0x000000d5,
    0x000000e5, 0x000000f5, 0x00000105, 0x00000125, 0x00000145, 0x00000165, 0x00000185, 0x000001a5,
    0x000001c5, 0x000001e5, 0x00000205, 0x00000225, 0x00000245, 0x00000265, 0x00000285, 0x000002a5,
    0x000002c5, 0x000002e5, 0x00000325, 0x00000365, 0x000003a5, 0x000003e5, 0x00000425, 0x00000465,
    0x000004a5, 0x000004e5, 0x00000525, 0x00000565, 0x000005a5, 0x000005e5, 0x00000625, 0x00000665,
    0x000006a5, 0x00000725, 0x000007a5, 0x00000825, 0x000008a5, 0x00000925, 0x000009a5, 0x00000a25,
    0x00000aa5, 0x00000b25, 0x00000ba5, 0x00000c25, 0x00000ca5, 0x00000d25, 0x00000da5, 0x00000e25,
    0x00000ea5, 0x00000f25, 0x00000fa5, 0x00001025, 0x000010a5, 0x000011a5, 0x000012a5, 0x000013a5,
    0x000014a5, 0x000015a5, 0x000016a5, 0x000017a5, 0x000018a5, 0x000019a5, 0x00001aa5, 0x00001ba5,
    0x00001ca5, 0x00001da5, 0x00001ea5, 0x00001fa5, 0x000020a5, 0x000021a5, 0x000022a5, 0x000023a5,
    0x000024a5, 0x000026a5, 0x000028a5, 0x00002aa5, 0x00002ca5, 0x00002ea5, 0x000030a5, 0x000032a5,
    0x000034a5, 0x000036a5, 0x000038a5, 0x00003aa5, 0x00003ca5, 0x00003ea5, 0x000040a5, 0x000042a5,
    0x000044a5, 0x000046a5, 0x000048a5, 0x00004aa5, 0x00004ca5, 0x00004ea5, 0x000050a5, 0x000052a5,
    0x000054a5, 0x000056a5, 0x000058a5, 0x00005aa5, 0x00005ca5, 0x00005ea5, 0x000060a5, 0x000064a5,
    0x000068a5, 0x00006ca5, 0x000070a5, 0x000074a5, 0x000078a5, 0x00007ca5, 0x000080a5, 0x000084a5,
    0x000088a5, 0x00008ca5, 0x000090a5, 0x000094a5, 0x000098a5, 0x00009ca5, 0x0000a0a5, 0x0000a4a5,
    0x0000a8a5, 0x0000aca5, 0x0000b0a5, 0x0000b4a5, 0x0000b8a5, 0x0000bca5, 0x0000c0a5, 0x0000c4a5,
    0x0000c8a5, 0x0000cca5, 0x0000d0a5, 0x0000d4a5, 0x0000d8a5, 0x0000dca5, 0x0000e0a5, 0x0000e4a5,
    0x0000eca5, 0x0000f4a5, 0x0000fca5, 0x000104a5, 0x00010ca5, 0x000114a5, 0x00011ca5, 0x000124a5,
    0x00012ca5, 0x000134a5, 0x00013ca5, 0x000144a5, 0x00014ca5, 0x000154a5, 0x00015ca5, 0x000164a5,
    0x00016ca5, 0x000174a5, 0x00017ca5, 0x000184a5, 0x00018ca5, 0x000194a5, 0x00019ca5, 0x0001a4a5,
    0x0001aca5, 0x0001b4a5, 0x0001bca5, 0x0001c4a5, 0x0001cca5, 0x0001d4a5, 0x0001dca5, 0x0001e4a5,
    0x0001eca5, 0x0001f4a5, 0x0001fca5, 0x000204a5, 0x00020ca5, 0x000214a5, 0x00021ca5, 0x000224a5,
    0x000234a5, 0x000244a5, 0x000254a5, 0x000264a5, 0x000274a5, 0x000284a5, 0x000294a5, 0x0002a4a5,
    0x0002b4a5, 0x0002c4a5, 0x0002d4a5, 0x0002e4a5, 0x0002f4a5, 0x000304a5, 0x000314a5, 0x000324a5,
    0x000334a5, 0x000344a5, 0x000354a5, 0x000364a5, 0x000374a5, 0x000384a5, 0x000394a5, 0x0003a4a5,
    0x0003b4a5, 0x0003c4a5, 0x0003d4a5, 0x0003e4a5, 0x0003f4a5, 0x000404a5, 0x000414a5, 0x000424a5,
    0x000434a5, 0x000444a5, 0x000454a5, 0x000464a5, 0x000474a5, 0x000484a5, 0x000494a5, 0x0004a4a5,
    0x0004b4a5, 0x0004c4a5, 0x0004e4a5, 0x000504a5, 0x000524a5, 0x000544a5, 0x000564a5, 0x000584a5,
    0x0005a4a5, 0x0005c4a5, 0x0005e4a5, 0x000604a5, 0x000624a5, 0x000644a5, 0x000664a5, 0x000684a5,
    0x0006a4a5, 0x0006c4a5, 0x0006e4a5, 0x000704a5, 0x000724a5, 0x000744a5, 0x000764a5, 0x000784a5,
    0x0007a4a5, 0x0007c4a5, 0x0007e4a5, 0x000804a5, 0x000824a5, 0x000844a5, 0x000864a5, 0x000884a5,
    0x0008a4a5, 0x0008c4a5, 0x0008e4a5, 0x000904a5, 0x000924a5, 0x000944a5, 0x000964a5, 0x000984a5,
    0x0009a4a5, 0x0009c4a5, 0x0009e4a5, 0x000a04a5, 0x000a24a5, 0x000a44a5, 0x000a64a5, 0x000aa4a5,
    0x000ae4a5, 0x000b24a5, 0x000b64a5, 0x000ba4a5, 0x000be4a5, 0x000c24a5, 0x000c64a5, 0x000ca4a5,
    0x000ce4a5, 0x000d24a5, 0x000d64a5, 0x000da4a5, 0x000de4a5, 0x000e24a5, 0x000e64a5, 0x000ea4a5,
    0x000ee4a5, 0x000f24a5, 0x000f64a5, 0x000fa4a5, 0x000fe4a5, 0x001024a5, 0x001064a5, 0x0010a4a5,
    0x0010e4a5, 0x001124a5, 0x001164a5, 0x0011a4a5, 0x0011e4a5, 0x001224a5, 0x001264a5, 0x0012a4a5,
    0x0012e4a5, 0x001324a5, 0x001364a5, 0x0013a4a5, 0x0013e4a5, 0x001424a5, 0x001464a5, 0x0014a4a5,
    0x0014e4a5, 0x001524a5, 0x001564a5, 0x0015a4a5, 0x0015e4a5, 0x001624a5, 0x001664a5, 0x0016a4a5,
    0x0016e4a5, 0x001724a5, 0x001764a5, 0x0017a4a5, 0x0017e4a5, 0x001824a5, 0x001864a5, 0x0018a4a5,
    0x0018e4a5, 0x001924a5, 0x001964a5, 0x0019e4a5, 0x001a64a5, 0x001ae4a5, 0x001b64a5, 0x001be4a5,
    0x001c64a5, 0x001ce4a5, 0x001d64a5, 0x001de4a5, 0x001e64a5, 0x001ee4a5, 0x001f64a5, 0x001fe4a5,
    0x002064a5, 0x0020e4a5, 0x002164a5, 0x0021e4a5, 0x002264a5, 0x0022e4a5, 0x002364a5, 0x0023e4a5,
    0x002464a5, 0x0024e4a5, 0x002564a5, 0x0025e4a5, 0x002664a5, 0x0026e4a5, 0x002764a5, 0x0027e4a5,
    0x002864a5, 0x0028e4a5, 0x002964a5, 0x0029e4a5, 0x002a64a5, 0x002ae4a5, 0x002b64a5, 0x002be4a5,
    0x002c64a5, 0x002ce4a5, 0x002d64a5, 0x002de4a5, 0x002e64a5, 0x002ee4a5, 0x002f64a5, 0x002fe4a5,
    0x003064a5, 0x0030e4a5, 0x003164a5, 0x0031e4a5, 0x003264a5, 0x0032e4a5, 0x003364a5, 0x0033e4a5,
    0x003464a5, 0x0034e4a5, 0x003564a5, 0x0035e4a5, 0x003664a5, 0x0036e4a5, 0x003764a5, 0x0037e4a5,
    0x003864a5, 0x0038e4a5, 0x003964a5, 0x0039e4a5, 0x003a64a5, 0x003ae4a5, 0x003b64a5, 0x003be4a5,
    0x003c64a5, 0x003ce4a5, 0x003d64a5, 0x003de4a5, 0x003ee4a5, 0x003fe4a5, 0x0040e4a5, 0x0041e4a5,
    0x0042e4a5, 0x0043e4a5, 0x0044e4a5, 0x0045e4a5, 0x0046e4a5, 0x0047e4a5, 0x0048e4a5, 0x0049e4a5,
    0x004ae4a5, 0x004be4a5, 0x004ce4a5, 0x004de4a5, 0x004ee4a5, 0x004fe4a5, 0x0050e4a5, 0x0051e4a5,
    0x0052e4a5, 0x0053e4a5, 0x0054e4a5, 0x0055e4a5, 0x0056e4a5, 0x0057e4a5, 0x0058e4a5, 0x0059e4a5,
    0x005ae4a5, 0x005be4a5, 0x005ce4a5, 0x005de4a5, 0x005ee4a5, 0x005fe4a5, 0x0060e4a5, 0x0061e4a5,
    0x0062e4a5, 0x0063e4a5, 0x0064e4a5, 0x0065e4a5, 0x0066e4a5, 0x0067e4a5, 0x0068e4a5, 0x0069e4a5,
    0x006ae4a5, 0x006be4a5, 0x006ce4a5, 0x006de4a5, 0x006ee4a5, 0x006fe4a5, 0x0070e4a5, 0x0071e4a5,
    0x0072e4a5, 0x0073e4a5, 0x0074e4a5, 0x0075e4a5, 0x0076e4a5, 0x0077e4a5, 0x0078e4a5, 0x0079e4a5,
    0x007ae4a5, 0x007be4a5, 0x007ce4a5, 0x007de4a5, 0x007ee4a5, 0x007fe4a5, 0x0080e4a5, 0x0081e4a5,
    0x0082e4a5, 0x0083e4a5, 0x0084e4a5, 0x0085e4a5, 0x0086e4a5, 0x0087e4a5, 0x0088e4a5, 0x0089e4a5,
    0x008ae4a5, 0x008be4a5, 0x008ce4a5, 0x008de4a5, 0x008fe4a5, 0x0091e4a5, 0x0093e4a5, 0x0095e4a5,
    0x0097e4a5, 0x0099e4a5, 0x009be4a5, 0x009de4a5, 0x009fe4a5, 0x00a1e4a5, 0x00a3e4a5, 0x00a5e4a5,
    0x00a7e4a5, 0x00a9e4a5, 0x00abe4a5, 0x00ade4a5, 0x00afe4a5, 0x00b1e4a5, 0x00b3e4a5, 0x00b5e4a5,
    0x00b7e4a5, 0x00b9e4a5, 0x00bbe4a5, 0x00bde4a5, 0x00bfe4a5, 0x00c1e4a5, 0x00c3e4a5, 0x00c5e4a5,
    0x00c7e4a5, 0x00c9e4a5, 0x00cbe4a5, 0x00cde4a5, 0x00cfe4a5, 0x00d1e4a5, 0x00d3e4a5, 0x00d5e4a5,
    0x00d7e4a5, 0x00d9e4a5, 0x00dbe4a5, 0x00dde4a5, 0x00dfe4a5, 0x00e1e4a5, 0x00e3e4a5, 0x00e5e4a5,
    0x00e7e4a5, 0x00e9e4a5, 0x00ebe4a5, 0x00ede4a5, 0x00efe4a5, 0x00f1e4a5, 0x00f3e4a5, 0x00f5e4a5,
    0x00f7e4a5, 0x00f9e4a5, 0x00fbe4a5, 0x00fde4a5, 0x00ffe4a5, 0x0101e4a5, 0x0103e4a5, 0x0105e4a5,
    0x0107e4a5, 0x0109e4a5, 0x010be4a5, 0x010de4a5, 0x010fe4a5, 0x0111e4a5, 0x0113e4a5, 0x0115e4a5,
    0x0117e4a5, 0x0119e4a5, 0x011be4a5, 0x011de4a5, 0x011fe4a5, 0x0121e4a5, 0x0123e4a5, 0x0125e4a5,
    0x0127e4a5, 0x0129e4a5, 0x012be4a5, 0x012de4a5, 0x012fe4a5, 0x0131e4a5, 0x0133e4a5, 0x0135e4a5,
    0x0137e4a5, 0x013be4a5, 0x013fe4a5, 0x0143e4a5, 0x0147e4a5, 0x014be4a5, 0x014fe4a5, 0x0153e4a5,
    0x0157e4a5, 0x015be4a5, 0x015fe4a5, 0x0163e4a5, 0x0167e4a5, 0x016be4a5, 0x016fe4a5, 0x0173e4a5,
    0x0177e4a5, 0x017be4a5, 0x017fe4a5, 0x0183e4a5, 0x0187e4a5, 0x018be4a5, 0x018fe4a5, 0x0193e4a5,
    0x0197e4a5, 0x019be4a5, 0x019fe4a5, 0x01a3e4a5, 0x01a7e4a5, 0x01abe4a5, 0x01afe4a5, 0x01b3e4a5,
    0x01b7e4a5, 0x01bbe4a5, 0x01bfe4a5, 0x01c3e4a5, 0x01c7e4a5, 0x01cbe4a5, 0x01cfe4a5, 0x01d3e4a5,
    0x01d7e4a5, 0x01dbe4a5, 0x01dfe4a5, 0x01e3e4a5, 0x01e7e4a5, 0x01ebe4a5, 0x01efe4a5, 0x01f3e4a5,
    0x01f7e4a5, 0x01fbe4a5, 0x01ffe4a5, 0x0203e4a5, 0x0207e4a5, 0x020be4a5, 0x020fe4a5, 0x0213e4a5,
    0x0217e4a5, 0x021be4a5, 0x021fe4a5, 0x0223e4a5, 0x0227e4a5, 0x022be4a5, 0x022fe4a5, 0x0233e4a5,
    0x0237e4a5, 0x023be4a5, 0x023fe4a5, 0x0243e4a5, 0x0247e4a5, 0x024be4a5, 0x024fe4a5, 0x0253e4a5,
    0x0257e4a5, 0x025be4a5, 0x025fe4a5, 0x0263e4a5, 0x0267e4a5, 0x026be4a5, 0x026fe4a5, 0x0273e4a5,
    0x0277e4a5, 0x027be4a5, 0x027fe4a5, 0x0283e4a5, 0x0287e4a5, 0x028be4a5, 0x028fe4a5, 0x0293e4a5,
    0x0297e4a5, 0x029be4a5, 0x029fe4a5, 0x02a3e4a5, 0x02a7e4a5, 0x02abe4a5, 0x02afe4a5, 0x02b3e4a5,
    0x02bbe4a5, 0x02c3e4a5, 0x02cbe4a5, 0x02d3e4a5, 0x02dbe4a5, 0x02e3e4a5, 0x02ebe4a5, 0x02f3e4a5,
    0x02fbe4a5, 0x0303e4a5, 0x030be4a5, 0x0313e4a5, 0x031be4a5, 0x0323e4a5, 0x032be4a5, 0x0333e4a5,
    0x033be4a5, 0x0343e4a5, 0x034be4a5, 0x0353e4a5, 0x035be4a5, 0x0363e4a5, 0x036be4a5, 0x0373e4a5,
    0x037be4a5, 0x0383e4a5, 0x038be4a5, 0x0393e4a5, 0x039be4a5, 0x03a3e4a5, 0x03abe4a5, 0x03b3e4a5,
    0x03bbe4a5, 0x03c3e4a5, 0x03cbe4a5, 0x03d3e4a5, 0x03dbe4a5, 0x03e3e4a5, 0x03ebe4a5, 0x03f3e4a5,
    0x03fbe4a5, 0x0403e4a5, 0x040be4a5, 0x0413e4a5, 0x041be4a5, 0x0423e4a5, 0x042be4a5, 0x0433e4a5,
    0x043be4a5, 0x0443e4a5, 0x044be4a5, 0x0453e4a5, 0x045be4a5, 0x0463e4a5, 0x046be4a5, 0x0473e4a5,
    0x047be4a5, 0x0483e4a5, 0x048be4a5, 0x0493e4a5, 0x049be4a5, 0x04a3e4a5, 0x04abe4a5, 0x04b3e4a5,
    0x04bbe4a5, 0x04c3e4a5, 0x04cbe4a5, 0x04d3e4a5, 0x04dbe4a5, 0x04e3e4a5, 0x04ebe4a5, 0x04f3e4a5,
    0x04fbe4a5, 0x0503e4a5, 0x050be4a5, 0x0513e4a5, 0x051be4a5, 0x0523e4a5, 0x052be4a5, 0x0533e4a5,
    0x053be4a5, 0x0543e4a5, 0x054be4a5, 0x0553e4a5, 0x055be4a5, 0x0563e4a5, 0x056be4a5, 0x0573e4a5,
    0x057be4a5, 0x0583e4a5, 0x058be4a5, 0x0593e4a5, 0x059be4a5, 0x05a3e4a5, 0x05abe4a5, 0x05b3e4a5,
    0x05bbe4a5, 0x05c3e4a5, 0x05cbe4a5, 0x05d3e4a5, 0x05dbe4a5, 0x05e3e4a5, 0x05ebe4a5, 0x05f3e4a5,
    0x05fbe4a5, 0x060be4a5, 0x061be4a5, 0x062be4a5, 0x063be4a5, 0x064be4a5, 0x065be4a5, 0x465be4a5,
];

const EXTRA_OFFSET_BITS: &[u32] = &[
    0, 0, 0, 0, 0, 0, 0, 0, 2, 2, 2, 2, 2, 2, 2, 2, 2, 3, 3, 3, 3, 3, 3, 3, 4, 4, 4, 4, 4, 4, 4, 4,
    4, 4, 5, 5, 5, 5, 5, 5, 5, 5, 5, 5, 5, 5, 5, 5, 5, 6, 6, 6, 6, 6, 6, 6, 6, 6, 6, 6, 6, 6, 6, 6,
    7, 7, 7, 7, 7, 7, 7, 7, 7, 7, 7, 7, 7, 7, 7, 7, 7, 7, 7, 7, 8, 8, 8, 8, 8, 8, 8, 8, 8, 8, 8, 8,
    8, 8, 8, 8, 8, 8, 8, 8, 9, 9, 9, 9, 9, 9, 9, 9, 9, 9, 9, 9, 9, 9, 9, 9, 9, 9, 9, 9, 9, 9, 9, 9,
    9, 9, 9, 9, 9, 9, 10, 10, 10, 10, 10, 10, 10, 10, 10, 10, 10, 10, 10, 10, 10, 10, 10, 10, 10,
    10, 10, 10, 10, 10, 10, 10, 10, 10, 10, 10, 10, 10, 10, 11, 11, 11, 11, 11, 11, 11, 11, 11, 11,
    11, 11, 11, 11, 11, 11, 11, 11, 11, 11, 11, 11, 11, 11, 11, 11, 11, 11, 11, 11, 11, 11, 11, 11,
    11, 11, 11, 11, 11, 11, 12, 12, 12, 12, 12, 12, 12, 12, 12, 12, 12, 12, 12, 12, 12, 12, 12, 12,
    12, 12, 12, 12, 12, 12, 12, 12, 12, 12, 12, 12, 12, 12, 12, 12, 12, 12, 12, 12, 12, 12, 12, 12,
    13, 13, 13, 13, 13, 13, 13, 13, 13, 13, 13, 13, 13, 13, 13, 13, 13, 13, 13, 13, 13, 13, 13, 13,
    13, 13, 13, 13, 13, 13, 13, 13, 13, 13, 13, 13, 13, 13, 13, 13, 13, 13, 13, 13, 13, 14, 14, 14,
    14, 14, 14, 14, 14, 14, 14, 14, 14, 14, 14, 14, 14, 14, 14, 14, 14, 14, 14, 14, 14, 14, 14, 14,
    14, 14, 14, 14, 14, 14, 14, 14, 14, 14, 14, 14, 14, 14, 14, 14, 14, 14, 14, 14, 14, 14, 14, 14,
    14, 14, 14, 14, 14, 14, 14, 14, 14, 15, 15, 15, 15, 15, 15, 15, 15, 15, 15, 15, 15, 15, 15, 15,
    15, 15, 15, 15, 15, 15, 15, 15, 15, 15, 15, 15, 15, 15, 15, 15, 15, 15, 15, 15, 15, 15, 15, 15,
    15, 15, 15, 15, 15, 15, 15, 15, 15, 15, 15, 15, 15, 15, 15, 15, 15, 15, 15, 15, 15, 15, 15, 15,
    15, 15, 15, 15, 15, 15, 15, 15, 15, 15, 16, 16, 16, 16, 16, 16, 16, 16, 16, 16, 16, 16, 16, 16,
    16, 16, 16, 16, 16, 16, 16, 16, 16, 16, 16, 16, 16, 16, 16, 16, 16, 16, 16, 16, 16, 16, 16, 16,
    16, 16, 16, 16, 16, 16, 16, 16, 16, 16, 16, 16, 16, 16, 16, 16, 16, 16, 16, 16, 16, 16, 16, 16,
    16, 16, 16, 16, 16, 16, 16, 16, 16, 16, 16, 16, 16, 16, 16, 16, 16, 16, 17, 17, 17, 17, 17, 17,
    17, 17, 17, 17, 17, 17, 17, 17, 17, 17, 17, 17, 17, 17, 17, 17, 17, 17, 17, 17, 17, 17, 17, 17,
    17, 17, 17, 17, 17, 17, 17, 17, 17, 17, 17, 17, 17, 17, 17, 17, 17, 17, 17, 17, 17, 17, 17, 17,
    17, 17, 17, 17, 17, 17, 17, 17, 17, 17, 17, 17, 17, 17, 17, 17, 17, 17, 17, 17, 17, 17, 17, 17,
    17, 17, 17, 17, 17, 17, 17, 18, 18, 18, 18, 18, 18, 18, 18, 18, 18, 18, 18, 18, 18, 18, 18, 18,
    18, 18, 18, 18, 18, 18, 18, 18, 18, 18, 18, 18, 18, 18, 18, 18, 18, 18, 18, 18, 18, 18, 18, 18,
    18, 18, 18, 18, 18, 18, 18, 18, 18, 18, 18, 18, 18, 18, 18, 18, 18, 18, 18, 18, 18, 18, 18, 18,
    18, 18, 18, 18, 18, 18, 18, 18, 18, 18, 18, 18, 18, 18, 18, 18, 18, 18, 18, 18, 18, 18, 18, 18,
    18, 18, 18, 18, 18, 18, 19, 19, 19, 19, 19, 19, 19, 19, 19, 19, 19, 19, 19, 19, 19, 19, 19, 19,
    19, 19, 19, 19, 19, 19, 19, 19, 19, 19, 19, 19, 19, 19, 19, 19, 19, 19, 19, 19, 19, 19, 19, 19,
    19, 19, 19, 19, 19, 19, 19, 19, 19, 19, 19, 19, 19, 19, 19, 19, 19, 19, 19, 19, 19, 19, 19, 19,
    19, 19, 19, 19, 19, 19, 19, 19, 19, 19, 19, 19, 19, 19, 19, 19, 19, 19, 19, 19, 19, 19, 19, 19,
    19, 19, 19, 19, 19, 19, 19, 19, 19, 19, 19, 19, 19, 19, 19, 20, 20, 20, 20, 20, 20, 30,
];

const LENGTH_BASES: &[u32] = &[
    0x00000001, 0x00000002, 0x00000003, 0x00000004, 0x00000005, 0x00000006, 0x00000007, 0x00000008,
    0x00000009, 0x0000000a, 0x0000000b, 0x0000000c, 0x0000000d, 0x0000000e, 0x0000000f, 0x00000010,
    0x00000011, 0x00000012, 0x00000013, 0x00000014, 0x00000015, 0x00000016, 0x00000017, 0x00000018,
    0x00000019, 0x0000001a, 0x0000001b, 0x0000001d, 0x0000001f, 0x00000021, 0x00000023, 0x00000027,
    0x0000002b, 0x0000002f, 0x00000033, 0x00000037, 0x0000003b, 0x00000043, 0x0000004b, 0x00000053,
    0x0000005b, 0x0000006b, 0x0000007b, 0x0000008b, 0x0000009b, 0x000000ab, 0x000000cb, 0x000000eb,
    0x0000012b, 0x000001ab, 0x000002ab, 0x000004ab, 0x000008ab, 0x000108ab, 0x400108ab,
];

const EXTRA_LENGTH_BITS: &[u32] = &[
    0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 1, 1, 1, 1, 2, 2,
    2, 2, 2, 2, 3, 3, 3, 3, 4, 4, 4, 4, 4, 5, 5, 6, 7, 8, 9, 10, 16, 30,
];

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn word_probe_never_skips_an_opcode_in_any_lane() {
        for opcode in [0xff, 0xf0, 0x48, 0x4c, 0xe8, 0xe9] {
            for lane in 0..8 {
                let mut bytes = [0x21; 8];
                bytes[lane] = opcode;
                assert!(x86_opcode_in_word(&bytes));
            }
        }
    }

    #[test]
    fn filter_generations_isolate_previous_blocks_and_clear_on_wrap() {
        let mut state = X86FilterState::new().unwrap();
        let first = state.begin(32768);
        state.targets[23] = first + 100;
        let second = state.begin(17);
        assert!(second - state.targets[23] > 65535);
        state.next_base = i32::MAX - 1;
        assert_eq!(state.begin(32768), 0);
        assert!(state.targets.iter().all(|&stamp| stamp == -65536));
    }

    #[test]
    fn uniform_reset_preserves_balanced_symbol_order_and_single_symbol_lengths() {
        let mut code = Huffman::new(54, 512).unwrap();
        // The original canonical code assigns the 44 longer codes first and
        // the ten shorter codes last for this 54-symbol equal-frequency tree.
        assert_eq!(&code.lengths[..44], &[6; 44]);
        assert_eq!(&code.lengths[44..], &[5; 10]);
        code.frequencies[0] = 2000;
        code.rebuild().unwrap();
        code.reset(54).unwrap();
        assert_eq!(&code.lengths[..44], &[6; 44]);
        code.reset(1).unwrap();
        assert_eq!(&code.lengths[..], &[1]);
        assert_eq!(&code.table[..1 << 14], &[1; 1 << 14]);
        code.reset(0).unwrap();
        assert!(code.table.iter().all(|&entry| entry == 0));
    }

    #[test]
    fn bounded_huffman_lengths_match_original_c_for_deep_fibonacci_tree() {
        let mut code = Huffman::new(20, 1024).unwrap();
        code.frequencies[0] = 1;
        code.frequencies[1] = 1;
        for symbol in 2..20 {
            code.frequencies[symbol] = code.frequencies[symbol - 1] + code.frequencies[symbol - 2];
        }
        code.rebuild().unwrap();
        // Independent C make_canonical_huffman_code(20,15,...) observation;
        // generator: tests/fixtures/lzms_huffman_oracle.c.
        let expected = [
            15, 15, 15, 15, 15, 15, 15, 15, 12, 11, 10, 9, 8, 7, 6, 5, 4, 3, 2, 1,
        ];
        for (symbol, length) in expected.into_iter().enumerate() {
            let entry = code
                .table
                .iter()
                .find(|&&entry| usize::from(entry >> 4) == symbol)
                .unwrap();
            assert_eq!(entry & 15, length);
        }
        assert!(code.table.iter().all(|entry| entry & 15 > 0));
    }

    #[test]
    fn unrepresentable_allocation_returns_recoverable_error() {
        assert_eq!(filled_buffer(usize::MAX, 0u16), Err(LzmsError::OutOfMemory));
    }

    #[test]
    fn repeat_queues_delay_new_source_for_all_three_indices() {
        for (same, expected) in [
            (
                false,
                [
                    (10, [10, 20, 30, 40]),
                    (20, [10, 10, 30, 40]),
                    (30, [10, 10, 20, 40]),
                ],
            ),
            (
                true,
                [
                    (20, [10, 10, 30, 40]),
                    (30, [10, 10, 20, 40]),
                    (40, [10, 10, 20, 30]),
                ],
            ),
        ] {
            for (index, (value, queue)) in expected.into_iter().enumerate() {
                let mut actual = [10, 20, 30, 40];
                assert_eq!(repeat(&mut actual, index, same), value);
                assert_eq!(actual, queue);
            }
        }
    }

    #[test]
    fn backwards_words_supply_high_bits_first_and_zero_pad_exhaustion() {
        let mut bits = ReverseBits::new(&[0x34, 0x12, 0xcd, 0xab]);
        assert_eq!(bits.read(4), 0xa);
        assert_eq!(bits.read(12), 0xbcd);
        assert_eq!(bits.read(16), 0x1234);
        assert_eq!(bits.read(30), 0);
    }

    #[test]
    fn delta_copy_overlaps_with_bytewise_wrapping_arithmetic() {
        let mut output = [250, 253, 0, 0, 0, 0, 0];
        assert_eq!(copy_delta(&mut output, 2, 5, 0, 1), Ok(()));
        assert_eq!(output, [250, 253, 0, 3, 6, 9, 12]);
    }

    #[test]
    fn delta_copy_power_scales_both_offset_and_span() {
        let mut output = [1, 9, 3, 13, 0, 0, 0, 0];
        assert_eq!(copy_delta(&mut output, 4, 4, 1, 1), Ok(()));
        assert_eq!(output, [1, 9, 3, 13, 5, 17, 7, 21]);
    }

    #[test]
    fn delta_copy_rejects_overflow_underrun_and_output_overrun() {
        let mut output = [0; 8];
        assert_eq!(
            copy_delta(&mut output, 4, 1, 7, u32::MAX),
            Err(LzmsError::InvalidMatchOffset)
        );
        assert_eq!(
            copy_delta(&mut output, 4, 1, 0, u32::MAX),
            Err(LzmsError::InvalidMatchOffset)
        );
        assert_eq!(
            copy_delta(&mut output, 1, 1, 0, 1),
            Err(LzmsError::InvalidMatchOffset)
        );
        assert_eq!(
            copy_delta(&mut output, 4, 5, 0, 1),
            Err(LzmsError::MatchExceedsOutput)
        );
    }

    #[test]
    fn x86_filter_ignores_first_byte_and_last_sixteen_bytes() {
        let mut data = [0x90; 48];
        data[0] = 0xe8;
        data[32] = 0xe8;
        let expected = data;
        undo_x86_filter(&mut data).unwrap();
        assert_eq!(data, expected);
    }
}
