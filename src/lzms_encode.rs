// SPDX-License-Identifier: LGPL-2.1-or-later
//! Native LZMS range/Huffman encoding with greedy LZ and delta matches.
//! Format algorithms translated from wimlib 1.14.5, Eric Biggers, 2013-2016.
use super::*;
use crate::memory::ensure_capacity;
/// Failure to allocate encoder storage or an oversized block.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EncodeError {
    /// The format permits at most 2^30 bytes.
    InputTooLarge,
    /// Encoder storage could not be allocated.
    OutOfMemory,
    /// Adaptive code construction failed.
    InvalidHuffmanCode,
}
impl core::fmt::Display for EncodeError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        write!(f, "LZMS encode error: {self:?}")
    }
}
impl core::error::Error for EncodeError {}
impl From<LzmsError> for EncodeError {
    fn from(error: LzmsError) -> Self {
        if error == LzmsError::OutOfMemory {
            Self::OutOfMemory
        } else {
            Self::InvalidHuffmanCode
        }
    }
}
#[derive(Debug)]
struct Forward {
    low: u64,
    range: u32,
    cache: u16,
    pending: usize,
    dummy: bool,
    words: Vec<u16>,
}
impl Forward {
    fn new(maximum: usize) -> Result<Self, EncodeError> {
        // A literal emits one decision; a match emits at most five and consumes
        // at least three bytes. Each decision can shift at most one 16-bit word.
        // Pending carry words count earlier shifts, not additional output.
        // Two words per input byte plus flush slack therefore suffice.
        Ok(Self {
            low: 0,
            range: u32::MAX,
            cache: 0,
            pending: 1,
            dummy: true,
            words: filled_buffer(
                maximum
                    .checked_mul(2)
                    .and_then(|size| size.checked_add(16))
                    .ok_or(EncodeError::OutOfMemory)?,
                0,
            )?,
        })
    }
    fn reset(&mut self) {
        self.low = 0;
        self.range = u32::MAX;
        self.cache = 0;
        self.pending = 1;
        self.dummy = true;
        self.words.clear();
    }
    fn shift(&mut self) -> Result<(), EncodeError> {
        if (self.low as u32) < 0xffff0000 || self.low >> 32 != 0 {
            ensure_capacity(&self.words, self.pending).map_err(|_| EncodeError::OutOfMemory)?;
            for _ in 0..self.pending {
                if self.dummy {
                    self.dummy = false;
                } else {
                    self.words
                        .push(self.cache.wrapping_add((self.low >> 32) as u16));
                }
                self.cache = 0xffff;
            }
            self.pending = 0;
            self.cache = (self.low >> 16) as u16;
        }
        self.pending += 1;
        self.low = (self.low & 0xffff) << 16;
        Ok(())
    }
    fn bit(&mut self, d: &mut Decision, bit: bool) -> Result<(), EncodeError> {
        if self.range <= 0xffff {
            self.range <<= 16;
            self.shift()?;
        }
        let entry = &mut d.probabilities[d.state];
        let bound = (self.range >> 6) * entry.zeros.clamp(1, 63);
        if bit {
            self.low += u64::from(bound);
            self.range -= bound;
        } else {
            self.range = bound;
        }
        entry.zeros = entry.zeros + (entry.recent >> 63) as u32 - u32::from(bit);
        entry.recent = (entry.recent << 1) | u64::from(bit);
        d.state = ((d.state << 1) | usize::from(bit)) & (d.probabilities.len() - 1);
        Ok(())
    }
}
#[derive(Debug)]
struct Code {
    huffman: Huffman,
    codes: Vec<(u32, u32)>,
}
impl Code {
    fn new(symbols: usize, period: usize) -> Result<Self, EncodeError> {
        let mut code = Self {
            huffman: Huffman::for_encoding(symbols, period)?,
            codes: filled_buffer(symbols, (0, 0))?,
        };
        code.rebuild_codes();
        Ok(code)
    }
    fn reset(&mut self, symbols: usize) -> Result<(), EncodeError> {
        self.huffman.reset(symbols)?;
        resize_within_capacity(&mut self.codes, symbols, (0, 0))
            .map_err(|_| EncodeError::OutOfMemory)?;
        self.rebuild_codes();
        Ok(())
    }
    fn rebuild_codes(&mut self) {
        // The decoder already retains each canonical length. Reconstruct codes
        // by symbol, avoiding a scan of all 32768 decoder-table entries.
        let lengths = &self.huffman.lengths[..self.codes.len()];
        let mut counts = [0u32; 16];
        for &length in lengths {
            if length != 0 {
                counts[length as usize] += 1;
            }
        }
        let mut next = [0u32; 16];
        for length in 2..16 {
            next[length] = (next[length - 1] + counts[length - 1]) << 1;
        }
        for (code, &length) in self.codes.iter_mut().zip(lengths) {
            *code = (next[length as usize], length);
            next[length as usize] += u32::from(length != 0);
        }
    }
}
#[derive(Debug)]
struct Backward {
    buffer: u64,
    count: u32,
    words: Vec<u16>,
}
impl Backward {
    fn new(maximum: usize) -> Result<Self, EncodeError> {
        // Codes have at most 15 bits and value extras at most 30. A literal
        // costs <=15 bits/byte; an LZ match <=90 bits per >=3 bytes; a delta
        // match <=105 bits per >=8 bytes. Thus <=30 bits/input byte, rounded
        // to two 16-bit words, covers every parser choice and final padding.
        Ok(Self {
            buffer: 0,
            count: 0,
            words: filled_buffer(
                maximum
                    .checked_mul(2)
                    .and_then(|size| size.checked_add(16))
                    .ok_or(EncodeError::OutOfMemory)?,
                0,
            )?,
        })
    }
    fn reset(&mut self) {
        self.buffer = 0;
        self.count = 0;
        self.words.clear();
    }
    fn bits(&mut self, value: u32, count: u32) -> Result<(), EncodeError> {
        self.buffer = (self.buffer << count) | u64::from(value);
        self.count += count;
        while self.count >= 16 {
            self.count -= 16;
            ensure_capacity(&self.words, 1).map_err(|_| EncodeError::OutOfMemory)?;
            self.words.push((self.buffer >> self.count) as u16);
        }
        Ok(())
    }
    fn symbol(&mut self, h: &mut Code, symbol: usize) -> Result<(), EncodeError> {
        let (code, length) = h.codes[symbol];
        self.bits(code, length)?;
        h.huffman.frequencies[symbol] += 1;
        h.huffman.remaining -= 1;
        if h.huffman.remaining == 0 {
            h.huffman.rebuild()?;
            h.rebuild_codes();
            for f in &mut h.huffman.frequencies {
                *f = (*f >> 1) + 1;
            }
        }
        Ok(())
    }

    fn value(
        &mut self,
        h: &mut Code,
        value: u32,
        bases: &[u32],
        extra: &[u32],
    ) -> Result<(), EncodeError> {
        let slot = bases.partition_point(|&b| b <= value) - 1;
        self.symbol(h, slot)?;
        self.bits(value - bases[slot], extra[slot])
    }
}
/// Compress one LZMS block into at most `capacity` bytes.
///
/// Returns `None` for fewer than four input bytes or insufficient capacity.
/// Uses native adaptive range and Huffman codes and a bounded greedy parser
/// with LZ offsets, repeat queues, delta spans, and x86 preprocessing;
/// compressed bytes and ratio differ from upstream's near-optimal parser.
/// All allocations are fallible. Compression levels are not exposed yet.
/// # Errors
/// Reports allocation failure, invalid internal codes, or input over 2^30 bytes.
pub fn compress_lzms(input: &[u8], capacity: usize) -> Result<Option<Vec<u8>>, EncodeError> {
    if input.len() > 1 << 30 {
        return Err(EncodeError::InputTooLarge);
    }
    if input.len() < 4 || capacity < 8 {
        return Ok(None);
    }
    let mut compressor = LzmsCompressor::new(input.len())?;
    let Some(bytes) = compressor.compress(input, capacity)? else {
        return Ok(None);
    };
    let mut output = Vec::new();
    output
        .try_reserve_exact(bytes.len())
        .map_err(|_| EncodeError::OutOfMemory)?;
    output.extend_from_slice(bytes);
    Ok(Some(output))
}
/// Actual retained LZMS match, code, probability, history and output workspaces.
#[derive(Debug)]
pub struct LzmsCompressor {
    maximum: usize,
    data: Vec<u8>,
    heads: Vec<u32>,
    links: Vec<u32>,
    last_target: X86FilterState,
    literals: Code,
    offsets: Code,
    lengths: Code,
    delta_offsets: Code,
    powers: Code,
    main: Decision,
    matched: Decision,
    lz: Decision,
    reps: [Decision; 2],
    delta: Decision,
    delta_reps: [Decision; 2],
    forward: Forward,
    backward: Backward,
    output: Vec<u8>,
}
impl LzmsCompressor {
    /// Allocate actual bounded workspaces through the selected ownership strategy.
    pub fn new(maximum: usize) -> Result<Self, EncodeError> {
        if maximum == 0 || maximum > 1 << 30 {
            return Err(EncodeError::InputTooLarge);
        }
        let slots = OFFSET_BASES.partition_point(|&b| b < maximum as u32);
        Ok(Self {
            maximum,
            data: filled_buffer(maximum, 0)?,
            heads: filled_buffer(65536, u32::MAX)?,
            links: filled_buffer(maximum, u32::MAX)?,
            last_target: X86FilterState::new()?,
            literals: Code::new(256, 1024)?,
            offsets: Code::new(slots, 1024)?,
            lengths: Code::new(54, 512)?,
            delta_offsets: Code::new(slots, 1024)?,
            powers: Code::new(8, 512)?,
            main: Decision::new(16)?,
            matched: Decision::new(32)?,
            lz: Decision::new(64)?,
            reps: [Decision::new(64)?, Decision::new(64)?],
            delta: Decision::new(64)?,
            delta_reps: [Decision::new(64)?, Decision::new(64)?],
            forward: Forward::new(maximum)?,
            backward: Backward::new(maximum)?,
            // Both streams reserve 2*maximum + 16 words, each two bytes.
            output: filled_buffer(
                maximum
                    .checked_mul(8)
                    .and_then(|size| size.checked_add(64))
                    .ok_or(EncodeError::OutOfMemory)?,
                0,
            )?,
        })
    }
    /// Compress into retained output without allocating; reset all adaptive state.
    pub fn compress(
        &mut self,
        input: &[u8],
        capacity: usize,
    ) -> Result<Option<&[u8]>, EncodeError> {
        if input.len() > self.maximum {
            return Err(EncodeError::InputTooLarge);
        }
        if input.len() < 4 || capacity < 8 {
            return Ok(None);
        }
        let Self {
            data,
            heads,
            links,
            last_target,
            literals,
            offsets,
            lengths,
            delta_offsets,
            powers,
            main,
            matched,
            lz,
            reps,
            delta,
            delta_reps,
            forward,
            backward,
            output,
            ..
        } = self;
        resize_within_capacity(data, input.len(), 0).map_err(|_| EncodeError::OutOfMemory)?;
        data.copy_from_slice(input);
        filter(data, last_target);
        let slots = OFFSET_BASES.partition_point(|&b| b < input.len() as u32);
        literals.reset(256)?;
        offsets.reset(slots)?;
        lengths.reset(54)?;
        delta_offsets.reset(slots)?;
        powers.reset(8)?;
        for decision in [&mut *main, &mut *matched, &mut *lz, &mut *delta] {
            decision.state = 0;
            decision.probabilities.fill(Probability::default());
        }
        for decision in reps.iter_mut().chain(delta_reps.iter_mut()) {
            decision.state = 0;
            decision.probabilities.fill(Probability::default());
        }
        forward.reset();
        backward.reset();
        // Every link reachable from a fresh head is assigned during insertion.
        // The previous block's unreachable links need no clearing.
        heads.fill(u32::MAX);
        let data = &data[..];
        let heads = &mut heads[..];
        let links = &mut links[..];
        let mut recent = [1u32, 2, 3, 4];
        let mut previous = 0u8;
        let mut recent_delta = [(0u32, 1u32), (0, 2), (0, 3), (0, 4)];
        let hash = |p: usize| {
            let value =
                u32::from(data[p]) | (u32::from(data[p + 1]) << 8) | (u32::from(data[p + 2]) << 16);
            (value.wrapping_mul(0x1e35a7bd) >> 16) as usize
        };
        let mut pos = 0;
        while pos < data.len() {
            let mut best = 0;
            let mut distance = 0;
            if pos + 3 <= data.len() {
                let mut candidate = heads[hash(pos)];
                let mut visits = 0;
                while candidate != u32::MAX && visits < 64 {
                    let at = candidate as usize;
                    if data[at + best] == data[pos + best] {
                        let limit = data.len() - pos;
                        let left = &data[at..at + limit];
                        let right = &data[pos..];
                        let len = crate::zlib::common_prefix(left, right);
                        if len > best {
                            best = len;
                            distance = pos - at;
                        }
                        if best >= 4096 || best == limit {
                            break;
                        }
                    }
                    candidate = links[candidate as usize];
                    visits += 1;
                }
            }
            let mut delta_best = 0;
            let mut delta_pair = (0u32, 1u32);
            if best != data.len() - pos {
                let _ = search_delta::<0>(data, pos, best, &mut delta_best, &mut delta_pair)
                    || search_delta::<1>(data, pos, best, &mut delta_best, &mut delta_pair)
                    || search_delta::<2>(data, pos, best, &mut delta_best, &mut delta_pair)
                    || search_delta::<3>(data, pos, best, &mut delta_best, &mut delta_pair)
                    || search_delta::<4>(data, pos, best, &mut delta_best, &mut delta_pair)
                    || search_delta::<5>(data, pos, best, &mut delta_best, &mut delta_pair)
                    || search_delta::<6>(data, pos, best, &mut delta_best, &mut delta_pair)
                    || search_delta::<7>(data, pos, best, &mut delta_best, &mut delta_pair);
            }
            let use_delta = delta_best >= 8 && delta_best > best;
            let take = if use_delta {
                delta_best
            } else if best >= 3 {
                best
            } else {
                1
            };
            forward.bit(main, use_delta || best >= 3)?;
            if use_delta {
                forward.bit(matched, true)?;
                let rep =
                    (0..3).find(|&i| recent_delta[i + usize::from(previous == 2)] == delta_pair);
                forward.bit(delta, rep.is_some())?;
                if let Some(i) = rep {
                    forward.bit(&mut delta_reps[0], i != 0)?;
                    if i != 0 {
                        forward.bit(&mut delta_reps[1], i == 2)?;
                    }
                    repeat(&mut recent_delta, i, previous == 2);
                } else {
                    backward.symbol(powers, delta_pair.0 as usize)?;
                    backward.value(delta_offsets, delta_pair.1, OFFSET_BASES, EXTRA_OFFSET_BITS)?;
                    insert(&mut recent_delta);
                }
                recent_delta[0] = delta_pair;
                previous = 2;
                backward.value(lengths, delta_best as u32, LENGTH_BASES, EXTRA_LENGTH_BITS)?;
            } else if best < 3 {
                backward.symbol(literals, usize::from(data[pos]))?;
                previous = 0;
            } else {
                forward.bit(matched, false)?;
                let rep =
                    (0..3).find(|&i| recent[i + usize::from(previous == 1)] == distance as u32);
                forward.bit(lz, rep.is_some())?;
                if let Some(i) = rep {
                    forward.bit(&mut reps[0], i != 0)?;
                    if i != 0 {
                        forward.bit(&mut reps[1], i == 2)?;
                    }
                    repeat(&mut recent, i, previous == 1);
                } else {
                    backward.value(offsets, distance as u32, OFFSET_BASES, EXTRA_OFFSET_BITS)?;
                    insert(&mut recent);
                }
                recent[0] = distance as u32;
                previous = 1;
                backward.value(lengths, best as u32, LENGTH_BASES, EXTRA_LENGTH_BITS)?;
            }
            let insert = data.len().saturating_sub(2).saturating_sub(pos).min(take);
            for (relative, link) in links[pos..pos + insert].iter_mut().enumerate() {
                let p = pos + relative;
                let h = hash(p);
                *link = heads[h];
                heads[h] = p as u32;
            }
            pos += take;
        }
        if backward.count != 0 {
            backward.bits(0, 16 - backward.count)?;
        }
        for _ in 0..4 {
            forward.shift()?;
        }
        let size = (forward.words.len() + backward.words.len()) * 2;
        if size > capacity & !1 {
            return Ok(None);
        }
        let output = output.get_mut(..size).ok_or(EncodeError::OutOfMemory)?;
        for (bytes, word) in output
            .chunks_exact_mut(2)
            .zip(forward.words.iter().chain(backward.words.iter().rev()))
        {
            bytes.copy_from_slice(&word.to_le_bytes());
        }
        Ok(Some(output))
    }
}

// Keep the fixed stride visible to the caller so index checks can simplify.
#[inline(always)]
fn search_delta<const POWER: u32>(
    data: &[u8],
    pos: usize,
    best: usize,
    delta_best: &mut usize,
    delta_pair: &mut (u32, u32),
) -> bool {
    let span = 1usize << POWER;
    for raw in [1usize, 2, 3, 4] {
        let offset = raw * span;
        if offset + span > pos {
            continue;
        }
        let at = pos + best.max(*delta_best);
        if data[at - offset]
            .wrapping_add(data[at - span])
            .wrapping_sub(data[at - offset - span])
            != data[at]
        {
            continue;
        }
        let mut len = 0;
        while pos + len < data.len() {
            let at = pos + len;
            let prediction = data[at - offset]
                .wrapping_add(data[at - span])
                .wrapping_sub(data[at - offset - span]);
            if prediction != data[at] {
                break;
            }
            len += 1;
        }
        if len > *delta_best {
            *delta_best = len;
            *delta_pair = (POWER, raw as u32);
            if len == data.len() - pos {
                return true;
            }
        }
    }
    false
}

fn filter(data: &mut [u8], state: &mut X86FilterState) {
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
        let target = ((index as u32).wrapping_add(u32::from(u16::from_le_bytes([
            data[start],
            data[start + 1],
        ]))) & 0xffff) as usize;
        if index - last_x86 <= max_offset {
            let value = u32::from_le_bytes([
                data[start],
                data[start + 1],
                data[start + 2],
                data[start + 3],
            ]);
            data[start..start + 4].copy_from_slice(&value.wrapping_add(index as u32).to_le_bytes());
        }
        let instruction_end = index + opcode_length as i32 + 3;
        let stamp = base + instruction_end;
        if last_target[target] >= base && stamp - last_target[target] <= 65535 {
            last_x86 = instruction_end;
        }
        last_target[target] = stamp;
        position = start + 4;
    }
}

#[cfg(test)]
mod workspace_tests {
    use super::*;
    use alloc::vec;

    #[test]
    fn retained_output_overwrites_stale_bytes_for_shorter_reused_blocks() {
        let mut reused = LzmsCompressor::new(65536).unwrap();
        for length in [65536, 4, 17, 18, 640, 4096, 31, 65536] {
            let input: Vec<u8> = (0..length).map(|i| (i * 37) as u8).collect();
            reused.output.fill(0xa5);
            let actual = reused
                .compress(&input, usize::MAX)
                .unwrap()
                .unwrap()
                .to_vec();
            let mut fresh = LzmsCompressor::new(65536).unwrap();
            assert_eq!(actual, fresh.compress(&input, usize::MAX).unwrap().unwrap());
        }
    }

    #[test]
    fn specialized_delta_strides_preserve_scalar_selection_and_ties() {
        let mut random = 19u32;
        for pattern in 0..10 {
            let data: Vec<u8> = (0..2048)
                .map(|i| {
                    random ^= random << 13;
                    random ^= random >> 17;
                    random ^= random << 5;
                    if pattern == 0 {
                        random as u8
                    } else {
                        ((i / (1 << (pattern - 1))) * 37 + i % (1 << (pattern - 1))) as u8
                    }
                })
                .collect();
            for pos in [0, 1, 2, 5, 31, 127, 128, 255, 639, 640, 641, 1024, 2047] {
                for best in [
                    0,
                    1.min(data.len() - pos - 1),
                    (data.len() - pos) / 2,
                    data.len() - pos - 1,
                ] {
                    let mut delta_best = 0;
                    let mut delta_pair = (0, 1);
                    let _ = search_delta::<0>(&data, pos, best, &mut delta_best, &mut delta_pair)
                        || search_delta::<1>(&data, pos, best, &mut delta_best, &mut delta_pair)
                        || search_delta::<2>(&data, pos, best, &mut delta_best, &mut delta_pair)
                        || search_delta::<3>(&data, pos, best, &mut delta_best, &mut delta_pair)
                        || search_delta::<4>(&data, pos, best, &mut delta_best, &mut delta_pair)
                        || search_delta::<5>(&data, pos, best, &mut delta_best, &mut delta_pair)
                        || search_delta::<6>(&data, pos, best, &mut delta_best, &mut delta_pair)
                        || search_delta::<7>(&data, pos, best, &mut delta_best, &mut delta_pair);
                    assert_eq!(
                        (delta_best, delta_pair),
                        scalar_delta_reference(&data, pos, best),
                        "pattern={pattern}, pos={pos}, best={best}"
                    );
                }
            }
        }
    }

    fn scalar_delta_reference(data: &[u8], pos: usize, best: usize) -> (usize, (u32, u32)) {
        let mut delta_best = 0;
        let mut delta_pair = (0u32, 1u32);
        'delta_candidates: for power in 0..8 {
            // Delta ties lose to LZ, so no delta can improve a match that
            // already reaches the end of this independent input block.
            if best == data.len() - pos {
                break;
            }
            let span = 1usize << power;
            for raw in 1..=4usize {
                let offset = raw * span;
                if offset + span > pos {
                    continue;
                }
                let probe = best.max(delta_best);
                let at = pos + probe;
                if data[at - offset]
                    .wrapping_add(data[at - span])
                    .wrapping_sub(data[at - offset - span])
                    != data[at]
                {
                    continue;
                }
                let mut len = 0;
                while pos + len < data.len() {
                    let at = pos + len;
                    let prediction = data[at - offset]
                        .wrapping_add(data[at - span])
                        .wrapping_sub(data[at - offset - span]);
                    if prediction != data[at] {
                        break;
                    }
                    len += 1;
                }
                if len > delta_best {
                    delta_best = len;
                    delta_pair = (power as u32, raw as u32);
                    // Later candidates cannot improve this length; retain
                    // the first pair, matching the existing strict tie rule.
                    if len == data.len() - pos {
                        break 'delta_candidates;
                    }
                }
            }
        }
        (delta_best, delta_pair)
    }

    #[test]
    fn word_scanning_and_reused_filter_state_match_scalar_reference() {
        let mut state = X86FilterState::new().unwrap();
        let mut random = 7u32;
        for case in 0..24 {
            let mut input: Vec<u8> = (0..4096)
                .map(|_| {
                    random ^= random << 13;
                    random ^= random >> 17;
                    random ^= random << 5;
                    random as u8
                })
                .collect();
            for at in (1 + case % 8..4000).step_by(23) {
                input[at..at + 5].copy_from_slice(&[0xe8, 0x42, 0, 0, 0]);
            }
            if case == 12 {
                state.next_base = i32::MAX - 1;
            }
            let mut expected = input.clone();
            scalar_filter_reference(&mut expected, &mut vec![-65536; 65536]);
            filter(&mut input, &mut state);
            assert_eq!(input, expected, "case={case}");
        }
    }
    fn scalar_filter_reference(data: &mut [u8], last_target: &mut [i32]) {
        if data.len() <= 17 {
            return;
        }
        last_target.fill(-65536);
        let mut last_x86 = -1024i32;
        let mut position = 1usize;
        while position < data.len() - 16 {
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
            let target = ((index as u32).wrapping_add(u32::from(u16::from_le_bytes([
                data[start],
                data[start + 1],
            ]))) & 0xffff) as usize;
            if index - last_x86 <= max_offset {
                let value = u32::from_le_bytes([
                    data[start],
                    data[start + 1],
                    data[start + 2],
                    data[start + 3],
                ]);
                data[start..start + 4]
                    .copy_from_slice(&value.wrapping_add(index as u32).to_le_bytes());
            }
            let instruction_end = index + opcode_length as i32 + 3;
            if instruction_end - last_target[target] <= 65535 {
                last_x86 = instruction_end;
            }
            last_target[target] = instruction_end;
            position = start + 4;
        }
    }

    #[test]
    fn expanded_blocks_reuse_bounded_stream_storage() {
        for maximum in [4, 17, 257, 4096, 32768] {
            let mut encoder = LzmsCompressor::new(maximum).unwrap();
            let capacities = (
                encoder.forward.words.capacity(),
                encoder.backward.words.capacity(),
                encoder.output.capacity(),
            );
            let mut state = 0x57494du32;
            let random: Vec<_> = (0..maximum)
                .map(|_| {
                    state ^= state << 13;
                    state ^= state >> 17;
                    state ^= state << 5;
                    state as u8
                })
                .collect();
            for input in [
                random,
                vec![0; maximum],
                (0..maximum).map(|i| i as u8).collect(),
            ] {
                let encoded = encoder.compress(&input, usize::MAX).unwrap().unwrap();
                let mut decoded = vec![0; maximum];
                decompress_lzms(encoded, &mut decoded).unwrap();
                assert_eq!(decoded, input);
                assert_eq!(
                    (
                        encoder.forward.words.capacity(),
                        encoder.backward.words.capacity(),
                        encoder.output.capacity(),
                    ),
                    capacities,
                );
            }
        }
    }
}
