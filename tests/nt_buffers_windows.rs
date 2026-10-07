//! Native Windows interoperability gate; production codecs never call ntdll.
#![cfg(windows)]

use ms_compress::{lznt1, xpress_plain};

#[link(name = "ntdll")]
unsafe extern "system" {
    fn RtlGetCompressionWorkSpaceSize(format: u16, compress: *mut u32, decompress: *mut u32)
    -> i32;
    fn RtlCompressBuffer(
        format: u16,
        input: *const u8,
        input_size: u32,
        output: *mut u8,
        output_size: u32,
        chunk_size: u32,
        final_size: *mut u32,
        workspace: *mut u8,
    ) -> i32;
    fn RtlDecompressBuffer(
        format: u16,
        output: *mut u8,
        output_size: u32,
        input: *const u8,
        input_size: u32,
        final_size: *mut u32,
    ) -> i32;
}

#[test]
#[ignore = "requires native Windows ntdll; verifies both codec directions"]
fn rust_and_ntdll_buffers_interoperate() {
    for format in [2, 3] {
        let mut compress_size = 0;
        let mut decompress_size = 0;
        // SAFETY: Both size pointers refer to live u32 values.
        assert_eq!(
            unsafe {
                RtlGetCompressionWorkSpaceSize(format, &mut compress_size, &mut decompress_size)
            },
            0
        );
        // u64 storage supplies workspace alignment as well as sufficient bytes.
        let mut workspace = vec![0u64; (compress_size as usize).div_ceil(8)];
        for size in [1, 3, 32, 280, 4095, 4096, 4097, 8192, 65539, 100000] {
            let mut seed = 0xabcdef01u32;
            let random: Vec<_> = (0..size)
                .map(|_| {
                    seed ^= seed << 13;
                    seed ^= seed >> 17;
                    seed ^= seed << 5;
                    seed as u8
                })
                .collect();
            for input in [
                vec![b'a'; size],
                vec![0; size],
                random,
                (0..size).map(|i| (i % 17) as u8).collect(),
            ] {
                let encoded = if format == 2 {
                    lznt1::compress(&input)
                } else {
                    xpress_plain::compress(&input)
                }
                .unwrap();
                let mut output = vec![0; size];
                let mut written = 0;
                // SAFETY: All buffers are live, sizes match allocations, and do
                // not overlap. The sizes in this test fit into u32.
                let status = unsafe {
                    RtlDecompressBuffer(
                        format,
                        output.as_mut_ptr(),
                        size as u32,
                        encoded.as_ptr(),
                        encoded.len() as u32,
                        &mut written,
                    )
                };
                assert_eq!(status, 0, "format={format} size={size} Rust -> Windows");
                assert_eq!(written as usize, size);
                assert_eq!(output, input);
                // Give the native compressor ample block capacity even for short inputs.
                let mut native = vec![0; size * 2 + 65536];
                // SAFETY: Workspace has the requested size and alignment; input
                // and output buffers are live, sized correctly, and disjoint.
                let status = unsafe {
                    RtlCompressBuffer(
                        format,
                        input.as_ptr(),
                        size as u32,
                        native.as_mut_ptr(),
                        native.len() as u32,
                        4096,
                        &mut written,
                        workspace.as_mut_ptr().cast(),
                    )
                };
                // STATUS_BUFFER_ALL_ZEROS is an informational success: the
                // filesystem may represent this result as a sparse zero unit
                // instead of a compressed stream. Verify that exact contract.
                if status == 0x117 {
                    assert!(input.iter().all(|&byte| byte == 0));
                    continue;
                }
                assert_eq!(status, 0, "format={format} size={size} Windows -> Rust");
                native.truncate(written as usize);
                output.fill(0);
                let written = if format == 2 {
                    lznt1::decompress(&native, &mut output)
                } else {
                    xpress_plain::decompress(&native, &mut output)
                }
                .unwrap();
                assert_eq!(written, size);
                assert_eq!(output, input);
            }
        }
    }
}
