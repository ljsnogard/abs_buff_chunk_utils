use core::error::Error;

use abs_buff::x_deps::abs_sync;
use abs_sync::cancellation::TrIntoFutureMayCancel;

/// To report the detail of aborted IO from a chunk filler or writer.
/// 
/// The report is about how many bytes is performed before the abortion occurs,
/// and the reason why causes the abortion.
pub trait TrChunkIoAbort {
    type LastErr: Error;

    /// Number of units that has been operated before aborted.
    fn perform_len(&self) -> usize;

    /// The error causes the abort.
    fn last_error(&self) -> &Self::LastErr;
}

/// A reader that is supposed to copy the minimum number of units (for example,
/// bytes), from the internal buffer this reader is holding, into the target
/// buffer.
pub trait TrChunkFiller<T = u8>
where
    T: Clone,
{
    type IoAbort: TrChunkIoAbort;

    type FillAsync<'a>: TrIntoFutureMayCancel<'a, MayCancelOutput =
        Result<usize, Self::IoAbort>>
    where
        T: 'a,
        Self: 'a;

    fn fill_async<'a>(
        &'a mut self,
        target: &'a mut [T],
    ) -> Self::FillAsync<'a>;
}

/// A writer that is supposed to copy the minimum number of units (for example, 
/// bytes), from the source buffer, into the internal buffer this writer is
/// holding.
pub trait TrChunkLoader<T = u8>
where
    T: Clone,
{
    type IoAbort: TrChunkIoAbort;

    type LoadAsync<'a>: TrIntoFutureMayCancel<'a, MayCancelOutput = 
        Result<usize, Self::IoAbort>>
    where
        T: 'a,
        Self: 'a;

    fn load_async<'a>(
        &'a mut self,
        source: &'a [T],
    ) -> Self::LoadAsync<'a>;
}

#[derive(Debug)]
pub struct ChunkIoAbort<E>
where
    E: Error,
{
    perform_len_: usize,
    last_error_: E,
}

impl<E> ChunkIoAbort<E>
where
    E: Error,
{
    pub const fn new(perform_len: usize, last_error: E) -> Self {
        ChunkIoAbort {
            perform_len_: perform_len,
            last_error_: last_error,
        }
    }

    pub const fn perform_len(&self) -> usize {
        self.perform_len_
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
    fn perform_len(&self) -> usize {
        ChunkIoAbort::perform_len(self)
    }

    #[inline]
    fn last_error(&self) -> &E {
        ChunkIoAbort::last_error(self)
    }
}
