// SPDX-License-Identifier: LGPL-2.1-or-later
//! MS-PATCH LZX DELTA decoding with a caller-supplied reference dictionary.

use super::{Code, EXTRA_BITS, HuffmanBits, LENGTH_SYMBOLS, LzxError, OFFSET_BASE};
use alloc::boxed::Box;
use alloc::vec::Vec;

/// Failure to decode an MS-PATCH LZX DELTA stream.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LzxDeltaError {
    /// LZX DELTA windows must be 128 KiB through 32 MiB (orders 17 through 25).
    InvalidWindow,
    /// Reference data cannot fit within the configured dictionary.
    ReferenceExceedsWindow,
    /// Input framing or zero padding is invalid.
    InvalidFraming,
    /// Final output ends before the current block is complete.
    IncompleteBlock,
    /// Each frame must produce between one and 32768 bytes.
    InvalidFrameSize,
    /// Input ended before the required bits or literal bytes were available.
    TruncatedInput,
    /// A required canonical code was empty.
    EmptyCode,
    /// A previous failure has invalidated this stream's decoder state.
    FailedDecoder,
    /// Allocation of the history window failed.
    AllocationFailed,
    /// The shared LZX decoder rejected a block, code, or match.
    InvalidData(LzxError),
}

impl core::fmt::Display for LzxDeltaError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::ReferenceExceedsWindow => f.write_str("LZXD reference exceeds window"),
            Self::InvalidFraming => f.write_str("invalid LZXD framing or padding"),
            Self::IncompleteBlock => f.write_str("incomplete LZXD block"),
            Self::InvalidWindow => f.write_str("invalid LZX DELTA window order (expected 17..=25)"),
            Self::InvalidFrameSize => f.write_str("invalid LZX DELTA frame size"),
            Self::TruncatedInput => f.write_str("truncated LZX DELTA input"),
            Self::EmptyCode => f.write_str("empty required LZX DELTA code"),
            Self::FailedDecoder => f.write_str("LZX DELTA decoder invalidated by a previous error"),
            Self::AllocationFailed => f.write_str("LZX DELTA window allocation failed"),
            Self::InvalidData(error) => error.fmt(f),
        }
    }
}

impl core::error::Error for LzxDeltaError {
    fn source(&self) -> Option<&(dyn core::error::Error + 'static)> {
        match self {
            Self::InvalidData(error) => Some(error),
            _ => None,
        }
    }
}

impl From<LzxError> for LzxDeltaError {
    fn from(error: LzxError) -> Self {
        Self::InvalidData(error)
    }
}

/// Stateful decoder for MS-PATCH 32-KiB chunks, retaining untransformed history.
///
/// Supports verbatim, aligned, and uncompressed blocks, windows from 128 KiB
/// through 32 MiB, extended match lengths, and optional Intel E8 translation.
/// Supply exactly the reference bytes used during compression. This decoder
/// does not parse PA19/PA30 patch envelopes or verify source/target hashes.
pub struct LzxDeltaDecoder {
    workspace: Box<DeltaWorkspace>,
    window: Vec<u8>,
    window_position: usize,
    window_mask: usize,
    produced: u64,
    reference_size: usize,
    main_symbols: usize,
    recent: [usize; 3],
    first_frame: bool,
    intel_size: i32,
    intel_started: bool,
    block_type: u32,
    block_size: usize,
    remaining: usize,
    failed: bool,
}

impl LzxDeltaDecoder {
    /// Allocate a decoder for window orders 17 through 25 and prepend reference data.
    pub fn new(window_order: u8, reference: &[u8]) -> Result<Self, LzxDeltaError> {
        if !(17..=25).contains(&window_order) {
            return Err(LzxDeltaError::InvalidWindow);
        }
        let size = 1usize << window_order;
        if reference.len() > size - 3 {
            return Err(LzxDeltaError::ReferenceExceedsWindow);
        }
        let mut window = Vec::new();
        window
            .try_reserve_exact(size)
            .map_err(|_| LzxDeltaError::AllocationFailed)?;
        window.resize(size, 0);
        let slots = match window_order {
            20 => 42,
            21..=25 => 34 + (1usize << (window_order - 17)),
            _ => usize::from(window_order) * 2,
        };
        window[..reference.len()].copy_from_slice(reference);
        Ok(Self {
            workspace: Box::new(DeltaWorkspace::empty()),
            window,
            window_position: reference.len(),
            window_mask: size - 1,
            produced: 0,
            reference_size: reference.len(),
            main_symbols: 256 + slots * 8,
            recent: [1; 3],
            first_frame: true,
            intel_size: 0,
            intel_started: false,
            block_type: 0,
            block_size: 0,
            remaining: 0,
            failed: false,
        })
    }

    /// Decode one chunk payload (without its two-byte prefix), producing 1..=32768 bytes.
    /// All nonfinal chunks must produce exactly 32768 bytes. Call `finish` after the last.
    ///
    /// On error, the output prefix and decoder state must be discarded. Further
    /// calls on that decoder return [`LzxDeltaError::FailedDecoder`].
    pub fn decompress_chunk(
        &mut self,
        input: &[u8],
        output: &mut [u8],
    ) -> Result<(), LzxDeltaError> {
        if self.failed {
            return Err(LzxDeltaError::FailedDecoder);
        }
        let result = self.decode(input, output);
        if result.is_err() {
            self.failed = true;
        }
        result
    }

    fn decode(&mut self, input: &[u8], output: &mut [u8]) -> Result<(), LzxDeltaError> {
        if output.is_empty() || output.len() > 32768 {
            return Err(LzxDeltaError::InvalidFrameSize);
        }
        let mut bits = DeltaBits::new(input);
        if self.first_frame {
            if bits.read(1) != 0 {
                self.intel_size = ((bits.read(16) << 16) | bits.read(16)) as i32;
            }
            bits.check()?;
            self.first_frame = false;
        }
        let frame_start = self.produced;
        let history_start = self.window_position;
        let mut position = 0;
        while position < output.len() {
            if self.remaining == 0 {
                if self.block_type == 3 && self.block_size & 1 != 0 && bits.literal_bytes(1)? != [0]
                {
                    return Err(LzxDeltaError::InvalidFraming);
                }
                self.read_block(&mut bits)?;
            }
            if self.block_type == 3 {
                let count = self.remaining.min(output.len() - position);
                let literals = bits.literal_bytes(count)?;
                self.store(literals);
                position += count;
                self.remaining -= count;
                continue;
            }
            let run_start = position;
            let run_end = position + self.remaining.min(output.len() - position);
            // Hoist block/frame limits out of the symbol loop. A run cannot
            // cross either boundary, so remaining advances once per run.
            while position < run_end {
                let symbol = self.workspace.main.read_nonempty(&mut bits, 16)?;
                bits.check()?;
                if symbol < 256 {
                    self.window[self.window_position] = symbol as u8;
                    self.window_position = (self.window_position + 1) & self.window_mask;
                    position += 1;
                    continue;
                }
                let mut length = symbol & 7;
                if length == 7 {
                    if self.workspace.lengths.empty {
                        return Err(LzxDeltaError::EmptyCode);
                    }
                    length += self.workspace.lengths.read_nonempty(&mut bits, 16)?;
                }
                length += 2;
                let slot = (symbol - 256) / 8;
                let offset = if slot < 3 {
                    let value = self.recent[slot];
                    self.recent[slot] = self.recent[0];
                    value
                } else {
                    let aligned = self.block_type == 2 && slot >= 8;
                    let mut extra =
                        bits.read(footer_bits(slot) - if aligned { 3 } else { 0 }) as usize;
                    if aligned {
                        if self.workspace.aligned.empty {
                            return Err(LzxDeltaError::EmptyCode);
                        }
                        extra =
                            (extra << 3) | self.workspace.aligned.read_nonempty(&mut bits, 7)?;
                    }
                    self.recent[2] = self.recent[1];
                    self.recent[1] = self.recent[0];
                    offset_base(slot) + extra
                };
                bits.check()?;
                if length == 257 {
                    length = if bits.read(1) == 0 {
                        257 + bits.read(8) as usize
                    } else if bits.read(1) == 0 {
                        513 + bits.read(10) as usize
                    } else if bits.read(1) == 0 {
                        1537 + bits.read(12) as usize
                    } else {
                        257 + bits.read(15) as usize
                    };
                    bits.check()?;
                }
                self.recent[0] = offset;
                if offset == 0
                    || offset > self.window.len() - 3
                    || offset as u64 > self.reference_size as u64 + frame_start + position as u64
                    || length > run_end - position
                {
                    return Err(LzxError::InvalidMatch.into());
                }
                self.copy_match(offset, length);
                position += length;
            }
            self.remaining -= position - run_start;
        }
        // Materialize output once; Intel E8 transforms must never alter history.
        let first = output.len().min(self.window.len() - history_start);
        output[..first].copy_from_slice(&self.window[history_start..history_start + first]);
        let second = output.len() - first;
        output[first..].copy_from_slice(&self.window[..second]);
        self.produced += output.len() as u64;
        // Keep untransformed bytes in the history window for subsequent matches.
        if self.intel_started && self.intel_size != 0 && frame_start < 0x4000_0000 {
            inverse_delta_e8(output, frame_start, self.intel_size);
        }
        if self.remaining == 0 && self.block_type == 3 && self.block_size & 1 != 0 {
            if bits.literal_bytes(1)? != [0] {
                return Err(LzxDeltaError::InvalidFraming);
            }
            self.block_size = 0;
        }
        bits.finish()?;
        Ok(())
    }

    /// Require that all bytes declared by the last block were produced.
    pub fn finish(&self) -> Result<(), LzxDeltaError> {
        if self.failed {
            return Err(LzxDeltaError::FailedDecoder);
        }
        if self.remaining != 0 {
            return Err(LzxDeltaError::IncompleteBlock);
        }
        Ok(())
    }

    fn copy_match(&mut self, offset: usize, length: usize) {
        let destination = self.window_position;
        let source = (destination + self.window.len() - offset) & self.window_mask;
        if destination + length <= self.window.len() && source + length <= self.window.len() {
            if offset >= length {
                self.window
                    .copy_within(source..source + length, destination);
            } else if offset == 1 {
                let byte = self.window[source];
                self.window[destination..destination + length].fill(byte);
            } else {
                // Seed one period, then double the available repeated prefix.
                self.window
                    .copy_within(source..source + offset, destination);
                let mut copied = offset;
                while copied < length {
                    let count = copied.min(length - copied);
                    self.window
                        .copy_within(destination..destination + count, destination + copied);
                    copied += count;
                }
            }
        } else {
            // Split at both ring boundaries and the repetition distance. Each
            // span reads only bytes already present or emitted by an earlier span.
            let mut copied = 0;
            while copied < length {
                let destination = (destination + copied) & self.window_mask;
                let source = (source + copied) & self.window_mask;
                let count = (length - copied)
                    .min(offset)
                    .min(self.window.len() - destination)
                    .min(self.window.len() - source);
                self.window.copy_within(source..source + count, destination);
                copied += count;
            }
        }
        self.window_position = (destination + length) & self.window_mask;
    }

    fn store(&mut self, bytes: &[u8]) {
        let first = bytes.len().min(self.window.len() - self.window_position);
        self.window[self.window_position..self.window_position + first]
            .copy_from_slice(&bytes[..first]);
        self.window[..bytes.len() - first].copy_from_slice(&bytes[first..]);
        self.window_position = (self.window_position + bytes.len()) & self.window_mask;
    }

    fn read_block(&mut self, bits: &mut DeltaBits<'_>) -> Result<(), LzxDeltaError> {
        self.block_type = bits.read(3);
        self.block_size = ((bits.read(16) << 8) | bits.read(8)) as usize;
        bits.check()?;
        if self.block_size == 0 {
            return Err(LzxError::InvalidBlockSize.into());
        }
        self.remaining = self.block_size;
        match self.block_type {
            1 | 2 => {
                if self.block_type == 2 {
                    let mut lengths = [0; 8];
                    for length in &mut lengths {
                        *length = bits.read(3) as u8;
                    }
                    self.workspace.aligned = Code::new(&lengths, 7)?;
                }
                let workspace = &mut self.workspace;
                read_delta_lengths(bits, &mut workspace.main_lengths[..256], 256)?;
                read_delta_lengths(
                    bits,
                    &mut workspace.main_lengths[256..self.main_symbols],
                    self.main_symbols - 256,
                )?;
                read_delta_lengths(
                    bits,
                    &mut workspace.length_lengths[..LENGTH_SYMBOLS],
                    LENGTH_SYMBOLS,
                )?;
                bits.check()?;
                workspace.main = Code::new(&workspace.main_lengths[..self.main_symbols], 16)?;
                workspace.lengths = Code::new(&workspace.length_lengths[..LENGTH_SYMBOLS], 16)?;
                if workspace.main.empty {
                    return Err(LzxDeltaError::EmptyCode);
                }
                self.intel_started |= workspace.main_lengths[0xe8] != 0;
            }
            3 => {
                bits.align_uncompressed()?;
                for offset in &mut self.recent {
                    let bytes = bits.literal_bytes(4)?;
                    *offset = u32::from_le_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]) as usize;
                }
                if self
                    .recent
                    .iter()
                    .any(|&offset| offset == 0 || offset > self.window.len() - 3)
                {
                    return Err(LzxError::InvalidRecentOffset.into());
                }
                self.intel_started = true;
            }
            _ => return Err(LzxError::InvalidBlockType.into()),
        }
        Ok(())
    }
}

fn inverse_delta_e8(output: &mut [u8], frame_start: u64, file_size: i32) {
    let mut position = 0;
    while position + 10 < output.len() {
        // Skip eight non-E8 bytes with one word probe. The zero-byte test may
        // report extra candidates, but can never skip a real E8 byte.
        let bytes = &output[position..position + 8];
        let word = u64::from_ne_bytes([
            bytes[0], bytes[1], bytes[2], bytes[3], bytes[4], bytes[5], bytes[6], bytes[7],
        ]) ^ 0xe8e8_e8e8_e8e8_e8e8;
        if word.wrapping_sub(0x0101_0101_0101_0101) & !word & 0x8080_8080_8080_8080 == 0 {
            position += 8;
            continue;
        }
        if output[position] != 0xe8 {
            position += 1;
            continue;
        }
        let bytes = &output[position + 1..position + 5];
        let absolute = i32::from_le_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]);
        let current = (frame_start + position as u64) as i32;
        if absolute >= -current && absolute < file_size {
            let relative = if absolute >= 0 {
                absolute.wrapping_sub(current)
            } else {
                absolute.wrapping_add(file_size)
            };
            output[position + 1..position + 5].copy_from_slice(&relative.to_le_bytes());
        }
        position += 5;
    }
}

struct DeltaBits<'a> {
    input: &'a [u8],
    next: usize,
    buffer: u64,
    available: usize,
    truncated: bool,
}

impl<'a> DeltaBits<'a> {
    fn new(input: &'a [u8]) -> Self {
        Self {
            input,
            next: 0,
            buffer: 0,
            available: 0,
            truncated: false,
        }
    }

    fn check(&self) -> Result<(), LzxDeltaError> {
        if self.truncated {
            Err(LzxDeltaError::TruncatedInput)
        } else {
            Ok(())
        }
    }

    fn finish(&self) -> Result<(), LzxDeltaError> {
        self.check()?;
        if self.available > 15 || self.buffer != 0 || self.next != self.input.len() {
            return Err(LzxDeltaError::InvalidFraming);
        }
        Ok(())
    }

    fn align_uncompressed(&mut self) -> Result<(), LzxDeltaError> {
        let count = if self.available.is_multiple_of(16) {
            16
        } else {
            self.available % 16
        };
        if self.read(count) != 0 {
            return Err(LzxDeltaError::InvalidFraming);
        }
        self.check()?;
        // Rewind a prefetched whole word; it belongs to the literal stream.
        self.next -= self.available / 8;
        self.align();
        Ok(())
    }

    fn align(&mut self) {
        self.buffer = 0;
        self.available = 0;
    }

    fn literal_bytes(&mut self, count: usize) -> Result<&'a [u8], LzxDeltaError> {
        let bytes = self.input[self.next..]
            .get(..count)
            .ok_or(LzxDeltaError::TruncatedInput)?;
        self.next += count;
        Ok(bytes)
    }
}

impl HuffmanBits for DeltaBits<'_> {
    fn ensure(&mut self, count: usize) {
        while self.available < count {
            let Some(word) = self.input[self.next..].get(..2) else {
                break;
            };
            self.buffer |=
                u64::from(u16::from_le_bytes([word[0], word[1]])) << (48 - self.available);
            self.next += 2;
            self.available += 16;
        }
    }

    fn peek(&self, count: usize) -> u32 {
        if count == 0 {
            0
        } else {
            (self.buffer >> (64 - count)) as u32
        }
    }

    fn remove(&mut self, count: usize) {
        if count > self.available {
            self.truncated = true;
            self.align();
        } else {
            self.buffer <<= count;
            self.available -= count;
        }
    }
}

#[derive(Debug)]
struct DeltaWorkspace {
    main_lengths: [u8; 2576],
    length_lengths: [u8; LENGTH_SYMBOLS],
    main: Code<2576>,
    lengths: Code<LENGTH_SYMBOLS>,
    aligned: Code<8>,
}
impl DeltaWorkspace {
    fn empty() -> Self {
        Self {
            main_lengths: [0; 2576],
            length_lengths: [0; LENGTH_SYMBOLS],
            main: Code::empty(16),
            lengths: Code::empty(16),
            aligned: Code::empty(7),
        }
    }
}
fn footer_bits(slot: usize) -> usize {
    if slot < EXTRA_BITS.len() {
        EXTRA_BITS[slot]
    } else {
        17
    }
}
fn offset_base(slot: usize) -> usize {
    if slot < OFFSET_BASE.len() {
        OFFSET_BASE[slot]
    } else {
        1048574 + (slot - 42) * 131072
    }
}
fn read_delta_lengths(
    bits: &mut DeltaBits<'_>,
    lengths: &mut [u8],
    count: usize,
) -> Result<(), LzxDeltaError> {
    let mut pre_lengths = [0u8; 20];
    for length in &mut pre_lengths {
        *length = bits.read(4) as u8;
    }
    bits.check()?;
    let pre = Code::<20>::new(&pre_lengths, 15)?;
    if pre.empty {
        return Err(LzxDeltaError::EmptyCode);
    }
    let mut position = 0;
    while position < count {
        let symbol = pre.read_nonempty(bits, 15)?;
        let (run, length) = match symbol {
            0..=16 => (1, super::delta_length(lengths[position], symbol)),
            17 => (4 + bits.read(4) as usize, 0),
            18 => (20 + bits.read(5) as usize, 0),
            19 => {
                let run = 4 + bits.read(1) as usize;
                let delta = pre.read_nonempty(bits, 15)?;
                if delta > 16 {
                    return Err(LzxError::InvalidLengthRun.into());
                }
                (run, super::delta_length(lengths[position], delta))
            }
            _ => return Err(LzxError::InvalidLengthRun.into()),
        };
        bits.check()?;
        let end = position + run;
        if end > count {
            return Err(LzxError::InvalidLengthRun.into());
        }
        lengths[position..end].fill(length);
        position = end;
    }
    Ok(())
}

/// Decode an entire MS-PATCH chunk-prefixed stream into exact-sized output.
///
/// `window_order` must be 17..=25. Reference data must fit within the window
/// minus three bytes. Allocations are bounded by the window; output is supplied
/// by the caller. Trailing input, truncated chunks, nonzero padding, and blocks
/// that exceed output are rejected. On failure discard the output prefix.
pub fn decompress_lzxd(
    input: &[u8],
    reference: &[u8],
    output: &mut [u8],
    window_order: u8,
) -> Result<(), LzxDeltaError> {
    let mut decoder = LzxDeltaDecoder::new(window_order, reference)?;
    let mut remaining = input;
    for chunk in output.chunks_mut(32768) {
        let prefix = remaining.get(..2).ok_or(LzxDeltaError::TruncatedInput)?;
        let length = usize::from(u16::from_le_bytes([prefix[0], prefix[1]]));
        remaining = &remaining[2..];
        let payload = remaining
            .get(..length)
            .ok_or(LzxDeltaError::TruncatedInput)?;
        if length == 0 {
            return Err(LzxDeltaError::InvalidFraming);
        }
        decoder.decompress_chunk(payload, chunk)?;
        remaining = &remaining[length..];
    }
    if !remaining.is_empty() {
        return Err(LzxDeltaError::InvalidFraming);
    }
    decoder.finish()
}
