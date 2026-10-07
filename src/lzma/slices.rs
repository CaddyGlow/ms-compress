use super::{
    Error, Lzma2Options, Lzma2Reader, Lzma2Writer, LzmaOptions, LzmaReader, LzmaWriter, Read,
    Result, Write, error_invalid_input,
};
use alloc::vec::Vec;

fn options(input_len: usize, preset: u32) -> Result<LzmaOptions> {
    if preset > 9 {
        return Err(error_invalid_input("LZMA preset must be between 0 and 9"));
    }
    let mut options = LzmaOptions::with_preset(preset);
    // A dictionary larger than the input cannot improve a one-shot encoding.
    // Keep small-buffer presets from allocating tens of megabytes unnecessarily.
    options.dict_size = options
        .dict_size
        .min(input_len.max(4096).min(u32::MAX as usize) as u32);
    Ok(options)
}

/// Compress a buffer into the .lzma format using preset 0 through 9.
///
/// The header records the input size. Workspace uses Rust's global allocator;
/// the dictionary is capped at the input length (minimum 4 KiB).
///
/// ```
/// let input = b"hello";
/// let encoded = ms_compress::lzma::compress(input, 6).unwrap();
/// let mut output = [0; 5];
/// let size = ms_compress::lzma::decompress(&encoded, &mut output, 1024).unwrap();
/// assert_eq!(&output[..size], input);
/// ```
pub fn compress(input: &[u8], preset: u32) -> Result<Vec<u8>> {
    let options = options(input.len(), preset)?;
    let mut writer = LzmaWriter::new_use_header(Vec::new(), &options, Some(input.len() as u64))?;
    writer.write_all(input)?;
    writer.finish()
}

/// Decode a .lzma buffer into bounded caller-owned output.
///
/// Returns the decoded length. Excess output, truncated input, and decoder
/// workspace exceeding `memory_limit_kib` are errors. Trailing input is allowed.
pub fn decompress(input: &[u8], output: &mut [u8], memory_limit_kib: u32) -> Result<usize> {
    let reader = LzmaReader::new_mem_limit(input, memory_limit_kib, None)?;
    read_bounded(reader, output)
}

/// Compress a buffer into raw LZMA2 using preset 0 through 9.
///
/// The dictionary size is returned with the stream because raw LZMA2 has no
/// dictionary header. Workspace uses Rust's global allocator.
pub fn compress_lzma2(input: &[u8], preset: u32) -> Result<(Vec<u8>, u32)> {
    let options = options(input.len(), preset)?;
    let dictionary_size = options.dict_size;
    let mut writer = Lzma2Writer::new(
        Vec::new(),
        Lzma2Options {
            lzma_options: options,
            chunk_size: None,
        },
    );
    writer.write_all(input)?;
    Ok((writer.finish()?, dictionary_size))
}

/// Decode raw LZMA2 into bounded output, with the supplied dictionary size.
///
/// Excess output, truncated input, and decoder workspace exceeding the given
/// memory limit in KiB are errors. Trailing input is allowed.
pub fn decompress_lzma2(
    input: &[u8],
    output: &mut [u8],
    dictionary_size: u32,
    memory_limit_kib: u32,
) -> Result<usize> {
    let reader = Lzma2Reader::new_mem_limit(input, dictionary_size, memory_limit_kib, None)?;
    read_bounded(reader, output)
}

fn read_bounded(mut reader: impl Read, output: &mut [u8]) -> Result<usize> {
    let mut produced = 0;
    while produced < output.len() {
        match reader.read(&mut output[produced..]) {
            Ok(0) => return Ok(produced),
            Ok(count) => produced += count,
            Err(error) if interrupted(&error) => continue,
            Err(error) => return Err(error),
        }
    }
    loop {
        match reader.read(&mut [0]) {
            Ok(0) => return Ok(produced),
            Ok(_) => {
                return Err(error_invalid_input(
                    "LZMA output exceeds the supplied buffer",
                ));
            }
            Err(error) if interrupted(&error) => continue,
            Err(error) => return Err(error),
        }
    }
}

fn interrupted(error: &Error) -> bool {
    #[cfg(feature = "std")]
    return error.kind() == std::io::ErrorKind::Interrupted;
    #[cfg(not(feature = "std"))]
    matches!(error, Error::Interrupted)
}
