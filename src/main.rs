//! Optional command-line frontend for raw Windows compression buffers.
#![forbid(unsafe_code)]

use clap::{Args, Parser, Subcommand, ValueEnum};
use std::{
    error::Error,
    fs::File,
    io::{self, Read, Write},
    path::{Path, PathBuf},
};

const VERSION: &str = match option_env!("MS_COMPRESS_BUILD_TAG") {
    Some(tag) => tag,
    None => env!("CARGO_PKG_VERSION"),
};

#[derive(Parser)]
#[command(
    version = VERSION,
    about = "Pure Rust Windows buffer compression and decompression"
)]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Compress a raw buffer (no container or uncompressed-size header).
    Rtlcompress(Streams),
    /// Decompress a raw buffer; its exact uncompressed size must be supplied.
    Rtldecompress {
        #[command(flatten)]
        streams: Streams,
        /// Exact uncompressed size in bytes, including zero for empty buffers.
        #[arg(long)]
        output_size: usize,
    },
}

#[derive(Clone, Copy, ValueEnum)]
enum Format {
    /// Windows compression format 2.
    Lznt1,
    /// Plain XPRESS, Windows compression format 3.
    Xpress,
    /// Single WIM XPRESS Huffman block (at most 65536 uncompressed bytes).
    XpressHuffman,
}

#[derive(Args)]
struct Streams {
    #[arg(long, value_enum, default_value = "lznt1")]
    format: Format,
    /// Input path; '-' reads stdin.
    #[arg(short, long, default_value = "-")]
    input: PathBuf,
    /// Output path; '-' writes stdout. Existing files require --force.
    #[arg(short, long, default_value = "-")]
    output: PathBuf,
    /// Replace an existing output file after successful conversion.
    #[arg(long)]
    force: bool,
    /// Maximum input bytes read (default 256 MiB).
    #[arg(long, default_value_t = 268435456)]
    max_input: usize,
    /// Maximum output bytes (default 256 MiB).
    #[arg(long, default_value_t = 268435456)]
    max_output: usize,
}

fn read_bounded(reader: impl Read, maximum: usize) -> Result<Vec<u8>, Box<dyn Error>> {
    let limit = u64::try_from(maximum)?
        .checked_add(1)
        .ok_or("input limit overflow")?;
    let mut input = Vec::new();
    let mut reader = reader.take(limit);
    let mut buffer = [0; 65536];
    loop {
        let count = reader.read(&mut buffer)?;
        if count == 0 {
            break;
        }
        if count > maximum - input.len() {
            return Err("input exceeds --max-input".into());
        }
        input.try_reserve(count)?;
        input.extend_from_slice(&buffer[..count]);
    }
    Ok(input)
}

fn convert(
    streams: &Streams,
    input: &[u8],
    output_size: Option<usize>,
) -> Result<Vec<u8>, Box<dyn Error>> {
    if let Some(size) = output_size {
        if size > streams.max_output {
            return Err("--output-size exceeds --max-output".into());
        }
        if matches!(streams.format, Format::XpressHuffman) && size > 65536 {
            return Err("XPRESS Huffman blocks are limited to 65536 uncompressed bytes".into());
        }
        let mut output = Vec::new();
        output.try_reserve_exact(size)?;
        output.resize(size, 0);
        let written = match streams.format {
            Format::Lznt1 => ms_compress::lznt1::decompress(input, &mut output)?,
            Format::Xpress => ms_compress::xpress_plain::decompress(input, &mut output)?,
            Format::XpressHuffman => {
                ms_compress::decompress_xpress(input, &mut output)?;
                size
            }
        };
        if written != size {
            return Err("decoded size does not match --output-size".into());
        }
        Ok(output)
    } else {
        // Reject oversized workloads before allocating encoder output. The
        // codec APIs reserve against the input's worst-case literal encoding.
        let bound = match streams.format {
            Format::Lznt1 => input.len().checked_add(
                input
                    .len()
                    .div_ceil(4096)
                    .checked_mul(2)
                    .ok_or("size overflow")?,
            ),
            Format::Xpress => input.len().checked_add(
                (input.len() / 32 + 1)
                    .checked_mul(4)
                    .ok_or("size overflow")?,
            ),
            Format::XpressHuffman => input.len().checked_mul(2).and_then(|n| n.checked_add(4096)),
        }
        .ok_or("size overflow")?;
        if bound > streams.max_output {
            return Err("worst-case encoded size exceeds --max-output".into());
        }
        Ok(match streams.format {
            Format::Lznt1 => ms_compress::lznt1::compress(input)?,
            Format::Xpress => ms_compress::xpress_plain::compress(input)?,
            Format::XpressHuffman => ms_compress::xpress_encode::compress_xpress(input, bound)?
                .ok_or(
                    "input cannot be encoded as a WIM XPRESS Huffman block (minimum 25 bytes)",
                )?,
        })
    }
}

fn run() -> Result<(), Box<dyn Error>> {
    let cli = Cli::parse();
    let (streams, size) = match cli.command {
        Command::Rtlcompress(streams) => (streams, None),
        Command::Rtldecompress {
            streams,
            output_size,
        } => (streams, Some(output_size)),
    };
    let input = if streams.input == Path::new("-") {
        read_bounded(io::stdin().lock(), streams.max_input)?
    } else {
        read_bounded(File::open(&streams.input)?, streams.max_input)?
    };
    let output = convert(&streams, &input, size)?;
    if streams.output == Path::new("-") {
        let mut stdout = io::stdout().lock();
        stdout.write_all(&output)?;
        stdout.flush()?;
    } else {
        let parent = streams
            .output
            .parent()
            .filter(|p| !p.as_os_str().is_empty())
            .unwrap_or_else(|| Path::new("."));
        let mut temporary = tempfile::NamedTempFile::new_in(parent)?;
        temporary.write_all(&output)?;
        temporary.as_file().sync_all()?;
        if streams.force {
            temporary.persist(&streams.output)?;
        } else {
            temporary.persist_noclobber(&streams.output)?;
        }
    }
    Ok(())
}

fn main() -> std::process::ExitCode {
    match run() {
        Ok(()) => std::process::ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("ms-compress: {error}");
            std::process::ExitCode::FAILURE
        }
    }
}
