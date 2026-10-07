use ms_compress::lzms::{LzmsError, decompress_lzms};

#[test]
fn decoder_rejects_odd_sized_and_short_blocks() {
    for input in [&[][..], &[0][..], &[0, 0][..], &[0, 0, 0][..], &[0; 5][..]] {
        assert_eq!(
            decompress_lzms(input, &mut []),
            Err(LzmsError::InvalidInputSize)
        );
    }
}

#[test]
fn empty_output_still_requires_valid_word_stream_structure() {
    assert_eq!(decompress_lzms(&[0; 4], &mut []), Ok(()));
}

#[test]
fn tiny_output_alphabets_decode_literals_without_upstream_zero_symbol_crash() {
    for size in 0..=31 {
        let mut output = vec![0x99; size];
        assert_eq!(decompress_lzms(&[0; 4], &mut output), Ok(()));
        assert_eq!(output, vec![0; size]);
    }
}

#[test]
fn tiny_output_alphabets_reject_match_sources_before_output() {
    for size in 1..=31 {
        let mut output = vec![0x99; size];
        assert_eq!(
            decompress_lzms(&[0xff; 4], &mut output),
            Err(LzmsError::InvalidMatchOffset)
        );
    }
}
