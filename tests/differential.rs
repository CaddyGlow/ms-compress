use ms_compress::decompress_xpress;

struct Cases<'a> {
    remaining: &'a [u8],
}

impl<'a> Cases<'a> {
    fn bytes(&mut self, count: usize) -> &'a [u8] {
        let (value, remaining) = self.remaining.split_at(count);
        self.remaining = remaining;
        value
    }

    fn u16(&mut self) -> usize {
        u16::from_le_bytes(self.bytes(2).try_into().unwrap()) as usize
    }

    fn u32(&mut self) -> usize {
        u32::from_le_bytes(self.bytes(4).try_into().unwrap()) as usize
    }
}

#[test]
fn native_decoder_matches_original_c_on_valid_corrupted_and_truncated_blocks() {
    let mut cases = Cases {
        remaining: include_bytes!("fixtures/xpress-oracle.wxp"),
    };
    assert_eq!(cases.bytes(4), b"WXP1");
    let count = cases.u32();
    for _ in 0..count {
        let name_len = cases.u16();
        let name = std::str::from_utf8(cases.bytes(name_len)).unwrap();
        let input_len = cases.u32();
        let output_len = cases.u32();
        let success = cases.bytes(1)[0] != 0;
        let input = cases.bytes(input_len);
        let mut output = vec![0x99; output_len];
        let result = decompress_xpress(input, &mut output);
        assert_eq!(
            result.is_ok(),
            success,
            "oracle status differs: {name}: {result:?}"
        );
        if success {
            let expected = cases.bytes(output_len);
            assert_eq!(output, expected, "oracle output differs: {name}");
        }
    }
    assert!(cases.remaining.is_empty(), "unparsed fixture records");
}
