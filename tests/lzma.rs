use ms_compress::lzma::{self, Action, LzmaReader, LzmaStream, Read, Status};

const PLAIN: &[u8] = b"Hello, world!";
const LZMA: &[u8] = include_bytes!("fixtures/lzma/hello.lzma");
const LZMA2: &[u8] = include_bytes!("fixtures/lzma/hello.lzma2");

#[test]
fn independent_liblzma_fixtures_decode() {
    let mut output = [0; 13];
    assert_eq!(lzma::decompress(LZMA, &mut output, 1024).unwrap(), 13);
    assert_eq!(&output, PLAIN);
    assert_eq!(
        lzma::decompress_lzma2(LZMA2, &mut output, 4096, 1024).unwrap(),
        13
    );
    assert_eq!(&output, PLAIN);
}

#[test]
fn original_raw_lzma_vector_decodes() {
    // lzma-rust2's LzmaCore test vector: lc=3, lp=0, pb=2, EOS marker.
    let raw = [
        0, 36, 25, 73, 152, 111, 22, 2, 140, 232, 230, 91, 177, 71, 198, 206, 183, 99, 255, 255,
        60, 172, 0, 0,
    ];
    let mut reader = LzmaReader::new(&raw[..], u64::MAX, 3, 0, 2, 4096, None).unwrap();
    let mut output = [0; 13];
    reader.read_exact(&mut output).unwrap();
    assert_eq!(&output, PLAIN);
    assert_eq!(reader.read(&mut [0]).unwrap(), 0);
}

#[test]
fn all_presets_roundtrip_empty_repeated_and_varied_inputs() {
    for preset in 0..10 {
        for size in [0, 1, 273, 4096, 32768] {
            for varied in [false, true] {
                let input: Vec<u8> = (0..size)
                    .map(|i| if varied { (i % 251) as u8 } else { b'A' })
                    .collect();
                let mut output = vec![0; size];
                let encoded = lzma::compress(&input, preset).unwrap();
                assert_eq!(lzma::decompress(&encoded, &mut output, 1024).unwrap(), size);
                assert_eq!(output, input);
                let (encoded, dictionary) = lzma::compress_lzma2(&input, preset).unwrap();
                assert_eq!(
                    lzma::decompress_lzma2(&encoded, &mut output, dictionary, 1024).unwrap(),
                    size
                );
                assert_eq!(output, input);
            }
        }
    }
}

#[test]
fn bounds_and_invalid_options_are_rejected() {
    assert!(lzma::compress(PLAIN, 10).is_err());
    assert!(lzma::compress_lzma2(PLAIN, u32::MAX).is_err());
    assert!(lzma::decompress(LZMA, &mut [0; 12], 1024).is_err());
    assert!(lzma::decompress_lzma2(LZMA2, &mut [0; 12], 4096, 1024).is_err());
    assert!(lzma::decompress(LZMA, &mut [0; 13], 0).is_err());
    assert!(lzma::decompress_lzma2(LZMA2, &mut [0; 13], u32::MAX, 1024).is_err());
    assert!(lzma::decompress_lzma2(LZMA2, &mut [0; 13], 0, 1024).is_err());
    let mut invalid = LZMA.to_vec();
    invalid[0] = 255;
    assert!(lzma::decompress(&invalid, &mut [0; 13], 1024).is_err());
}

#[test]
fn every_truncated_fixture_is_rejected() {
    for cut in 0..LZMA.len() {
        assert!(
            lzma::decompress(&LZMA[..cut], &mut [0; 13], 1024).is_err(),
            "LZMA cut {cut}"
        );
    }
    for cut in 0..LZMA2.len() {
        assert!(
            lzma::decompress_lzma2(&LZMA2[..cut], &mut [0; 13], 4096, 1024).is_err(),
            "LZMA2 cut {cut}"
        );
    }
}

#[test]
fn streaming_preserves_trailing_input_with_single_byte_chunks() {
    for chunk in [1, 7, 4096] {
        let mut stream = LzmaStream::new_mem_limit(1024, None);
        let mut input = LZMA.to_vec();
        input.extend_from_slice(b"tail");
        let mut consumed = 0;
        let mut output = Vec::new();
        loop {
            let end = (consumed + chunk).min(input.len());
            let mut buffer = [0; 3];
            let result = stream
                .process(
                    &input[consumed..end],
                    &mut buffer,
                    if end == input.len() {
                        Action::Finish
                    } else {
                        Action::Run
                    },
                )
                .unwrap();
            consumed += result.bytes_consumed;
            output.extend_from_slice(&buffer[..result.bytes_produced]);
            if result.status == Status::StreamEnd {
                break;
            }
            assert!(
                result.bytes_consumed != 0 || result.bytes_produced != 0,
                "stream stalled"
            );
        }
        assert_eq!(output, PLAIN);
        let mut unused = stream.unused_input().to_vec();
        unused.extend_from_slice(&input[consumed..]);
        assert_eq!(unused, b"tail");
    }
}

#[cfg(feature = "std")]
mod threaded {
    use super::*;
    use ms_compress::lzma::{Lzma2Options, Lzma2ReaderMt, Lzma2Writer, Write};
    use std::num::NonZeroU64;

    fn independent_blocks() -> (Vec<u8>, Vec<u8>) {
        let input: Vec<u8> = (0..262144)
            .map(|i| ((i / 4096 + i * 37) % 251) as u8)
            .collect();
        let mut options = Lzma2Options::with_preset(1);
        options.lzma_options.dict_size = 4096;
        options.set_chunk_size(NonZeroU64::new(4096));
        let mut writer = Lzma2Writer::new(Vec::new(), options);
        for chunk in input.chunks(4096) {
            writer.write_all(chunk).unwrap();
            writer.flush().unwrap();
        }
        (input, writer.finish().unwrap())
    }

    #[test]
    fn parallel_decoder_preserves_independent_block_order() {
        let (input, encoded) = independent_blocks();
        for workers in [1, 2, 4] {
            let mut reader =
                Lzma2ReaderMt::new_bounded(&encoded[..], 4096, workers, 1024, 65536).unwrap();
            let mut output = Vec::new();
            let mut buffer = [0; 777];
            loop {
                let count = reader.read(&mut buffer).unwrap();
                if count == 0 {
                    break;
                }
                output.extend_from_slice(&buffer[..count]);
            }
            assert_eq!(output, input);
            assert!(reader.chunk_count() >= 64);
            assert!(reader.worker_count() <= workers as usize);
            if workers > 1 {
                assert!(reader.worker_count() > 1);
            }
        }
    }

    #[test]
    fn parallel_decoder_handles_empty_stream_and_early_drop() {
        let mut empty = Lzma2ReaderMt::new(&[0][..], 4096, None, 2);
        assert_eq!(empty.read(&mut [0]).unwrap(), 0);
        let (_, encoded) = independent_blocks();
        let mut reader = Lzma2ReaderMt::new(&encoded[..], 4096, None, 4);
        assert_eq!(reader.read(&mut []).unwrap(), 0);
        reader.read_exact(&mut [0; 1]).unwrap();
        drop(reader); // Shutdown must unblock result senders and join the workers.
    }

    #[test]
    fn parallel_decoder_rejects_truncation_and_limits_without_hanging() {
        for input in [
            &[][..],
            &[1][..],
            &[2, 0, 0][..],
            &[0xff; 6][..],
            &LZMA2[..LZMA2.len() - 1],
        ] {
            let mut reader = Lzma2ReaderMt::new_bounded(input, 4096, 2, 1024, 65536).unwrap();
            assert!(reader.read_to_end(&mut Vec::new()).is_err());
        }
        assert!(Lzma2ReaderMt::new_bounded(&[][..], 4096, 2, 0, 65536).is_err());
        let mut reader = Lzma2ReaderMt::new_bounded(LZMA2, 4096, 2, 1024, 12).unwrap();
        assert!(reader.read_to_end(&mut Vec::new()).is_err());
        let encoded = lzma::compress_lzma2(&vec![b'A'; 32768], 1).unwrap().0;
        let mut reader = Lzma2ReaderMt::new_bounded(&encoded[..], 32768, 2, 1024, 1024).unwrap();
        assert!(reader.read_to_end(&mut Vec::new()).is_err());
    }

    #[test]
    #[ignore = "requires Python 3 with its liblzma-backed lzma module"]
    fn python_liblzma_accepts_encoded_streams() {
        use std::process::{Command, Stdio};
        let input: Vec<u8> = (0..32768).map(|i| (i % 251) as u8).collect();
        for raw in [false, true] {
            let (encoded, dictionary) = if raw {
                lzma::compress_lzma2(&input, 6).unwrap()
            } else {
                (lzma::compress(&input, 6).unwrap(), 4096)
            };
            let script = if raw {
                format!(
                    "import lzma,sys; sys.stdout.buffer.write(lzma.decompress(sys.stdin.buffer.read(),format=lzma.FORMAT_RAW,filters=[{{'id':lzma.FILTER_LZMA2,'dict_size':{dictionary}}}]))"
                )
            } else {
                "import lzma,sys; sys.stdout.buffer.write(lzma.decompress(sys.stdin.buffer.read(),format=lzma.FORMAT_ALONE))".into()
            };
            let mut child = Command::new("python3")
                .args(["-c", &script])
                .stdin(Stdio::piped())
                .stdout(Stdio::piped())
                .spawn()
                .unwrap();
            child.stdin.take().unwrap().write_all(&encoded).unwrap();
            let result = child.wait_with_output().unwrap();
            assert!(result.status.success());
            assert_eq!(result.stdout, input);
        }
    }
}
