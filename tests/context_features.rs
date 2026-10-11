#![cfg(any(feature = "xpress", feature = "lzx", feature = "lzms"))]

use ms_compress::context::{
    Codec, CompressionDefaults, ContextError, Decompressor, FixedStrategyCompressor,
};

#[test]
fn enabled_contexts_roundtrip_and_disabled_ids_are_rejected() {
    let input = b"selective codec context ".repeat(128);
    for (id, enabled) in [
        (1, cfg!(feature = "xpress")),
        (2, cfg!(feature = "lzx")),
        (3, cfg!(feature = "lzms")),
    ] {
        if enabled {
            let mut encoder = FixedStrategyCompressor::new(id, input.len()).unwrap();
            let mut decoder = Decompressor::new(id, input.len()).unwrap();
            let encoded = encoder.compress(&input, input.len() * 2).unwrap().unwrap();
            let mut output = vec![0; input.len()];
            decoder.decompress(&encoded, &mut output).unwrap();
            assert_eq!(output, input, "codec {id}");
        } else {
            assert_eq!(
                Codec::from_wimlib(id),
                Err(ContextError::InvalidCompressionType)
            );
            assert_eq!(
                Decompressor::new(id, input.len()).unwrap_err(),
                ContextError::InvalidCompressionType
            );
            assert_eq!(
                FixedStrategyCompressor::new(id, input.len()).unwrap_err(),
                ContextError::InvalidCompressionType
            );
            assert_eq!(
                CompressionDefaults::default().resolve(id, input.len(), 0),
                Err(ContextError::InvalidCompressionType)
            );
        }
    }
}
