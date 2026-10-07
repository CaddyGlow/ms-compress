//! Compress a single native XPRESS block and check the native decoder.
fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args: Vec<_> = std::env::args_os().collect();
    if !(3..=4).contains(&args.len()) {
        return Err("usage: xpress_encode INPUT OUTPUT [CAPACITY]".into());
    }
    let input = std::fs::read(&args[1])?;
    let capacity = match args.get(3) {
        Some(value) => value.to_str().ok_or("non-UTF8 capacity")?.parse()?,
        None => input.len(),
    };
    match ms_compress::xpress_encode::compress_xpress(&input, capacity)? {
        Some(output) => {
            let mut decoded = vec![0; input.len()];
            ms_compress::decompress_xpress(&output, &mut decoded)?;
            if decoded != input {
                return Err("native roundtrip mismatch".into());
            }
            std::fs::write(&args[2], output)?;
        }
        None => std::fs::write(&args[2], [])?,
    }
    Ok(())
}
