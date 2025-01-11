use core::{
    borrow::{Borrow, BorrowMut},
    future::{Future, IntoFuture},
    marker::PhantomData,
    ops::AsyncFnOnce,
    pin::Pin,
};

use abs_buff::{
    x_deps::abs_sync,
    TrBuffIterRead, TrBuffSegmMut,
};
use abs_sync::cancellation::{
    NonCancellableToken, TrCancellationToken, TrIntoFutureMayCancel,
};

use crate::{ChunkIoAbort, TrChunkFiller};

pub struct BuffReadAsChunkFiller<B, R, T>
where
    B: BorrowMut<R>,
    R: TrBuffIterRead<T>,
{
    _use_r_: PhantomData<R>,
    _use_t_: PhantomData<[T]>,
    buffer_: B,
}

impl<B, R, T> BuffReadAsChunkFiller<B, R, T>
where
    B: BorrowMut<R>,
    R: TrBuffIterRead<T>,
{
    pub const fn new(read: B) -> Self {
        BuffReadAsChunkFiller {
            _use_r_: PhantomData,
            _use_t_: PhantomData,
            buffer_: read,
        }
    }

    pub fn fill_async<'a, S: TrBuffSegmMut<T>>(
        &'a mut self,
        target: &'a mut S,
    ) -> FillAsync<'a, S, B, R, T> {
        FillAsync::new(self, target)
    }
}

impl<'a, R, T> From<&'a mut R> for BuffReadAsChunkFiller<&'a mut R, R, T>
where
    R: TrBuffIterRead<T>,
{
    fn from(value: &'a mut R) -> Self {
        BuffReadAsChunkFiller::new(value)
    }
}

impl<B, R, T> TrChunkFiller<T> for BuffReadAsChunkFiller<B, R, T>
where
    B: BorrowMut<R>,
    R: TrBuffIterRead<T>,
{
    type IoAbort = ChunkIoAbort<<R as TrBuffIterRead<T>>::Err>;

    type FillAsync<'a, S> = FillAsync<'a, S, B, R, T>
    where
        S: 'a + TrBuffSegmMut<T>,
        Self: 'a;

    #[inline]
    fn fill_async<'a, S: TrBuffSegmMut<T>>(
        &'a mut self,
        target: &'a mut S,
    ) -> Self::FillAsync<'a, S>
    where
        Self: 'a,
    {
        BuffReadAsChunkFiller::fill_async(self, target)
    }
}

pub struct FillAsync<'a, S, B, R, T>
where
    S: TrBuffSegmMut<T>,
    B: BorrowMut<R>,
    R: TrBuffIterRead<T>,
{
    filler_: &'a mut BuffReadAsChunkFiller<B, R, T>,
    target_: &'a mut S,
}

impl<'a, S, B, R, T> FillAsync<'a, S, B, R, T>
where
    S: TrBuffSegmMut<T>,
    B: BorrowMut<R>,
    R: TrBuffIterRead<T>,
{
    pub fn new(
        filler: &'a mut BuffReadAsChunkFiller<B, R, T>,
        target: &'a mut S,
    ) -> Self {
        FillAsync {
            filler_: filler,
            target_: target,
        }
    }

    pub async fn may_cancel_with<C: TrCancellationToken>(
        self,
        mut cancel: Pin<&'a mut C>,
    ) -> Result<usize, ChunkIoAbort<<R as TrBuffIterRead<T>>::Err>> {
        let dst_init_len = self.target_.len();
        let mut item_count = 0usize;
        loop {
            if item_count == dst_init_len {
                break Result::Ok(item_count);
            }
            #[cfg(test)]
            log::trace!(
                "[reader_::FillAsync::may_cancel_with] \
                dst_init_len({dst_init_len}), item_count({item_count})"
            );
            let reader = self.filler_.buffer_.borrow_mut();
            let r = reader
                .read_async(dst_init_len - item_count)
                .may_cancel_with(cancel.as_mut())
                .await;
            match r {
                Result::Ok(src_iter) => {
                    for mut src in src_iter.into_iter() {
                        let opr_len = self.target_.dump_from_segm(&mut src);
                        debug_assert!(src.borrow().is_empty());
                        item_count += opr_len;
                    }
                },
                Result::Err(last_error) => {
                    break Result::Err(ChunkIoAbort::new(last_error, item_count));
                },
            }
        }
    }
}

impl<'a, C, S, B, R, T> AsyncFnOnce<(Pin<&'a mut C>,)>
for FillAsync<'a, S, B, R, T>
where
    C: TrCancellationToken,
    S: TrBuffSegmMut<T>,
    B: BorrowMut<R>,
    R: TrBuffIterRead<T>,
{
    type CallOnceFuture = impl Future<Output = Self::Output>;

    type Output = Result<usize, ChunkIoAbort<<R as TrBuffIterRead<T>>::Err>>;

    extern "rust-call" fn async_call_once(
        self,
        args: (Pin<&'a mut C>,),
    ) -> Self::CallOnceFuture {
        let cancel = args.0;
        self.may_cancel_with(cancel)
    }
}

impl<'a, S, B, R, T> IntoFuture for FillAsync<'a, S, B, R, T>
where
    S: TrBuffSegmMut<T>,
    B: BorrowMut<R>,
    R: TrBuffIterRead<T>,
{
    type Output = <Self::IntoFuture as Future>::Output;

    type IntoFuture = <Self as
        AsyncFnOnce<(Pin<&'a mut NonCancellableToken>,)>>::CallOnceFuture;

    #[inline]
    fn into_future(self) -> Self::IntoFuture {
        self(NonCancellableToken::pinned())
    }
}

impl<S, B, R, T> TrIntoFutureMayCancel for FillAsync<'_, S, B, R, T>
where
    S: TrBuffSegmMut<T>,
    B: BorrowMut<R>,
    R: TrBuffIterRead<T>,
{
    type MayCancelOutput = <Self as IntoFuture>::Output;

    #[inline]
    fn may_cancel_with<'f, C: TrCancellationToken>(
        self,
        cancel: Pin<&'f mut C>,
    ) -> impl Future<Output = Self::MayCancelOutput>
    where
        Self: 'f,
    {
        FillAsync::may_cancel_with(self, cancel)
    }
}
