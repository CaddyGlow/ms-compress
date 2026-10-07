use ms_compress::decompress_xpress;
use ms_compress::xpress_encode::{EncodeError, compress_xpress};

#[test]
fn short_input_has_no_compressed_block() {
    assert_eq!(compress_xpress(&[7; 24], 1000).unwrap(), None);
}
#[test]
fn block_above_xpress_limit_is_rejected() {
    assert_eq!(
        compress_xpress(&vec![0; 65537], 65537),
        Err(EncodeError::InputTooLarge)
    );
}
#[test]
fn insufficient_capacity_has_no_partial_output() {
    assert_eq!(compress_xpress(&[7; 1000], 260).unwrap(), None);
}
#[test]
fn overlapping_long_matches_roundtrip() {
    for length in [273, 274, 1000, 32768, 65536] {
        let input = vec![b'a'; length];
        let encoded = compress_xpress(&input, input.len()).unwrap().unwrap();
        let mut decoded = vec![0; length];
        decompress_xpress(&encoded, &mut decoded).unwrap();
        assert_eq!(decoded, input);
    }
}
#[test]
fn periodic_binary_data_roundtrips_and_compresses() {
    let input: Vec<u8> = (0..65536).map(|i| (i % 256) as u8).collect();
    let encoded = compress_xpress(&input, input.len()).unwrap().unwrap();
    assert!(encoded.len() < 1000);
    let mut decoded = vec![0; input.len()];
    decompress_xpress(&encoded, &mut decoded).unwrap();
    assert_eq!(decoded, input);
}
#[test]
fn capacity_requires_two_trailing_bytes_after_output() {
    let input = vec![9; 4000];
    let encoded = compress_xpress(&input, input.len()).unwrap().unwrap();
    assert_eq!(
        compress_xpress(&input, encoded.len() + 2).unwrap(),
        Some(encoded.clone())
    );
    assert_eq!(compress_xpress(&input, encoded.len()).unwrap(), None);
    assert_eq!(compress_xpress(&input, encoded.len() + 1).unwrap(), None);
}

#[test]
fn random_bytes_expand_when_capacity_permits() {
    let mut state = 0x591adfu32;
    let input: Vec<_> = (0..4096)
        .map(|_| {
            state ^= state << 13;
            state ^= state >> 17;
            state ^= state << 5;
            state as u8
        })
        .collect();
    let compressed = compress_xpress(&input, usize::MAX).unwrap().unwrap();
    assert!(compressed.len() > input.len());
    let mut decoded = vec![0; input.len()];
    decompress_xpress(&compressed, &mut decoded).unwrap();
    assert_eq!(decoded, input);
}

#[test]
fn generated_low_entropy_inputs_roundtrip() {
    let mut state = 0x12345678u32;
    for alphabet in [2, 3, 4, 8, 16, 32, 64] {
        let input: Vec<_> = (0..16000)
            .map(|_| {
                state ^= state << 13;
                state ^= state >> 17;
                state ^= state << 5;
                (state % alphabet) as u8
            })
            .collect();
        let compressed = compress_xpress(&input, input.len()).unwrap().unwrap();
        let mut decoded = vec![0; input.len()];
        decompress_xpress(&compressed, &mut decoded).unwrap();
        assert_eq!(decoded, input);
    }
}
