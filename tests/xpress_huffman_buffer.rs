use ms_compress::{BufferError as Error, decompress_xpress_huffman_buffer};

// Two one-bit literal codes: A = 0, B = 1. Coding words are little endian,
// with two lookahead bytes after the words containing the actual symbols.
fn literals(count: usize, byte: u8) -> Vec<u8> {
    let mut input = vec![0; 256];
    input[32] = 0x10;
    input[33] = 0x01;
    input.resize(256 + count.div_ceil(16) * 2 + 2, byte);
    input
}

#[test]
fn successive_blocks_replace_the_table_and_bit_buffer() {
    let mut input = literals(65536, 0);
    input.extend(literals(19, 255));
    let mut output = vec![0; 65555];
    assert_eq!(
        decompress_xpress_huffman_buffer(&input, &mut output),
        Ok(65555)
    );
    assert!(output[..65536].iter().all(|&b| b == b'A'));
    assert!(output[65536..].iter().all(|&b| b == b'B'));
}

#[test]
fn truncated_coding_unit_is_rejected() {
    let mut input = literals(16, 0);
    input.pop();
    assert_eq!(
        decompress_xpress_huffman_buffer(&input, &mut [0; 16]),
        Err(Error::TruncatedInput)
    );
}

#[test]
fn missing_second_table_is_rejected() {
    assert_eq!(
        decompress_xpress_huffman_buffer(&literals(65536, 0), &mut vec![0; 65537]),
        Err(Error::TruncatedInput)
    );
}

#[test]
fn empty_code_is_rejected() {
    assert_eq!(
        decompress_xpress_huffman_buffer(&[0; 260], &mut [0]),
        Err(Error::InvalidData)
    );
}

#[test]
fn match_can_cross_block_boundary_and_next_block_uses_history() {
    let mut input = vec![0; 256];
    input[32] = 0x10; // A: 0
    input[135] = 0x10; // symbol 271: 1, offset 1, extended length
    input.extend([0, 0x40, 0, 0, 255]);
    input.extend(65533u16.to_le_bytes()); // match length 65536
    let mut next = vec![0; 256];
    next[32] = 0x10; // A: 0
    next[128] = 0x01; // symbol 256: 1, offset 1, length 3
    next.extend([0, 0x80, 0, 0]);
    input.extend(next);
    let mut output = vec![0; 65540];
    assert_eq!(
        decompress_xpress_huffman_buffer(&input, &mut output),
        Ok(65540)
    );
    assert!(output.iter().all(|&b| b == b'A'));
}

#[test]
fn match_without_history_is_rejected() {
    let mut input = vec![0; 256];
    input[32] = 0x10;
    input[128] = 0x01;
    input.extend([0, 0x80, 0, 0]);
    assert_eq!(
        decompress_xpress_huffman_buffer(&input, &mut [0; 3]),
        Err(Error::InvalidOffset)
    );
}

#[test]
fn match_exceeding_output_is_rejected() {
    let mut input = vec![0; 256];
    input[32] = 0x10;
    input[128] = 0x01;
    input.extend([0, 0x40, 0, 0]);
    assert_eq!(
        decompress_xpress_huffman_buffer(&input, &mut [0; 3]),
        Err(Error::OutputTooSmall)
    );
}
