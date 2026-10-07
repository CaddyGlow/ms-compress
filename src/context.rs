// SPDX-License-Identifier: LGPL-2.1-or-later
//! Safe codec configuration and reusable decoding contexts.
//!
//! Configuration follows wimlib's compress.c/decompress.c validation rules.
//! Defaults are explicit values rather than process-global mutable state.
//! Decompressors allocate all scratch at creation and reuse it without
//! allocation per block; compressor scratch currently remains per operation.
//! Compression levels are resolved here for future tunable
//! encoders, not silently advertised as implemented compression policies.

use alloc::vec::Vec;
/// wimlib's permission to modify compression input.
pub const DESTRUCTIVE: u32 = 0x8000_0000;

/// An actual compression codec; uncompressed storage has no codec context.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Codec {
    /// WIM XPRESS Huffman.
    Xpress,
    /// WIM LZX.
    Lzx,
    /// WIM LZMS.
    Lzms,
}
impl Codec {
    /// Resolve the original numeric compression type.
    pub fn from_wimlib(value: i32) -> Result<Self, ContextError> {
        match value {
            1 => Ok(Self::Xpress),
            2 => Ok(Self::Lzx),
            3 => Ok(Self::Lzms),
            _ => Err(ContextError::InvalidCompressionType),
        }
    }
    fn index(self) -> usize {
        match self {
            Self::Xpress => 0,
            Self::Lzx => 1,
            Self::Lzms => 2,
        }
    }
    fn maximum(self) -> usize {
        match self {
            Self::Xpress => 65536,
            Self::Lzx => 1 << 21,
            Self::Lzms => 1 << 30,
        }
    }
}

/// Factory errors and decode outcomes, independent of C pointer ownership.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ContextError {
    /// Numeric codec is not XPRESS, LZX, or LZMS.
    InvalidCompressionType,
    /// Maximum size or compression level is invalid.
    InvalidParameter,
    /// Requested output exceeds the handle's configured maximum (C returns -2).
    OutputExceedsMaximum,
    /// Codec rejected the compressed stream (C returns -1).
    InvalidCompressedData,
    /// Codec scratch allocation failed.
    OutOfMemory,
}
impl core::fmt::Display for ContextError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.write_str(match self {
            Self::InvalidCompressionType => "invalid compression type",
            Self::InvalidParameter => "invalid codec parameter",
            Self::OutputExceedsMaximum => "output exceeds codec maximum",
            Self::InvalidCompressedData => "invalid compressed data",
            Self::OutOfMemory => "codec allocation failed",
        })
    }
}
impl core::error::Error for ContextError {}

/// Explicit equivalent of the original per-codec default level table.
#[derive(Clone, Debug, Default)]
pub struct CompressionDefaults {
    levels: [u32; 3],
}
impl CompressionDefaults {
    /// Read a codec's raw stored default; zero retains the built-in fallback.
    pub fn raw_level(&self, codec: i32) -> Result<u32, ContextError> {
        Ok(self.levels[Codec::from_wimlib(codec)?.index()])
    }
    /// Set a codec's raw default, or all codecs when `codec == -1`.
    /// Like upstream, this setter accepts every unsigned value without masking.
    pub fn set(&mut self, codec: i32, level: u32) -> Result<(), ContextError> {
        if codec == -1 {
            self.levels.fill(level);
        } else {
            self.levels[Codec::from_wimlib(codec)?.index()] = level;
        }
        Ok(())
    }
    /// Validate a compressor request and resolve zero through the defaults.
    /// The destructive bit is extracted before default resolution, matching C;
    /// an arbitrary default is deliberately not revalidated or remasked.
    pub fn resolve(
        &self,
        codec: i32,
        maximum: usize,
        level: u32,
    ) -> Result<CompressionConfig, ContextError> {
        let codec = Codec::from_wimlib(codec)?;
        let destructive = level & DESTRUCTIVE != 0;
        let mut level = level & !DESTRUCTIVE;
        if level > 0x00ff_ffff || maximum == 0 || maximum > codec.maximum() {
            return Err(ContextError::InvalidParameter);
        }
        if level == 0 {
            level = self.levels[codec.index()];
        }
        if level == 0 {
            level = 50;
        }
        Ok(CompressionConfig {
            codec,
            maximum,
            level,
            destructive,
        })
    }
}

/// Validated compressor configuration, without claiming level-tuning support.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct CompressionConfig {
    /// Selected codec.
    pub codec: Codec,
    /// Largest input block allowed.
    pub maximum: usize,
    /// Resolved level, including raw defaults accepted by upstream.
    pub level: u32,
    /// Caller permits destructive input processing.
    pub destructive: bool,
}

/// A reusable native decompressor with a fixed codec and maximum output size.
#[derive(Debug)]
pub struct Decompressor {
    codec: Codec,
    maximum: usize,
    lzms: Option<crate::lzms::LzmsDecoder>,
    xpress: Option<crate::xpress::XpressDecoder>,
    lzx: Option<crate::lzx::LzxDecoder>,
}
impl Decompressor {
    /// Create a handle using upstream's numeric type and maximum-size rules.
    /// Rust ownership replaces `free_decompressor`; no foreign code is linked.
    pub fn new(codec: i32, maximum: usize) -> Result<Self, ContextError> {
        let codec = Codec::from_wimlib(codec)?;
        if maximum == 0 || maximum > codec.maximum() {
            return Err(ContextError::InvalidParameter);
        }
        let lzms = if codec == Codec::Lzms {
            Some(crate::lzms::LzmsDecoder::new().map_err(|_| ContextError::OutOfMemory)?)
        } else {
            None
        };
        Ok(Self {
            codec,
            maximum,
            lzms,
            xpress: if codec == Codec::Xpress {
                Some(crate::xpress::XpressDecoder::new().map_err(|_| ContextError::OutOfMemory)?)
            } else {
                None
            },
            lzx: if codec == Codec::Lzx {
                Some(crate::lzx::LzxDecoder::new().map_err(|_| ContextError::OutOfMemory)?)
            } else {
                None
            },
        })
    }
    /// Decode one independent block. A failed call does not poison the handle.
    pub fn decompress(&mut self, input: &[u8], output: &mut [u8]) -> Result<(), ContextError> {
        if output.len() > self.maximum {
            return Err(ContextError::OutputExceedsMaximum);
        }
        match self.codec {
            Codec::Xpress => self
                .xpress
                .as_mut()
                .ok_or(ContextError::InvalidParameter)?
                .decompress(input, output)
                .map_err(|_| ContextError::InvalidCompressedData),
            Codec::Lzx => self
                .lzx
                .as_mut()
                .ok_or(ContextError::InvalidParameter)?
                .decompress(input, output, self.maximum)
                .map_err(|_| ContextError::InvalidCompressedData),
            Codec::Lzms => self
                .lzms
                .as_mut()
                .ok_or(ContextError::InvalidParameter)?
                .decompress(input, output)
                .map_err(|error| {
                    if error == crate::lzms::LzmsError::OutOfMemory {
                        ContextError::OutOfMemory
                    } else {
                        ContextError::InvalidCompressedData
                    }
                }),
        }
    }
}

/// Reusable native compressor with the current codec's fixed search strategy.
///
/// This deliberately takes no compression level: accepting and ignoring levels
/// would imply wimlib's tuning policies were implemented. Input is preserved;
/// destructive permission is unnecessary for this safe API. Owned scratch storage is retained and reset across calls, while maximum-size
/// validation persists. Owned output copies use the default Rust allocator.
#[derive(Debug)]
pub struct FixedStrategyCompressor {
    maximum: usize,
    workspace: CompressorWorkspace,
}
// Keep ownership inline in the fallibly allocated context. A Box would add
// an infallible allocation and a new OOM boundary.
#[allow(clippy::large_enum_variant)]
#[derive(Debug)]
enum CompressorWorkspace {
    Xpress(crate::xpress_encode::XpressCompressor),
    Lzx(crate::lzx_encode::LzxCompressor),
    Lzms(crate::lzms::encode::LzmsCompressor),
}
impl FixedStrategyCompressor {
    /// Largest input block accepted by this compressor.
    pub fn maximum(&self) -> usize {
        self.maximum
    }
    /// Validate numeric codec and maximum input length, then create a handle.
    pub fn new(codec: i32, maximum: usize) -> Result<Self, ContextError> {
        let codec = Codec::from_wimlib(codec)?;
        if maximum == 0 || maximum > codec.maximum() {
            return Err(ContextError::InvalidParameter);
        }
        let workspace = match codec {
            Codec::Xpress => CompressorWorkspace::Xpress(
                crate::xpress_encode::XpressCompressor::new(maximum)
                    .map_err(|_| ContextError::OutOfMemory)?,
            ),
            Codec::Lzx => CompressorWorkspace::Lzx(
                crate::lzx_encode::LzxCompressor::new(maximum)
                    .map_err(|_| ContextError::OutOfMemory)?,
            ),
            Codec::Lzms => CompressorWorkspace::Lzms(
                crate::lzms::encode::LzmsCompressor::new(maximum)
                    .map_err(|_| ContextError::OutOfMemory)?,
            ),
        };
        Ok(Self { maximum, workspace })
    }
    /// Compress one independent block with bounded output capacity.
    ///
    /// Returns `None` when input is empty, exceeds this handle's maximum, or
    /// cannot be encoded within capacity, corresponding to C's zero result.
    /// Allocation failures are explicit rather than conflated with capacity.
    pub fn compress(
        &mut self,
        input: &[u8],
        capacity: usize,
    ) -> Result<Option<Vec<u8>>, ContextError> {
        let Some(bytes) = self.compress_borrowed(input, capacity)? else {
            return Ok(None);
        };
        let mut output = Vec::new();
        output
            .try_reserve_exact(bytes.len())
            .map_err(|_| ContextError::OutOfMemory)?;
        output.extend_from_slice(bytes);
        Ok(Some(output))
    }
    /// Compress into retained storage without allocation; the next call invalidates the result.
    pub fn compress_borrowed(
        &mut self,
        input: &[u8],
        capacity: usize,
    ) -> Result<Option<&[u8]>, ContextError> {
        if input.is_empty() || input.len() > self.maximum {
            return Ok(None);
        }
        match &mut self.workspace {
            CompressorWorkspace::Xpress(workspace) => workspace
                .compress(input, capacity)
                .map_err(|_| ContextError::OutOfMemory),
            CompressorWorkspace::Lzx(workspace) => workspace
                .compress(input, capacity)
                .map_err(|_| ContextError::OutOfMemory),
            CompressorWorkspace::Lzms(workspace) => workspace
                .compress(input, capacity)
                .map_err(|_| ContextError::OutOfMemory),
        }
    }
}
