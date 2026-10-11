#![cfg(feature = "std")]
use ms_compress::zlib::{InflateConfig, ReturnCode, classic::gzip_level6, decompress_slice};

#[test]
fn classic_gzip_preserves_native_golden_empty_stream_and_ntfs_header() {
    let mut output = [0; 32];
    let mut scratch = vec![0; 300000];
    let length = gzip_level6(b"", &mut output, &mut scratch).unwrap();
    assert_eq!(
        &output[..length],
        &[
            31, 139, 8, 0, 0, 0, 0, 0, 0, 10, 3, 0, 0, 0, 0, 0, 0, 0, 0, 0
        ]
    );
}

#[test]
fn classic_gzip_reuses_unaligned_scratch_and_round_trips_binary_window_boundaries() {
    let mut scratch = vec![0; 300001];
    for size in [
        1, 16383, 16384, 16385, 32767, 32768, 32769, 65535, 65536, 65537,
    ] {
        let input: Vec<_> = (0..size).map(|i| (i * 71 % 251) as u8).collect();
        let mut output = vec![0; size + size / 8 + 128];
        let length = gzip_level6(&input, &mut output, &mut scratch[1..]).unwrap();
        let mut decoded = vec![0; size];
        let (decoded, code) = decompress_slice(
            &mut decoded,
            &output[..length],
            InflateConfig { window_bits: 31 },
        );
        assert_eq!(code, ReturnCode::Ok);
        assert_eq!(decoded, input);
    }
}

#[test]
fn classic_gzip_refuses_insufficient_buffers_without_overwriting_guards() {
    let mut output = [85; 64];
    assert!(gzip_level6(b"hello", &mut output, &mut [0; 64]).is_none());
    assert_eq!(output, [85; 64]);
    let mut scratch = vec![0; 300000];
    let mut guarded = [85; 40];
    assert!(gzip_level6(b"hello world", &mut guarded[1..19], &mut scratch).is_none());
    assert_eq!(guarded[0], 85);
    assert_eq!(&guarded[19..], &[85; 21]);
}

#[test]
fn classic_raw_level1_round_trips_chunk_and_window_boundaries() {
    for length in [0, 1, 261, 262, 32767, 32768, 32769, 65536, 131072] {
        let input: Vec<u8> = (0..length).map(|i| (i * 37 + i / 11) as u8).collect();
        let mut output = vec![0; length * 2 + 1024];
        let mut scratch = vec![0; 400001];
        let used =
            ms_compress::zlib::classic::raw_level1(&input, &mut output, &mut scratch[1..]).unwrap();
        let mut restored = vec![0; length];
        let (decoded, code) = decompress_slice(
            &mut restored,
            &output[..used],
            InflateConfig { window_bits: -15 },
        );
        assert_eq!(code, ReturnCode::Ok);
        assert_eq!(decoded, input);
    }
}

// ZCL1 stores independently generated inputs and native zlib output. See
// fixtures/ZLIB-CLASSIC.md for the generator, parameters, and provenance.
#[test]
fn classic_encoders_match_native_zlib_golden_streams_with_reused_scratch() {
    fn take<'a>(bytes: &mut &'a [u8], length: usize) -> &'a [u8] {
        let (value, rest) = bytes.split_at(length);
        *bytes = rest;
        value
    }
    fn u32_le(bytes: &mut &[u8]) -> usize {
        u32::from_le_bytes(take(bytes, 4).try_into().unwrap()) as usize
    }
    let mut bytes = &include_bytes!("fixtures/zlib-classic.bin")[..];
    assert_eq!(take(&mut bytes, 4), b"ZCL1");
    let count = u32_le(&mut bytes);
    assert_eq!(count, 7);
    let mut scratch = vec![0x55; 400002];
    for _ in 0..count {
        let name_len = u16::from_le_bytes(take(&mut bytes, 2).try_into().unwrap()) as usize;
        let name = std::str::from_utf8(take(&mut bytes, name_len)).unwrap();
        let input_len = u32_le(&mut bytes);
        let raw_len = u32_le(&mut bytes);
        let gzip_len = u32_le(&mut bytes);
        let input = take(&mut bytes, input_len);
        let raw = take(&mut bytes, raw_len);
        let gzip = take(&mut bytes, gzip_len);

        // Exact output capacity, minimum scratch capacity, unaligned scratch,
        // and guard bytes on both sides of both caller-owned buffers.
        let mut output = vec![0x55; raw_len + 2];
        let used = ms_compress::zlib::classic::raw_level1(
            input,
            &mut output[1..raw_len + 1],
            &mut scratch[1..400001],
        );
        assert_eq!(used, Some(raw_len), "{name}");
        assert_eq!(&output[1..raw_len + 1], raw, "{name}");
        assert_eq!((output[0], output[raw_len + 1]), (0x55, 0x55));
        assert_eq!((scratch[0], scratch[400001]), (0x55, 0x55));

        output.fill(0x55);
        assert!(
            ms_compress::zlib::classic::raw_level1(
                input,
                &mut output[1..raw_len],
                &mut scratch[1..400001],
            )
            .is_none(),
            "one byte short: {name}"
        );
        assert_eq!(output[0], 0x55);
        assert_eq!(&output[raw_len..], &[0x55; 2]);
        assert_eq!((scratch[0], scratch[400001]), (0x55, 0x55));

        // Exercise the shared level-six machinery after the new fast matcher,
        // including stale scratch contents from the rejected call.
        let mut output = vec![0; gzip_len];
        assert_eq!(
            gzip_level6(input, &mut output, &mut scratch[1..300001]),
            Some(gzip_len),
            "{name}"
        );
        assert_eq!(output, gzip, "{name}");
    }
    assert!(bytes.is_empty());
}

#[test]
fn classic_raw_level1_rejects_short_scratch_and_empty_output() {
    let mut output = [0x55; 128];
    let mut scratch = vec![0x66; 400000];
    assert!(
        ms_compress::zlib::classic::raw_level1(b"hello", &mut output, &mut scratch[..399999],)
            .is_none()
    );
    assert_eq!(output, [0x55; 128]);
    assert!(scratch.iter().all(|&byte| byte == 0x66));
    for input in [b"".as_slice(), b"hello".as_slice()] {
        assert!(ms_compress::zlib::classic::raw_level1(input, &mut [], &mut scratch).is_none());
        let used = ms_compress::zlib::classic::raw_level1(input, &mut output, &mut scratch)
            .expect("scratch remains reusable after insufficient output");
        let mut decoded = vec![0; input.len()];
        let (decoded, code) = decompress_slice(
            &mut decoded,
            &output[..used],
            InflateConfig { window_bits: -15 },
        );
        assert_eq!(code, ReturnCode::Ok);
        assert_eq!(decoded, input);
    }
}
