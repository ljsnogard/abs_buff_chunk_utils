use core::{
    borrow::BorrowMut,
    future::{Future, IntoFuture},
    iter::IntoIterator,
    marker::PhantomData,
    ops::AsyncFnOnce,
    pin::Pin,
};

use abs_buff::{
    x_deps::abs_sync,
    TrBuffIterWrite, TrBuffSegmMut, TrBuffSegmRef,
};
use abs_sync::cancellation::{
    NonCancellableToken, TrCancellationToken, TrIntoFutureMayCancel,
};

use crate::{ChunkIoAbort, TrChunkDumper};

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

    pub fn dump_async<'a, S: TrBuffSegmRef<T>>(
        &'a mut self,
        source: &'a mut S,
    ) -> DumpAsync<'a, S, B, W, T> {
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
    type IoAbort = ChunkIoAbort<<W as TrBuffIterWrite<T>>::Err>;

    type DumpAsync<'a, S> = DumpAsync<'a, S, B, W, T>
    where
        S: 'a + TrBuffSegmRef<T>,
        Self: 'a;

    #[inline]
    fn dump_async<'a, S: TrBuffSegmRef<T>>(
        &'a mut self,
        source: &'a mut S,
    ) -> Self::DumpAsync<'a, S>
    where
        Self: 'a,
    {
        ChunkDumper::dump_async(self, source)
    }
}

pub struct DumpAsync<'a, S, B, W, T>
where
    S: TrBuffSegmRef<T>,
    B: BorrowMut<W>,
    W: TrBuffIterWrite<T>,
{
    dumper_: &'a mut ChunkDumper<B, W, T>,
    source_: &'a mut S,
}

impl<'a, S, B, W, T> DumpAsync<'a, S, B, W, T>
where
    S: TrBuffSegmRef<T>,
    B: BorrowMut<W>,
    W: TrBuffIterWrite<T>,
{
    pub const fn new(
        dumper: &'a mut ChunkDumper<B, W, T>,
        source: &'a mut S,
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
        let src_init_len = self.source_.len();
        let mut item_count = 0usize;
        loop {
            if item_count == src_init_len {
                break Result::Ok(item_count);
            }
            #[cfg(test)]
            log::trace!(
                "[writer_::DumpAsync::may_cancel_with] \
                src_init_len({src_init_len}), item_count({item_count})"
            );
            let writer = self.dumper_.buffer_.borrow_mut();
            let res = writer
                .write_async(src_init_len - item_count)
                .may_cancel_with(cancel.as_mut())
                .await;
            match res {
                Result::Ok(dst_iter) => {
                    for mut dst_segm in dst_iter.into_iter() {
                        let c = dst_segm.dump_from_segm(self.source_);
                        item_count += c;
                    }
                },
                Result::Err(last_error) => {
                    break Result::Err(ChunkIoAbort::new(last_error, item_count));
                },
            }
        }
    }
}

impl<'a, C, S, B, W, T> AsyncFnOnce<(Pin<&'a mut C>, )>
for DumpAsync<'a, S, B, W, T>
where
    C: TrCancellationToken,
    S: TrBuffSegmRef<T>,
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

impl<'a, S, B, W, T> IntoFuture for DumpAsync<'a, S, B, W, T>
where
    S: TrBuffSegmRef<T>,
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

impl<S, B, W, T> TrIntoFutureMayCancel for DumpAsync<'_, S, B, W, T>
where
    S: TrBuffSegmRef<T>,
    B: BorrowMut<W>,
    W: TrBuffIterWrite<T>,
{
    type MayCancelOutput =
        <<Self as IntoFuture>::IntoFuture as Future>::Output;

    #[inline]
    fn may_cancel_with<'f, C: TrCancellationToken>(
        self,
        cancel: Pin<&'f mut C>,
    ) -> impl Future<Output = Self::MayCancelOutput>
    where
        Self: 'f,
    {
        DumpAsync::may_cancel_with(self, cancel)
    }
}
