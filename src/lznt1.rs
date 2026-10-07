//! LZNT1 (MS-XCA section 2.5), Windows COMPRESSION_FORMAT_LZNT1 (2).
//!
//! Chunks have independent histories and expand to at most 4096 bytes.
//! Reference: https://learn.microsoft.com/en-us/openspecs/windows_protocols/ms-xca/124d9696-a69c-409a-a055-2562fbe255f9

pub use crate::nt_common::Error;
use crate::nt_common::{Matcher, Reader, copy, storage};
use alloc::vec::Vec;

fn length_bits(position: usize) -> u32 {
    let mut bits = 12;
    while bits > 4 && position > (1 << (16 - bits)) {
        bits -= 1;
    }
    bits
}

/// Decompress LZNT1 chunks into a caller-bounded buffer, returning bytes written.
///
/// Accepts an optional zero chunk header and zero padding after it. An error
/// may leave partial output. No heap allocation occurs.
///
/// # Errors
/// Returns an error for malformed/truncated chunks or insufficient output space.
pub fn decompress(input: &[u8], output: &mut [u8]) -> Result<usize, Error> {
    decompress_inner(input, output, false).map(|(written, _)| written)
}

/// Decompress a caller-sized storage unit, returning written and consumed bytes.
///
/// Unlike [`decompress`], this stops at a fully validated chunk boundary once
/// `output` is exactly full. Remaining input belongs to the container's allocated
/// storage padding and is not parsed. The caller must establish the logical unit
/// size independently; this is not a general decoder for untrusted buffer lengths.
/// Short output retains strict header, payload, token and zero-terminal checks.
/// Consumed bytes include a zero terminal header, but exclude padding after it.
///
/// # Errors
/// Returns an error for malformed/truncated required chunks or insufficient
/// output space. Nonempty input with zero output capacity is rejected.
pub fn decompress_storage_unit(input: &[u8], output: &mut [u8]) -> Result<(usize, usize), Error> {
    if !input.is_empty() && output.is_empty() {
        return Err(Error::OutputTooSmall);
    }
    decompress_inner(input, output, true)
}

fn decompress_inner(
    input: &[u8],
    output: &mut [u8],
    storage_unit: bool,
) -> Result<(usize, usize), Error> {
    let mut reader = Reader(input);
    let mut pos = 0;
    while !reader.0.is_empty() {
        if storage_unit && pos == output.len() {
            return Ok((pos, input.len() - reader.0.len()));
        }
        let header = reader.word()?;
        if header == 0 {
            return if reader.0.iter().all(|&b| b == 0) {
                Ok((pos, input.len() - reader.0.len()))
            } else {
                Err(Error::InvalidData)
            };
        }
        if header & 0x7000 != 0x3000 {
            return Err(Error::InvalidData);
        }
        let payload = reader.take(usize::from(header & 0xfff) + 1)?;
        if header & 0x8000 == 0 {
            if payload.len() > output.len() - pos {
                return Err(Error::OutputTooSmall);
            }
            output[pos..pos + payload.len()].copy_from_slice(payload);
            pos += payload.len();
        } else {
            let mut chunk = Reader(payload);
            let start = pos;
            while !chunk.0.is_empty() {
                let flags = chunk.byte()?;
                if chunk.0.is_empty() {
                    return Err(Error::TruncatedInput);
                }
                for bit in 0..8 {
                    if chunk.0.is_empty() {
                        break;
                    }
                    let local = pos - start;
                    if flags & (1 << bit) == 0 {
                        if local == 4096 {
                            return Err(Error::InvalidData);
                        }
                        *output.get_mut(pos).ok_or(Error::OutputTooSmall)? = chunk.byte()?;
                        pos += 1;
                    } else {
                        let token = usize::from(chunk.word()?);
                        let bits = length_bits(local);
                        let offset = (token >> bits) + 1;
                        let length = (token & ((1 << bits) - 1)) + 3;
                        if offset > local {
                            return Err(Error::InvalidOffset);
                        }
                        if length > 4096 - local {
                            return Err(Error::InvalidData);
                        }
                        copy(output, pos, offset, length)?;
                        pos += length;
                    }
                }
            }
        }
    }
    Ok((pos, input.len()))
}

/// Compress using 4096-byte LZNT1 chunks and a bounded greedy match search.
///
/// Incompressible chunks are stored verbatim. Empty input produces empty output.
/// Output is at most input size plus two header bytes per chunk.
///
/// # Errors
/// Returns an error if output or scratch storage cannot be allocated.
pub fn compress(input: &[u8]) -> Result<Vec<u8>, Error> {
    let bound = input
        .len()
        .div_ceil(4096)
        .checked_mul(2)
        .and_then(|n| n.checked_add(input.len()));
    let mut out = storage(bound)?;
    let mut scratch = storage(Some(4608))?;
    for chunk in input.chunks(4096) {
        scratch.clear();
        let mut matcher = Matcher::new();
        let mut pos = 0;
        while pos < chunk.len() {
            let flag_pos = scratch.len();
            scratch.push(0);
            for bit in 0..8 {
                if pos == chunk.len() {
                    break;
                }
                let bits = length_bits(pos);
                let (offset, length) = matcher.find(chunk, pos, 1 << (16 - bits), (1 << bits) + 2);
                let advance = if length >= 3 {
                    scratch[flag_pos] |= 1 << bit;
                    let token = (((offset - 1) << bits) | (length - 3)) as u16;
                    scratch.extend_from_slice(&token.to_le_bytes());
                    length
                } else {
                    scratch.push(chunk[pos]);
                    1
                };
                for index in pos..pos + advance {
                    matcher.insert(chunk, index);
                }
                pos += advance;
            }
        }
        let (signature, payload) = if scratch.len() < chunk.len() {
            (0xb000, scratch.as_slice())
        } else {
            (0x3000, chunk)
        };
        out.extend_from_slice(&(signature | (payload.len() - 1) as u16).to_le_bytes());
        out.extend_from_slice(payload);
    }
    Ok(out)
}
