use ms_compress::lzx::{LzxError, decompress_lzx};

struct Bits(Vec<bool>);

impl Bits {
    fn push(&mut self, value: usize, count: usize) {
        for shift in (0..count).rev() {
            self.0.push(value & (1 << shift) != 0);
        }
    }

    fn bytes(&self) -> Vec<u8> {
        self.0
            .chunks(16)
            .flat_map(|bits| {
                let word = bits.iter().enumerate().fold(0u16, |value, (index, &bit)| {
                    value | (u16::from(bit) << (15 - index))
                });
                word.to_le_bytes()
            })
            .collect()
    }
}

fn uncompressed(raw: &[u8], window_order: usize, offsets: [u32; 3]) -> Vec<u8> {
    let mut bits = Bits(Vec::new());
    bits.push(3, 3);
    bits.push(0, 1);
    bits.push(raw.len(), if window_order == 15 { 16 } else { 24 });
    let mut input = bits.bytes();
    for offset in offsets {
        input.extend_from_slice(&offset.to_le_bytes());
    }
    input.extend_from_slice(raw);
    if !raw.len().is_multiple_of(2) {
        input.push(0);
    }
    input
}

fn write_lens(bits: &mut Bits, desired: &[u8]) {
    // Precode 0 and 16 both have length 1: delta 0 => 0, delta16 => 1.
    for symbol in 0..20 {
        bits.push(usize::from(symbol == 0 || symbol == 16), 4);
    }
    for &length in desired {
        assert!(length <= 1);
        bits.push(usize::from(length != 0), 1);
    }
}

fn literal_and_match(symbol: usize, count: usize) -> Vec<u8> {
    let mut bits = Bits(Vec::new());
    bits.push(1, 3);
    bits.push(0, 1);
    bits.push(count, 16);
    let mut lengths = [0; 496]; // window order15: 256 +30*8 main symbols
    lengths[65] = 1;
    lengths[symbol] = 1;
    write_lens(&mut bits, &lengths[..256]);
    write_lens(&mut bits, &lengths[256..]);
    write_lens(&mut bits, &[0; 249]);
    bits.push(0, 1); // A literal
    bits.push(1, 1); // match
    bits.bytes()
}

#[test]
fn uncompressed_block_copies_literal_bytes() {
    let input = uncompressed(b"hello world", 15, [1, 1, 1]);
    let mut output = [0; 11];
    decompress_lzx(&input, &mut output, 32768).unwrap();
    assert_eq!(output, *b"hello world");
}

#[test]
fn all_extended_window_orders_use_24_bit_block_sizes() {
    for order in 16..=21 {
        let input = uncompressed(b"abcdef", order, [1, 1, 1]);
        let mut output = [0; 6];
        decompress_lzx(&input, &mut output, 1 << order).unwrap();
        assert_eq!(output, *b"abcdef", "order{order}");
    }
}

#[test]
fn non_power_of_two_capacity_rounds_window_up() {
    let input = uncompressed(b"abc", 16, [1, 1, 1]);
    let mut output = [0; 3];
    decompress_lzx(&input, &mut output, 32769).unwrap();
    assert_eq!(output, *b"abc");
}

#[test]
fn invalid_maximum_capacity_is_rejected() {
    assert_eq!(
        decompress_lzx(&[], &mut [], 2097153),
        Err(LzxError::InvalidMaxBlockSize)
    );
}

#[test]
fn zero_maximum_capacity_is_rejected() {
    assert_eq!(
        decompress_lzx(&[], &mut [], 0),
        Err(LzxError::InvalidMaxBlockSize)
    );
}

#[test]
fn output_beyond_capacity_is_rejected_before_reading_input() {
    assert_eq!(
        decompress_lzx(&[], &mut [0; 2], 1),
        Err(LzxError::OutputExceedsLimit)
    );
}

#[test]
fn empty_output_succeeds_without_reading_any_header() {
    assert_eq!(decompress_lzx(&[], &mut [], 1), Ok(()));
}

#[test]
fn missing_header_zero_padding_yields_invalid_block_type() {
    assert_eq!(
        decompress_lzx(&[], &mut [0], 1),
        Err(LzxError::InvalidBlockType)
    );
}

#[test]
fn zero_recent_offset_is_rejected() {
    let input = uncompressed(b"a", 15, [1, 0, 1]);
    assert_eq!(
        decompress_lzx(&input, &mut [0], 32768),
        Err(LzxError::InvalidRecentOffset)
    );
}

#[test]
fn uncompressed_literals_are_not_zero_padded() {
    let mut input = uncompressed(b"abcd", 15, [1, 1, 1]);
    input.truncate(input.len() - 1);
    assert_eq!(
        decompress_lzx(&input, &mut [0; 4], 32768),
        Err(LzxError::TruncatedUncompressedBlock)
    );
}

#[test]
fn missing_final_odd_padding_byte_is_permitted() {
    let mut input = uncompressed(b"abc", 15, [1, 1, 1]);
    input.pop();
    let mut output = [0; 3];
    decompress_lzx(&input, &mut output, 32768).unwrap();
    assert_eq!(output, *b"abc");
}

#[test]
fn block_larger_than_requested_output_is_rejected() {
    let input = uncompressed(b"abc", 15, [1, 1, 1]);
    assert_eq!(
        decompress_lzx(&input, &mut [0; 2], 32768),
        Err(LzxError::InvalidBlockSize)
    );
}

#[test]
fn verbatim_empty_tables_emit_zeroes_using_bit_padding() {
    let mut output = vec![99; 32768];
    decompress_lzx(&[0, 0x30], &mut output, 32768).unwrap();
    assert_eq!(output, vec![0; 32768]);
}

#[test]
fn aligned_empty_tables_emit_zeroes_using_bit_padding() {
    let mut output = vec![99; 32768];
    decompress_lzx(&[0, 0x50], &mut output, 32768).unwrap();
    assert_eq!(output, vec![0; 32768]);
}

#[test]
fn repeated_offset_zero_slot_uses_initial_offset_one() {
    let input = literal_and_match(256, 3);
    let mut output = [0; 3];
    decompress_lzx(&input, &mut output, 32768).unwrap();
    assert_eq!(output, *b"AAA");
}

#[test]
fn repeated_offset_one_slot_uses_initial_offset_one() {
    let input = literal_and_match(264, 3);
    let mut output = [0; 3];
    decompress_lzx(&input, &mut output, 32768).unwrap();
    assert_eq!(output, *b"AAA");
}

#[test]
fn repeated_offset_two_slot_uses_initial_offset_one() {
    let input = literal_and_match(272, 3);
    let mut output = [0; 3];
    decompress_lzx(&input, &mut output, 32768).unwrap();
    assert_eq!(output, *b"AAA");
}

#[test]
fn explicit_offset_slot_three_matches_previous_literal() {
    let input = literal_and_match(280, 3);
    let mut output = [0; 3];
    decompress_lzx(&input, &mut output, 32768).unwrap();
    assert_eq!(output, *b"AAA");
}

#[test]
fn length_header_seven_reads_empty_length_code_as_zero() {
    let input = literal_and_match(263, 10);
    let mut output = [0; 10];
    decompress_lzx(&input, &mut output, 32768).unwrap();
    assert_eq!(output, [b'A'; 10]);
}

#[test]
fn e8_inverse_transforms_absolute_target_to_relative() {
    let mut raw = [0; 20];
    raw[1] = 0xe8;
    raw[2..6].copy_from_slice(&100i32.to_le_bytes());
    let input = uncompressed(&raw, 15, [1, 1, 1]);
    let mut output = [0; 20];
    decompress_lzx(&input, &mut output, 32768).unwrap();
    assert_eq!(&output[2..6], &99i32.to_le_bytes());
}

#[test]
fn e8_inverse_compensates_negative_target_in_valid_range() {
    let mut raw = [0; 20];
    raw[1] = 0xe8;
    raw[2..6].copy_from_slice(&(-1i32).to_le_bytes());
    let input = uncompressed(&raw, 15, [1, 1, 1]);
    let mut output = [0; 20];
    decompress_lzx(&input, &mut output, 32768).unwrap();
    assert_eq!(&output[2..6], &11999999i32.to_le_bytes());
}

#[test]
fn e8_does_not_process_instruction_starting_in_last_ten_bytes() {
    let mut raw = [0; 20];
    raw[10] = 0xe8;
    raw[11..15].copy_from_slice(&100i32.to_le_bytes());
    let input = uncompressed(&raw, 15, [1, 1, 1]);
    let mut output = [0; 20];
    decompress_lzx(&input, &mut output, 32768).unwrap();
    assert_eq!(output, raw);
}

fn compressed_header(bits: &mut Bits, block_type: usize, size: usize, order: usize) {
    bits.push(block_type, 3);
    bits.push(0, 1);
    bits.push(size, if order == 15 { 16 } else { 24 });
}

#[test]
fn aligned_offset_symbol_supplies_low_three_offset_bits() {
    let mut input = uncompressed(b"ABCDEFGHIJKLMN", 15, [1, 1, 1]);
    let mut bits = Bits(Vec::new());
    compressed_header(&mut bits, 2, 2, 15);
    for _ in 0..8 {
        bits.push(3, 3);
    }
    let mut lengths = [0; 496];
    lengths[65] = 1;
    lengths[320] = 1; // offset slot8: base14, no raw extra bits
    write_lens(&mut bits, &lengths[..256]);
    write_lens(&mut bits, &lengths[256..]);
    write_lens(&mut bits, &[0; 249]);
    bits.push(1, 1);
    bits.push(0, 3); // aligned symbol0
    input.extend_from_slice(&bits.bytes());
    let mut output = [0; 16];
    decompress_lzx(&input, &mut output, 32768).unwrap();
    assert_eq!(output, *b"ABCDEFGHIJKLMNAB");
}

#[test]
fn repeated_offset_two_does_not_demote_recent_offset_one() {
    let mut input = uncompressed(b"ABCDE", 15, [2, 3, 4]);
    let mut bits = Bits(Vec::new());
    compressed_header(&mut bits, 1, 6, 15);
    let mut lengths = [0; 496];
    lengths[264] = 1;
    lengths[272] = 1;
    write_lens(&mut bits, &lengths[..256]);
    write_lens(&mut bits, &lengths[256..]);
    write_lens(&mut bits, &[0; 249]);
    bits.push(0b101, 3); // R2(4), R1(3), R2(2)
    input.extend_from_slice(&bits.bytes());
    let mut output = [0; 11];
    decompress_lzx(&input, &mut output, 32768).unwrap();
    assert_eq!(output, *b"ABCDEBCEBEB");
}

#[test]
fn verbatim_seventeen_bit_offset_reads_across_coding_word_boundary() {
    let prefix = vec![0; 262142];
    let mut input = uncompressed(&prefix, 19, [1, 1, 1]);
    let mut bits = Bits(Vec::new());
    compressed_header(&mut bits, 1, 2, 19);
    let mut lengths = vec![0; 560]; // order19 uses38 offset slots
    lengths[0] = 1;
    lengths[544] = 1; // offset slot36 => base262142 +17 extra bits
    write_lens(&mut bits, &lengths[..256]);
    write_lens(&mut bits, &lengths[256..]);
    write_lens(&mut bits, &[0; 249]);
    bits.push(1, 1);
    bits.push(0, 17);
    input.extend_from_slice(&bits.bytes());
    let mut output = vec![99; 262144];
    decompress_lzx(&input, &mut output, 1 << 19).unwrap();
    assert_eq!(output, vec![0; 262144]);
}

fn zero_run_lens(bits: &mut Bits, count: usize, symbol: usize) {
    for index in 0..20 {
        bits.push(usize::from(index == 0 || index == symbol), 4);
    }
    let run = if symbol == 17 { 19 } else { 51 };
    for _ in 0..count.div_ceil(run) {
        bits.push(1, 1);
        bits.push(
            if symbol == 17 { 15 } else { 31 },
            if symbol == 17 { 4 } else { 5 },
        );
    }
}

#[test]
fn short_zero_runs_allow_documented_code_length_scratch_overrun() {
    let mut bits = Bits(Vec::new());
    compressed_header(&mut bits, 1, 1, 15);
    zero_run_lens(&mut bits, 256, 17);
    zero_run_lens(&mut bits, 240, 17);
    zero_run_lens(&mut bits, 249, 17);
    let mut output = [99; 1];
    decompress_lzx(&bits.bytes(), &mut output, 32768).unwrap();
    assert_eq!(output, [0]);
}

#[test]
fn long_zero_runs_allow_documented_fifty_length_scratch_overrun() {
    let mut bits = Bits(Vec::new());
    compressed_header(&mut bits, 1, 1, 15);
    zero_run_lens(&mut bits, 256, 18);
    zero_run_lens(&mut bits, 240, 18);
    zero_run_lens(&mut bits, 249, 18);
    let mut output = [99; 1];
    decompress_lzx(&bits.bytes(), &mut output, 32768).unwrap();
    assert_eq!(output, [0]);
}

#[test]
fn identical_length_runs_decode_delta_from_first_previous_length() {
    let mut bits = Bits(Vec::new());
    compressed_header(&mut bits, 1, 1, 15);
    for count in [256usize, 240, 249] {
        for index in 0..20 {
            bits.push(usize::from(index == 0 || index == 19), 4);
        }
        for _ in 0..count.div_ceil(5) {
            bits.push(1, 1); // precode19
            bits.push(1, 1); // run5
            bits.push(0, 1); // delta0 from first existing length
        }
    }
    let mut output = [99; 1];
    decompress_lzx(&bits.bytes(), &mut output, 32768).unwrap();
    assert_eq!(output, [0]);
}

#[test]
fn identical_length_run_rejects_precode_symbol_above_seventeen() {
    let mut bits = Bits(Vec::new());
    compressed_header(&mut bits, 1, 1, 15);
    for index in 0..20 {
        bits.push(usize::from(index == 0 || index == 19), 4);
    }
    bits.push(1, 1);
    bits.push(1, 1);
    bits.push(1, 1); // repeated delta symbol19 is invalid
    assert_eq!(
        decompress_lzx(&bits.bytes(), &mut [0], 32768),
        Err(LzxError::InvalidLengthRun)
    );
}

#[test]
fn uncompressed_header_already_aligned_discards_an_extra_coding_word() {
    let mut bits = Bits(Vec::new());
    compressed_header(&mut bits, 1, 15, 15);
    let mut lengths = [0; 496];
    lengths[65] = 1;
    lengths[66] = 1;
    write_lens(&mut bits, &lengths[..256]);
    write_lens(&mut bits, &lengths[256..]);
    write_lens(&mut bits, &[0; 249]);
    bits.push(0, 15); // 15 A literals leave next header ending on a word boundary
    compressed_header(&mut bits, 3, 1, 15);
    assert!(bits.0.len().is_multiple_of(16));
    let mut input = bits.bytes();
    input.extend_from_slice(&[0xff, 0xff]); // mandatory discarded coding word
    for _ in 0..3 {
        input.extend_from_slice(&1u32.to_le_bytes());
    }
    input.extend_from_slice(b"B\0");
    let mut output = [0; 16];
    decompress_lzx(&input, &mut output, 32768).unwrap();
    assert_eq!(output, *b"AAAAAAAAAAAAAAAB");
}

#[test]
fn huffman_lengths_are_delta_encoded_against_previous_block() {
    let mut bits = Bits(Vec::new());
    compressed_header(&mut bits, 1, 1, 15);
    let mut lengths = [0; 496];
    lengths[65] = 1;
    lengths[66] = 1;
    write_lens(&mut bits, &lengths[..256]);
    write_lens(&mut bits, &lengths[256..]);
    write_lens(&mut bits, &[0; 249]);
    bits.push(0, 1); // A
    compressed_header(&mut bits, 1, 1, 15);
    write_lens(&mut bits, &[0; 256]); // delta0 keeps both literal lengths at1
    write_lens(&mut bits, &[0; 240]);
    write_lens(&mut bits, &[0; 249]);
    bits.push(1, 1); // B
    let mut output = [0; 2];
    decompress_lzx(&bits.bytes(), &mut output, 32768).unwrap();
    assert_eq!(output, *b"AB");
}

#[test]
fn match_cannot_cross_its_block_end() {
    let input = literal_and_match(256, 2);
    assert_eq!(
        decompress_lzx(&input, &mut [0; 2], 32768),
        Err(LzxError::InvalidMatch)
    );
}
