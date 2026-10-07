//! End-to-end codec throughput, including context creation and allocation.
use ms_compress::{
    context::{Decompressor, FixedStrategyCompressor},
    lznt1, xpress_plain,
};
use std::{
    error::Error,
    hint::black_box,
    time::{Duration, Instant},
};

fn encode(codec: usize, input: &[u8]) -> Result<Vec<u8>, Box<dyn Error>> {
    Ok(match codec {
        0 => lznt1::compress(input)?,
        1 => xpress_plain::compress(input)?,
        2..=4 => FixedStrategyCompressor::new((codec - 1) as i32, 32768)?
            .compress(input, input.len() * 2 + 4096)?
            .ok_or("unencodable benchmark input")?
            .to_vec(),
        5 => ms_compress::lzx_encode::CabinetLzxEncoder::new(15)?
            .compress_frame(input)?
            .to_vec(),
        6 => ms_compress::quantum::QuantumEncoder::new(15, 4)?.compress_frame(input)?,
        7 => ms_compress::lzma::compress(input, 6)?,
        8 => {
            let (encoded, dictionary) = ms_compress::lzma::compress_lzma2(input, 6)?;
            let mut framed = dictionary.to_le_bytes().to_vec();
            framed.extend_from_slice(&encoded);
            framed
        }
        9..=11 => {
            let mut output = vec![0; ms_compress::zlib::compress_bound(input.len()) + 32];
            let (encoded, result) = ms_compress::zlib::compress_slice(
                &mut output,
                input,
                ms_compress::zlib::DeflateConfig {
                    window_bits: [-15, 15, 31][codec - 9],
                    ..Default::default()
                },
            );
            assert_eq!(result, ms_compress::zlib::ReturnCode::Ok);
            encoded.to_vec()
        }
        _ => return Err("unknown codec".into()),
    })
}

fn decode(codec: usize, input: &[u8], output: &mut [u8]) -> Result<(), Box<dyn Error>> {
    match codec {
        0 => {
            assert_eq!(lznt1::decompress(input, output)?, output.len());
        }
        1 => {
            assert_eq!(xpress_plain::decompress(input, output)?, output.len());
        }
        2..=4 => Decompressor::new((codec - 1) as i32, 32768)?.decompress(input, output)?,
        5 => ms_compress::lzx::CabinetLzxDecoder::new(15)?.decompress_frame(input, output)?,
        6 => ms_compress::quantum::QuantumDecoder::new(15)?.decompress_frame(input, output)?,
        7 => {
            assert_eq!(
                ms_compress::lzma::decompress(input, output, 1024)?,
                output.len()
            );
        }
        8 => {
            let dictionary = u32::from_le_bytes(input[..4].try_into()?);
            assert_eq!(
                ms_compress::lzma::decompress_lzma2(&input[4..], output, dictionary, 1024)?,
                output.len()
            );
        }
        9..=11 => {
            let (decoded, result) = ms_compress::zlib::decompress_slice(
                output,
                input,
                ms_compress::zlib::InflateConfig {
                    window_bits: [-15, 15, 31][codec - 9],
                },
            );
            assert_eq!(result, ms_compress::zlib::ReturnCode::Ok);
            assert_eq!(decoded.len(), output.len());
        }
        _ => return Err("unknown codec".into()),
    }
    Ok(())
}

fn measure(mut operation: impl FnMut(), budget: Duration) -> (u64, Duration) {
    let start = Instant::now();
    let mut iterations = 0;
    loop {
        operation();
        iterations += 1;
        if start.elapsed() >= budget {
            return (iterations, start.elapsed());
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
        "codec,dataset,input_bytes,compressed_bytes,operation,sample,iterations,seconds,mib_per_second"
    );
    for (codec, name) in [
        "lznt1",
        "xpress-plain",
        "xpress-huffman",
        "lzx",
        "lzms",
        "cab-lzx",
        "quantum",
        "lzma",
        "lzma2",
        "deflate",
        "zlib",
        "gzip",
    ]
    .iter()
    .enumerate()
    {
        for size in [4096, 32768] {
            for dataset in ["repeat", "pattern", "random"] {
                let mut seed = 0x12345678u32;
                let input: Vec<u8> = (0..size)
                    .map(|i| match dataset {
                        "repeat" => b'A',
                        "pattern" => (i % 251) as u8,
                        _ => {
                            seed ^= seed << 13;
                            seed ^= seed >> 17;
                            seed ^= seed << 5;
                            seed as u8
                        }
                    })
                    .collect();
                let encoded = encode(codec, &input)?;
                let mut output = vec![0; size];
                // Validate every workload and warm the code before timing.
                decode(codec, &encoded, &mut output)?;
                assert_eq!(output, input);
                for sample in 1..=3 {
                    let (iterations, elapsed) = measure(
                        || {
                            black_box(encode(codec, black_box(&input)).expect("validated encoder"));
                        },
                        budget,
                    );
                    report(
                        name,
                        dataset,
                        size,
                        encoded.len(),
                        "compress",
                        sample,
                        iterations,
                        elapsed,
                    );
                    let (iterations, elapsed) = measure(
                        || {
                            decode(codec, black_box(&encoded), black_box(&mut output))
                                .expect("validated decoder");
                            black_box(&output);
                        },
                        budget,
                    );
                    report(
                        name,
                        dataset,
                        size,
                        encoded.len(),
                        "decompress",
                        sample,
                        iterations,
                        elapsed,
                    );
                }
            }
        }
    }
    // Independent reference fixture exercises framed LZX Delta decoding.
    let encoded = include_bytes!("../tests/fixtures/lzxd/extended-32768.lzxd");
    let mut output = vec![0; 32768];
    ms_compress::lzx::decompress_lzxd(encoded, b"Z", &mut output, 17)?;
    for sample in 1..=3 {
        let (iterations, elapsed) = measure(
            || {
                ms_compress::lzx::decompress_lzxd(
                    black_box(encoded),
                    b"Z",
                    black_box(&mut output),
                    17,
                )
                .expect("validated fixture");
            },
            budget,
        );
        report(
            "lzxd",
            "fixture",
            output.len(),
            encoded.len(),
            "decompress",
            sample,
            iterations,
            elapsed,
        );
    }
    Ok(())
}

#[expect(
    clippy::too_many_arguments,
    reason = "one CSV row with explicit measurement fields"
)]
fn report(
    codec: &str,
    dataset: &str,
    size: usize,
    compressed: usize,
    operation: &str,
    sample: usize,
    iterations: u64,
    elapsed: Duration,
) {
    let seconds = elapsed.as_secs_f64();
    let throughput = size as f64 * iterations as f64 / seconds / 1048576.0;
    println!(
        "{codec},{dataset},{size},{compressed},{operation},{sample},{iterations},{seconds:.6},{throughput:.3}"
    );
}
