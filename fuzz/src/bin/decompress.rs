fn main() {
    loop {
        honggfuzz::fuzz!(|data: &[u8]| {
            ms_compress_fuzz::decompress(data);
        });
    }
}
