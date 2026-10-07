use ms_compress::{DecodeError, decompress_xpress};

fn header(lengths: &[(usize, u8)]) -> Vec<u8> {
    let mut input = vec![0; 256];
    for &(symbol, len) in lengths {
        input[symbol / 2] |= len << ((symbol % 2) * 4);
    }
    input
}

fn two_symbols(match_symbol: usize, bytes: &[u8]) -> Vec<u8> {
    let mut input = header(&[(65, 1), (match_symbol, 1)]);
    input.extend_from_slice(bytes);
    input
}

#[test]
fn decodes_literal_without_end_marker() {
    let input = header(&[(65, 1), (66, 1)]);
    let mut output = [99];
    decompress_xpress(&input, &mut output).unwrap();
    assert_eq!(output, *b"A");
}

#[test]
fn reads_little_endian_words_with_most_significant_bit_first() {
    let mut input = header(&[(65, 1), (66, 1)]);
    input.extend_from_slice(&[0x00, 0x60]); // 0, 1, 1, 0
    let mut output = [0; 4];
    decompress_xpress(&input, &mut output).unwrap();
    assert_eq!(output, *b"ABBA");
}

#[test]
fn overlapping_offset_one_match_repeats_previous_literal() {
    let input = two_symbols(256, &[0, 0x40, 0, 0]);
    let mut output = [0; 4];
    decompress_xpress(&input, &mut output).unwrap();
    assert_eq!(output, *b"AAAA");
}

#[test]
fn overlapping_offset_two_match_repeats_two_literals() {
    let mut input = header(&[(65, 1), (66, 2), (273, 2)]);
    input.extend_from_slice(&[0, 0x58, 0, 0]); // A=0, B=10, match=11, offset extra=0
    let mut output = [0; 6];
    decompress_xpress(&input, &mut output).unwrap();
    assert_eq!(output, *b"ABABAB");
}

#[test]
fn nonoverlapping_offset_three_match_copies_prefix() {
    let mut input = header(&[(65, 2), (66, 2), (67, 2), (272, 2)]);
    input.extend_from_slice(&[0x80, 0x1b, 0, 0]); // 00 01 10 11 1
    let mut output = [0; 6];
    decompress_xpress(&input, &mut output).unwrap();
    assert_eq!(output, *b"ABCABC");
}

#[test]
fn byte_length_extension_is_after_prefetched_coding_words() {
    let input = two_symbols(271, &[0, 0x40, 0, 0, 7]);
    let mut output = [0; 26]; // literal + (15 + 7 + 3)
    decompress_xpress(&input, &mut output).unwrap();
    assert_eq!(output, [b'A'; 26]);
}

#[test]
fn u16_length_extension_replaces_both_short_length_fields() {
    let input = two_symbols(271, &[0, 0x40, 0, 0, 255, 0x2c, 0x01]);
    let mut output = [0; 304]; // literal + (300 + 3)
    decompress_xpress(&input, &mut output).unwrap();
    assert_eq!(output, [b'A'; 304]);
}

#[test]
fn maximum_u16_length_extension_is_bounded_by_output() {
    let input = two_symbols(271, &[0, 0x40, 0, 0, 255, 255, 255]);
    let mut output = vec![0; 65539];
    decompress_xpress(&input, &mut output).unwrap();
    assert_eq!(output, vec![b'A'; 65539]);
}

#[test]
fn missing_length_extension_byte_is_zero_padded() {
    let input = two_symbols(271, &[0, 0x40]);
    let mut output = [0; 19];
    decompress_xpress(&input, &mut output).unwrap();
    assert_eq!(output, [b'A'; 19]);
}

#[test]
fn missing_u16_length_extension_is_zero_padded() {
    let input = two_symbols(271, &[0, 0x40, 0, 0, 255, 0x99]);
    let mut output = [0; 4];
    decompress_xpress(&input, &mut output).unwrap();
    assert_eq!(output, *b"AAAA");
}

#[test]
fn empty_huffman_table_decodes_zero_literals_without_consuming_bits() {
    let mut output = [99; 64];
    decompress_xpress(&[0; 256], &mut output).unwrap();
    assert_eq!(output, [0; 64]);
}

#[test]
fn incomplete_nonempty_huffman_code_is_rejected() {
    assert_eq!(
        decompress_xpress(&header(&[(65, 1)]), &mut [0]),
        Err(DecodeError::InvalidHuffmanCode)
    );
}

#[test]
fn oversubscribed_huffman_code_is_rejected_even_for_empty_output() {
    assert_eq!(
        decompress_xpress(&header(&[(65, 1), (66, 1), (67, 1)]), &mut []),
        Err(DecodeError::InvalidHuffmanCode)
    );
}

#[test]
fn complete_header_is_required_even_for_empty_output() {
    assert_eq!(
        decompress_xpress(&[0; 255], &mut []),
        Err(DecodeError::HeaderTooShort)
    );
}

#[test]
fn empty_output_accepts_empty_huffman_code() {
    assert_eq!(decompress_xpress(&[0; 256], &mut []), Ok(()));
}

#[test]
fn match_before_any_literals_is_rejected() {
    let input = two_symbols(256, &[0, 0x80]);
    assert_eq!(
        decompress_xpress(&input, &mut [0; 3]),
        Err(DecodeError::InvalidMatchOffset {
            offset: 1,
            produced: 0
        })
    );
}

#[test]
fn match_past_output_end_is_rejected() {
    let input = two_symbols(256, &[0, 0x40]);
    assert_eq!(
        decompress_xpress(&input, &mut [0; 3]),
        Err(DecodeError::MatchExceedsOutput {
            length: 3,
            remaining: 2
        })
    );
}

#[test]
fn incomplete_final_coding_word_is_ignored_like_original() {
    let mut input = header(&[(65, 1), (66, 1)]);
    input.push(255);
    let mut output = [0; 2];
    decompress_xpress(&input, &mut output).unwrap();
    assert_eq!(output, *b"AA");
}

#[test]
fn maximum_fifteen_bit_codeword_decodes_correctly() {
    let lengths: Vec<_> = (0..14)
        .map(|symbol| (symbol, (symbol + 1) as u8))
        .chain([(14, 15), (15, 15)])
        .collect();
    let mut input = header(&lengths);
    input.extend_from_slice(&[0xfe, 0xff]);
    let mut output = [0];
    decompress_xpress(&input, &mut output).unwrap();
    assert_eq!(output, [15]);
}

#[test]
fn trailing_data_is_not_required_to_be_end_marker() {
    let mut input = header(&[(65, 1), (66, 1)]);
    input.extend_from_slice(&[0, 0, 255, 255, 254, 253]);
    let mut output = [0; 1];
    decompress_xpress(&input, &mut output).unwrap();
    assert_eq!(output, *b"A");
}
