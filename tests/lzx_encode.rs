use ms_compress::lzx::decompress_lzx;
use ms_compress::lzx_encode::{EncodeError, compress_lzx};

fn roundtrip(input: &[u8], maximum: usize) -> Vec<u8> {
    let compressed = compress_lzx(input, input.len() * 2 + 4096, maximum)
        .unwrap()
        .unwrap();
    let mut decoded = vec![0; input.len()];
    decompress_lzx(&compressed, &mut decoded, maximum).unwrap();
    assert_eq!(decoded, input);
    compressed
}
#[test]
fn matches_compress_repetition_substantially() {
    let input = vec![b'a'; 32768];
    assert!(roundtrip(&input, 32768).len() < 2048);
}
#[test]
fn literals_roundtrip_all_byte_values_and_small_sizes() {
    for size in [
        1, 2, 3, 9, 10, 11, 255, 256, 257, 32767, 32768, 32769, 65536,
    ] {
        let data: Vec<_> = (0..size)
            .map(|i| ((i * 73 + i / 251) & 255) as u8)
            .collect();
        roundtrip(&data, size.max(32768));
    }
}
#[test]
fn e8_relative_targets_roundtrip_and_preserve_input() {
    let mut data = vec![0x90; 4096];
    for (index, relative) in [
        (5, 0i32),
        (17, -17),
        (53, 11999999),
        (101, -102),
        (201, 12000000),
    ] {
        data[index] = 0xe8;
        data[index + 1..index + 5].copy_from_slice(&relative.to_le_bytes());
    }
    let original = data.clone();
    roundtrip(&data, 32768);
    assert_eq!(data, original);
}
#[test]
fn output_capacity_accepts_exact_size_and_rejects_one_less() {
    let data = vec![1; 4096];
    let compressed = roundtrip(&data, 32768);
    assert_eq!(
        compress_lzx(&data, compressed.len(), 32768).unwrap(),
        Some(compressed.clone())
    );
    assert_eq!(
        compress_lzx(&data, compressed.len() - 1, 32768).unwrap(),
        None
    );
}
#[test]
fn validates_configuration_and_empty_input() {
    assert_eq!(
        compress_lzx(&[], 100, 0),
        Err(EncodeError::InvalidMaxBlockSize)
    );
    assert_eq!(
        compress_lzx(&[], 100, (1 << 21) + 1),
        Err(EncodeError::InvalidMaxBlockSize)
    );
    assert_eq!(
        compress_lzx(&[1, 2], 100, 1),
        Err(EncodeError::InputExceedsLimit)
    );
    assert_eq!(compress_lzx(&[], 100, 32768).unwrap(), None);
    assert_eq!(compress_lzx(&[1], 0, 32768).unwrap(), None);
}
#[test]
fn large_window_and_far_offsets_roundtrip() {
    let mut data = Vec::new();
    let mut state = 0x12345678u32;
    for _ in 0..70000 {
        state ^= state << 13;
        state ^= state >> 17;
        state ^= state << 5;
        data.push(state as u8);
    }
    let prefix = data[..4096].to_vec();
    data.extend_from_slice(&prefix);
    roundtrip(&data, 131072);
}
