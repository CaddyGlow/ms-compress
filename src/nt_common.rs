//! Shared bounded primitives for Windows buffer codecs.

use alloc::vec::Vec;
/// A Windows buffer codec failure.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Error {
    /// The input ends inside a header, literal, or match.
    TruncatedInput,
    /// A header, length, or terminal marker is invalid.
    InvalidData,
    /// A match points before the beginning of its history.
    InvalidOffset,
    /// The supplied output buffer is too small.
    OutputTooSmall,
    /// Output allocation failed or its size overflowed.
    AllocationFailed,
}

impl core::fmt::Display for Error {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.write_str(match self {
            Self::TruncatedInput => "truncated compressed buffer",
            Self::InvalidData => "invalid compressed buffer",
            Self::InvalidOffset => "match precedes available history",
            Self::OutputTooSmall => "decompressed output exceeds buffer",
            Self::AllocationFailed => "codec allocation failed",
        })
    }
}
impl core::error::Error for Error {}

pub(crate) fn storage(size: Option<usize>) -> Result<Vec<u8>, Error> {
    let mut bytes = Vec::new();
    bytes
        .try_reserve_exact(size.ok_or(Error::AllocationFailed)?)
        .map_err(|_| Error::AllocationFailed)?;
    Ok(bytes)
}

pub(crate) struct Reader<'a>(pub(crate) &'a [u8]);
impl<'a> Reader<'a> {
    pub(crate) fn take(&mut self, count: usize) -> Result<&'a [u8], Error> {
        let bytes = self.0.get(..count).ok_or(Error::TruncatedInput)?;
        self.0 = &self.0[count..];
        Ok(bytes)
    }
    pub(crate) fn byte(&mut self) -> Result<u8, Error> {
        Ok(self.take(1)?[0])
    }
    pub(crate) fn word(&mut self) -> Result<u16, Error> {
        let b = self.take(2)?;
        Ok(u16::from_le_bytes([b[0], b[1]]))
    }
    pub(crate) fn dword(&mut self) -> Result<u32, Error> {
        let b = self.take(4)?;
        Ok(u32::from_le_bytes([b[0], b[1], b[2], b[3]]))
    }
}

pub(crate) fn copy(
    output: &mut [u8],
    position: usize,
    offset: usize,
    length: usize,
) -> Result<(), Error> {
    if offset > position {
        return Err(Error::InvalidOffset);
    }
    if length > output.len() - position {
        return Err(Error::OutputTooSmall);
    }
    crate::copy_lz_match(output, position, offset, length);
    Ok(())
}

// One candidate per hash keeps memory and search work independent of input size.
pub(crate) struct Matcher {
    heads: [usize; 4096],
}
impl Matcher {
    pub(crate) fn new() -> Self {
        Self {
            heads: [usize::MAX; 4096],
        }
    }
    fn hash(input: &[u8], pos: usize) -> usize {
        ((usize::from(input[pos]) * 251 + usize::from(input[pos + 1])) * 251
            + usize::from(input[pos + 2]))
            & 4095
    }
    pub(crate) fn insert(&mut self, input: &[u8], pos: usize) {
        if input.len() - pos >= 3 {
            self.heads[Self::hash(input, pos)] = pos;
        }
    }
    pub(crate) fn find(
        &self,
        input: &[u8],
        pos: usize,
        window: usize,
        maximum: usize,
    ) -> (usize, usize) {
        if input.len() - pos < 3 {
            return (0, 0);
        }
        let previous = self.heads[Self::hash(input, pos)];
        if previous >= pos || pos - previous > window {
            return (0, 0);
        }
        let limit = maximum.min(input.len() - pos);
        let length = crate::zlib::common_prefix(
            &input[previous..previous + limit],
            &input[pos..pos + limit],
        );
        (pos - previous, length)
    }
}
