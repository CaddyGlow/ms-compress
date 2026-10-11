//! Independent raw-DEFLATE implementations. See README.md for timing scope.
use std::{
    error::Error,
    hint::black_box,
    time::{Duration, Instant},
};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Codec {
    Ms,
    Classic,
    Zlib,
    Miniz,
    Libdeflate,
}

const ENCODERS: [Codec; 5] = [
    Codec::Ms,
    Codec::Classic,
    Codec::Zlib,
    Codec::Miniz,
    Codec::Libdeflate,
];
const DECODERS: [Codec; 4] = [Codec::Ms, Codec::Zlib, Codec::Miniz, Codec::Libdeflate];

impl Codec {
    fn name(self) -> &'static str {
        match self {
            Self::Ms => "ms-compress",
            Self::Classic => "ms-compress-classic",
            Self::Zlib => "zlib-c",
            Self::Miniz => "miniz_oxide",
            Self::Libdeflate => "libdeflate",
        }
    }

    fn mem_level(self) -> &'static str {
        match self {
            Self::Ms | Self::Zlib => "8",
            Self::Classic => "9",
            _ => "na",
        }
    }

    fn encode(self, input: &[u8], level: i32) -> Vec<u8> {
        if self == Self::Miniz {
            return miniz_oxide::deflate::compress_to_vec(input, level as u8);
        }
        let mut output = vec![0; input.len() + input.len() / 8 + 4096];
        let used = match self {
            Self::Ms => {
                let (encoded, status) = ms_compress::zlib::compress_slice(
                    &mut output,
                    input,
                    ms_compress::zlib::DeflateConfig {
                        level,
                        window_bits: -15,
                        mem_level: 8,
                        ..Default::default()
                    },
                );
                assert_eq!(status, ms_compress::zlib::ReturnCode::Ok);
                encoded.len()
            }
            Self::Classic => {
                assert_eq!(level, 1);
                let mut scratch = vec![0; 400000];
                ms_compress::zlib::classic::raw_level1(input, &mut output, &mut scratch)
                    .expect("sufficient output and scratch capacity")
            }
            Self::Zlib => {
                let mut compressor =
                    flate2::Compress::new(flate2::Compression::new(level as u32), false);
                assert_eq!(
                    compressor
                        .compress(input, &mut output, flate2::FlushCompress::Finish)
                        .unwrap(),
                    flate2::Status::StreamEnd
                );
                assert_eq!(compressor.total_in(), input.len() as u64);
                compressor.total_out() as usize
            }
            Self::Libdeflate => {
                let mut compressor =
                    libdeflater::Compressor::new(libdeflater::CompressionLvl::new(level).unwrap());
                assert!(output.len() >= compressor.deflate_compress_bound(input.len()));
                compressor.deflate_compress(input, &mut output).unwrap()
            }
            Self::Miniz => unreachable!(),
        };
        output.truncate(used);
        output
    }

    fn decode(self, input: &[u8], output: &mut [u8]) {
        let used = match self {
            Self::Ms => {
                let (decoded, status) = ms_compress::zlib::decompress_slice(
                    output,
                    input,
                    ms_compress::zlib::InflateConfig { window_bits: -15 },
                );
                assert_eq!(status, ms_compress::zlib::ReturnCode::Ok);
                decoded.len()
            }
            Self::Zlib => {
                let mut decoder = flate2::Decompress::new(false);
                assert_eq!(
                    decoder
                        .decompress(input, output, flate2::FlushDecompress::Finish)
                        .unwrap(),
                    flate2::Status::StreamEnd
                );
                assert_eq!(decoder.total_in(), input.len() as u64);
                decoder.total_out() as usize
            }
            Self::Miniz => miniz_oxide::inflate::decompress_slice_iter_to_slice(
                output,
                std::iter::once(input),
                false,
                false,
            )
            .unwrap(),
            Self::Libdeflate => libdeflater::Decompressor::new()
                .deflate_decompress(input, output)
                .unwrap(),
            Self::Classic => unreachable!("classic exposes encoders only"),
        };
        assert_eq!(used, output.len());
    }
}

fn dataset(kind: &str, length: usize) -> Vec<u8> {
    let mut state = 0x12345678u32;
    (0..length)
        .map(|i| match kind {
            "repeat" => b'A',
            "pattern" => (i * 37 + i / 11) as u8,
            "random" => {
                state ^= state << 13;
                state ^= state >> 17;
                state ^= state << 5;
                state as u8
            }
            _ => unreachable!(),
        })
        .collect()
}

fn measure(mut operation: impl FnMut(), budget: Duration) -> (u64, f64) {
    let start = Instant::now();
    let mut iterations = 0;
    loop {
        operation();
        iterations += 1;
        let elapsed = start.elapsed();
        if elapsed >= budget {
            return (iterations, elapsed.as_secs_f64());
        }
    }
}

fn main() -> Result<(), Box<dyn Error>> {
    let milliseconds: u64 = std::env::var("MS_COMPRESS_BENCH_MS")
        .unwrap_or_else(|_| "100".into())
        .parse()?;
    if milliseconds == 0 {
        return Err("MS_COMPRESS_BENCH_MS must be positive".into());
    }
    let budget = Duration::from_millis(milliseconds);
    println!(
        "format,implementation,source,level,source_mem_level,dataset,input_bytes,compressed_bytes,operation,sample,iterations,seconds,mib_per_second"
    );
    for size in [4096, 32768, 131072, 1048576] {
        for kind in ["repeat", "pattern", "random"] {
            let input = dataset(kind, size);
            let mut output = vec![0; size];
            for level in [1, 6] {
                let streams: Vec<_> = ENCODERS
                    .iter()
                    .copied()
                    .filter(|codec| *codec != Codec::Classic || level == 1)
                    .map(|codec| (codec, codec.encode(&input, level)))
                    .collect();
                // Every encoder's output must decode with every implementation
                // before any sample is recorded. Poison prevents stale matches.
                for &(source, ref encoded) in &streams {
                    for decoder in DECODERS {
                        output.fill(0x55);
                        decoder.decode(encoded, &mut output);
                        assert_eq!(output, input, "{} -> {}", source.name(), decoder.name());
                    }
                }
                for sample in 0..3 {
                    // Rotate encoder and decoder order to reduce ordering bias.
                    for index in 0..streams.len() {
                        let (source, encoded) = &streams[(index + sample) % streams.len()];
                        let (iterations, seconds) = measure(
                            || {
                                black_box(source.encode(black_box(&input), level));
                            },
                            budget,
                        );
                        let rate = size as f64 * iterations as f64 / seconds / 1048576.0;
                        println!(
                            "deflate,{0},{0},{level},{1},{kind},{size},{2},compress,{sample},{iterations},{seconds:.6},{rate:.2}",
                            source.name(),
                            source.mem_level(),
                            encoded.len()
                        );
                        for index in 0..DECODERS.len() {
                            let decoder = DECODERS[(index + sample) % DECODERS.len()];
                            let (iterations, seconds) = measure(
                                || {
                                    decoder.decode(black_box(encoded), black_box(&mut output));
                                    black_box(&output);
                                },
                                budget,
                            );
                            let rate = size as f64 * iterations as f64 / seconds / 1048576.0;
                            println!(
                                "deflate,{0},{1},{level},{2},{kind},{size},{3},decompress,{sample},{iterations},{seconds:.6},{rate:.2}",
                                decoder.name(),
                                source.name(),
                                source.mem_level(),
                                encoded.len()
                            );
                        }
                    }
                }
            }
        }
    }
    Ok(())
}
