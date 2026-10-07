//! Encode raw input as a WIM LZX chunk for differential tests.
fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args: Vec<_> = std::env::args().collect();
    if args.len() != 4 {
        return Err("usage: encode_lzx INPUT OUTPUT MAXIMUM".into());
    }
    let input = std::fs::read(&args[1])?;
    let compressed =
        ms_compress::lzx_encode::compress_lzx(&input, input.len() * 2 + 4096, args[3].parse()?)?
            .ok_or("no compressed output")?;
    std::fs::write(&args[2], compressed)?;
    Ok(())
}
