use ms_compress::zstd::decoding::{BlockDecodingStrategy, Dictionary, FrameDecoder};
use ms_compress::zstd::io::Read;
fn payload() -> Vec<u8> {
    let pattern: Vec<_> = (0..8192)
        .map(|i| ((i * 37 + i / 251) % 256) as u8)
        .collect();
    let mut bytes = pattern.repeat(83);
    bytes.extend_from_slice(&b"end-marker".repeat(133));
    bytes
}
fn retained(frame: &[u8], chunk: usize, expected: &[u8]) -> (usize, u32) {
    let mut source = frame;
    let mut decoder = FrameDecoder::new();
    decoder.init(&mut source).unwrap();
    let mut emitted = 0;
    let mut output = Vec::new();
    let mut early = 0;
    while !decoder.is_finished() {
        decoder
            .decode_blocks(&mut source, BlockDecodingStrategy::UptoBlocks(1))
            .unwrap();
        while decoder.remaining_output_retaining_history(emitted) > 0 {
            let mut bytes = vec![0; chunk];
            let count = decoder.copy_output_retaining_history(&mut bytes, &mut emitted);
            assert!(count > 0);
            if !decoder.is_finished() {
                early += count;
            }
            output.extend_from_slice(&bytes[..count]);
        }
    }
    assert_eq!(output, expected);
    // Fully emitted bytes retain at most the advertised 32KiB match window.
    assert!(decoder.can_collect() <= 32768);
    assert!(source.is_empty());
    assert_eq!(decoder.remaining_output_retaining_history(emitted), 0);
    let mut empty = [0; 1];
    assert_eq!(
        decoder.copy_output_retaining_history(&mut empty, &mut emitted),
        0
    );
    let checksum = decoder.get_calculated_checksum().unwrap();
    assert_eq!(decoder.get_checksum_from_data(), Some(checksum));
    (early, checksum)
}
#[test]
fn emits_before_last_block_and_keeps_cross_block_matches() {
    let frame = include_bytes!("fixtures/zstd-multiblock.bin");
    let expected = payload();
    let mut hash = None;
    for chunk in [1, 7, 4096, 8192, 65536] {
        let (early, checksum) = retained(frame, chunk, &expected);
        assert!(early > 0);
        assert_eq!(*hash.get_or_insert(checksum), checksum);
    }
}
#[test]
fn checksum_corruption_and_window_limit_are_observable() {
    let mut frame = include_bytes!("fixtures/zstd-multiblock.bin").to_vec();
    *frame.last_mut().unwrap() ^= 1;
    let mut source = frame.as_slice();
    let mut decoder = FrameDecoder::new();
    decoder.set_max_window_size(1024);
    assert!(decoder.init(&mut source).is_err());
    let mut decoder = FrameDecoder::new();
    let mut source = frame.as_slice();
    decoder.init(&mut source).unwrap();
    decoder
        .decode_blocks(&mut source, BlockDecodingStrategy::All)
        .unwrap();
    let mut out = vec![0; payload().len()];
    let mut emitted = 0;
    assert_eq!(
        decoder.copy_output_retaining_history(&mut out, &mut emitted),
        out.len()
    );
    assert_eq!(out, payload());
    assert_ne!(
        decoder.get_calculated_checksum(),
        decoder.get_checksum_from_data()
    );
}
#[test]
fn reset_cursor_and_checksum_for_concatenated_frames() {
    let frame = include_bytes!("fixtures/zstd-multiblock.bin");
    let mut frames = frame.repeat(2);
    let mut source = frames.as_slice();
    let mut decoder = FrameDecoder::new();
    for _ in 0..2 {
        decoder.init(&mut source).unwrap();
        let mut emitted = 0;
        let mut out = Vec::new();
        while !decoder.is_finished() {
            decoder
                .decode_blocks(&mut source, BlockDecodingStrategy::UptoBlocks(1))
                .unwrap();
            loop {
                let mut bytes = [0; 123];
                let n = decoder.copy_output_retaining_history(&mut bytes, &mut emitted);
                if n == 0 {
                    break;
                }
                out.extend_from_slice(&bytes[..n]);
            }
        }
        assert_eq!(out, payload());
        assert_eq!(
            decoder.get_calculated_checksum(),
            decoder.get_checksum_from_data()
        );
    }
    assert!(source.is_empty());
    frames.clear();
}
#[test]
fn raw_dictionary_and_destructive_read_checksum_remain_supported() {
    let dictionary: Vec<_> = (0..8192).map(|i| ((i * 19 + i / 97) % 256) as u8).collect();
    let mut expected = dictionary[2192..].repeat(7);
    expected.extend_from_slice(b"dictionary-end");
    let dictionary = Dictionary {
        id: 1,
        fse: Default::default(),
        huf: Default::default(),
        dict_content: dictionary,
        offset_hist: [1, 4, 8],
    };
    let mut decoder = FrameDecoder::new();
    decoder.add_dict(dictionary).unwrap();
    let mut source = include_bytes!("fixtures/zstd-dictionary.bin").as_slice();
    decoder.init(&mut source).unwrap();
    decoder.force_dict(1).unwrap();
    let mut output = Vec::new();
    while !decoder.is_finished() {
        decoder
            .decode_blocks(&mut source, BlockDecodingStrategy::UptoBlocks(1))
            .unwrap();
        loop {
            let mut bytes = [0; 257];
            let count = decoder.read(&mut bytes).unwrap();
            if count == 0 {
                break;
            }
            output.extend_from_slice(&bytes[..count]);
        }
    }
    assert_eq!(output, expected);
    assert_eq!(
        decoder.get_calculated_checksum(),
        decoder.get_checksum_from_data()
    );
    assert!(source.is_empty());
}
