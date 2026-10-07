//! Fixtures independently constructed from MS-PATCH sections 2 and 3.
use ms_compress::lzx::{LzxDeltaDecoder, LzxDeltaError, decompress_lzxd};

const ABC: &[u8] = &[
    0x14, 0, 0, 0x30, 0x30, 0, 1, 0, 0, 0, 1, 0, 0, 0, 1, 0, 0, 0, 0x61, 0x62, 0x63, 0,
];

#[derive(Default)]
struct Writer {
    bits: Vec<bool>,
}
impl Writer {
    fn put(&mut self, value: usize, count: usize) {
        for shift in (0..count).rev() {
            self.bits.push((value >> shift) & 1 != 0);
        }
    }
    fn lengths(&mut self, lengths: &[u8]) {
        // Complete pretree: symbol 0 -> 0, 15 -> 10, 16 -> 11.
        for symbol in 0..20 {
            self.put(
                if symbol == 0 {
                    1
                } else if symbol == 15 || symbol == 16 {
                    2
                } else {
                    0
                },
                4,
            );
        }
        for &length in lengths {
            match length {
                0 => self.put(0, 1),
                1 => self.put(3, 2),
                2 => self.put(2, 2),
                _ => panic!(),
            }
        }
    }
    fn payload(mut self) -> Vec<u8> {
        while !self.bits.len().is_multiple_of(16) {
            self.bits.push(false);
        }
        self.bits
            .chunks(16)
            .flat_map(|bits| {
                bits.iter()
                    .fold(0u16, |n, &b| (n << 1) | u16::from(b))
                    .to_le_bytes()
            })
            .collect()
    }
    fn stream(self) -> Vec<u8> {
        let payload = self.payload();
        let mut result = (payload.len() as u16).to_le_bytes().to_vec();
        result.extend(payload);
        result
    }
}
fn compressed(
    window_order: u8,
    block_type: usize,
    length: usize,
    symbol: usize,
    extra: impl FnOnce(&mut Writer),
) -> Vec<u8> {
    let slots = match window_order {
        17 => 34,
        18 => 36,
        19 => 38,
        20 => 42,
        _ => 34 + (1usize << (window_order - 17)),
    };
    let mut w = Writer::default();
    w.put(0, 1);
    w.put(block_type, 3);
    w.put(length, 24);
    if block_type == 2 {
        for _ in 0..8 {
            w.put(3, 3);
        }
    }
    let mut main = vec![0; 256 + slots * 8];
    main[0] = 1;
    main[symbol] = 1;
    w.lengths(&main[..256]);
    w.lengths(&main[256..]);
    let mut lens = vec![0; 249];
    if symbol & 7 == 7 {
        lens[0] = 1;
        lens[248] = 1;
    }
    w.lengths(&lens);
    w.put(1, 1);
    if symbol & 7 == 7 {
        w.put(1, 1);
    }
    extra(&mut w);
    w.stream()
}
#[test]
fn official_abc_example_decodes() {
    let mut out = [0; 3];
    decompress_lzxd(ABC, b"", &mut out, 17).unwrap();
    assert_eq!(&out, b"abc");
}
#[test]
fn every_truncation_of_official_example_is_rejected() {
    for n in 0..ABC.len() {
        assert!(
            decompress_lzxd(&ABC[..n], b"", &mut [0; 3], 17).is_err(),
            "{n}"
        );
    }
}
#[test]
fn reference_dictionary_produces_match_without_literals() {
    // Slot 6 has base formatted offset 8; footer 2 yields distance 8.
    let input = compressed(17, 1, 3, 256 + 6 * 8 + 1, |w| w.put(2, 2));
    let mut out = [0; 3];
    decompress_lzxd(&input, b"ABCDEFGH", &mut out, 17).unwrap();
    assert_eq!(&out, b"ABC");
    assert!(decompress_lzxd(&input, b"", &mut out, 17).is_err());
}
#[test]
fn aligned_reference_match_uses_low_three_offset_bits() {
    let input = compressed(17, 2, 3, 256 + 8 * 8 + 1, |w| w.put(2, 3));
    let mut out = [0; 3];
    decompress_lzxd(&input, b"abcdefghijklmnop", &mut out, 17).unwrap();
    assert_eq!(&out, b"abc");
}
#[test]
fn all_extended_length_prefixes_decode() {
    for (length, prefix, prefix_bits, value, value_bits) in [
        (257, 0, 1, 0, 8),
        (512, 0, 1, 255, 8),
        (513, 2, 2, 0, 10),
        (1536, 2, 2, 1023, 10),
        (1537, 6, 3, 0, 12),
        (5632, 6, 3, 4095, 12),
        (32768, 7, 3, 32511, 15),
    ] {
        let input = compressed(17, 1, length, 263, |w| {
            w.put(prefix, prefix_bits);
            w.put(value, value_bits);
        });
        let mut out = vec![0; length];
        decompress_lzxd(&input, b"Z", &mut out, 17).unwrap();
        assert_eq!(out, vec![b'Z'; length]);
    }
}
#[test]
fn largest_window_main_symbols_do_not_truncate_lookup_entries() {
    // Last slot base =33423358 actual distance; footer zero.
    let input = compressed(25, 1, 2, 256 + 289 * 8, |w| w.put(0, 17));
    let mut reference = vec![b'R'; 33423358];
    reference[..2].copy_from_slice(b"OK");
    let mut out = [0; 2];
    decompress_lzxd(&input, &reference, &mut out, 25).unwrap();
    assert_eq!(&out, b"OK");
}
#[test]
fn malformed_padding_trailing_data_and_output_mismatch_are_rejected() {
    let mut input = ABC.to_vec();
    *input.last_mut().unwrap() = 1;
    assert_eq!(
        decompress_lzxd(&input, b"", &mut [0; 3], 17),
        Err(LzxDeltaError::InvalidFraming)
    );
    let mut input = ABC.to_vec();
    input.push(0);
    assert!(decompress_lzxd(&input, b"", &mut [0; 3], 17).is_err());
    assert_eq!(
        decompress_lzxd(ABC, b"", &mut [0; 2], 17),
        Err(LzxDeltaError::InvalidFraming)
    );
}
#[test]
fn invalid_configuration_and_failed_decoder_are_rejected() {
    assert!(LzxDeltaDecoder::new(16, b"").is_err());
    assert!(LzxDeltaDecoder::new(26, b"").is_err());
    assert!(LzxDeltaDecoder::new(17, &vec![0; 131070]).is_err());
    let mut decoder = LzxDeltaDecoder::new(17, b"").unwrap();
    assert!(decoder.decompress_chunk(&[], &mut [0; 1]).is_err());
    assert_eq!(
        decoder.decompress_chunk(&ABC[2..], &mut [0; 3]),
        Err(LzxDeltaError::FailedDecoder)
    );
}

#[test]
fn compressed_block_and_trees_survive_chunk_boundary() {
    let mut input = compressed(17, 1, 33025, 263, |w| {
        w.put(7, 3);
        w.put(32511, 15);
    });
    let mut second = Writer::default();
    second.put(1, 1);
    second.put(1, 1);
    second.put(0, 1);
    second.put(0, 8);
    input.extend(second.stream());
    let mut out = vec![0; 33025];
    decompress_lzxd(&input, b"K", &mut out, 17).unwrap();
    assert_eq!(out, vec![b'K'; 33025]);
    assert_eq!(
        decompress_lzxd(&input[..input.len() - 4], b"K", &mut vec![0; 32768], 17),
        Err(LzxDeltaError::IncompleteBlock)
    );
}

#[test]
fn uncompressed_block_survives_chunk_boundary_and_odd_padding() {
    let mut header = Writer::default();
    header.put(0, 1);
    header.put(3, 3);
    header.put(32771, 24);
    let mut first = header.payload();
    for _ in 0..3 {
        first.extend(1u32.to_le_bytes());
    }
    first.extend(vec![b'A'; 32768]);
    let mut input = (first.len() as u16).to_le_bytes().to_vec();
    input.extend(first);
    input.extend([4, 0, b'B', b'C', b'D', 0]);
    let mut out = vec![0; 32771];
    decompress_lzxd(&input, b"", &mut out, 17).unwrap();
    assert_eq!(&out[32768..], b"BCD");
}

#[test]
fn e8_translation_uses_subject_position_and_keeps_history_untranslated() {
    let mut header = Writer::default();
    header.put(1, 1);
    header.put(0, 16);
    header.put(1000, 16);
    header.put(3, 3);
    header.put(16, 24);
    let mut payload = header.payload();
    for _ in 0..3 {
        payload.extend(1u32.to_le_bytes());
    }
    let mut raw = [0; 16];
    raw[1] = 0xe8;
    raw[2..6].copy_from_slice(&100i32.to_le_bytes());
    raw[10] = 0xe8;
    raw[11..15].copy_from_slice(&100i32.to_le_bytes());
    payload.extend(raw);
    let mut input = (payload.len() as u16).to_le_bytes().to_vec();
    input.extend(payload);
    let mut out = [0; 16];
    decompress_lzxd(&input, b"REFERENCE", &mut out, 17).unwrap();
    assert_eq!(i32::from_le_bytes(out[2..6].try_into().unwrap()), 99);
    assert_eq!(i32::from_le_bytes(out[11..15].try_into().unwrap()), 100);
}

#[test]
fn independent_libmspack_oracle_vectors_decode_exactly() {
    let vectors: &[(&[u8], &[u8], &[u8])] = &[
        (
            include_bytes!("fixtures/lzxd/official-abc.lzxd"),
            b"",
            b"abc",
        ),
        (
            include_bytes!("fixtures/lzxd/reference-verbatim.lzxd"),
            b"ABCDEFGH",
            b"ABC",
        ),
        (
            include_bytes!("fixtures/lzxd/reference-aligned.lzxd"),
            b"abcdefghijklmnop",
            b"abc",
        ),
    ];
    for &(input, reference, expected) in vectors {
        let mut out = vec![0; expected.len()];
        decompress_lzxd(input, reference, &mut out, 17).unwrap();
        assert_eq!(out, expected);
    }
    for (input, length) in [
        (
            include_bytes!("fixtures/lzxd/extended-257.lzxd").as_slice(),
            257,
        ),
        (
            include_bytes!("fixtures/lzxd/extended-513.lzxd").as_slice(),
            513,
        ),
        (
            include_bytes!("fixtures/lzxd/extended-1537.lzxd").as_slice(),
            1537,
        ),
        (
            include_bytes!("fixtures/lzxd/extended-32768.lzxd").as_slice(),
            32768,
        ),
    ] {
        let mut out = vec![0; length];
        decompress_lzxd(input, b"Z", &mut out, 17).unwrap();
        assert_eq!(out, vec![b'Z'; length]);
    }
}

#[test]
fn bounded_random_chunks_and_mutated_oracle_vectors_never_panic() {
    let mut seed = 0x8a73_62cd_109f_4567u64;
    for case in 0..2048 {
        let length = case % 193;
        let mut input = vec![0; length];
        for byte in &mut input {
            seed ^= seed << 13;
            seed ^= seed >> 7;
            seed ^= seed << 17;
            *byte = seed as u8;
        }
        let mut decoder = LzxDeltaDecoder::new(17, b"reference").unwrap();
        let _ = decoder.decompress_chunk(&input, &mut [0; 257]);
    }
    for fixture in [
        ABC,
        include_bytes!("fixtures/lzxd/reference-verbatim.lzxd").as_slice(),
        include_bytes!("fixtures/lzxd/reference-aligned.lzxd").as_slice(),
        include_bytes!("fixtures/lzxd/extended-32768.lzxd").as_slice(),
    ] {
        for byte in 0..fixture.len() {
            for bit in 0..8 {
                let mut input = fixture.to_vec();
                input[byte] ^= 1 << bit;
                let _ = decompress_lzxd(&input, b"abcdefghijklmnop", &mut [0; 32768], 17);
            }
        }
    }
}
