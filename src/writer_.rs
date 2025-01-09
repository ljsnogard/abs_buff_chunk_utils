use core::{
    borrow::BorrowMut,
    cmp,
    future::{Future, IntoFuture},
    iter::IntoIterator,
    marker::PhantomData,
    ops::AsyncFnOnce,
    pin::Pin,
};

use abs_buff::{x_deps::abs_sync, TrBuffIterWrite};
use abs_sync::cancellation::{
    NonCancellableToken, TrCancellationToken, TrIntoFutureMayCancel};

use crate::{
    chunk_io_::bitwise_copy_items_,
    ChunkIoAbort, TrChunkDumper,
};

pub struct ChunkDumper<B, W, T>
where
    B: BorrowMut<W>,
    W: TrBuffIterWrite<T>,
{
    _use_w_: PhantomData<W>,
    _use_t_: PhantomData<[T]>,
    buffer_: B,
}

impl<B, W, T> ChunkDumper<B, W, T>
where
    B: BorrowMut<W>,
    W: TrBuffIterWrite<T>,
{
    pub const fn new(buffer: B) -> Self {
        ChunkDumper {
            _use_w_: PhantomData,
            _use_t_: PhantomData,
            buffer_: buffer,
        }
    }

    pub fn dump_async<'a>(
        &'a mut self,
        source: &'a [T],
    ) -> DumpAsync<'a, B, W, T> {
        DumpAsync::new(self, source)
    }
}

impl<'a, W, T> From<&'a mut W> for ChunkDumper<&'a mut W, W, T>
where
    W: TrBuffIterWrite<T>,
{
    fn from(value: &'a mut W) -> Self {
        ChunkDumper::new(value)
    }
}

impl<B, W, T> TrChunkDumper<T> for ChunkDumper<B, W, T>
where
    B: BorrowMut<W>,
    W: TrBuffIterWrite<T>,
{
    type IoAbort<'a> = ChunkIoAbort<<W as TrBuffIterWrite<T>>::Err>
    where
        T: 'a,
        Self: 'a;

    type DumpAsync<'a> = DumpAsync<'a, B, W, T>
    where
        T: 'a,
        Self: 'a;

    #[inline]
    fn dump_async<'a>(
        &'a mut self,
        source: &'a [T],
    ) -> Self::DumpAsync<'a>
    where
        T: 'a,
    {
        ChunkDumper::dump_async(self, source)
    }
}

pub struct DumpAsync<'a, B, W, T>
where
    B: BorrowMut<W>,
    W: TrBuffIterWrite<T>,
{
    dumper_: &'a mut ChunkDumper<B, W, T>,
    source_: &'a [T],
}

impl<'a, B, W, T> DumpAsync<'a, B, W, T>
where
    B: BorrowMut<W>,
    W: TrBuffIterWrite<T>,
{
    pub const fn new(
        dumper: &'a mut ChunkDumper<B, W, T>,
        source: &'a [T],
    ) -> Self {
        DumpAsync {
            dumper_: dumper,
            source_: source,
        }
    }

    pub async fn may_cancel_with<C: TrCancellationToken>(
        self,
        mut cancel: Pin<&'a mut C>,
    ) -> Result<usize, ChunkIoAbort<<W as TrBuffIterWrite<T>>::Err>> {
        let target = self.dumper_.buffer_.borrow_mut();
        let source: &[T] = self.source_;
        let source_len = source.len();
        let mut perform_len = 0usize;
        loop {
            if perform_len >= source_len {
                break Result::Ok(perform_len);
            }
            #[cfg(test)]
            log::trace!(
                "[DumpAsync::dump_slice_async_] \
                source_len({source_len}), perform_len({perform_len})"
            );
            let w = target
                .write_async(source_len - perform_len)
                .may_cancel_with(cancel.as_mut())
                .await;
            let Result::Ok(dst_iter) = w else {
                let Result::Err(last_error) = w else {
                    unreachable!("[DumpAsync::dump_slice_async_]")
                };
                break Result::Err(ChunkIoAbort::new(last_error, perform_len));
            };
            for mut dst in dst_iter.into_iter() {
                let dst_len = dst.len();
                let opr_len = cmp::min(dst_len, source_len - perform_len);
                debug_assert!(opr_len + perform_len <= source_len);
                let src = &source[perform_len..perform_len + opr_len];
                let dst = &mut dst[..opr_len];
                unsafe { bitwise_copy_items_(src, dst) };
                perform_len += opr_len;
            }
        }
    }
}

impl<'a, C, B, W, T> AsyncFnOnce<(Pin<&'a mut C>, )> for DumpAsync<'a, B, W, T>
where
    C: TrCancellationToken,
    B: BorrowMut<W>,
    W: TrBuffIterWrite<T>,
{
    type CallOnceFuture = impl Future<Output = Self::Output>;
    type Output =
        Result<usize, ChunkIoAbort<<W as TrBuffIterWrite<T>>::Err>>;

    extern "rust-call" fn async_call_once(
        self,
        args: (Pin<&'a mut C>,),
    ) -> Self::CallOnceFuture {
        let cancel = args.0;
        self.may_cancel_with(cancel)
    }
}

impl<'a, B, W, T> IntoFuture for DumpAsync<'a, B, W, T>
where
    B: BorrowMut<W>,
    W: TrBuffIterWrite<T>,
{
    type Output = <Self::IntoFuture as Future>::Output;
    type IntoFuture = <Self as
        AsyncFnOnce<(Pin<&'a mut NonCancellableToken>,)>>::CallOnceFuture;

    #[inline]
    fn into_future(self) -> Self::IntoFuture {
        self(NonCancellableToken::pinned())
    }
}

impl<'a, B, W, T> TrIntoFutureMayCancel<'a> for DumpAsync<'a, B, W, T>
where
    B: BorrowMut<W>,
    W: TrBuffIterWrite<T>,
{
    type MayCancelOutput =
        <<Self as IntoFuture>::IntoFuture as Future>::Output;

    #[inline]
    fn may_cancel_with<C>(
        self,
        cancel: Pin<&'a mut C>,
    ) -> impl Future<Output = Self::MayCancelOutput>
    where
        C: TrCancellationToken,
    {
        DumpAsync::may_cancel_with(self, cancel)
    }
}
