// Adapted from lzma-rust2 8fdb428469c3a11564583941f4621883c2f242f4.
mod encoder;
mod encoder_fast;
mod encoder_normal;
mod lzma2_writer;
mod lzma_writer;
mod range_enc;

pub use encoder::EncodeMode;
pub use lzma_writer::*;
pub use lzma2_writer::*;

use super::*;
