use core::error::Error;

use abs_buff::{
    x_deps::abs_sync,
    TrBuffSegmMut, TrBuffSegmRef,
};
use abs_sync::cancellation::TrIntoFutureMayCancel;

/// To report the detail of aborted IO from a chunk filler or writer.
/// 
/// The report is about how many bytes is performed before the abortion occurs,
/// and the reason why causes the abortion.
pub trait TrChunkIoAbort {
    type LastErr: Error;

    /// The error that caused the abort.
    fn last_error(&self) -> &Self::LastErr;

    /// The item count of io operation performed before error occurs.
    fn item_count(&self) -> usize;
}

/// A buffer consumer (reader or peeker) that is supposed to move or copy the
/// minimum number of units (for example, bytes), from the internal buffer this 
/// reader is holding, into the target buffer.
pub trait TrChunkFiller<T = u8> {
    type IoAbort: TrChunkIoAbort;

    type FillAsync<'a, S>: TrIntoFutureMayCancel<
        MayCancelOutput = Result<usize, Self::IoAbort>>
    where
        S: 'a + TrBuffSegmMut<T>,
        Self: 'a;

    fn fill_async<'a, S: TrBuffSegmMut<T>>(
        &'a mut self,
        target: &'a mut S,
    ) -> Self::FillAsync<'a, S>
    where
        Self: 'a;
}

/// A producer that is supposed to dump (move, clone, or copy) an exact number
/// of units (for example, bytes), from the source buffer, into the internal
/// storage this producer is holding.
pub trait TrChunkDumper<T = u8> {
    type IoAbort: TrChunkIoAbort;

    type DumpAsync<'a, S>: TrIntoFutureMayCancel<
        MayCancelOutput = Result<usize, Self::IoAbort>>
    where
        S: 'a + TrBuffSegmRef<T>,
        Self: 'a;

    fn dump_async<'a, S: TrBuffSegmRef<T>>(
        &'a mut self,
        source: &'a mut S,
    ) -> Self::DumpAsync<'a, S>
    where
        Self: 'a;
}

#[derive(Debug)]
pub struct ChunkIoAbort<E>
where
    E: Error,
{
    last_error_: E,
    item_count_: usize,
}

impl<E> ChunkIoAbort<E>
where
    E: Error,
{
    pub const fn new(last_error: E, item_count: usize) -> Self {
        ChunkIoAbort {
            last_error_: last_error,
            item_count_: item_count,
        }
    }
}

impl<E> ChunkIoAbort<E>
where
    E: Error,
{
    pub const fn item_count(&self) -> usize {
        self.item_count_
    }

    pub const fn last_error(&self) -> &E {
        &self.last_error_
    }
}

impl<E> TrChunkIoAbort for ChunkIoAbort<E>
where
    E: Error,
{
    type LastErr = E;

    #[inline]
    fn last_error(&self) -> &Self::LastErr {
        ChunkIoAbort::last_error(self)
    }

    #[inline]
    fn item_count(&self) -> usize {
        ChunkIoAbort::item_count(self)
    }
}
