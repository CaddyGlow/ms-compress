#![forbid(unsafe_code)]

// Adapted from zlib-rs 7909c0fc48f5d29f6610770e31d6f0c924f9ec3b.
#[cfg(test)]
#[allow(unused_imports)]
use alloc::{boxed::Box, format, string::String, vec, vec::Vec};

use super::flush_block;
use crate::zlib::{
    DeflateFlush,
    deflate::{BlockState, DeflateStream, fill_window},
};

pub fn deflate_huff(stream: &mut DeflateStream, flush: DeflateFlush) -> BlockState {
    loop {
        /* Make sure that we have a literal to write. */
        if stream.state.lookahead == 0 {
            fill_window(stream);

            if stream.state.lookahead == 0 {
                match flush {
                    DeflateFlush::NoFlush => return BlockState::NeedMore,
                    _ => break, /* flush the current block */
                }
            }
        }

        /* Output a literal byte */
        let state = &mut stream.state;
        let lc = state.window.filled()[state.strstart];
        let bflush = state.tally_lit(lc);
        state.lookahead -= 1;
        state.strstart += 1;
        if bflush {
            flush_block!(stream, false);
        }
    }

    stream.state.insert = 0;

    if flush == DeflateFlush::Finish {
        flush_block!(stream, true);
        return BlockState::FinishDone;
    }

    if !stream.state.sym_buf.is_empty() {
        flush_block!(stream, false);
    }

    BlockState::BlockDone
}
