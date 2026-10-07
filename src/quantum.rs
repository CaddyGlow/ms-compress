// SPDX-License-Identifier: LGPL-2.1-only
// Format/model references: (C) 2003-2023 Stuart Caie, Matthew Russotto;
// (C) 1999-2026 Igor Pavlov. Rust implementation added in 2026.
//! CAB Quantum arithmetic/LZ decoding with persistent models and history.
//!
//! Format and adaptive-model behavior reviewed against libmspack's `qtmd.c`
//! (Stuart Caie / Matthew Russotto) and 7-Zip's Quantum decoder. See
//! `QUANTUM-NOTICE.md` for provenance and LGPL terms. No unsafe code is used.

use alloc::vec;
use alloc::vec::Vec;
/// A malformed Quantum frame or invalid decoder configuration.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum QuantumError {
    /// The encoder level is outside 1 through 7.
    InvalidLevel,
    /// The dictionary order is outside CAB's 10 through 21 range.
    InvalidWindow,
    /// Output must contain 1 through 32768 bytes.
    InvalidFrameSize,
    /// The compressed input exceeds the CAB block bound of 38912 bytes.
    InvalidInputSize,
    /// A required bit is missing; implicit zero padding is never supplied.
    TruncatedInput,
    /// The arithmetic code lies outside its current interval.
    InvalidArithmeticCode,
    /// A match refers to unavailable history or crosses the output boundary.
    InvalidMatch,
    /// The frame termination or trailing padding is invalid.
    InvalidPadding,
    /// A previous error invalidated this decoder's persistent state.
    FailedDecoder,
}

impl core::fmt::Display for QuantumError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.write_str(match self {
            Self::InvalidLevel => "Quantum encoder level must be 1 through 7",
            Self::InvalidWindow => "Quantum dictionary order must be 10 through 21",
            Self::InvalidFrameSize => "Quantum frame size must be 1 through 32768",
            Self::InvalidInputSize => "Quantum input exceeds the CAB block size limit",
            Self::TruncatedInput => "truncated Quantum frame",
            Self::InvalidArithmeticCode => "invalid Quantum arithmetic interval",
            Self::InvalidMatch => "Quantum match exceeds history or frame boundary",
            Self::InvalidPadding => "invalid Quantum frame termination or padding",
            Self::FailedDecoder => "Quantum decoder invalidated by a previous error",
        })
    }
}
impl core::error::Error for QuantumError {}

type Result<T> = core::result::Result<T, QuantumError>;

#[derive(Clone, Copy)]
struct Symbol {
    value: usize,
    cumulative: u32,
}

struct Model {
    symbols: Vec<Symbol>,
    shifts_left: u8,
}

impl Model {
    fn new(start: usize, count: usize) -> Self {
        Self {
            symbols: (0..=count)
                .map(|index| Symbol {
                    value: start + index,
                    cumulative: (count - index) as u32,
                })
                .collect(),
            shifts_left: 4,
        }
    }

    fn decode(&mut self, arithmetic: &mut Arithmetic<'_>) -> Result<usize> {
        let total = self.symbols[0].cumulative;
        let threshold = arithmetic.threshold(total)?;
        let index = self.symbols[1..]
            .iter()
            .position(|s| s.cumulative <= threshold)
            .ok_or(QuantumError::InvalidArithmeticCode)?
            + 1;
        let symbol = self.symbols[index - 1];
        arithmetic.narrow(self.symbols[index].cumulative, symbol.cumulative, total)?;
        self.update(index);
        Ok(symbol.value)
    }

    fn update(&mut self, index: usize) {
        for entry in &mut self.symbols[..index] {
            entry.cumulative += 8;
        }
        if self.symbols[0].cumulative > 3800 {
            self.rescale();
        }
    }

    fn encode(&mut self, value: usize, arithmetic: &mut ArithmeticEncoder) -> Result<()> {
        // Encoder symbols come exclusively from the model's validated alphabet.
        if let Some(index) = self.symbols[..self.symbols.len() - 1]
            .iter()
            .position(|s| s.value == value)
        {
            arithmetic.narrow(
                self.symbols[index + 1].cumulative,
                self.symbols[index].cumulative,
                self.symbols[0].cumulative,
            );
            self.update(index + 1);
            Ok(())
        } else {
            Err(QuantumError::InvalidArithmeticCode)
        }
    }

    fn rescale(&mut self) {
        let count = self.symbols.len() - 1;
        self.shifts_left -= 1;
        if self.shifts_left != 0 {
            for index in (0..count).rev() {
                self.symbols[index].cumulative = (self.symbols[index].cumulative >> 1)
                    .max(self.symbols[index + 1].cumulative + 1);
            }
        } else {
            self.shifts_left = 50;
            for index in 0..count {
                self.symbols[index].cumulative =
                    (self.symbols[index].cumulative - self.symbols[index + 1].cumulative + 1) >> 1;
            }
            // This exact swap order is part of the format. A stable sort (or
            // choosing only one maximum per pass) gives different tied models.
            for index in 0..count - 1 {
                for other in index + 1..count {
                    if self.symbols[index].cumulative < self.symbols[other].cumulative {
                        self.symbols.swap(index, other);
                    }
                }
            }
            for index in (0..count).rev() {
                self.symbols[index].cumulative += self.symbols[index + 1].cumulative;
            }
        }
    }
}

struct Bits<'a> {
    input: &'a [u8],
    position: usize,
}
impl Bits<'_> {
    fn read(&mut self, count: usize) -> Result<u32> {
        if count > 19 || self.position + count > self.input.len() * 8 {
            return Err(QuantumError::TruncatedInput);
        }
        let mut value = 0;
        for _ in 0..count {
            value = (value << 1)
                | u32::from((self.input[self.position / 8] >> (7 - self.position % 8)) & 1);
            self.position += 1;
        }
        Ok(value)
    }
    fn finish(&mut self) -> Result<()> {
        // Arithmetic termination contains at least two zero bits and ends on
        // a byte boundary. CAB producers may append zero through four bytes.
        let count = 2 + ((14 - self.position % 8) & 7);
        if self.read(count)? != 0 {
            return Err(QuantumError::InvalidPadding);
        }
        let tail = &self.input[self.position / 8..];
        if tail.len() > 4 || tail.iter().any(|&byte| byte != 0) {
            return Err(QuantumError::InvalidPadding);
        }
        Ok(())
    }
}

struct Arithmetic<'a> {
    low: u32,
    high: u32,
    code: u32,
    bits: Bits<'a>,
}
impl<'a> Arithmetic<'a> {
    fn new(input: &'a [u8]) -> Result<Self> {
        let mut bits = Bits { input, position: 0 };
        let code = bits.read(16)?;
        Ok(Self {
            low: 0,
            high: 65535,
            code,
            bits,
        })
    }
    fn threshold(&self, total: u32) -> Result<u32> {
        if self.code < self.low || self.code > self.high {
            return Err(QuantumError::InvalidArithmeticCode);
        }
        Ok(((self.code - self.low + 1) * total - 1) / (self.high - self.low + 1))
    }
    fn narrow(&mut self, lower: u32, upper: u32, total: u32) -> Result<()> {
        let range = self.high - self.low + 1;
        self.high = self.low + upper * range / total - 1;
        self.low += lower * range / total;
        loop {
            if (self.low ^ self.high) & 0x8000 != 0 {
                if self.low & 0x4000 == 0 || self.high & 0x4000 != 0 {
                    break;
                }
                self.code ^= 0x4000;
                self.low &= 0x3fff;
                self.high |= 0x4000;
            }
            self.low = (self.low << 1) & 65535;
            self.high = ((self.high << 1) | 1) & 65535;
            self.code = ((self.code << 1) | self.bits.read(1)?) & 65535;
        }
        Ok(())
    }
}

fn position_slot(slot: usize) -> (usize, usize) {
    let extra = slot.saturating_sub(2) / 2;
    let base = if slot < 4 {
        slot
    } else {
        (2 | (slot & 1)) << extra
    };
    (base, extra)
}
fn length_slot(slot: usize) -> (usize, usize) {
    if slot == 26 {
        return (254, 0);
    }
    let extra = slot.saturating_sub(2) / 4;
    let mut base = 0;
    for index in 0..slot {
        base += 1 << (index.saturating_sub(2) / 4);
    }
    (base, extra)
}

/// Persistent decoder for complete, reassembled CFDATA Quantum frames.
///
/// Arithmetic intervals restart for each frame; adaptive models and dictionary
/// history persist across frames and cabinet boundaries. Errors invalidate the
/// decoder. Discard output from a failed call and start a fresh folder decoder.
pub struct QuantumDecoder {
    models: [Model; 9],
    window: Vec<u8>,
    position: usize,
    available: usize,
    failed: bool,
}
impl QuantumDecoder {
    /// Create a folder decoder using the dictionary order from `typeCompress`.
    pub fn new(window_order: u8) -> Result<Self> {
        if !(10..=21).contains(&window_order) {
            return Err(QuantumError::InvalidWindow);
        }
        let positions = usize::from(window_order) * 2;
        Ok(Self {
            models: initial_models(positions),
            window: vec![0; 1usize << window_order],
            position: 0,
            available: 0,
            failed: false,
        })
    }
    /// Decode exactly one frame of 1 through 32768 uncompressed bytes.
    pub fn decompress_frame(&mut self, input: &[u8], output: &mut [u8]) -> Result<()> {
        if self.failed {
            return Err(QuantumError::FailedDecoder);
        }
        let result = self.decode(input, output);
        if result.is_err() {
            self.failed = true;
        }
        result
    }
    fn emit(&mut self, byte: u8) {
        self.window[self.position] = byte;
        self.position = (self.position + 1) & (self.window.len() - 1);
        self.available = (self.available + 1).min(self.window.len());
    }
    fn decode(&mut self, input: &[u8], output: &mut [u8]) -> Result<()> {
        if output.is_empty() || output.len() > 32768 {
            return Err(QuantumError::InvalidFrameSize);
        }
        if input.len() > 38912 {
            return Err(QuantumError::InvalidInputSize);
        }
        let mut arithmetic = Arithmetic::new(input)?;
        let mut written = 0;
        while written < output.len() {
            let selector = self.models[8].decode(&mut arithmetic)?;
            if selector < 4 {
                let byte = self.models[selector].decode(&mut arithmetic)? as u8;
                output[written] = byte;
                written += 1;
                self.emit(byte);
            } else {
                let length = if selector == 6 {
                    let slot = self.models[7].decode(&mut arithmetic)?;
                    let (base, extra) = length_slot(slot);
                    base + arithmetic.bits.read(extra)? as usize + 5
                } else {
                    selector - 1
                };
                let slot = self.models[selector].decode(&mut arithmetic)?;
                let (base, extra) = position_slot(slot);
                let distance = base + arithmetic.bits.read(extra)? as usize + 1;
                if length > output.len() - written || distance > self.available {
                    return Err(QuantumError::InvalidMatch);
                }
                for byte in &mut output[written..written + length] {
                    *byte = self.window
                        [(self.position + self.window.len() - distance) & (self.window.len() - 1)];
                    self.emit(*byte);
                }
                written += length;
            }
        }
        arithmetic.bits.finish()
    }
}

struct ArithmeticEncoder {
    low: u32,
    high: u32,
    pending: usize,
    shifts: usize,
    bits: Vec<bool>,
    raw: Vec<(usize, usize, usize)>,
}
impl ArithmeticEncoder {
    fn new() -> Self {
        Self {
            low: 0,
            high: 65535,
            pending: 0,
            shifts: 0,
            bits: Vec::new(),
            raw: Vec::new(),
        }
    }
    fn emit(&mut self, bit: bool) {
        self.bits.push(bit);
        self.bits.extend(core::iter::repeat_n(!bit, self.pending));
        self.pending = 0;
    }
    fn narrow(&mut self, lower: u32, upper: u32, total: u32) {
        let range = self.high - self.low + 1;
        self.high = self.low + upper * range / total - 1;
        self.low += lower * range / total;
        loop {
            if self.high < 0x8000 {
                self.emit(false);
            } else if self.low >= 0x8000 {
                self.emit(true);
                self.low -= 0x8000;
                self.high -= 0x8000;
            } else if self.low >= 0x4000 && self.high < 0xc000 {
                self.pending += 1;
                self.low -= 0x4000;
                self.high -= 0x4000;
            } else {
                break;
            }
            self.low <<= 1;
            self.high = (self.high << 1) | 1;
            self.shifts += 1;
        }
    }
    fn raw(&mut self, value: usize, count: usize) {
        if count != 0 {
            self.raw.push((16 + self.shifts, value, count));
        }
    }
    fn finish(mut self) -> Vec<u8> {
        self.pending += 1;
        self.emit(self.low >= 0x4000);
        self.bits.extend([false; 16]);
        // Raw match bits are inserted after the decoder's 16-bit lookahead,
        // at the point where narrowing has consumed the corresponding shifts.
        let mut output = Vec::new();
        let mut byte = 0u8;
        let mut used = 0;
        let mut put = |bit: bool| {
            byte = (byte << 1) | u8::from(bit);
            used += 1;
            if used == 8 {
                output.push(byte);
                byte = 0;
                used = 0;
            }
        };
        let mut events = self.raw.into_iter().peekable();
        for index in 0..=self.bits.len() {
            while events.peek().is_some_and(|event| event.0 == index) {
                if let Some((_, value, count)) = events.next() {
                    for shift in (0..count).rev() {
                        put(value & (1 << shift) != 0);
                    }
                }
            }
            if let Some(&bit) = self.bits.get(index) {
                put(bit);
            }
        }
        if used != 0 {
            output.push(byte << (8 - used));
        }
        output
    }
}

/// Persistent Quantum encoder with greedy LZ matches and adaptive arithmetic models.
///
/// Models and dictionary history persist across frames. Arithmetic intervals
/// restart per frame, as required by CAB. Levels control bounded match searches.
pub struct QuantumEncoder {
    models: [Model; 9],
    history: Vec<u8>,
    window: usize,
    positions: usize,
    level: u8,
}
impl QuantumEncoder {
    /// Create an encoder with dictionary order 10 through 21 and level 1 through 7.
    pub fn new(window_order: u8, level: u8) -> Result<Self> {
        if !(10..=21).contains(&window_order) {
            return Err(QuantumError::InvalidWindow);
        }
        if !(1..=7).contains(&level) {
            return Err(QuantumError::InvalidLevel);
        }
        Ok(Self {
            models: initial_models(usize::from(window_order) * 2),
            history: Vec::new(),
            window: 1 << window_order,
            positions: usize::from(window_order) * 2,
            level,
        })
    }

    /// Encode one frame of 1 through 32768 bytes, retaining folder history.
    pub fn compress_frame(&mut self, input: &[u8]) -> Result<Vec<u8>> {
        if input.is_empty() || input.len() > 32768 {
            return Err(QuantumError::InvalidFrameSize);
        }
        let mut data = Vec::with_capacity(self.history.len() + input.len());
        data.extend_from_slice(&self.history);
        data.extend_from_slice(input);
        let mut heads = vec![usize::MAX; 65536];
        let mut links = vec![usize::MAX; data.len()];
        let hash = |position: usize| {
            let bytes = &data[position..position + 3];
            let word = u32::from(bytes[0]) | u32::from(bytes[1]) << 8 | u32::from(bytes[2]) << 16;
            (word.wrapping_mul(0x1e35a7bd) >> 16) as usize
        };
        for (position, link) in links.iter_mut().enumerate().take(self.history.len()) {
            if position + 3 <= data.len() {
                let bucket = hash(position);
                *link = heads[bucket];
                heads[bucket] = position;
            }
        }
        let mut arithmetic = ArithmeticEncoder::new();
        let mut position = self.history.len();
        while position < data.len() {
            let mut length = 0;
            let mut distance = 0;
            if position + 3 <= data.len() {
                let mut candidate = heads[hash(position)];
                let mut visits = 0;
                let limit = (data.len() - position).min(259);
                while candidate != usize::MAX
                    && position - candidate <= self.window
                    && visits < (4usize << self.level)
                {
                    let found = crate::zlib::common_prefix(
                        &data[candidate..candidate + limit],
                        &data[position..position + limit],
                    );
                    let dist = position - candidate;
                    let slots = if found == 3 {
                        self.positions.min(24)
                    } else if found == 4 {
                        self.positions.min(36)
                    } else {
                        self.positions
                    };
                    let (base, extra) = position_slot(slots - 1);
                    if found > length && dist - 1 < base + (1 << extra) {
                        length = found;
                        distance = dist;
                    }
                    if length == limit {
                        break;
                    }
                    candidate = links[candidate];
                    visits += 1;
                }
            }
            let consumed = if length >= 3 {
                let selector = if length == 3 {
                    4
                } else if length == 4 {
                    5
                } else {
                    6
                };
                self.models[8].encode(selector, &mut arithmetic)?;
                if selector == 6 {
                    let value = length - 5;
                    let slot = (0..27)
                        .find(|&slot| {
                            let (base, extra) = length_slot(slot);
                            value >= base && value < base + (1 << extra)
                        })
                        .ok_or(QuantumError::InvalidMatch)?;
                    self.models[7].encode(slot, &mut arithmetic)?;
                    let (base, extra) = length_slot(slot);
                    arithmetic.raw(value - base, extra);
                }
                let value = distance - 1;
                let slot = (0..self.positions)
                    .find(|&slot| {
                        let (base, extra) = position_slot(slot);
                        value >= base && value < base + (1 << extra)
                    })
                    .ok_or(QuantumError::InvalidMatch)?;
                self.models[selector].encode(slot, &mut arithmetic)?;
                let (base, extra) = position_slot(slot);
                arithmetic.raw(value - base, extra);
                length
            } else {
                let byte = usize::from(data[position]);
                let selector = byte / 64;
                self.models[8].encode(selector, &mut arithmetic)?;
                self.models[selector].encode(byte, &mut arithmetic)?;
                1
            };
            for (p, link) in links.iter_mut().enumerate().skip(position).take(consumed) {
                if p + 3 <= data.len() {
                    let bucket = hash(p);
                    *link = heads[bucket];
                    heads[bucket] = p;
                }
            }
            position += consumed;
        }
        self.history.clear();
        self.history
            .extend_from_slice(&data[data.len().saturating_sub(self.window)..]);
        Ok(arithmetic.finish())
    }
}

fn initial_models(positions: usize) -> [Model; 9] {
    [
        Model::new(0, 64),
        Model::new(64, 64),
        Model::new(128, 64),
        Model::new(192, 64),
        Model::new(0, positions.min(24)),
        Model::new(0, positions.min(36)),
        Model::new(0, positions),
        Model::new(0, 27),
        Model::new(0, 7),
    ]
}
