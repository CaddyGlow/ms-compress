use ms_compress::zlib::{
    Deflate, DeflateConfig, DeflateFlush, Inflate, InflateConfig, InflateError, InflateFlush,
    ReturnCode, Status,
};

// From upstream zlib-rs's deflate::test::hello_world_quick at the pinned commit.
const PLAINTEXT: &[u8] = b"Hello World!\n";
const UPSTREAM_ZLIB: &[u8] = &[
    0x78, 0x01, 0xf3, 0x48, 0xcd, 0xc9, 0xc9, 0x57, 0x08, 0xcf, 0x2f, 0xca, 0x49, 0x51, 0xe4, 0x02,
    0x00, 0x20, 0x91, 0x04, 0x48,
];

#[test]
fn upstream_hello_world_vector_decodes_through_public_api() {
    let mut output = [0; 13];
    let (decoded, result) =
        ms_compress::zlib::decompress_slice(&mut output, UPSTREAM_ZLIB, InflateConfig::default());
    assert_eq!(result, ReturnCode::Ok);
    assert_eq!(decoded, PLAINTEXT);
    let mut encoded = [0; 128];
    let (encoded, result) =
        ms_compress::zlib::compress_slice(&mut encoded, PLAINTEXT, DeflateConfig::best_speed());
    assert_eq!(result, ReturnCode::Ok);
    assert_eq!(encoded, UPSTREAM_ZLIB);
}

#[test]
fn independent_raw_zlib_and_gzip_fixtures_decode() {
    for (input, window_bits) in [
        (&UPSTREAM_ZLIB[2..UPSTREAM_ZLIB.len() - 4], -15),
        (UPSTREAM_ZLIB, 15),
        (&include_bytes!("fixtures/zlib/hello-world.gz")[..], 31),
    ] {
        let mut output = [0; 13];
        let (decoded, result) =
            ms_compress::zlib::decompress_slice(&mut output, input, InflateConfig { window_bits });
        assert_eq!(result, ReturnCode::Ok);
        assert_eq!(decoded, PLAINTEXT);
    }
}

#[test]
fn truncated_stream_small_output_and_bad_checksum_are_rejected() {
    let mut output = [0; 13];
    for end in 0..UPSTREAM_ZLIB.len() {
        let (_, result) = ms_compress::zlib::decompress_slice(
            &mut output,
            &UPSTREAM_ZLIB[..end],
            InflateConfig::default(),
        );
        assert_ne!(result, ReturnCode::Ok, "truncated at {end}");
    }
    let (_, result) = ms_compress::zlib::decompress_slice(
        &mut output[..12],
        UPSTREAM_ZLIB,
        InflateConfig::default(),
    );
    assert_eq!(result, ReturnCode::BufError);
    let mut corrupt = UPSTREAM_ZLIB.to_vec();
    *corrupt.last_mut().unwrap() ^= 1;
    let (_, result) =
        ms_compress::zlib::decompress_slice(&mut output, &corrupt, InflateConfig::default());
    assert_eq!(result, ReturnCode::DataError);
}

#[test]
fn streaming_compression_and_reset_preserve_raw_zlib_and_gzip_formats() {
    for window_bits in [-15i32, 15, 31] {
        let mut encoder = Deflate::new_with_config(DeflateConfig {
            window_bits,
            ..DeflateConfig::default()
        });
        for input in [PLAINTEXT, &b"a different second stream"[..]] {
            encoder.reset();
            let mut encoded = Vec::new();
            for _ in 0..1024 {
                let before = encoder.total_in();
                let mut chunk = [0; 7];
                let previous_output = encoder.total_out();
                let status = encoder
                    .compress(&input[before as usize..], &mut chunk, DeflateFlush::Finish)
                    .unwrap();
                let produced = (encoder.total_out() - previous_output) as usize;
                encoded.extend_from_slice(&chunk[..produced]);
                if status == Status::StreamEnd {
                    break;
                }
                assert!(encoder.total_in() > before || produced > 0);
            }
            assert_eq!(encoder.total_in(), input.len() as u64);
            let mut decoder = Inflate::new(window_bits > 0, window_bits.unsigned_abs() as u8);
            let mut decoded = Vec::new();
            let mut ended = false;
            for _ in 0..1024 {
                let before = decoder.total_in();
                let previous_output = decoder.total_out();
                let mut chunk = [0; 2];
                let status = decoder
                    .decompress(
                        &encoded[before as usize..],
                        &mut chunk,
                        InflateFlush::Finish,
                    )
                    .unwrap();
                let produced = (decoder.total_out() - previous_output) as usize;
                decoded.extend_from_slice(&chunk[..produced]);
                if status == Status::StreamEnd {
                    ended = true;
                    break;
                }
                assert!(decoder.total_in() > before || produced > 0);
            }
            assert!(ended);
            assert_eq!(decoded, input);
        }
    }
}

#[test]
fn dictionary_identity_and_decoder_reset_follow_the_public_contract() {
    let dictionary = b"a shared dictionary with repeated words";
    let plaintext = b"repeated words from a shared dictionary";
    let mut encoder = Deflate::new(6, true, 15);
    let dictionary_id = encoder.set_dictionary(dictionary).unwrap();
    let mut encoded = [0; 256];
    assert_eq!(
        encoder.compress(plaintext, &mut encoded, DeflateFlush::Finish),
        Ok(Status::StreamEnd)
    );
    let encoded = &encoded[..encoder.total_out() as usize];
    let mut decoder = Inflate::new(true, 15);
    let mut output = [0; 128];
    assert_eq!(
        decoder.decompress(encoded, &mut output, InflateFlush::Finish),
        Err(InflateError::NeedDict {
            dict_id: dictionary_id
        })
    );
    assert_eq!(decoder.set_dictionary(dictionary).unwrap(), dictionary_id);
    let consumed = decoder.total_in() as usize;
    assert_eq!(
        decoder.decompress(&encoded[consumed..], &mut output, InflateFlush::Finish),
        Ok(Status::StreamEnd)
    );
    assert_eq!(&output[..decoder.total_out() as usize], plaintext);
    decoder.reset(true);
    assert_eq!(
        decoder.decompress(UPSTREAM_ZLIB, &mut output, InflateFlush::Finish),
        Ok(Status::StreamEnd)
    );
    assert_eq!(&output[..decoder.total_out() as usize], PLAINTEXT);
}
