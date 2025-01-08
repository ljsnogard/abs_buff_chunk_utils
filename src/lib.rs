// to enable no hand-written poll
#![feature(async_closure)]
#![feature(async_fn_traits)]
#![feature(impl_trait_in_assoc_type)]
#![feature(unboxed_closures)]

#![no_std]

mod abs_;
mod peeker_;
mod reader_;
mod writer_;

pub use abs_::{ChunkIoAbort, TrChunkFiller, TrChunkDumper, TrChunkIoAbort};
pub use peeker_::BuffPeekAsChunkFiller;
pub use reader_::BuffReadAsChunkFiller;
pub use writer_::ChunkDumper;

#[cfg(test)]
mod tests_;

pub mod x_deps {
    pub use abs_buff;
}