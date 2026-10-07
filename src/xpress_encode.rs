//! Native WIM XPRESS Huffman compression.
//!
//! A bounded hash-chain LZ77 parser feeds a frequency-built canonical Huffman
//! code. This emits the WIM XPRESS dialect, including the end-of-data symbol
//! and interleaved little-endian coding units and match-length bytes.
//
// Format reference: wimlib cd5e231c348c255ae5088873b5a66ee0eb96fa07,
// src/xpress_compress.c, Copyright (C) 2012-2016 Eric Biggers,
// LGPL-2.1-or-later. This independently structured Rust implementation is
// distributed under LGPL-2.1-or-later. No production C code is linked.
use alloc::vec::Vec;
/// An XPRESS compression failure.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EncodeError {
    /// XPRESS accepts at most 65536 bytes in one block.
    InputTooLarge,
    /// Allocation failed.
    OutOfMemory,
}
impl core::fmt::Display for EncodeError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.write_str(match self {
            Self::InputTooLarge => "XPRESS input exceeds 65536 bytes",
            Self::OutOfMemory => "XPRESS encoder allocation failed",
        })
    }
}
impl core::error::Error for EncodeError {}
/// Compress one XPRESS Huffman block using a bounded greedy LZ77 parser.
///
/// `capacity` is the maximum returned compressed byte length. Returns `None`
/// if input is shorter than 25 bytes or the capacity is insufficient. As in
/// wimlib, two unused trailing capacity bytes are required beyond the returned
/// compressed byte length. Output may expand input if `capacity` permits it.
/// The returned block can
/// be decompressed by [`crate::decompress_xpress`] or wimlib's XPRESS decoder.
/// The caller retains responsibility for storing raw chunks when this returns
/// `None` or output is at least as large as input, and for recording the exact
/// uncompressed size in its container.
///
/// This API does not expose wimlib's compression levels or reproduce its parse
/// choices or compressed bytes.
/// Every heap allocation is fallible; output and match storage are bounded by
/// the 65536-byte input limit rather than the caller's capacity.
///
/// # Errors
///
/// Returns [`EncodeError::InputTooLarge`] if input exceeds 65536 bytes, or
/// [`EncodeError::OutOfMemory`] if temporary or output storage cannot be allocated.
pub fn compress_xpress(input: &[u8], capacity: usize) -> Result<Option<Vec<u8>>, EncodeError> {
    if input.len() > 65536 {
        return Err(EncodeError::InputTooLarge);
    }
    if input.len() < 25 || capacity <= 260 {
        return Ok(None);
    }
    let mut compressor = XpressCompressor::new(input.len())?;
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
/// Actual reusable XPRESS match/parser/output storage.
#[derive(Debug)]
pub struct XpressCompressor {
    maximum: usize,
    next_base: u32,
    heads: Vec<u32>,
    previous: Vec<u32>,
    items: Vec<Item>,
    output: Vec<u8>,
}
impl XpressCompressor {
    /// Allocate all bounded storage through the Rust global allocator.
    pub fn new(maximum: usize) -> Result<Self, EncodeError> {
        if maximum == 0 || maximum > 65536 {
            return Err(EncodeError::InputTooLarge);
        }
        let storage = |size, value| {
            crate::memory::filled_vec(size, value).map_err(|_| EncodeError::OutOfMemory)
        };
        Ok(Self {
            maximum,
            next_base: 1,
            heads: storage(65536, 0)?,
            previous: storage(maximum, 0)?,
            items: crate::memory::filled_vec(
                maximum + 1,
                Item {
                    symbol: 0,
                    length: 0,
                    offset: 0,
                },
            )
            .map_err(|_| EncodeError::OutOfMemory)?,
            output: crate::memory::filled_vec(maximum * 8 + 264, 0)
                .map_err(|_| EncodeError::OutOfMemory)?,
        })
    }
    /// Compress into retained storage; the next call invalidates this borrowed result.
    pub fn compress(
        &mut self,
        input: &[u8],
        capacity: usize,
    ) -> Result<Option<&[u8]>, EncodeError> {
        if input.len() < 25 || capacity <= 260 {
            return Ok(None);
        }
        self.compress_block(input, capacity)
    }
    /// Encode one nonempty raw block, including short inputs that expand.
    /// Unlike `compress`, this does not apply the WIM small-input policy.
    /// Returns `None` for empty input or insufficient output capacity.
    pub fn compress_block(
        &mut self,
        input: &[u8],
        capacity: usize,
    ) -> Result<Option<&[u8]>, EncodeError> {
        if input.len() > self.maximum {
            return Err(EncodeError::InputTooLarge);
        }
        if input.is_empty() || capacity <= 260 {
            return Ok(None);
        }
        // Stamped positions invalidate old chains without clearing both buffers.
        // Zero remains the sentinel after the rare counter-wrap reset.
        let span = input.len() as u32 + 1;
        if self.next_base > u32::MAX - span {
            self.heads.fill(0);
            self.next_base = 1;
        }
        let base = self.next_base;
        self.next_base += span;
        let (frequencies, extension_bytes, item_count) = parse(
            input,
            base,
            &mut self.heads,
            &mut self.previous,
            &mut self.items,
        )?;
        let items = &self.items[..item_count];
        let (lengths, codes) = code(&frequencies);
        // The parsed tokens determine the exact final size, including coding
        // units, the terminal zero word and embedded match-length bytes.
        // Avoid emitting a block that the caller's capacity cannot accept.
        let bit_count: u64 = frequencies
            .iter()
            .zip(&lengths)
            .enumerate()
            .map(|(symbol, (&frequency, &length))| {
                let offset_bits = symbol.saturating_sub(256) >> 4;
                frequency * (u64::from(length) + offset_bits as u64)
            })
            .sum();
        let encoded_size = 258 + 2 * bit_count.div_ceil(16) as usize + extension_bytes;
        if encoded_size + 2 > capacity {
            return Ok(None);
        }
        // Each token consumes at least one input byte and emits at most 30 bits
        // plus three extension bytes; 8 bytes per input byte safely bounds storage.
        let mut writer = Writer::new(&mut self.output[..]);
        for index in 0..256 {
            writer.output[index] = lengths[index * 2] | (lengths[index * 2 + 1] << 4);
        }
        for item in items {
            writer.bits(
                codes[usize::from(item.symbol)],
                usize::from(lengths[usize::from(item.symbol)]),
            )?;
            if item.length >= 3 {
                let adjusted = item.length - 3;
                if adjusted >= 15 {
                    writer.append(&[(adjusted - 15).min(255) as u8])?;
                    if adjusted >= 270 {
                        writer.append(&adjusted.to_le_bytes())?;
                    }
                }
                let extra = item.offset.ilog2() as usize;
                writer.bits(item.offset - (1 << extra), extra)?;
            }
        }
        let output = writer.finish();
        if output.len() + 2 <= capacity {
            Ok(Some(output))
        } else {
            Ok(None)
        }
    }
}

#[derive(Clone, Copy, Debug)]
struct Item {
    symbol: u16,
    length: u16,
    offset: u16,
}

fn hash(input: &[u8], position: usize) -> usize {
    let value = u32::from(input[position])
        | (u32::from(input[position + 1]) << 8)
        | (u32::from(input[position + 2]) << 16);
    (value.wrapping_mul(0x1e35a7bd) >> 16) as usize
}

fn match_length(input: &[u8], candidate: usize, position: usize) -> usize {
    let limit = input.len() - position;
    let left = &input[candidate..candidate + limit];
    let right = &input[position..];
    crate::zlib::common_prefix(left, right)
}

fn parse(
    input: &[u8],
    base: u32,
    heads: &mut [u32],
    previous: &mut [u32],
    items: &mut [Item],
) -> Result<([u64; 512], usize, usize), EncodeError> {
    // Retain initialized storage; expose only the tokens written this time.
    let mut item_count = 0;
    let mut frequencies = [0u64; 512];
    let mut extension_bytes = 0;
    let mut position = 0;
    while position < input.len() {
        let mut best_length = 0;
        let mut best_offset = 0;
        if position + 3 <= input.len() {
            let mut candidate = heads[hash(input, position)];
            for _ in 0..64 {
                if candidate < base || position - (candidate - base) as usize > 65535 {
                    break;
                }
                let at = (candidate - base) as usize;
                // This candidate cannot improve the retained match if it differs
                // at that match's boundary. Equal-length ties keep the old offset.
                if input[at + best_length] != input[position + best_length] {
                    candidate = previous[at];
                    continue;
                }
                let length = match_length(input, at, position);
                if length >= 3 && length > best_length {
                    best_length = length;
                    best_offset = position - at;
                    if position + length == input.len() {
                        break;
                    }
                }
                candidate = previous[at];
            }
        }
        let item = if best_length >= 3 {
            let log_offset = best_offset.ilog2() as usize;
            Item {
                symbol: (256 + (log_offset << 4) + (best_length - 3).min(15)) as u16,
                length: best_length as u16,
                offset: best_offset as u16,
            }
        } else {
            Item {
                symbol: u16::from(input[position]),
                length: 1,
                offset: 0,
            }
        };
        frequencies[usize::from(item.symbol)] += 1;
        if item.length >= 18 {
            extension_bytes += 1 + 2 * usize::from(item.length >= 273);
        }
        *items.get_mut(item_count).ok_or(EncodeError::OutOfMemory)? = item;
        item_count += 1;
        for (relative, link) in previous[position..position + usize::from(item.length)]
            .iter_mut()
            .enumerate()
        {
            let inserted = position + relative;
            if inserted + 3 <= input.len() {
                let bucket = hash(input, inserted);
                *link = heads[bucket];
                heads[bucket] = base + inserted as u32;
            }
        }
        position += usize::from(item.length);
    }
    frequencies[256] += 1;
    *items.get_mut(item_count).ok_or(EncodeError::OutOfMemory)? = Item {
        symbol: 256,
        length: 0,
        offset: 0,
    };
    Ok((frequencies, extension_bytes, item_count + 1))
}

fn code(frequencies: &[u64; 512]) -> ([u8; 512], [u16; 512]) {
    let mut weights = [u64::MAX; 1023];
    let mut parents = [usize::MAX; 1023];
    weights[..512].copy_from_slice(frequencies);
    for weight in &mut weights[..512] {
        if *weight == 0 {
            *weight = u64::MAX;
        }
    }
    let mut active = frequencies.iter().filter(|&&f| f != 0).count();
    if active == 1
        && let Some(dummy) = frequencies.iter().position(|&f| f == 0)
    {
        weights[dummy] = 1;
        active = 2;
    }
    let mut leaves = [0usize; 512];
    let mut count = 0;
    for (index, &weight) in weights[..512].iter().enumerate() {
        if weight != u64::MAX {
            leaves[count] = index;
            count += 1;
        }
    }
    leaves[..count].sort_unstable_by_key(|&index| (weights[index], index));
    let mut leaf = 0;
    let mut internal = 512;
    let mut next = 512;
    while active > 1 {
        // Sorted leaves and creation-ordered internal nodes form two monotonic
        // queues. Equal weights retain the original lowest-node-index tie rule.
        let mut take = || {
            if leaf < count && (internal == next || weights[leaves[leaf]] <= weights[internal]) {
                let index = leaves[leaf];
                leaf += 1;
                index
            } else {
                let index = internal;
                internal += 1;
                index
            }
        };
        let first = take();
        let second = take();
        weights[next] = weights[first] + weights[second];
        parents[first] = next;
        parents[second] = next;
        next += 1;
        active -= 1;
    }
    let mut lengths = [0u8; 512];
    for symbol in 0..512 {
        if weights[symbol] == u64::MAX {
            continue;
        }
        let mut node = symbol;
        while parents[node] != usize::MAX {
            lengths[symbol] += 1;
            node = parents[node];
        }
    }
    if lengths.iter().any(|&length| length > 15) {
        // A complete balanced tree keeps pathological frequency distributions
        // valid without exceeding the format's four-bit length field.
        let mut symbols = [0usize; 512];
        let mut count = 0;
        for (symbol, &length) in lengths.iter().enumerate() {
            if length != 0 {
                symbols[count] = symbol;
                count += 1;
            }
        }
        symbols[..count]
            .sort_unstable_by_key(|&symbol| (core::cmp::Reverse(frequencies[symbol]), symbol));
        let short = count.ilog2() as u8;
        let short_count = (1usize << (short + 1)) - count;
        for (rank, &symbol) in symbols[..count].iter().enumerate() {
            lengths[symbol] = short + u8::from(rank >= short_count);
        }
    }
    let mut counts = [0u16; 16];
    for &length in &lengths {
        if length != 0 {
            counts[usize::from(length)] += 1;
        }
    }
    let mut next_codes = [0u16; 16];
    for length in 1..16 {
        next_codes[length] = (next_codes[length - 1] + counts[length - 1]) << 1;
    }
    let mut codes = [0u16; 512];
    for (symbol, &length) in lengths.iter().enumerate() {
        if length != 0 {
            codes[symbol] = next_codes[usize::from(length)];
            next_codes[usize::from(length)] += 1;
        }
    }
    (lengths, codes)
}

struct Writer<'a> {
    output: &'a mut [u8],
    length: usize,
    first: usize,
    second: usize,
    buffer: u32,
    count: usize,
}
impl<'a> Writer<'a> {
    fn new(output: &'a mut [u8]) -> Self {
        Self {
            output,
            length: 260,
            first: 256,
            second: 258,
            buffer: 0,
            count: 0,
        }
    }
    fn append(&mut self, bytes: &[u8]) -> Result<(), EncodeError> {
        let end = self.length + bytes.len();
        self.output
            .get_mut(self.length..end)
            .ok_or(EncodeError::OutOfMemory)?
            .copy_from_slice(bytes);
        self.length = end;
        Ok(())
    }
    fn put_word(&mut self, position: usize, value: u16) {
        self.output[position..position + 2].copy_from_slice(&value.to_le_bytes());
    }
    fn bits(&mut self, value: u16, count: usize) -> Result<(), EncodeError> {
        self.buffer = (self.buffer << count) | u32::from(value);
        self.count += count;
        if self.count > 16 {
            self.count -= 16;
            self.put_word(self.first, (self.buffer >> self.count) as u16);
            self.first = self.second;
            self.second = self.length;
            self.append(&[0, 0])?;
        }
        Ok(())
    }
    fn finish(mut self) -> &'a [u8] {
        self.put_word(self.first, (self.buffer << (16 - self.count)) as u16);
        self.put_word(self.second, 0);
        &self.output[..self.length]
    }
}

#[cfg(test)]
mod tests {
    use super::code;
    use alloc::{vec, vec::Vec};

    #[test]
    fn retained_output_overwrites_stale_bytes_across_varying_block_sizes() {
        let mut reused = super::XpressCompressor::new(65536).unwrap();
        let mut random = 23u32;
        for length in [65536, 25, 65535, 31, 4096, 256, 65536, 100] {
            let input: Vec<u8> = (0..length)
                .map(|_| {
                    random ^= random << 13;
                    random ^= random >> 17;
                    random ^= random << 5;
                    random as u8
                })
                .collect();
            reused.output.fill(0xa5);
            let actual = reused
                .compress(&input, usize::MAX)
                .unwrap()
                .unwrap()
                .to_vec();
            let mut fresh = super::XpressCompressor::new(65536).unwrap();
            assert_eq!(actual, fresh.compress(&input, usize::MAX).unwrap().unwrap());
        }
    }

    #[test]
    fn reused_hash_chains_and_generation_wrap_match_fresh_compression() {
        let mut reused = super::XpressCompressor::new(65536).unwrap();
        for case in 0..12 {
            let input: Vec<u8> = (0..32768).map(|i| ((i % (case + 1)) * 17) as u8).collect();
            if case == 6 {
                reused.next_base = u32::MAX - 1;
            }
            let actual = reused.compress(&input, 65536).unwrap().unwrap().to_vec();
            let mut fresh = super::XpressCompressor::new(65536).unwrap();
            assert_eq!(actual, fresh.compress(&input, 65536).unwrap().unwrap());
        }
    }

    #[test]
    fn wordwise_matches_preserve_overlap_and_final_partial_word() {
        for length in 1..80 {
            let mut input = vec![3u8; length + 3];
            for mismatch in 0..length {
                input[3 + mismatch] = 7;
                let expected = input[..]
                    .iter()
                    .zip(&input[3..])
                    .take_while(|(a, b)| a == b)
                    .count();
                assert_eq!(super::match_length(&input, 0, 3), expected);
                input[3 + mismatch] = 3;
            }
            assert_eq!(super::match_length(&input, 0, 3), length);
        }
    }

    #[test]
    fn sorted_queue_preserves_scan_based_huffman_ties() {
        // Independent slow reference: choose the two smallest active nodes by
        // weight/index rather than using the optimized pair of queues.
        let mut state = 7u64;
        for case in 0..64 {
            let mut frequencies = [0u64; 512];
            for frequency in &mut frequencies {
                state = state.wrapping_mul(6364136223846793005).wrapping_add(1);
                *frequency = if case == 0 { 1 } else { (state >> 32) % 32 };
            }
            let mut nodes: Vec<(u64, usize)> = frequencies
                .iter()
                .enumerate()
                .filter(|(_, weight)| **weight != 0)
                .map(|(index, &weight)| (weight, index))
                .collect();
            let mut parents = [usize::MAX; 1023];
            let mut next = 512;
            while nodes.len() > 1 {
                nodes.sort_unstable();
                let (weight_a, a) = nodes.remove(0);
                let (weight_b, b) = nodes.remove(0);
                parents[a] = next;
                parents[b] = next;
                nodes.push((weight_a + weight_b, next));
                next += 1;
            }
            let mut expected = [0u8; 512];
            for (symbol, length) in expected.iter_mut().enumerate() {
                let mut node = symbol;
                while parents[node] != usize::MAX {
                    *length += 1;
                    node = parents[node];
                }
            }
            assert_eq!(code(&frequencies).0, expected, "frequency case {case}");
        }
    }

    #[test]
    fn pathological_huffman_depth_has_complete_bounded_fallback() {
        let mut frequencies = [0u64; 512];
        let (mut first, mut second) = (1, 1);
        for frequency in &mut frequencies[..24] {
            *frequency = first;
            (first, second) = (second, first + second);
        }
        let (lengths, _) = code(&frequencies);
        assert!(lengths.iter().all(|&length| length <= 15));
        assert_eq!(
            lengths
                .iter()
                .filter(|&&length| length != 0)
                .map(|&length| 1u32 << (15 - length))
                .sum::<u32>(),
            1 << 15
        );
    }

    #[test]
    fn single_symbol_uses_complete_two_symbol_tree() {
        let mut frequencies = [0u64; 512];
        frequencies[256] = 1;
        let (lengths, _) = code(&frequencies);
        assert_eq!(lengths[256], 1);
        assert_eq!(lengths.iter().filter(|&&length| length == 1).count(), 2);
    }
}
