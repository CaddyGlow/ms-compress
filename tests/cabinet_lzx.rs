//! Cabinet framing gates independent of the CAB directory parser.
use ms_compress::lzx::{CabinetLzxDecoder, CabinetLzxError};

fn pack(fields: &[(u32, usize)]) -> Vec<u8> {
    let mut bits = Vec::new();
    for &(value, count) in fields {
        for shift in (0..count).rev() {
            bits.push(((value >> shift) & 1) as u16);
        }
    }
    pack_stream(&bits)
}

fn pack_stream(bits: &[u16]) -> Vec<u8> {
    let mut bytes = Vec::new();
    for chunk in bits.chunks(16) {
        let mut word = 0;
        for (index, &bit) in chunk.iter().enumerate() {
            word |= bit << (15 - index);
        }
        bytes.extend_from_slice(&word.to_le_bytes());
    }
    bytes
}

fn stored(size: usize) -> Vec<u8> {
    let mut bytes = pack(&[
        (0, 1),
        (3, 3),
        ((size >> 8) as u32, 16),
        ((size & 255) as u32, 8),
    ]);
    for _ in 0..3 {
        bytes.extend_from_slice(&1u32.to_le_bytes());
    }
    bytes
}

#[test]
fn cabinet_windows_are_bounded_and_every_supported_window_decodes() {
    for order in 0..=32 {
        let decoder = CabinetLzxDecoder::new(order);
        if (15..=21).contains(&order) {
            let mut decoder = decoder.unwrap();
            let mut bytes = stored(3);
            bytes.extend_from_slice(b"abc");
            let mut output = [0; 3];
            decoder.decompress_frame(&bytes, &mut output).unwrap();
            assert_eq!(&output, b"abc");
        } else {
            assert!(matches!(decoder, Err(CabinetLzxError::InvalidWindow)));
        }
    }
}

#[test]
fn truncated_literals_bits_and_invalid_headers_fail_without_panicking() {
    let mut frame = stored(3);
    frame.extend_from_slice(b"abc");
    for length in 0..frame.len() {
        let mut decoder = CabinetLzxDecoder::new(15).unwrap();
        assert!(
            decoder
                .decompress_frame(&frame[..length], &mut [0; 3])
                .is_err()
        );
        assert_eq!(
            decoder.decompress_frame(&frame, &mut [0; 3]),
            Err(CabinetLzxError::FailedDecoder)
        );
    }
    for kind in [0, 4, 5, 6, 7] {
        let frame = pack(&[(0, 1), (kind, 3), (0, 16), (3, 8)]);
        let mut decoder = CabinetLzxDecoder::new(15).unwrap();
        assert!(decoder.decompress_frame(&frame, &mut [0; 3]).is_err());
    }
}

#[test]
fn invalid_output_sizes_fail() {
    for size in [0, 32769] {
        let mut decoder = CabinetLzxDecoder::new(15).unwrap();
        assert_eq!(
            decoder.decompress_frame(&stored(size), &mut vec![0; size]),
            Err(CabinetLzxError::InvalidFrameSize)
        );
    }
}

#[test]
fn odd_uncompressed_block_padding_is_consumed_before_next_header() {
    let mut first = stored(3);
    first.extend_from_slice(b"abc");
    let mut second = vec![0]; // Padding for the preceding odd-sized block.
    second.extend(pack(&[(3, 3), (0, 16), (4, 8)]));
    for _ in 0..3 {
        second.extend_from_slice(&1u32.to_le_bytes());
    }
    second.extend_from_slice(b"defg");
    let mut decoder = CabinetLzxDecoder::new(15).unwrap();
    let mut output = [0; 3];
    decoder.decompress_frame(&first, &mut output).unwrap();
    assert_eq!(&output, b"abc");
    let mut output = [0; 4];
    decoder.decompress_frame(&second, &mut output).unwrap();
    assert_eq!(&output, b"defg");
}

#[test]
fn verbatim_huffman_matches_reuse_the_native_ms_compress() {
    let source = b"cabinet native codec sharing ".repeat(500);
    let wim = ms_compress::lzx_encode::compress_lzx(&source, 32768, 32768)
        .unwrap()
        .unwrap();
    let bits: Vec<u16> = wim
        .chunks_exact(2)
        .flat_map(|bytes| {
            let word = u16::from_le_bytes([bytes[0], bytes[1]]);
            (0..16).rev().map(move |shift| (word >> shift) & 1)
        })
        .collect();
    assert_eq!(&bits[..4], &[0, 0, 1, 0]); // Verbatim WIM block, explicit length.
    // CAB adds a stream-header bit and always uses a 24-bit block size.
    let mut cabinet = vec![0, 0, 0, 1];
    for shift in (0..24).rev() {
        cabinet.push(((source.len() >> shift) & 1) as u16);
    }
    cabinet.extend_from_slice(&bits[20..]);
    let mut decoder = CabinetLzxDecoder::new(15).unwrap();
    let mut output = vec![0; source.len()];
    decoder
        .decompress_frame(&pack_stream(&cabinet), &mut output)
        .unwrap();
    assert_eq!(output, source);
}

#[test]
fn deterministic_malformed_frames_never_panic() {
    let mut state = 0x7351_2908u32;
    for length in 0..128 {
        let input: Vec<u8> = (0..length)
            .map(|_| {
                state ^= state << 13;
                state ^= state >> 17;
                state ^= state << 5;
                state as u8
            })
            .collect();
        let mut decoder = CabinetLzxDecoder::new(15).unwrap();
        let _ = decoder.decompress_frame(&input, &mut [0; 64]);
    }
}

#[test]
fn bulk_literal_history_wraps_without_losing_bytes_or_frame_position() {
    let source: Vec<_> = (0..65536)
        .map(|i| ((i * 79 + i / 251) % 256) as u8)
        .collect();
    let mut header = stored(source.len());
    header.extend_from_slice(&source[..1024]);
    let mut decoder = CabinetLzxDecoder::new(15).unwrap();
    let mut first = vec![0; 1024];
    decoder.decompress_frame(&header, &mut first).unwrap();
    assert_eq!(first, source[..1024]);
    let mut middle = vec![0; 32768];
    decoder
        .decompress_frame(&source[1024..33792], &mut middle)
        .unwrap();
    assert_eq!(middle, source[1024..33792]);
    let mut last = vec![0; 31744];
    decoder
        .decompress_frame(&source[33792..], &mut last)
        .unwrap();
    assert_eq!(last, source[33792..]);
}
