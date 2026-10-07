//! Quantum framing and malformed-input gates, independent of the CAB parser.
use ms_compress::quantum::{QuantumDecoder, QuantumError};

fn sample() -> (&'static [u8], usize) {
    let cab = include_bytes!("fixtures/quantum/mszip_lzx_qtm.cab");
    let folder = &cab[52..60];
    let offset = u32::from_le_bytes(folder[..4].try_into().unwrap()) as usize;
    let input_size = u16::from_le_bytes(cab[offset + 4..offset + 6].try_into().unwrap()) as usize;
    let output_size = u16::from_le_bytes(cab[offset + 6..offset + 8].try_into().unwrap()) as usize;
    (&cab[offset + 8..offset + 8 + input_size], output_size)
}

#[test]
fn window_and_frame_bounds_are_checked() {
    for order in 0..=32 {
        assert_eq!(
            QuantumDecoder::new(order).is_ok(),
            (10..=21).contains(&order)
        );
    }
    for size in [0, 32769] {
        assert_eq!(
            QuantumDecoder::new(18)
                .unwrap()
                .decompress_frame(&[0; 32], &mut vec![0; size]),
            Err(QuantumError::InvalidFrameSize)
        );
    }
    assert_eq!(
        QuantumDecoder::new(18)
            .unwrap()
            .decompress_frame(&vec![0; 38913], &mut [0; 1]),
        Err(QuantumError::InvalidInputSize)
    );
}

#[test]
fn independent_frame_decodes_and_all_truncations_invalidate_state() {
    let (input, size) = sample();
    let mut output = vec![0; size];
    QuantumDecoder::new(18)
        .unwrap()
        .decompress_frame(input, &mut output)
        .unwrap();
    assert_eq!(
        output,
        b"If you can read this, the Quantum decompressor is working!\n"
    );
    for length in 0..input.len() {
        let mut decoder = QuantumDecoder::new(18).unwrap();
        assert!(
            decoder
                .decompress_frame(&input[..length], &mut output)
                .is_err(),
            "length {length}"
        );
        assert_eq!(
            decoder.decompress_frame(input, &mut output),
            Err(QuantumError::FailedDecoder)
        );
    }
}

#[test]
fn zero_through_four_padding_bytes_are_accepted_and_nonzero_or_excess_padding_fails() {
    let (input, size) = sample();
    for padding in 0..=4 {
        let mut input = input.to_vec();
        input.extend(vec![0; padding]);
        QuantumDecoder::new(18)
            .unwrap()
            .decompress_frame(&input, &mut vec![0; size])
            .unwrap();
    }
    for extra in [vec![1], vec![0; 5]] {
        let mut input = input.to_vec();
        input.extend(extra);
        assert_eq!(
            QuantumDecoder::new(18)
                .unwrap()
                .decompress_frame(&input, &mut vec![0; size]),
            Err(QuantumError::InvalidPadding)
        );
    }
}

#[test]
fn deterministic_malformed_frames_and_mutated_samples_never_panic() {
    let mut state = 0x73901204u32;
    for length in 0..256 {
        let input: Vec<_> = (0..length)
            .map(|_| {
                state ^= state << 13;
                state ^= state >> 17;
                state ^= state << 5;
                state as u8
            })
            .collect();
        let _ = QuantumDecoder::new(10)
            .unwrap()
            .decompress_frame(&input, &mut [0; 512]);
    }
    let (sample, size) = sample();
    for index in 0..sample.len() {
        for bit in 0..8 {
            let mut mutated = sample.to_vec();
            mutated[index] ^= 1 << bit;
            let _ = QuantumDecoder::new(18)
                .unwrap()
                .decompress_frame(&mutated, &mut vec![0; size]);
        }
    }
}
