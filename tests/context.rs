use ms_compress::context::{
    CompressionDefaults, ContextError, DESTRUCTIVE, Decompressor, FixedStrategyCompressor,
};

#[test]
fn compressor_defaults_resolve_zero_and_preserve_raw_default_bits() {
    let mut defaults = CompressionDefaults::default();
    assert_eq!(defaults.resolve(1, 65536, 0).unwrap().level, 50);
    defaults.set(-1, 87).unwrap();
    assert_eq!(defaults.resolve(2, 32768, 0).unwrap().level, 87);
    defaults.set(2, DESTRUCTIVE | 2).unwrap();
    let config = defaults.resolve(2, 32768, 0).unwrap();
    assert_eq!(config.level, DESTRUCTIVE | 2);
    assert!(!config.destructive);
    let config = defaults.resolve(2, 32768, DESTRUCTIVE | 9).unwrap();
    assert_eq!(config.level, 9);
    assert!(config.destructive);
}

#[test]
fn contexts_validate_each_codec_limit_and_numeric_type() {
    for (codec, maximum) in [(1, 65536), (2, 1 << 21), (3, 1 << 30)] {
        for valid in [1, maximum] {
            assert!(Decompressor::new(codec, valid).is_ok());
            assert!(
                CompressionDefaults::default()
                    .resolve(codec, valid, 0)
                    .is_ok()
            );
        }
        for invalid in [0, maximum + 1, usize::MAX] {
            assert_eq!(
                Decompressor::new(codec, invalid).unwrap_err(),
                ContextError::InvalidParameter
            );
            assert_eq!(
                CompressionDefaults::default().resolve(codec, invalid, 50),
                Err(ContextError::InvalidParameter)
            );
        }
    }
    for codec in [-2, -1, 0, 4, i32::MAX] {
        assert_eq!(
            Decompressor::new(codec, 0).unwrap_err(),
            ContextError::InvalidCompressionType
        );
    }
    assert_eq!(
        CompressionDefaults::default().resolve(1, 1, 0x0100_0000),
        Err(ContextError::InvalidParameter)
    );
    assert!(
        CompressionDefaults::default()
            .resolve(1, 1, DESTRUCTIVE | 0x00ff_ffff)
            .is_ok()
    );
}

#[test]
fn reused_decoder_recovers_after_oversize_and_malformed_calls() {
    let input = vec![b'a'; 32768];
    for codec in [1, 2] {
        let compressed = if codec == 1 {
            ms_compress::xpress_encode::compress_xpress(&input, 32768)
                .unwrap()
                .unwrap()
        } else {
            ms_compress::lzx_encode::compress_lzx(&input, 32768, 32768)
                .unwrap()
                .unwrap()
        };
        let mut context = Decompressor::new(codec, input.len()).unwrap();
        let mut output = vec![0; input.len() + 1];
        assert_eq!(
            context.decompress(&compressed, &mut output),
            Err(ContextError::OutputExceedsMaximum)
        );
        assert_eq!(
            context.decompress(&[], &mut output[..input.len()]),
            Err(ContextError::InvalidCompressedData)
        );
        for _ in 0..3 {
            context
                .decompress(&compressed, &mut output[..input.len()])
                .unwrap();
            assert_eq!(&output[..input.len()], input);
        }
    }
}

#[test]
fn fixed_strategy_handles_reuse_all_three_codecs_without_mutating_input() {
    let input = vec![b'a'; 32768];
    for codec in [1, 2, 3] {
        let mut compressor = FixedStrategyCompressor::new(codec, input.len()).unwrap();
        let mut decoder = Decompressor::new(codec, input.len()).unwrap();
        assert!(compressor.compress(&[], 32768).unwrap().is_none());
        assert!(compressor.compress(&input, 0).unwrap().is_none());
        assert!(
            compressor
                .compress(&vec![0; input.len() + 1], 32768)
                .unwrap()
                .is_none()
        );
        for _ in 0..3 {
            let compressed = compressor.compress(&input, 32768).unwrap().unwrap();
            let mut output = vec![0; input.len()];
            decoder.decompress(&compressed, &mut output).unwrap();
            assert_eq!(output, input);
        }
        assert!(input.iter().all(|&byte| byte == b'a'));
    }
}

#[test]
fn safe_codec_contexts_remain_send_sync_and_transfer_between_threads() {
    fn assert_send_sync<T: Send + Sync>() {}
    assert_send_sync::<ms_compress::context::Decompressor>();
    assert_send_sync::<ms_compress::context::FixedStrategyCompressor>();
    for codec in 1..=3 {
        let compressor = FixedStrategyCompressor::new(codec, 4096).unwrap();
        let mut compressor = std::thread::spawn(move || compressor).join().unwrap();
        assert!(
            compressor
                .compress_borrowed(&[b'A'; 4096], 8192)
                .unwrap()
                .is_some()
        );
        let decoder = ms_compress::context::Decompressor::new(codec, 32768).unwrap();
        let mut decoder = std::thread::spawn(move || decoder).join().unwrap();
        let _ = decoder.decompress(&[0; 4], &mut []);
    }
}
