use ms_compress_fuzz::{decompress, roundtrip};

#[test]
fn malformed_inputs_and_zero_output_are_accepted_without_panics() {
    for input in [&[][..], &[0][..], &[0xff; 256][..]] {
        decompress(input);
        roundtrip(input);
    }
    for codec in 0..13 {
        decompress(&[codec, 0, 0]);
        decompress(&[codec, 255, 127, 255, 255, 255, 255]);
    }
}

#[test]
fn all_encoders_roundtrip_repeated_and_varied_blocks_with_context_reuse() {
    for codec in 0..12 {
        for size in [0, 1, 256, 4096, 32768] {
            let mut input = vec![codec, 0, 0];
            input.extend(std::iter::repeat_n(b'A', size));
            roundtrip(&input);
            for (i, byte) in input[3..].iter_mut().enumerate() {
                *byte = (i % 251) as u8;
            }
            roundtrip(&input);
        }
    }
}

#[test]
fn zlib_all_levels_and_strategies_roundtrip_and_decode_generated_seeds() {
    for format in 0..3 {
        for level in 0..10 {
            for strategy in 0..5 {
                let mut input = vec![7 + format, level, strategy];
                input.extend((0..1024).map(|i| (i % 251) as u8));
                roundtrip(&input);
            }
        }
        let plaintext = b"a bounded zlib fuzz seed";
        let mut buffer = vec![0; 128];
        let window_bits = [-15, 15, 31][format as usize];
        let (encoded, result) = ms_compress::zlib::compress_slice(
            &mut buffer,
            plaintext,
            ms_compress::zlib::DeflateConfig {
                window_bits,
                ..ms_compress::zlib::DeflateConfig::default()
            },
        );
        assert_eq!(result, ms_compress::zlib::ReturnCode::Ok);
        let mut seed = vec![8 + format];
        seed.extend_from_slice(&(plaintext.len() as u16).to_le_bytes());
        seed.extend_from_slice(encoded);
        decompress(&seed);
        for length in 3..seed.len() {
            decompress(&seed[..length]);
        }
    }
}

#[test]
fn lzma_presets_and_truncated_fixtures_exercise_all_decoder_paths() {
    for selector in [10, 11] {
        for preset in 0..10 {
            let mut input = vec![selector, preset, 0];
            input.extend((0..1024).map(|i| (i % 251) as u8));
            roundtrip(&input);
        }
    }
    for (selector, encoded) in [
        (
            11,
            include_bytes!("../../tests/fixtures/lzma/hello.lzma").as_slice(),
        ),
        (
            12,
            include_bytes!("../../tests/fixtures/lzma/hello.lzma2").as_slice(),
        ),
    ] {
        let mut seed = vec![selector, 13, 0];
        if selector == 12 {
            seed.extend_from_slice(&4096u32.to_le_bytes());
        }
        seed.extend_from_slice(encoded);
        for cut in 3..=seed.len() {
            decompress(&seed[..cut]);
        }
    }
}

#[test]
fn lzma2_independent_blocks_exercise_threaded_fuzz_decoding() {
    let mut encoded = Vec::new();
    for byte in b"ABCD" {
        let (block, dictionary) = ms_compress::lzma::compress_lzma2(&vec![*byte; 4096], 1).unwrap();
        assert_eq!(dictionary, 4096);
        encoded.extend_from_slice(&block[..block.len() - 1]);
    }
    encoded.push(0);
    let mut seed = vec![12];
    seed.extend_from_slice(&16384u16.to_le_bytes());
    seed.extend_from_slice(&4096u32.to_le_bytes());
    seed.extend_from_slice(&encoded);
    decompress(&seed);
    for cut in [7, seed.len() / 2, seed.len() - 1] {
        decompress(&seed[..cut]);
    }
}
