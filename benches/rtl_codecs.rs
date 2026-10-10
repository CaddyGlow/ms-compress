//! Native Windows RTL compression and same-buffer decompression comparison.
#[cfg(not(windows))]
fn main() {
    eprintln!("rtl_codecs requires native Windows ntdll");
}

#[cfg(windows)]
fn main() -> Result<(), Box<dyn std::error::Error>> {
    windows::run()
}

#[cfg(windows)]
mod windows {
    use std::{
        error::Error,
        hint::black_box,
        time::{Duration, Instant},
    };
    type Result<T> = std::result::Result<T, Box<dyn Error>>;

    #[link(name = "ntdll")]
    unsafe extern "system" {
        fn RtlGetCompressionWorkSpaceSize(
            format: u16,
            compress: *mut u32,
            decompress: *mut u32,
        ) -> i32;
        fn RtlCompressBuffer(
            format: u16,
            input: *const u8,
            input_size: u32,
            output: *mut u8,
            output_size: u32,
            chunk_size: u32,
            final_size: *mut u32,
            workspace: *mut u8,
        ) -> i32;
        fn RtlDecompressBufferEx(
            format: u16,
            output: *mut u8,
            output_size: u32,
            input: *const u8,
            input_size: u32,
            final_size: *mut u32,
            workspace: *mut u8,
        ) -> i32;
    }

    fn success(status: i32) -> Result<()> {
        if status != 0 {
            return Err(format!("ntdll returned NTSTATUS {:#010x}", status as u32).into());
        }
        Ok(())
    }

    struct Native {
        format: u16,
        compression: Vec<u64>,
        decompression: Vec<u64>,
    }
    impl Native {
        fn new(format: u16) -> Result<Self> {
            let mut compression = 0;
            let mut decompression = 0;
            // SAFETY: Both pointers refer to live u32 values.
            success(unsafe {
                RtlGetCompressionWorkSpaceSize(format, &mut compression, &mut decompression)
            })?;
            Ok(Self {
                format,
                compression: vec![0; (compression as usize).div_ceil(8)],
                decompression: vec![0; (decompression as usize).div_ceil(8)],
            })
        }
        fn compress(&mut self, input: &[u8], output: &mut [u8]) -> Result<Option<usize>> {
            let mut written = 0;
            // SAFETY: Buffers are disjoint, workload lengths fit u32, and the
            // aligned workspace has the size requested by ntdll.
            let status = unsafe {
                RtlCompressBuffer(
                    self.format,
                    input.as_ptr(),
                    input.len() as u32,
                    output.as_mut_ptr(),
                    output.len() as u32,
                    4096,
                    &mut written,
                    self.compression.as_mut_ptr().cast(),
                )
            };
            // Plain XPRESS can refuse inputs that expand. Never report this
            // failed operation as successful compression throughput.
            if status == 0xc0000023u32 as i32 {
                return Ok(None);
            }
            success(status)?;
            Ok(Some(written as usize))
        }
        fn decompress(&mut self, input: &[u8], output: &mut [u8]) -> Result<()> {
            let mut written = 0;
            // SAFETY: Same live-buffer, u32-length and workspace guarantees.
            success(unsafe {
                RtlDecompressBufferEx(
                    self.format,
                    output.as_mut_ptr(),
                    output.len() as u32,
                    input.as_ptr(),
                    input.len() as u32,
                    &mut written,
                    self.decompression.as_mut_ptr().cast(),
                )
            })?;
            assert_eq!(written as usize, output.len());
            Ok(())
        }
    }

    fn rust_compress(format: u16, input: &[u8]) -> Result<Vec<u8>> {
        Ok(match format {
            2 => ms_compress::lznt1::compress(input)?,
            3 => ms_compress::xpress_plain::compress(input)?,
            _ => ms_compress::xpress_encode::compress_xpress(input, input.len() * 2 + 4096)?
                .ok_or("unencodable Rust input")?,
        })
    }
    fn rust_decompress(format: u16, input: &[u8], output: &mut [u8]) -> Result<()> {
        let written = match format {
            2 => ms_compress::lznt1::decompress(input, output)?,
            3 => ms_compress::xpress_plain::decompress(input, output)?,
            _ => ms_compress::decompress_xpress_huffman_buffer(input, output)?,
        };
        assert_eq!(written, output.len());
        Ok(())
    }
    fn measure(mut operation: impl FnMut() -> Result<()>, budget: Duration) -> Result<(u64, f64)> {
        let start = Instant::now();
        let mut iterations = 0;
        loop {
            operation()?;
            iterations += 1;
            if start.elapsed() >= budget {
                return Ok((iterations, start.elapsed().as_secs_f64()));
            }
        }
    }

    pub fn run() -> Result<()> {
        let milliseconds: u64 = std::env::var("MS_COMPRESS_BENCH_MS")
            .unwrap_or_else(|_| "100".into())
            .parse()?;
        if milliseconds == 0 {
            return Err("MS_COMPRESS_BENCH_MS must be positive".into());
        }
        let formats: Vec<u16> = match std::env::var("MS_COMPRESS_RTL_FORMAT").as_deref() {
            Ok("2") => vec![2],
            Ok("3") => vec![3],
            Ok("4") => vec![4],
            Ok(_) => return Err("MS_COMPRESS_RTL_FORMAT must be 2, 3, or 4".into()),
            Err(std::env::VarError::NotPresent) => vec![2, 3, 4],
            Err(error) => return Err(error.to_string().into()),
        };
        let budget = Duration::from_millis(milliseconds);
        println!(
            "format,implementation,dataset,input_bytes,compressed_bytes,operation,source,sample,iterations,seconds,mib_per_second"
        );
        for format in formats {
            let mut native = Native::new(format)?;
            for size in [4096, 32768, 65536, 131072, 1048576] {
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
                    let mut native_output = vec![0; size * 2 + 65536];
                    let native_size = native.compress(&input, &mut native_output)?;
                    let rust_encoded = if format != 4 || size <= 65536 {
                        Some(rust_compress(format, &input)?)
                    } else {
                        None
                    };
                    let mut output = vec![0; size];
                    // Verify both encoders with both decoders before timing.
                    let mut streams = Vec::new();
                    if let Some(length) = native_size {
                        streams.push(("rtl", &native_output[..length]));
                    } else {
                        eprintln!(
                            "skip RTL compression: format={format} dataset={dataset} size={size}: input expands"
                        );
                    }
                    if let Some(encoded) = &rust_encoded {
                        streams.push(("rust", encoded.as_slice()));
                    }
                    for &(source, encoded) in &streams {
                        output.fill(0);
                        native.decompress(encoded, &mut output)?;
                        assert!(
                            output == input,
                            "RTL mismatch: format={format} {source}/{dataset}/{size}"
                        );
                        output.fill(0);
                        rust_decompress(format, encoded, &mut output)?;
                        assert!(
                            output == input,
                            "Rust mismatch: format={format} {source}/{dataset}/{size}"
                        );
                    }
                    for sample in 1..=5 {
                        let order = if sample % 2 == 1 {
                            ["rust", "rtl"]
                        } else {
                            ["rtl", "rust"]
                        };
                        // Compression is one-shot for both implementations:
                        // allocation and workspace construction are included.
                        for implementation in order {
                            let compressed = if implementation == "rust" {
                                rust_encoded.as_ref().map(Vec::len)
                            } else {
                                native_size
                            };
                            if let Some(compressed) = compressed {
                                let (iterations, seconds) = measure(
                                    || {
                                        if implementation == "rust" {
                                            black_box(rust_compress(format, black_box(&input))?);
                                        } else {
                                            let mut compressor = Native::new(format)?;
                                            let mut encoded = vec![0; size * 2 + 65536];
                                            black_box(
                                                compressor
                                                    .compress(black_box(&input), &mut encoded)?,
                                            );
                                            black_box(encoded);
                                        }
                                        Ok(())
                                    },
                                    budget,
                                )?;
                                let mib = size as f64 * iterations as f64 / seconds / 1048576.0;
                                println!(
                                    "{format},{implementation},{dataset},{size},{compressed},compress,{implementation},{sample},{iterations},{seconds:.6},{mib:.3}"
                                );
                            }
                        }
                        // Both decoders consume identical streams; retained
                        // output buffers and native workspace are outside timing.
                        for &(source, encoded) in &streams {
                            for implementation in order {
                                let (iterations, seconds) = measure(
                                    || {
                                        if implementation == "rust" {
                                            rust_decompress(
                                                format,
                                                black_box(encoded),
                                                black_box(&mut output),
                                            )?;
                                        } else {
                                            native.decompress(
                                                black_box(encoded),
                                                black_box(&mut output),
                                            )?;
                                        }
                                        black_box(&output);
                                        Ok(())
                                    },
                                    budget,
                                )?;
                                let mib = size as f64 * iterations as f64 / seconds / 1048576.0;
                                println!(
                                    "{format},{implementation},{dataset},{size},{},decompress,{source},{sample},{iterations},{seconds:.6},{mib:.3}",
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
}
