use core::{
    borrow::Borrow,
    error::Error,
    mem::MaybeUninit,
    iter::IntoIterator,
};

use abs_buff::{x_deps::abs_sync, Chunk};
use abs_sync::cancellation::TrIntoFutureMayCancel;

/// To report the detail of aborted IO from a chunk filler or writer.
/// 
/// The report is about how many bytes is performed before the abortion occurs,
/// and the reason why causes the abortion.
pub trait TrChunkIoAbort {
    type LastErr: Error;
    type Remnants: ?Sized;

    /// The error that caused the abort.
    fn last_error(&self) -> &Self::LastErr;

    /// The remnants after the abort of the operation.
    fn remnants(&self) -> &Self::Remnants;
}

/// A buffer consumer (reader or peeker) that is supposed to move or copy the
/// minimum number of units (for example, bytes), from the internal buffer this 
/// reader is holding, into the target buffer.
pub trait TrChunkFiller<T = u8> {
    type IoAbort<'a>: TrChunkIoAbort
    where
        T: 'a,
        Self: 'a;

    type FillAsync<'a>: TrIntoFutureMayCancel<'a,
        MayCancelOutput = Result<usize, Self::IoAbort<'a>>>
    where
        T: 'a,
        Self: 'a;

    fn fill_async<'a>(
        &'a mut self,
        target: &'a mut [MaybeUninit<T>],
    ) -> Self::FillAsync<'a>;
}

/// A producer that is supposed to dump (move, clone, or copy) an exact number
/// of units (for example, bytes), from the source buffer, into the internal
/// storage this producer is holding.
pub trait TrChunkDumper<T = u8> {
    type IoAbort<'a>: TrChunkIoAbort
    where
        T: 'a,
        Self: 'a;

    type DumpAsync<'a, I, S>: TrIntoFutureMayCancel<'a,
        MayCancelOutput = Result<usize, Self::IoAbort<'a>>>
    where
        I: 'a + IntoIterator<Item = T>,
        S: 'a + Borrow<[T]>,
        T: 'a,
        Self: 'a;

    fn dump_async<'a, I, S>(
        &'a mut self,
        source: Chunk<I, S>,
    ) -> Self::DumpAsync<'a, I, S>
    where
        I: IntoIterator<Item = T>,
        S: Borrow<[T]>,
        T: 'a;
}

#[derive(Debug)]
pub struct ChunkIoAbort<E, T>
where
    E: Error,
    T: ?Sized,
{
    last_error_: E,
    remnants_: T,
}

impl<E, T> ChunkIoAbort<E, T>
where
    E: Error,
{
    pub const fn new(last_error: E, remnants: T) -> Self {
        ChunkIoAbort {
            last_error_: last_error,
            remnants_: remnants,
        }
    }
}

impl<E, T> ChunkIoAbort<E, T>
where
    E: Error,
    T: ?Sized,
{
    pub const fn remnants(&self) -> &T {
        &self.remnants_
    }

    pub const fn last_error(&self) -> &E {
        &self.last_error_
    }
}

impl<E, T> TrChunkIoAbort for ChunkIoAbort<E, T>
where
    E: Error,
    T: ?Sized,
{
    type LastErr = E;
    type Remnants = T;

    #[inline]
    fn last_error(&self) -> &Self::LastErr {
        ChunkIoAbort::last_error(self)
    }

    #[inline]
    fn remnants(&self) -> &Self::Remnants {
        ChunkIoAbort::remnants(self)
    }
}
