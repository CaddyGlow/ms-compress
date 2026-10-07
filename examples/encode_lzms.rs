//! Native LZMS encoder used by the independent C differential harness.
fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args: Vec<_> = std::env::args().collect();
    if args.len() != 4 {
        return Err("usage: encode_lzms INPUT OUTPUT CAPACITY".into());
    }
    let input = std::fs::read(&args[1])?;
    let compressed =
        ms_compress::lzms::encode::compress_lzms(&input, args[3].parse()?)?.unwrap_or_default();
    std::fs::write(&args[2], compressed)?;
    Ok(())
}
