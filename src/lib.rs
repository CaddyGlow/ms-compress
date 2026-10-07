//! Native, memory-safe WIM, CAB, and Windows buffer compression codecs.
//!
//! XPRESS here means the LZ77+Huffman variant used in WIM, not plain XPRESS.
//! The caller supplies the exact decompressed block size through the output slice.
//!
//! Disable the default `std` feature for `no_std` with `alloc`. All codecs
//! remain available; heap-backed storage still requires a global allocator.
//! The optional `cli` feature enables `std`.
#![cfg_attr(not(feature = "std"), no_std)]
#![cfg_attr(
    all(any(miri, feature = "lsx"), target_arch = "loongarch64"),
    feature(stdarch_loongarch)
)]
#![deny(unsafe_code)]
#![deny(missing_docs)]

extern crate alloc;

/// Version of this library, as declared in `Cargo.toml`.
pub const VERSION: &str = env!("CARGO_PKG_VERSION");

#[cfg(test)]
extern crate std;

/// DEFLATE, zlib, gzip, and checksum APIs adapted from zlib-rs.
// Upstream uses unsafe aligned storage and SIMD behind a safe public API.
#[allow(unsafe_code, missing_docs)]
pub mod zlib;

/// LZMA1 and LZMA2 codecs using Rust global allocation.
#[allow(unsafe_code, missing_docs)]
pub mod lzma;

pub mod context;
pub mod lznt1;
mod memory;
mod nt_common;
mod xpress;
pub mod xpress_plain;

pub mod lzms;
pub mod lzx;
pub mod lzx_encode;
pub mod quantum;
pub mod xpress_encode;

pub use xpress::{decompress_xpress, decompress_xpress_windows};

// Callers validate the match's distance and output extent before entering here.
fn copy_lz_match(output: &mut [u8], position: usize, offset: usize, length: usize) {
    if length <= offset {
        output.copy_within(position - offset..position - offset + length, position);
    } else if offset == 1 {
        let byte = output[position - 1];
        output[position..position + length].fill(byte);
    } else {
        output.copy_within(position - offset..position, position);
        let mut copied = offset;
        while copied < length {
            let count = copied.min(length - copied);
            output.copy_within(position..position + count, position + copied);
            copied += count;
        }
    }
}

/// A malformed XPRESS Huffman block.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DecodeError {
    /// The 256-byte Huffman length header is incomplete.
    HeaderTooShort,
    /// The Huffman code is incomplete or oversubscribed.
    InvalidHuffmanCode,
    /// A match refers to bytes before the start of the output.
    InvalidMatchOffset {
        /// Distance backwards from the output cursor.
        offset: usize,
        /// Number of bytes already written.
        produced: usize,
    },
    /// A match extends beyond the requested output.
    MatchExceedsOutput {
        /// Decoded length of the match.
        length: usize,
        /// Capacity remaining in the requested output.
        remaining: usize,
    },
}

impl core::fmt::Display for DecodeError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match *self {
            Self::HeaderTooShort => f.write_str("XPRESS Huffman header requires 256 bytes"),
            Self::InvalidHuffmanCode => f.write_str("invalid XPRESS Huffman code lengths"),
            Self::InvalidMatchOffset { offset, produced } => {
                write!(
                    f,
                    "XPRESS match offset {offset} exceeds {produced} produced bytes"
                )
            }
            Self::MatchExceedsOutput { length, remaining } => {
                write!(
                    f,
                    "XPRESS match length {length} exceeds {remaining} remaining bytes"
                )
            }
        }
    }
}

impl core::error::Error for DecodeError {}

/// Zstandard frame encoding and decoding from the patched MIT-licensed ruzstd fork.
pub use ruzstd as zstd;

#[cfg(test)]
mod match_copy_tests {
    use alloc::vec;
    #[test]
    fn bulk_matches_agree_with_forward_byte_copy() {
        for offset in 1..=64 {
            for length in [0, 1, 2, 7, 8, 15, 16, 31, 64, 127, 4096] {
                let mut expected = vec![0; 64 + length];
                for (i, byte) in expected[..64].iter_mut().enumerate() {
                    *byte = (i * 37) as u8;
                }
                let mut actual = expected.clone();
                for i in 64..64 + length {
                    expected[i] = expected[i - offset];
                }
                super::copy_lz_match(&mut actual, 64, offset, length);
                assert_eq!(actual, expected, "offset={offset}, length={length}");
            }
        }
    }
}
