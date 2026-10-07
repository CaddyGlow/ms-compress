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
