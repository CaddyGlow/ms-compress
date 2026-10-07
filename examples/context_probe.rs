//! Print configuration statuses for original-library differential checks.
use ms_compress::context::{CompressionDefaults, ContextError, Decompressor};

fn status<T>(value: Result<T, ContextError>) -> i32 {
    match value {
        Ok(_) => 0,
        Err(ContextError::InvalidCompressionType) => 16,
        Err(ContextError::InvalidParameter) => 24,
        Err(_) => 8,
    }
}
fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut args = std::env::args().skip(1);
    let codec: i32 = args.next().ok_or("codec")?.parse()?;
    let maximum: usize = args.next().ok_or("maximum")?.parse()?;
    let level: u32 = args.next().ok_or("level")?.parse()?;
    println!(
        "{} {}",
        status(Decompressor::new(codec, maximum)),
        status(CompressionDefaults::default().resolve(codec, maximum, level))
    );
    Ok(())
}
