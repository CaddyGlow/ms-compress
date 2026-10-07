//! Bounded codec fuzz harnesses shared by honggfuzz and corpus replay.
use ms_compress::context::{Decompressor, FixedStrategyCompressor};
/// Maximum encoded input bytes per iteration.
pub const ARCHIVE_LIMIT: usize = 1 << 20;
/// Maximum expanded block size.
pub const BLOCK_LIMIT: usize = 32768;

/// Input: codec byte, LE output length (modulo 32769), compressed bytes.
/// Selectors 0..2 are WIM XPRESS/LZX/LZMS; 3..4 are CAB LZX/Quantum.
/// Selectors 5..7 are LZNT1, plain XPRESS, and LZX Delta.
/// Selectors 8..10 are raw DEFLATE, zlib, and gzip.
/// Selector 11 is .lzma; 12 is raw LZMA2 prefixed by LE u32 dictionary size.
/// CAB decoders retain history across two frames split halfway through payload.
pub fn decompress(data: &[u8]) {
    if data.len() < 3 || data.len() > ARCHIVE_LIMIT {
        return;
    }
    let codec = data[0] % 13;
    let length = u16::from_le_bytes([data[1], data[2]]) as usize % (BLOCK_LIMIT + 1);
    let input = &data[3..];
    let mut output = vec![0; length];
    if codec == 11 {
        let _ = ms_compress::lzma::decompress(input, &mut output, 1024);
        let mut stream = ms_compress::lzma::LzmaStream::new_mem_limit(1024, None);
        let _ = stream.process(input, &mut output, ms_compress::lzma::Action::Finish);
    } else if codec == 12 {
        if let Some(dictionary) = input.get(..4) {
            let dictionary = u32::from_le_bytes(dictionary.try_into().unwrap());
            let payload = &input[4..];
            let _ = ms_compress::lzma::decompress_lzma2(payload, &mut output, dictionary, 1024);
            let mut stream = ms_compress::lzma::Lzma2Stream::new_mem_limit(dictionary, 1024);
            let _ = stream.process(payload, &mut output, ms_compress::lzma::Action::Finish);
            // Bound threaded work units as well as caller output. Larger inputs
            // retain single-threaded coverage without spawning worker threads.
            if payload.len() <= 65536
                && let Ok(mut reader) = ms_compress::lzma::Lzma2ReaderMt::new_bounded(
                    payload, dictionary, 2, 1024, 65536,
                )
            {
                let mut produced = 0;
                while produced < output.len() {
                    match ms_compress::lzma::Read::read(&mut reader, &mut output[produced..]) {
                        Ok(0) | Err(_) => break,
                        Ok(count) => produced += count,
                    }
                }
                let _ = ms_compress::lzma::Read::read(&mut reader, &mut [0]);
            }
        }
    } else if codec >= 8 {
        let window_bits = match codec {
            8 => -15,
            9 => 15,
            _ => 31,
        };
        let config = ms_compress::zlib::InflateConfig { window_bits };
        let _ = ms_compress::zlib::decompress_slice(&mut output, input, config);
        // Exercise state after malformed/truncated input, then reset it and
        // compare with a fresh stream. All output remains bounded by length.
        let mut decoder =
            ms_compress::zlib::Inflate::new(codec != 8, window_bits.unsigned_abs() as u8);
        let _ = decoder.decompress(input, &mut output, ms_compress::zlib::InflateFlush::Finish);
        decoder.reset(codec != 8);
        // reset selects raw/zlib; preserve gzip configuration with a new stream.
        if codec == 10 {
            decoder = ms_compress::zlib::Inflate::new(true, 31);
        }
        output.fill(0);
        let mut fresh =
            ms_compress::zlib::Inflate::new(codec != 8, window_bits.unsigned_abs() as u8);
        let mut expected = vec![0; length];
        let actual =
            decoder.decompress(input, &mut output, ms_compress::zlib::InflateFlush::Finish);
        let result = fresh.decompress(
            input,
            &mut expected,
            ms_compress::zlib::InflateFlush::Finish,
        );
        assert_eq!(actual, result);
        assert_eq!(decoder.total_in(), fresh.total_in());
        assert_eq!(decoder.total_out(), fresh.total_out());
        assert_eq!(
            &output[..decoder.total_out() as usize],
            &expected[..fresh.total_out() as usize]
        );
    } else if codec == 5 || codec == 6 {
        let _ = if codec == 5 {
            ms_compress::lznt1::decompress(input, &mut output)
        } else {
            ms_compress::xpress_plain::decompress(input, &mut output)
        };
    } else if codec == 7 {
        // Bound the reference independently from the compressed payload.
        let reference_size = input.len().min(256);
        let _ = ms_compress::lzx::decompress_lzxd(
            &input[reference_size..],
            &input[..reference_size],
            &mut output,
            17,
        );
    } else if codec < 3 {
        let mut decoder =
            Decompressor::new(i32::from(codec) + 1, BLOCK_LIMIT).expect("valid decoder");
        let _ = decoder.decompress(input, &mut output);
        // Independent WIM blocks must reset state, even after a failed decode.
        let mut fresh =
            Decompressor::new(i32::from(codec) + 1, BLOCK_LIMIT).expect("valid decoder");
        output.fill(0);
        let reused_result = decoder.decompress(input, &mut output);
        let mut expected = vec![0; length];
        let fresh_result = fresh.decompress(input, &mut expected);
        assert_eq!(reused_result, fresh_result);
        if reused_result.is_ok() {
            assert_eq!(output, expected);
        }
    } else {
        let split = input.len() / 2;
        if codec == 3 {
            let mut decoder =
                ms_compress::lzx::CabinetLzxDecoder::new(15 + data[0] % 7).expect("valid window");
            let _ = decoder.decompress_frame(input, &mut output);
            let mut decoder =
                ms_compress::lzx::CabinetLzxDecoder::new(15 + data[0] % 7).expect("valid window");
            if decoder
                .decompress_frame(&input[..split], &mut output)
                .is_ok()
            {
                let _ = decoder.decompress_frame(&input[split..], &mut output);
            }
        } else {
            let mut decoder =
                ms_compress::quantum::QuantumDecoder::new(10 + data[0] % 12).expect("valid window");
            let _ = decoder.decompress_frame(input, &mut output);
            let mut decoder =
                ms_compress::quantum::QuantumDecoder::new(10 + data[0] % 12).expect("valid window");
            if decoder
                .decompress_frame(&input[..split], &mut output)
                .is_ok()
            {
                let _ = decoder.decompress_frame(&input[split..], &mut output);
            }
        }
    }
}

/// Input: codec selector (modulo 12), LE output-capacity selector, plaintext.
/// Selectors 0..2 are WIM codecs, 3..4 NT buffers, 5..6 CAB LZX/Quantum.
/// Selectors 7..9 are raw DEFLATE, zlib, and gzip; 10..11 are LZMA1/LZMA2.
/// Check every successful encoding and reuse contexts for a second block.
pub fn roundtrip(data: &[u8]) {
    if data.len() < 3 || data.len() > BLOCK_LIMIT + 3 {
        return;
    }
    let selector = data[0] % 12;
    if selector >= 10 {
        lzma_roundtrip(selector, data);
        return;
    }
    if selector >= 7 {
        zlib_roundtrip(selector, data);
        return;
    }
    if selector >= 3 {
        let plaintext = &data[3..];
        if selector == 3 || selector == 4 {
            for input in [plaintext, &plaintext[..plaintext.len() / 2]] {
                let encoded = if selector == 3 {
                    ms_compress::lznt1::compress(input)
                } else {
                    ms_compress::xpress_plain::compress(input)
                }
                .expect("bounded compression");
                let mut output = vec![0; input.len()];
                let size = if selector == 3 {
                    ms_compress::lznt1::decompress(&encoded, &mut output)
                } else {
                    ms_compress::xpress_plain::decompress(&encoded, &mut output)
                }
                .expect("invalid encoded buffer");
                assert_eq!(size, input.len());
                assert_eq!(output, input);
            }
        } else if !plaintext.is_empty() {
            // CAB codecs preserve history across frames. Unequal consecutive
            // frames exercise state and position rather than independent blocks.
            if selector == 5 {
                let mut encoder = ms_compress::lzx_encode::CabinetLzxEncoder::new(15).unwrap();
                let mut decoder = ms_compress::lzx::CabinetLzxDecoder::new(15).unwrap();
                for input in [plaintext, &plaintext[..plaintext.len().div_ceil(2)]] {
                    let encoded = encoder.compress_frame(input).unwrap();
                    let mut output = vec![0; input.len()];
                    decoder.decompress_frame(encoded, &mut output).unwrap();
                    assert_eq!(output, input);
                }
            } else {
                let mut encoder = ms_compress::quantum::QuantumEncoder::new(15, 4).unwrap();
                let mut decoder = ms_compress::quantum::QuantumDecoder::new(15).unwrap();
                for input in [plaintext, &plaintext[..plaintext.len().div_ceil(2)]] {
                    let encoded = encoder.compress_frame(input).unwrap();
                    let mut output = vec![0; input.len()];
                    decoder.decompress_frame(&encoded, &mut output).unwrap();
                    assert_eq!(output, input);
                }
            }
        }
        return;
    }
    let codec = i32::from(selector) + 1;
    let plaintext = &data[3..];
    let capacity = u16::from_le_bytes([data[1], data[2]]) as usize;
    let mut compressor =
        FixedStrategyCompressor::new(codec, BLOCK_LIMIT).expect("valid compressor");
    let mut decoder = Decompressor::new(codec, BLOCK_LIMIT).expect("valid decoder");
    for input in [plaintext, &plaintext[..plaintext.len() / 2]] {
        // Cover both capacity failures and successful encodings.
        for limit in [capacity, input.len() * 2 + 4096] {
            if let Some(encoded) = compressor
                .compress(input, limit)
                .expect("bounded compression")
            {
                assert!(encoded.len() <= limit);
                let mut decoded = vec![0; input.len()];
                decoder
                    .decompress(&encoded, &mut decoded)
                    .expect("encoder produced invalid block");
                assert_eq!(decoded, input);
            }
        }
    }
}

fn zlib_roundtrip(selector: u8, data: &[u8]) {
    use ms_compress::zlib::{
        Deflate, DeflateConfig, DeflateFlush, Inflate, InflateConfig, InflateFlush, ReturnCode,
        Status, Strategy,
    };
    let window_bits = match selector {
        7 => -15,
        8 => 15,
        _ => 31,
    };
    let config = DeflateConfig {
        window_bits,
        level: i32::from(data[1] % 10),
        strategy: match data[2] % 5 {
            0 => Strategy::Default,
            1 => Strategy::Filtered,
            2 => Strategy::HuffmanOnly,
            3 => Strategy::Rle,
            _ => Strategy::Fixed,
        },
        ..DeflateConfig::default()
    };
    let plaintext = &data[3..];
    let capacity = u16::from_le_bytes([data[1], data[2]]) as usize;
    let mut encoder = Deflate::new_with_config(config);
    let mut decoder = Inflate::new(selector != 7, window_bits.unsigned_abs() as u8);
    for input in [plaintext, &plaintext[..plaintext.len() / 2]] {
        // A deliberately restricted output exercises BufError/partial output.
        let mut restricted = vec![0; capacity.min(input.len() * 2 + 128)];
        let (encoded, result) = ms_compress::zlib::compress_slice(&mut restricted, input, config);
        if result == ReturnCode::Ok {
            let mut output = vec![0; input.len()];
            let (decoded, result) = ms_compress::zlib::decompress_slice(
                &mut output,
                encoded,
                InflateConfig { window_bits },
            );
            assert_eq!(result, ReturnCode::Ok);
            assert_eq!(decoded, input);
        }

        encoder.reset();
        decoder.reset(selector != 7);
        if selector == 9 {
            decoder = Inflate::new(true, 31);
        }
        let mut encoded = vec![0; input.len() * 2 + 128];
        assert_eq!(
            encoder.compress(input, &mut encoded, DeflateFlush::Finish),
            Ok(Status::StreamEnd)
        );
        assert_eq!(encoder.total_in(), input.len() as u64);
        encoded.truncate(encoder.total_out() as usize);
        let mut output = vec![0; input.len() + 1];
        assert_eq!(
            decoder.decompress(&encoded, &mut output, InflateFlush::Finish),
            Ok(Status::StreamEnd)
        );
        assert_eq!(decoder.total_out(), input.len() as u64);
        assert_eq!(&output[..input.len()], input);
    }
}

/// Run a codec harness by name.
pub fn run(target: &str, data: &[u8]) -> Result<(), &'static str> {
    match target {
        "decompress" => decompress(data),
        "roundtrip" => roundtrip(data),
        _ => return Err("target must be decompress or roundtrip"),
    }
    Ok(())
}

fn lzma_roundtrip(selector: u8, data: &[u8]) {
    let preset = u32::from(data[1] % 10);
    for input in [&data[3..], &data[3..3 + (data.len() - 3) / 2]] {
        let mut output = vec![0; input.len()];
        if selector == 10 {
            let encoded = ms_compress::lzma::compress(input, preset).unwrap();
            let count = ms_compress::lzma::decompress(&encoded, &mut output, 1024).unwrap();
            assert_eq!(count, input.len());
            assert_eq!(output, input);
        } else {
            let (encoded, dictionary) = ms_compress::lzma::compress_lzma2(input, preset).unwrap();
            let count =
                ms_compress::lzma::decompress_lzma2(&encoded, &mut output, dictionary, 1024)
                    .unwrap();
            assert_eq!(count, input.len());
            assert_eq!(output, input);
            let mut reader = ms_compress::lzma::Lzma2ReaderMt::new_bounded(
                &encoded[..],
                dictionary,
                2,
                1024,
                65536,
            )
            .unwrap();
            ms_compress::lzma::Read::read_exact(&mut reader, &mut output).unwrap();
            assert_eq!(output, input);
            assert_eq!(
                ms_compress::lzma::Read::read(&mut reader, &mut [0]).unwrap(),
                0
            );
        }
    }
}
