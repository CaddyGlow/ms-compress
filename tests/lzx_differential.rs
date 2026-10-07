use ms_compress::lzx::decompress_lzx;

struct Cases<'a>(&'a [u8]);

impl<'a> Cases<'a> {
    fn bytes(&mut self, count: usize) -> &'a [u8] {
        let (value, remaining) = self.0.split_at(count);
        self.0 = remaining;
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
fn native_lzx_matches_original_c_for_all_windows_and_malformed_blocks() {
    let mut cases = Cases(include_bytes!("fixtures/lzx-oracle.bin"));
    assert_eq!(cases.bytes(4), b"LZX1");
    let count = cases.u32();
    for _ in 0..count {
        let name_size = cases.u16();
        let name = std::str::from_utf8(cases.bytes(name_size)).unwrap();
        let input_size = cases.u32();
        let output_size = cases.u32();
        let maximum = cases.u32();
        let success = cases.bytes(1)[0] != 0;
        let expected_kind = cases.bytes(1)[0];
        let expected_size = cases.u32();
        let input = cases.bytes(input_size);
        let expected = cases.bytes(expected_size);
        let mut output = vec![0x99; output_size];
        let result = decompress_lzx(input, &mut output, maximum);
        assert_eq!(
            result.is_ok(),
            success,
            "C status differs for {name}: {result:?}"
        );
        if !success {
            continue;
        }
        if expected_kind == 0 {
            assert_eq!(output, expected, "C output differs for {name}");
        } else {
            assert_eq!(expected_kind, 1, "unknown expected data encoding");
            assert!(!expected.is_empty(), "empty periodic expectation");
            for (index, actual) in output.into_iter().enumerate() {
                assert_eq!(
                    actual,
                    expected[index % expected.len()],
                    "C output differs at {index} for {name}"
                );
            }
        }
    }
    assert!(cases.0.is_empty(), "unparsed oracle records");
}
