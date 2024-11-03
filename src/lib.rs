#![no_std]

mod abs_;
mod peeker_;
mod reader_;
mod writer_;

pub use abs_::{ChunkIoAbort, TrChunkFiller, TrChunkLoader, TrChunkIoAbort};
pub use peeker_::BuffPeekAsChunkFiller;
pub use reader_::BuffReadAsChunkFiller;
pub use writer_::BuffWriteAsChunkLoader;

pub mod x_deps {
    pub use abs_buff;
}