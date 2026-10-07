// SPDX-License-Identifier: LGPL-2.1-or-later
// Format and E8 transformation derived from wimlib cd5e231c348c255ae5088873b5a66ee0eb96fa07.
// See LZX-NOTICE.md. Match selection and balanced Huffman encoder are original Rust.
//! Native WIM LZX encoding with greedy matches and frequency-weighted codes.

use alloc::vec::Vec;
/// An invalid compression configuration or allocation failure.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EncodeError {
    /// The configured maximum is zero or exceeds 2 MiB.
    InvalidMaxBlockSize,
    /// Input exceeds the configured maximum.
    InputExceedsLimit,
    /// Memory allocation failed.
    OutOfMemory,
}
impl core::fmt::Display for EncodeError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.write_str(match self {
            Self::InvalidMaxBlockSize => "invalid LZX compressor maximum",
            Self::InputExceedsLimit => "LZX input exceeds configured maximum",
            Self::OutOfMemory => "LZX compressor allocation failed",
        })
    }
}
impl core::error::Error for EncodeError {}
const BASE: [usize; 50] = [
    0, 0, 0, 1, 2, 4, 6, 10, 14, 22, 30, 46, 62, 94, 126, 190, 254, 382, 510, 766, 1022, 1534,
    2046, 3070, 4094, 6142, 8190, 12286, 16382, 24574, 32766, 49150, 65534, 98302, 131070, 196606,
    262142, 393214, 524286, 655358, 786430, 917502, 1048574, 1179646, 1310718, 1441790, 1572862,
    1703934, 1835006, 1966078,
];
const EXTRA: [usize; 50] = [
    0, 0, 0, 0, 1, 1, 2, 2, 3, 3, 4, 4, 5, 5, 6, 6, 7, 7, 8, 8, 9, 9, 10, 10, 11, 11, 12, 12, 13,
    13, 14, 14, 15, 15, 16, 16, 17, 17, 17, 17, 17, 17, 17, 17, 17, 17, 17, 17, 17, 17,
];
#[derive(Clone, Copy, Debug)]
struct Token {
    symbol: usize,
    length: usize,
    slot: usize,
    offset: usize,
}

/// Compress one independently framed WIM LZX chunk.
///
/// Uses a verbatim block, greedy three-byte hash matching, and frequency-ranked
/// canonical Huffman codes with a bounded balanced fallback. This is a format-compatible compressor;
/// Supports repeat offsets and run-length encoded code tables. Compression level
/// tuning, aligned blocks, and lazy parsing are not yet implemented. Reusable workspace
/// ownership is available through LzxCompressor. Input is preserved, including for E8 translation.
/// Returns `None` for empty input or when the encoded bytes exceed `capacity`.
/// Allocation failures and invalid sizes are reported without panicking.
pub fn compress_lzx(
    input: &[u8],
    capacity: usize,
    max_block_size: usize,
) -> Result<Option<Vec<u8>>, EncodeError> {
    if max_block_size == 0 || max_block_size > 1 << 21 {
        return Err(EncodeError::InvalidMaxBlockSize);
    }
    if input.len() > max_block_size {
        return Err(EncodeError::InputExceedsLimit);
    }
    if input.is_empty() || capacity == 0 {
        return Ok(None);
    }
    let mut compressor = LzxCompressor::new(max_block_size)?;
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
/// Reusable LZX preprocessing, match, token and output storage.
#[derive(Debug)]
pub struct LzxCompressor {
    maximum: usize,
    data: Vec<u8>,
    previous: Vec<usize>,
    links: Vec<usize>,
    tokens: Vec<Token>,
    output: Vec<u8>,
}
impl LzxCompressor {
    /// Allocate actual bounded compressor workspaces through an ownership strategy.
    pub fn new(maximum: usize) -> Result<Self, EncodeError> {
        if maximum == 0 || maximum > 1 << 21 {
            return Err(EncodeError::InvalidMaxBlockSize);
        }
        Ok(Self {
            maximum,
            data: crate::memory::filled_vec(maximum, 0).map_err(|_| EncodeError::OutOfMemory)?,
            previous: crate::memory::filled_vec(65536, usize::MAX)
                .map_err(|_| EncodeError::OutOfMemory)?,
            links: crate::memory::filled_vec(maximum, usize::MAX)
                .map_err(|_| EncodeError::OutOfMemory)?,
            tokens: crate::memory::filled_vec(
                maximum,
                Token {
                    symbol: 0,
                    length: 0,
                    slot: 0,
                    offset: 0,
                },
            )
            .map_err(|_| EncodeError::OutOfMemory)?,
            output: crate::memory::filled_vec(maximum * 3 + 4096, 0)
                .map_err(|_| EncodeError::OutOfMemory)?,
        })
    }
    /// Compress into retained output storage without allocating on each call.
    pub fn compress(
        &mut self,
        input: &[u8],
        capacity: usize,
    ) -> Result<Option<&[u8]>, EncodeError> {
        self.compress_frame(input, capacity, None)
    }

    fn compress_frame(
        &mut self,
        input: &[u8],
        capacity: usize,
        cabinet: Option<&mut CabinetState>,
    ) -> Result<Option<&[u8]>, EncodeError> {
        let max_block_size = self.maximum;
        if input.len() > max_block_size {
            return Err(EncodeError::InputExceedsLimit);
        }
        if input.is_empty() || capacity == 0 {
            return Ok(None);
        }
        let order = cabinet.as_ref().map_or_else(
            || (usize::BITS - (max_block_size - 1).leading_zeros()).max(15) as usize,
            |state| usize::from(state.window_order),
        );
        let max_offset = (1usize << order) - 3;
        let mut slots = 30;
        while slots < 50 && max_offset >= BASE[slots] {
            slots += 1;
        }
        let main_count = 256 + slots * 8;
        crate::memory::resize_within_capacity(&mut self.data, input.len(), 0)
            .map_err(|_| EncodeError::OutOfMemory)?;
        self.data.copy_from_slice(input);
        if cabinet.is_none() {
            forward_e8(&mut self.data);
        }
        self.previous.fill(usize::MAX);
        self.tokens.clear();
        let data = &self.data;
        let previous = &mut self.previous;
        let links = &mut self.links;
        let tokens = &mut self.tokens;
        let mut main_freq = [0usize; 656];
        let mut length_freq = [0usize; 249];
        let mut position = 0;
        let mut recent = cabinet.as_ref().map_or([1; 3], |state| state.recent);
        while position < data.len() {
            let mut length = 0usize;
            let mut offset = 0;
            if position + 3 <= data.len() {
                let limit = 257.min(data.len() - position);
                // Repeat candidates may be older than the bounded hash chain.
                // Check them directly and retain R0/R1/R2 order on equal lengths.
                for &distance in &recent {
                    if distance > position {
                        continue;
                    }
                    let candidate = position - distance;
                    if data[candidate..candidate + 3] != data[position..position + 3] {
                        continue;
                    }
                    let found = 3 + crate::zlib::common_prefix(
                        &data[candidate + 3..candidate + limit],
                        &data[position + 3..position + limit],
                    );
                    if found > length {
                        length = found;
                        offset = distance;
                    }
                    if length == limit {
                        break;
                    }
                }
                let h = hash(&data[position..]);
                let mut candidate = previous[h];
                let mut visits = 0;
                while length < limit
                    && candidate != usize::MAX
                    && position - candidate <= max_offset
                    && visits < 64
                {
                    if data[candidate + length.saturating_sub(1)]
                        == data[position + length.saturating_sub(1)]
                    {
                        let found = crate::zlib::common_prefix(
                            &data[candidate..candidate + limit],
                            &data[position..position + limit],
                        );
                        if found > length
                            || (found >= 3
                                && found == length
                                && recent
                                    .iter()
                                    .position(|&value| value == position - candidate)
                                    .unwrap_or(3)
                                    < recent
                                        .iter()
                                        .position(|&value| value == offset)
                                        .unwrap_or(3))
                        {
                            length = found;
                            offset = position - candidate;
                            if length == limit && recent.contains(&offset) {
                                break;
                            }
                        }
                    }
                    candidate = links[candidate];
                    visits += 1;
                }
            }
            let token = if length >= 3 {
                let slot = if let Some(slot) = recent.iter().position(|&value| value == offset) {
                    recent.swap(0, slot);
                    slot
                } else {
                    recent[2] = recent[1];
                    recent[1] = recent[0];
                    recent[0] = offset;
                    (3..slots).rev().find(|&s| BASE[s] <= offset).unwrap_or(3)
                };
                let header = (length - 2).min(7);
                if header == 7 {
                    length_freq[length - 9] += 1;
                }
                Token {
                    symbol: 256 + slot * 8 + header,
                    length,
                    slot,
                    offset,
                }
            } else {
                Token {
                    symbol: data[position] as usize,
                    length: 1,
                    slot: 0,
                    offset: 0,
                }
            };
            main_freq[token.symbol] += 1;
            for p in position..position + token.length {
                if p + 3 <= data.len() {
                    let bucket = hash(&data[p..]);
                    links[p] = previous[bucket];
                    previous[bucket] = p;
                }
            }
            position += token.length;
            crate::memory::ensure_capacity(tokens, 1).map_err(|_| EncodeError::OutOfMemory)?;
            tokens.push(token);
        }
        let main = select_lengths(&main_freq[..main_count], Some(256));
        let lengths = select_lengths(&length_freq, None);
        let main_codes = canonical(&main);
        let length_codes = canonical(&lengths);
        let mut writer = Writer::new(&mut self.output, capacity);
        if let Some(state) = cabinet {
            if state.first {
                writer.put(0, 1); // No Intel E8 translation in CAB output.
            }
            writer.put(1, 3); // Verbatim Huffman block.
            writer.put(input.len() as u32, 24);
            write_cabinet_lengths(&mut writer, &main[..256], &state.main[..256]);
            write_cabinet_lengths(
                &mut writer,
                &main[256..main_count],
                &state.main[256..main_count],
            );
            write_cabinet_lengths(&mut writer, &lengths[..249], &state.lengths[..249]);
            state.main = main;
            state.lengths = lengths;
            state.recent = recent;
            state.first = false;
        } else {
            writer.put(1, 3);
            if input.len() == 32768 {
                writer.put(1, 1);
            } else {
                writer.put(0, 1);
                writer.put(input.len() as u32, if order >= 16 { 24 } else { 16 });
            }
            write_lengths(&mut writer, &main[..256]);
            write_lengths(&mut writer, &main[256..main_count]);
            write_lengths(&mut writer, &lengths[..249]);
        }
        for token in tokens.iter() {
            writer.put(main_codes[token.symbol], main[token.symbol] as usize);
            if token.length >= 3 {
                if token.length >= 9 {
                    let symbol = token.length - 9;
                    writer.put(length_codes[symbol], lengths[symbol] as usize);
                }
                if token.slot >= 3 {
                    writer.put((token.offset - BASE[token.slot]) as u32, EXTRA[token.slot]);
                }
            }
        }
        Ok(writer.finish())
    }
}

struct CabinetState {
    window_order: u8,
    first: bool,
    recent: [usize; 3],
    main: [u8; 656],
    lengths: [u8; 656],
}

/// Stateful CAB LZX encoder with verbatim Huffman blocks and greedy LZ matches.
///
/// Trees and recent offsets persist between 32 KiB frames. Match searches are
/// frame-local; Intel E8 translation is disabled so input bytes remain unchanged.
pub struct CabinetLzxEncoder {
    compressor: LzxCompressor,
    state: CabinetState,
}

impl CabinetLzxEncoder {
    /// Create an encoder with a CAB window order from 15 through 21.
    pub fn new(window_order: u8) -> Result<Self, EncodeError> {
        if !(15..=21).contains(&window_order) {
            return Err(EncodeError::InvalidMaxBlockSize);
        }
        Ok(Self {
            compressor: LzxCompressor::new(32768)?,
            state: CabinetState {
                window_order,
                first: true,
                recent: [1; 3],
                main: [0; 656],
                lengths: [0; 656],
            },
        })
    }

    /// Encode one frame of 1 through 32768 bytes into retained output storage.
    pub fn compress_frame(&mut self, input: &[u8]) -> Result<&[u8], EncodeError> {
        if input.is_empty() || input.len() > 32768 {
            return Err(EncodeError::InputExceedsLimit);
        }
        self.compressor
            .compress_frame(input, 65535, Some(&mut self.state))?
            .ok_or(EncodeError::InputExceedsLimit)
    }
}

fn write_cabinet_lengths(writer: &mut Writer, lengths: &[u8], previous: &[u8]) {
    // A fixed complete precode covers all 17 deltas without depending on run encoding.
    let mut pre = [0u8; 20];
    pre[..15].fill(4);
    pre[15..17].fill(5);
    let codes = canonical(&pre);
    for &length in &pre {
        writer.put(u32::from(length), 4);
    }
    for (&length, &old) in lengths.iter().zip(previous) {
        let delta = usize::from((old + 17 - length) % 17);
        writer.put(codes[delta], usize::from(pre[delta]));
    }
}

fn hash(data: &[u8]) -> usize {
    let value = u32::from(data[0]) | (u32::from(data[1]) << 8) | (u32::from(data[2]) << 16);
    (value.wrapping_mul(0x1e35a7bd) >> 16) as usize
}
fn balanced(freq: &[usize]) -> [u8; 656] {
    let mut symbols = [0usize; 656];
    let mut count = 0;
    for (symbol, &frequency) in freq.iter().enumerate() {
        if frequency != 0 {
            symbols[count] = symbol;
            count += 1;
        }
    }
    if count == 1 {
        let extra = if symbols[0] == 0 { 1 } else { 0 };
        symbols[count] = extra;
        count += 1;
    }
    symbols[..count].sort_unstable_by_key(|&symbol| (core::cmp::Reverse(freq[symbol]), symbol));
    let mut result = [0; 656];
    if count != 0 {
        let bits = usize::BITS - (count - 1).leading_zeros();
        let short = (1usize << bits) - count;
        for (index, &symbol) in symbols[..count].iter().enumerate() {
            result[symbol] = (bits - u32::from(index < short)) as u8;
        }
    }
    result
}
fn huffman(freq: &[usize]) -> [u8; 656] {
    let mut weights = [usize::MAX; 1311];
    let mut parents = [usize::MAX; 1311];
    let mut leaves = [0usize; 656];
    let mut count = 0;
    for (symbol, &frequency) in freq.iter().enumerate() {
        if frequency != 0 {
            weights[symbol] = frequency;
            leaves[count] = symbol;
            count += 1;
        }
    }
    if count == 1 {
        let dummy = usize::from(leaves[0] == 0);
        weights[dummy] = 1;
        leaves[count] = dummy;
        count += 1;
    }
    leaves[..count].sort_unstable_by_key(|&symbol| (weights[symbol], symbol));
    let mut leaf = 0;
    let mut internal = 656;
    for next in 656..656 + count.saturating_sub(1) {
        let mut take = || {
            if leaf < count && (internal == next || weights[leaves[leaf]] <= weights[internal]) {
                let node = leaves[leaf];
                leaf += 1;
                node
            } else {
                let node = internal;
                internal += 1;
                node
            }
        };
        let left = take();
        let right = take();
        weights[next] = weights[left] + weights[right];
        parents[left] = next;
        parents[right] = next;
    }
    let mut lengths = [0u8; 656];
    for &symbol in &leaves[..count] {
        let mut node = symbol;
        while parents[node] != usize::MAX {
            lengths[symbol] += 1;
            // LZX lengths permit 16 bits; keep a complete bounded tree on
            // pathological distributions rather than emitting invalid codes.
            if lengths[symbol] > 16 {
                return balanced(freq);
            }
            node = parents[node];
        }
    }
    lengths
}
fn canonical(lengths: &[u8]) -> [u32; 656] {
    let mut counts = [0u32; 17];
    for &length in lengths {
        if length != 0 {
            counts[length as usize] += 1;
        }
    }
    let mut next = [0u32; 17];
    for n in 1..17 {
        next[n] = (next[n - 1] + counts[n - 1]) << 1;
    }
    let mut codes = [0u32; 656];
    for (symbol, &length) in lengths.iter().enumerate() {
        if length != 0 {
            codes[symbol] = next[length as usize];
            next[length as usize] += 1;
        }
    }
    codes
}
fn write_lengths(writer: &mut Writer, lengths: &[u8]) {
    let (_, pre, repeats) = length_description(lengths);
    let codes = canonical(&pre);
    for &length in &pre {
        writer.put(u32::from(length), 4);
    }
    visit_lengths(lengths, repeats, |symbol, value, bits| {
        writer.put(codes[symbol], pre[symbol] as usize);
        writer.put(value, bits);
    });
}
fn select_lengths(freq: &[usize], split: Option<usize>) -> [u8; 656] {
    let weighted = huffman(freq);
    let balanced = balanced(freq);
    let cost = |lengths: &[u8]| {
        let payload = freq
            .iter()
            .zip(lengths)
            .map(|(&frequency, &length)| frequency * usize::from(length))
            .sum::<usize>();
        let headers = if let Some(split) = split {
            length_description(&lengths[..split]).0
                + length_description(&lengths[split..freq.len()]).0
        } else {
            length_description(&lengths[..freq.len()]).0
        };
        payload + headers
    };
    if cost(&balanced) < cost(&weighted) {
        balanced
    } else {
        weighted
    }
}
fn length_description(lengths: &[u8]) -> (usize, [u8; 20], bool) {
    let with = length_description_with_runs(lengths, true);
    let without = length_description_with_runs(lengths, false);
    if with.0 < without.0 {
        (with.0, with.1, true)
    } else {
        (without.0, without.1, false)
    }
}
fn length_description_with_runs(lengths: &[u8], repeats: bool) -> (usize, [u8; 20]) {
    let mut frequencies = [0usize; 20];
    let mut extra = 0;
    visit_lengths(lengths, repeats, |symbol, _, bits| {
        frequencies[symbol] += 1;
        extra += bits;
    });
    let generated = huffman(&frequencies);
    let generated = if generated[..20].iter().any(|&length| length > 15) {
        balanced(&frequencies)
    } else {
        generated
    };
    let mut pre = [0u8; 20];
    pre.copy_from_slice(&generated[..20]);
    let cost = 80
        + extra
        + frequencies
            .iter()
            .zip(pre)
            .map(|(&frequency, length)| frequency * usize::from(length))
            .sum::<usize>();
    (cost, pre)
}

fn visit_lengths(lengths: &[u8], repeats: bool, mut emit: impl FnMut(usize, u32, usize)) {
    let mut position = 0;
    while position < lengths.len() {
        let length = lengths[position];
        if length == 0 {
            let run = lengths[position..]
                .iter()
                .take_while(|&&value| value == 0)
                .count();
            if run >= 4 {
                let (symbol, count, extra, base) = if run >= 20 {
                    (18, run.min(51), 5, 20)
                } else {
                    (17, run.min(19), 4, 4)
                };
                emit(symbol, (count - base) as u32, extra);
                position += count;
                continue;
            }
        }
        let delta = if length == 0 { 0 } else { 17 - length as usize };
        if repeats && length != 0 {
            let run = lengths[position..]
                .iter()
                .take_while(|&&value| value == length)
                .count();
            if run >= 4 {
                let count = run.min(5);
                emit(19, (count - 4) as u32, 1);
                emit(delta, 0, 0);
                position += count;
                continue;
            }
        }
        emit(delta, 0, 0);
        position += 1;
    }
}
fn forward_e8(data: &mut [u8]) {
    if data.len() <= 10 {
        return;
    }
    let mut p = 0;
    while p < data.len() - 10 {
        if data[p] != 0xe8 {
            p += 1;
            continue;
        }
        let relative = i32::from_le_bytes([data[p + 1], data[p + 2], data[p + 3], data[p + 4]]);
        if relative >= -(p as i32) && relative < 12000000 {
            let absolute = if relative < 12000000 - p as i32 {
                relative + p as i32
            } else {
                relative - 12000000
            };
            data[p + 1..p + 5].copy_from_slice(&absolute.to_le_bytes());
        }
        p += 5;
    }
}
struct Writer<'a> {
    bytes: &'a mut Vec<u8>,
    word: u16,
    used: usize,
    capacity: usize,
    overflow: bool,
}
impl<'a> Writer<'a> {
    fn new(bytes: &'a mut Vec<u8>, capacity: usize) -> Self {
        bytes.clear();
        Self {
            bytes,
            word: 0,
            used: 0,
            capacity,
            overflow: false,
        }
    }
    fn put(&mut self, value: u32, mut count: usize) {
        while count != 0 {
            let take = count.min(16 - self.used);
            self.word |=
                (((value >> (count - take)) & ((1 << take) - 1)) as u16) << (16 - self.used - take);
            self.used += take;
            count -= take;
            if self.used == 16 {
                self.flush();
            }
        }
    }
    fn flush(&mut self) {
        if self.bytes.len() + 2 <= self.capacity {
            if crate::memory::ensure_capacity(self.bytes, 2).is_ok() {
                self.bytes.extend_from_slice(&self.word.to_le_bytes());
            } else {
                self.overflow = true;
            }
        } else {
            self.overflow = true;
        }
        self.word = 0;
        self.used = 0;
    }
    fn finish(mut self) -> Option<&'a [u8]> {
        if self.used != 0 {
            self.flush();
        }
        if self.overflow {
            None
        } else {
            Some(&self.bytes[..])
        }
    }
}

#[cfg(test)]
mod coding_tests {
    use super::*;
    use alloc::vec;

    #[test]
    fn wordwise_writer_matches_bitwise_reference_across_word_boundaries() {
        let mut storage = crate::memory::filled_vec(1024, 0).unwrap();
        let mut writer = Writer::new(&mut storage, 1024);
        let mut reference_bits = Vec::new();
        let mut state = 0x57494du32;
        for count in (0..=24).cycle().take(300) {
            state ^= state << 13;
            state ^= state >> 17;
            state ^= state << 5;
            writer.put(state, count);
            reference_bits.extend((0..count).rev().map(|bit| ((state >> bit) & 1) as u16));
        }
        let expected: Vec<_> = reference_bits
            .chunks(16)
            .flat_map(|bits| {
                let word = bits
                    .iter()
                    .enumerate()
                    .fold(0u16, |word, (i, &bit)| word | (bit << (15 - i)));
                word.to_le_bytes()
            })
            .collect();
        assert_eq!(writer.finish().unwrap(), expected);
    }

    #[test]
    fn repeat_length_runs_roundtrip_dense_tables_and_reused_contexts() {
        let mut compressor = LzxCompressor::new(32768).unwrap();
        for period in [1, 7, 13, 256, 4096] {
            let input: Vec<_> = (0..32768).map(|i| ((i % period) * 73) as u8).collect();
            let encoded = compressor.compress(&input, 32768 * 2).unwrap().unwrap();
            let mut decoded = vec![0; input.len()];
            crate::lzx::decompress_lzx(encoded, &mut decoded, 32768).unwrap();
            assert_eq!(decoded, input);
        }
    }

    #[test]
    fn weighted_lengths_reduce_skewed_cost_and_keep_complete_bounded_trees() {
        let skewed = [1000, 1, 2, 3, 0, 4];
        let weighted = huffman(&skewed);
        let balanced = balanced(&skewed);
        let cost = |lengths: &[u8]| {
            skewed
                .iter()
                .zip(lengths)
                .map(|(&f, &l)| f * usize::from(l))
                .sum::<usize>()
        };
        assert!(cost(&weighted) < cost(&balanced));
        let mut fibonacci = [1usize; 30];
        for i in 2..fibonacci.len() {
            fibonacci[i] = fibonacci[i - 1] + fibonacci[i - 2];
        }
        for lengths in [
            weighted,
            huffman(&fibonacci),
            huffman(&[1]),
            huffman(&[1; 656]),
        ] {
            assert!(lengths.iter().all(|&length| length <= 16));
            assert_eq!(
                lengths
                    .iter()
                    .filter(|&&length| length != 0)
                    .map(|&length| 1usize << (16 - length))
                    .sum::<usize>(),
                1 << 16
            );
        }
    }

    #[test]
    fn zero_run_framing_preserves_lengths_at_format_boundaries() {
        for run in [0, 1, 3, 4, 19, 20, 51, 52, 102, 249, 656] {
            let mut lengths = vec![0; run];
            lengths.extend_from_slice(&[1, 8, 16]);
            let mut decoded = Vec::new();
            visit_lengths(&lengths, false, |symbol, value, bits| match symbol {
                17 => {
                    assert_eq!(bits, 4);
                    decoded.extend(core::iter::repeat_n(0, 4 + value as usize));
                }
                18 => {
                    assert_eq!(bits, 5);
                    decoded.extend(core::iter::repeat_n(0, 20 + value as usize));
                }
                _ => {
                    assert_eq!(bits, 0);
                    decoded.push(if symbol == 0 { 0 } else { (17 - symbol) as u8 });
                }
            });
            assert_eq!(decoded, lengths);
        }
    }
}
