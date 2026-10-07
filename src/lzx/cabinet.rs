// SPDX-License-Identifier: LGPL-2.1-or-later
//! Stateful cabinet LZX decoding using the existing WIM canonical-code machinery.

use super::{
    Code, EXTRA_BITS, HuffmanBits, LENGTH_SYMBOLS, LzxError, OFFSET_BASE, Workspace, read_lengths,
};
use alloc::boxed::Box;
use alloc::vec::Vec;

/// Failure to decode a cabinet LZX frame.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CabinetLzxError {
    /// Cabinet windows must be between 32 KiB and 2 MiB (orders 15 through 21).
    InvalidWindow,
    /// Each frame must produce between one and 32768 bytes.
    InvalidFrameSize,
    /// Input ended before the required bits or literal bytes were available.
    TruncatedInput,
    /// A required canonical code was empty.
    EmptyCode,
    /// A previous failure has invalidated this folder's decoder state.
    FailedDecoder,
    /// Allocation of the history window failed.
    AllocationFailed,
    /// The shared LZX decoder rejected a block, code, or match.
    InvalidData(LzxError),
}

impl core::fmt::Display for CabinetLzxError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::InvalidWindow => {
                f.write_str("invalid cabinet LZX window order (expected 15..=21)")
            }
            Self::InvalidFrameSize => f.write_str("invalid cabinet LZX frame size"),
            Self::TruncatedInput => f.write_str("truncated cabinet LZX input"),
            Self::EmptyCode => f.write_str("empty required cabinet LZX code"),
            Self::FailedDecoder => {
                f.write_str("cabinet LZX decoder invalidated by a previous error")
            }
            Self::AllocationFailed => f.write_str("cabinet LZX window allocation failed"),
            Self::InvalidData(error) => error.fmt(f),
        }
    }
}

impl core::error::Error for CabinetLzxError {
    fn source(&self) -> Option<&(dyn core::error::Error + 'static)> {
        match self {
            Self::InvalidData(error) => Some(error),
            _ => None,
        }
    }
}

impl From<LzxError> for CabinetLzxError {
    fn from(error: LzxError) -> Self {
        Self::InvalidData(error)
    }
}

/// Decoder for one CAB folder's sequential CFDATA frames.
///
/// Cabinet framing uses a 24-bit block size, an optional stream-wide Intel E8
/// size, and persistent trees, recent offsets, and history across 32 KiB frames.
/// These differ from WIM framing. Verbatim, aligned, and uncompressed blocks
/// reuse the WIM decoder's canonical codes and offset tables. LZX DELTA is not
/// supported. Create a fresh decoder for each folder or after an error.
pub struct CabinetLzxDecoder {
    workspace: Box<Workspace>,
    window: Vec<u8>,
    window_position: usize,
    window_mask: usize,
    produced: u64,
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

impl CabinetLzxDecoder {
    /// Allocate a decoder for a cabinet window order from 15 through 21.
    pub fn new(window_order: u8) -> Result<Self, CabinetLzxError> {
        if !(15..=21).contains(&window_order) {
            return Err(CabinetLzxError::InvalidWindow);
        }
        let size = 1usize << window_order;
        let mut window = Vec::new();
        window
            .try_reserve_exact(size)
            .map_err(|_| CabinetLzxError::AllocationFailed)?;
        window.resize(size, 0);
        let slots = match window_order {
            20 => 42,
            21 => 50,
            _ => usize::from(window_order) * 2,
        };
        Ok(Self {
            workspace: Box::new(Workspace::empty()),
            window,
            window_position: 0,
            window_mask: size - 1,
            produced: 0,
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

    /// Decode the next frame into an exact-sized output slice of at most 32 KiB.
    ///
    /// On error, the output prefix and decoder state must be discarded. Further
    /// calls on that decoder return [`CabinetLzxError::FailedDecoder`].
    pub fn decompress_frame(
        &mut self,
        input: &[u8],
        output: &mut [u8],
    ) -> Result<(), CabinetLzxError> {
        if self.failed {
            return Err(CabinetLzxError::FailedDecoder);
        }
        let result = self.decode(input, output);
        if result.is_err() {
            self.failed = true;
        }
        result
    }

    fn decode(&mut self, input: &[u8], output: &mut [u8]) -> Result<(), CabinetLzxError> {
        if output.is_empty() || output.len() > 32768 {
            return Err(CabinetLzxError::InvalidFrameSize);
        }
        let mut bits = CabinetBits::new(input);
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
                if self.block_type == 3 && self.block_size & 1 != 0 {
                    bits.literal_bytes(1)?;
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
                        return Err(CabinetLzxError::EmptyCode);
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
                        bits.read(EXTRA_BITS[slot] - if aligned { 3 } else { 0 }) as usize;
                    if aligned {
                        if self.workspace.aligned.empty {
                            return Err(CabinetLzxError::EmptyCode);
                        }
                        extra =
                            (extra << 3) | self.workspace.aligned.read_nonempty(&mut bits, 7)?;
                    }
                    self.recent[2] = self.recent[1];
                    self.recent[1] = self.recent[0];
                    OFFSET_BASE[slot] + extra
                };
                bits.check()?;
                self.recent[0] = offset;
                if offset == 0
                    || offset > self.window.len()
                    || offset as u64 > frame_start + position as u64
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
            inverse_cabinet_e8(output, frame_start, self.intel_size);
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

    fn read_block(&mut self, bits: &mut CabinetBits<'_>) -> Result<(), CabinetLzxError> {
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
                read_lengths(bits, &mut workspace.main_lengths[..256], 256)?;
                read_lengths(
                    bits,
                    &mut workspace.main_lengths[256..self.main_symbols],
                    self.main_symbols - 256,
                )?;
                read_lengths(
                    bits,
                    &mut workspace.length_lengths[..LENGTH_SYMBOLS],
                    LENGTH_SYMBOLS,
                )?;
                bits.check()?;
                workspace.main = Code::new(&workspace.main_lengths[..self.main_symbols], 16)?;
                workspace.lengths = Code::new(&workspace.length_lengths[..LENGTH_SYMBOLS], 16)?;
                if workspace.main.empty {
                    return Err(CabinetLzxError::EmptyCode);
                }
                self.intel_started |= workspace.main_lengths[0xe8] != 0;
            }
            3 => {
                bits.ensure(1);
                bits.check()?;
                bits.align();
                for offset in &mut self.recent {
                    let bytes = bits.literal_bytes(4)?;
                    *offset = u32::from_le_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]) as usize;
                }
                if self.recent.contains(&0) {
                    return Err(LzxError::InvalidRecentOffset.into());
                }
                self.intel_started = true;
            }
            _ => return Err(LzxError::InvalidBlockType.into()),
        }
        Ok(())
    }
}

fn inverse_cabinet_e8(output: &mut [u8], frame_start: u64, file_size: i32) {
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

struct CabinetBits<'a> {
    input: &'a [u8],
    next: usize,
    buffer: u64,
    available: usize,
    truncated: bool,
}

impl<'a> CabinetBits<'a> {
    fn new(input: &'a [u8]) -> Self {
        Self {
            input,
            next: 0,
            buffer: 0,
            available: 0,
            truncated: false,
        }
    }

    fn check(&self) -> Result<(), CabinetLzxError> {
        if self.truncated {
            Err(CabinetLzxError::TruncatedInput)
        } else {
            Ok(())
        }
    }

    fn align(&mut self) {
        self.buffer = 0;
        self.available = 0;
    }

    fn literal_bytes(&mut self, count: usize) -> Result<&'a [u8], CabinetLzxError> {
        let bytes = self.input[self.next..]
            .get(..count)
            .ok_or(CabinetLzxError::TruncatedInput)?;
        self.next += count;
        Ok(bytes)
    }
}

impl HuffmanBits for CabinetBits<'_> {
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

#[cfg(test)]
mod history_regression {
    use super::*;
    use alloc::vec;

    #[test]
    fn bounded_refill_matches_word_bit_order_and_rejects_missing_bits() {
        let data: Vec<u8> = (0..80).map(|index| (index * 37 + 19) as u8).collect();
        for length in 0..=data.len() {
            let input = &data[..length];
            let expected: Vec<u32> = input
                .chunks_exact(2)
                .flat_map(|word| {
                    let value = u16::from_le_bytes([word[0], word[1]]);
                    (0..16)
                        .rev()
                        .map(move |shift| u32::from((value >> shift) & 1))
                })
                .collect();
            for first in 0..=17 {
                let mut bits = CabinetBits::new(input);
                let mut position = 0;
                for count in core::iter::once(first).chain((0..=17).cycle()).take(200) {
                    let mut value = 0;
                    for index in position..position + count {
                        value = (value << 1) | expected.get(index).copied().unwrap_or(0);
                    }
                    assert_eq!(
                        bits.read(count),
                        value,
                        "length={length}, position={position}, count={count}"
                    );
                    if position + count > expected.len() {
                        assert_eq!(bits.check(), Err(CabinetLzxError::TruncatedInput));
                        break;
                    }
                    assert_eq!(bits.check(), Ok(()));
                    position += count;
                }
            }
        }
    }

    #[test]
    fn word_e8_scan_matches_scalar_reference_at_every_lane_and_tail() {
        fn scalar(output: &mut [u8], start: u64, size: i32) {
            let mut position = 0;
            while position + 10 < output.len() {
                if output[position] != 0xe8 {
                    position += 1;
                    continue;
                }
                let absolute =
                    i32::from_le_bytes(output[position + 1..position + 5].try_into().unwrap());
                let current = (start + position as u64) as i32;
                if absolute >= -current && absolute < size {
                    let relative = if absolute >= 0 {
                        absolute.wrapping_sub(current)
                    } else {
                        absolute.wrapping_add(size)
                    };
                    output[position + 1..position + 5].copy_from_slice(&relative.to_le_bytes());
                }
                position += 5;
            }
        }
        for length in (0..=80).chain([32768]) {
            for lane in 0..16 {
                for start in [0, 32768, 0x3fff_0000] {
                    let mut bytes = vec![0xe9; length];
                    for position in (lane..length.saturating_sub(4)).step_by(17) {
                        bytes[position] = 0xe8;
                        let absolute = [1200i32, -1, i32::MAX, i32::MIN][(position / 17) % 4];
                        bytes[position + 1..position + 5].copy_from_slice(&absolute.to_le_bytes());
                    }
                    let mut expected = bytes.clone();
                    scalar(&mut expected, start, 12000000);
                    inverse_cabinet_e8(&mut bytes, start, 12000000);
                    assert_eq!(
                        bytes, expected,
                        "length={length}, lane={lane}, start={start}"
                    );
                }
            }
        }
    }

    #[test]
    fn direct_history_matches_bytewise_reference_across_overlap_and_ring_boundaries() {
        for order in 15..=21 {
            let mut decoder = CabinetLzxDecoder::new(order).unwrap();
            let size = decoder.window.len();
            for destination in [0, 1, 255, size / 2, size - 257, size - 1] {
                for offset in [1, 2, 3, 16, 255, 257, size / 2, size - 1, size] {
                    for length in [2, 3, 17, 256, 257] {
                        for (index, byte) in decoder.window.iter_mut().enumerate() {
                            *byte = (index.wrapping_mul(37) ^ (index >> 8)) as u8;
                        }
                        let mut expected = decoder.window.clone();
                        let mut position = destination;
                        for _ in 0..length {
                            expected[position] = expected[(position + size - offset) & (size - 1)];
                            position = (position + 1) & (size - 1);
                        }
                        decoder.window_position = destination;
                        decoder.copy_match(offset, length);
                        assert_eq!(
                            decoder.window, expected,
                            "order={order}, destination={destination}, offset={offset}, length={length}"
                        );
                        assert_eq!(decoder.window_position, position);
                    }
                }
            }
        }
    }

    #[test]
    fn bulk_history_store_preserves_both_ring_spans_for_future_matches() {
        let mut decoder = CabinetLzxDecoder::new(15).unwrap();
        decoder.store(&vec![b'A'; 32760]);
        decoder.store(b"abcdefghijklmnop");
        assert_eq!(&decoder.window[32760..], b"abcdefgh");
        assert_eq!(&decoder.window[..8], b"ijklmnop");
        assert!(decoder.window[8..32760].iter().all(|&byte| byte == b'A'));
        assert_eq!(decoder.window_position, 8);
        decoder.copy_match(16, 32);
        assert_eq!(&decoder.window[8..40], b"abcdefghijklmnopabcdefghijklmnop");
        assert_eq!(decoder.window_position, 40);
    }
}
