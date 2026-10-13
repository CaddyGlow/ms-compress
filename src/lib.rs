//! Native, memory-safe WIM, CAB, and Windows buffer compression codecs.
//!
//! XPRESS here means the LZ77+Huffman variant used in WIM, not plain XPRESS.
//! The caller supplies the exact decompressed block size through the output slice.
//!
//! Disable default features for `no_std` with `alloc`, then enable the desired
//! codec features (or `all-codecs`). Heap-backed storage requires a global allocator.
//! The optional `cli` feature enables `std`.
#![cfg_attr(not(feature = "std"), no_std)]
#![cfg_attr(
    all(any(miri, feature = "lsx"), target_arch = "loongarch64"),
    feature(stdarch_loongarch)
)]
#![deny(unsafe_code)]
#![deny(missing_docs)]

extern crate alloc;

// Reuse the vendored SIMD helpers without enabling the DEFLATE codec.
#[cfg(any(
    feature = "lznt1",
    feature = "xpress",
    feature = "lzx",
    feature = "lzms",
    feature = "quantum",
    feature = "deflate",
    feature = "lzma-optimization"
))]
#[path = "zlib/cpu_features.rs"]
#[allow(missing_docs)]
mod cpu_features;

#[cfg(any(
    feature = "lznt1",
    feature = "xpress",
    feature = "lzx",
    feature = "lzms",
    feature = "quantum",
    feature = "deflate"
))]
#[path = "zlib/deflate/compare256.rs"]
// Upstream includes helpers used only by DEFLATE and architecture-specific paths.
#[allow(unsafe_code, missing_docs, dead_code)]
pub(crate) mod match_compare;

/// Version of this library, as declared in `Cargo.toml`.
pub const VERSION: &str = env!("CARGO_PKG_VERSION");

#[cfg(test)]
extern crate std;

/// DEFLATE, zlib, gzip, and checksum APIs adapted from zlib-rs.
// Upstream uses unsafe aligned storage and SIMD behind a safe public API.
#[allow(unsafe_code, missing_docs)]
#[cfg(feature = "deflate")]
pub mod zlib;

/// LZMA1 and LZMA2 codecs using Rust global allocation.
#[allow(unsafe_code, missing_docs)]
#[cfg(feature = "lzma")]
pub mod lzma;

#[cfg(any(feature = "xpress", feature = "lzx", feature = "lzms"))]
pub mod context;
#[cfg(feature = "lznt1")]
pub mod lznt1;
#[cfg(any(feature = "xpress", feature = "lzx", feature = "lzms"))]
mod memory;
#[cfg(any(feature = "lznt1", feature = "xpress"))]
mod nt_common;
#[cfg(feature = "xpress")]
mod xpress;
#[cfg(feature = "xpress")]
pub mod xpress_plain;

#[cfg(feature = "lzms")]
pub mod lzms;
#[cfg(feature = "lzx")]
pub mod lzx;
#[cfg(feature = "lzx")]
pub mod lzx_encode;
#[cfg(feature = "quantum")]
pub mod quantum;
#[cfg(feature = "xpress")]
pub mod xpress_encode;

/// A malformed Windows compression buffer or insufficient output capacity.
#[cfg(any(feature = "lznt1", feature = "xpress"))]
pub use nt_common::Error as BufferError;
#[cfg(feature = "xpress")]
pub use xpress::{decompress_xpress, decompress_xpress_huffman_buffer, decompress_xpress_windows};

// Callers validate the match's distance and output extent before entering here.
#[cfg(any(
    feature = "lznt1",
    feature = "xpress",
    feature = "lzx",
    feature = "lzms"
))]
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
#[cfg(feature = "xpress")]
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

#[cfg(feature = "xpress")]
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

#[cfg(feature = "xpress")]
impl core::error::Error for DecodeError {}

/// Zstandard frame encoding and decoding from the patched MIT-licensed ruzstd fork.
#[cfg(feature = "zstd")]
pub use ruzstd as zstd;

#[cfg(test)]
#[cfg(any(
    feature = "lznt1",
    feature = "xpress",
    feature = "lzx",
    feature = "lzms"
))]
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
