use ms_compress::lzms::{decompress_lzms, encode::compress_lzms};
#[test]
fn real_matches_roundtrip_and_compress() {
    for size in [4, 32, 1024, 4097, 65536] {
        let input: Vec<_> = (0..size).map(|i| b"native LZMS coding\0"[i % 19]).collect();
        let encoded = compress_lzms(&input, size * 4 + 128).unwrap().unwrap();
        let mut output = vec![0; size];
        decompress_lzms(&encoded, &mut output).unwrap();
        assert_eq!(output, input);
        if size > 1024 {
            assert!(encoded.len() < size / 4);
        }
    }
}
#[test]
fn random_literals_rebuild_adaptive_codes() {
    let mut state = 123u32;
    let input: Vec<_> = (0..8192)
        .map(|_| {
            state ^= state << 13;
            state ^= state >> 17;
            state ^= state << 5;
            state as u8
        })
        .collect();
    let encoded = compress_lzms(&input, 20000).unwrap().unwrap();
    let mut output = vec![0; input.len()];
    decompress_lzms(&encoded, &mut output).unwrap();
    assert_eq!(input, output);
}
#[test]
fn capacity_and_short_input_are_rejected() {
    assert!(compress_lzms(&[0; 3], 100).unwrap().is_none());
    assert!(compress_lzms(&[0; 1024], 7).unwrap().is_none());
}
#[test]
fn arithmetic_ramp_uses_delta_matches() {
    let input: Vec<_> = (0..4096).map(|i| (i * 37) as u8).collect();
    let encoded = compress_lzms(&input, 8192).unwrap().unwrap();
    assert!(
        encoded.len() < 40,
        "delta ramp encoded in {} bytes",
        encoded.len()
    );
    let mut output = vec![0; input.len()];
    decompress_lzms(&encoded, &mut output).unwrap();
    assert_eq!(output, input);
}
#[test]
fn long_length_slot_and_exact_capacity_roundtrip() {
    let input = vec![0x93; 131072];
    let encoded = compress_lzms(&input, 400000).unwrap().unwrap();
    assert_eq!(
        compress_lzms(&input, encoded.len()).unwrap(),
        Some(encoded.clone())
    );
    assert_eq!(compress_lzms(&input, encoded.len() - 1).unwrap(), None);
    assert_eq!(
        compress_lzms(&input, encoded.len() + 1).unwrap(),
        Some(encoded.clone())
    );
    let mut output = vec![0; input.len()];
    decompress_lzms(&encoded, &mut output).unwrap();
    assert_eq!(output, input);
}
#[test]
fn delta_spans_and_x86_preprocessing_roundtrip() {
    for power in 0..8 {
        let span = 1 << power;
        let input: Vec<_> = (0..16384)
            .map(|i| ((i / span) * 37 + (i % span) * 13) as u8)
            .collect();
        let encoded = compress_lzms(&input, 40000).unwrap().unwrap();
        let mut output = vec![0; input.len()];
        decompress_lzms(&encoded, &mut output).unwrap();
        assert_eq!(output, input);
    }
    let mut input = vec![0x90];
    for _ in 0..1000 {
        let pos = input.len() as u32;
        input.push(0xe8);
        input.extend_from_slice(&0x12345678u32.wrapping_sub(pos).to_le_bytes());
        input.push(0x90);
    }
    let encoded = compress_lzms(&input, 40000).unwrap().unwrap();
    let mut output = vec![0; input.len()];
    decompress_lzms(&encoded, &mut output).unwrap();
    assert_eq!(output, input);
}
