//! Plain XPRESS (MS-XCA sections 2.3 and 2.4), without Huffman coding.
//!
//! This is the buffer format for Windows COMPRESSION_FORMAT_XPRESS (3).
//! Encoders produce compatible streams, not identical Windows parse choices.
//! Reference: https://learn.microsoft.com/en-us/openspecs/windows_protocols/ms-xca/34cb9ab9-5ce6-42d7-a518-107c1c7c65e7

pub use crate::nt_common::Error;
use crate::nt_common::{Matcher, Reader, copy, storage};
use alloc::vec::Vec;

/// Decompress a complete plain XPRESS buffer, returning the bytes written.
///
/// `output` bounds expansion; no allocation occurs. An error may leave partial
/// output. A terminal match flag at the input end is required.
///
/// # Errors
/// Returns an error for malformed/truncated input or insufficient output space.
pub fn decompress(input: &[u8], output: &mut [u8]) -> Result<usize, Error> {
    let mut reader = Reader(input);
    let mut pos = 0;
    let mut high_nibble = None;
    loop {
        let flags = reader.dword()?;
        for bit in (0..32).rev() {
            if flags & (1 << bit) == 0 {
                let byte = reader.byte()?;
                *output.get_mut(pos).ok_or(Error::OutputTooSmall)? = byte;
                pos += 1;
            } else {
                if reader.0.is_empty() {
                    return Ok(pos);
                }
                let token = reader.word()?;
                let offset = usize::from(token >> 3) + 1;
                let mut length = usize::from(token & 7);
                if length == 7 {
                    length = if let Some(nibble) = high_nibble.take() {
                        nibble
                    } else {
                        let byte = reader.byte()?;
                        high_nibble = Some(usize::from(byte >> 4));
                        usize::from(byte & 15)
                    };
                    if length == 15 {
                        length = usize::from(reader.byte()?);
                        if length == 255 {
                            length = usize::from(reader.word()?);
                            if length == 0 {
                                length = usize::try_from(reader.dword()?)
                                    .map_err(|_| Error::InvalidData)?;
                            }
                            length = length.checked_sub(22).ok_or(Error::InvalidData)?;
                        }
                        length += 15;
                    }
                    length += 7;
                }
                length = length.checked_add(3).ok_or(Error::InvalidData)?;
                copy(output, pos, offset, length)?;
                pos += length;
            }
        }
    }
}

/// Compress a plain XPRESS buffer using a bounded greedy match search.
///
/// Output can exceed input size; allocation is bounded by the literal encoding.
/// Empty input produces a four-byte terminal flag group.
///
/// # Errors
/// Returns an error if output storage cannot be allocated.
pub fn compress(input: &[u8]) -> Result<Vec<u8>, Error> {
    let bound = input
        .len()
        .checked_div(32)
        .and_then(|n| n.checked_add(1))
        .and_then(|n| n.checked_mul(4))
        .and_then(|n| n.checked_add(input.len()));
    let mut out = storage(bound)?;
    out.extend_from_slice(&[0; 4]);
    let mut matcher = Matcher::new();
    let (mut pos, mut group, mut count, mut flags) = (0, 0, 0u32, 0u32);
    let mut nibble_position: Option<usize> = None;
    while pos < input.len() {
        let (offset, length) = matcher.find(input, pos, 8192, u32::MAX as usize - 3);
        let advance = if length >= 3 {
            flags |= 1 << (31 - count);
            let token = (((offset - 1) as u16) << 3) | ((length - 3).min(7) as u16);
            out.extend_from_slice(&token.to_le_bytes());
            if length >= 10 {
                let nibble = (length - 10).min(15) as u8;
                if let Some(index) = nibble_position.take() {
                    out[index] |= nibble << 4;
                } else {
                    nibble_position = Some(out.len());
                    out.push(nibble);
                }
                if length >= 25 {
                    out.push((length - 25).min(255) as u8);
                    if length >= 280 {
                        let encoded = length - 3;
                        if encoded < 65536 {
                            out.extend_from_slice(&(encoded as u16).to_le_bytes());
                        } else {
                            out.extend_from_slice(&[0, 0]);
                            out.extend_from_slice(&(encoded as u32).to_le_bytes());
                        }
                    }
                }
            }
            length
        } else {
            out.push(input[pos]);
            1
        };
        for index in pos..pos + advance {
            matcher.insert(input, index);
        }
        pos += advance;
        count += 1;
        if count == 32 {
            out[group..group + 4].copy_from_slice(&flags.to_le_bytes());
            group = out.len();
            out.extend_from_slice(&[0; 4]);
            count = 0;
            flags = 0;
        }
    }
    flags |= u32::MAX >> count;
    out[group..group + 4].copy_from_slice(&flags.to_le_bytes());
    Ok(out)
}
