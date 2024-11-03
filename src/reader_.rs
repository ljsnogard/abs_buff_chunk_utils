use core::{
    borrow::BorrowMut,
    cmp,
    future::{Future, IntoFuture},
    marker::PhantomData,
    pin::Pin,
    task::{Context, Poll},
};

use pin_project::pin_project;
use pin_utils::pin_mut;

use abs_buff::{x_deps::abs_sync, TrBuffIterRead};
use abs_sync::{cancellation::*, x_deps::pin_utils};

use crate::{ChunkIoAbort, TrChunkFiller};

pub struct BuffReadAsChunkFiller<B, R, T>
where
    B: BorrowMut<R>,
    R: TrBuffIterRead<T>,
    T: Clone,
{
    _use_r_: PhantomData<R>,
    _use_t_: PhantomData<[T]>,
    buffer_: B,
}

impl<B, R, T> BuffReadAsChunkFiller<B, R, T>
where
    B: BorrowMut<R>,
    R: TrBuffIterRead<T>,
    T: Clone,
{
    pub const fn new(read: B) -> Self {
        BuffReadAsChunkFiller {
            _use_r_: PhantomData,
            _use_t_: PhantomData,
            buffer_: read,
        }
    }

    pub fn fill_async<'a>(
        &'a mut self,
        target: &'a mut [T],
    ) -> BuffReadChunkFillAsync<'a, B, R, T> {
        BuffReadChunkFillAsync::new(self, target)
    }
}

impl<'a, R, T> From<&'a mut R> for BuffReadAsChunkFiller<&'a mut R, R, T>
where
    R: TrBuffIterRead<T>,
    T: Clone,
{
    fn from(value: &'a mut R) -> Self {
        BuffReadAsChunkFiller::new(value)
    }
}

impl<B, R, T> TrChunkFiller<T> for BuffReadAsChunkFiller<B, R, T>
where
    B: BorrowMut<R>,
    R: TrBuffIterRead<T>,
    T: Clone,
{
    type IoAbort = ChunkIoAbort<<R as TrBuffIterRead<T>>::Err>;
    type FillAsync<'a> = BuffReadChunkFillAsync<'a, B, R, T> where Self: 'a;

    #[inline(always)]
    fn fill_async<'a>(
        &'a mut self,
        target: &'a mut [T],
    ) -> Self::FillAsync<'a> {
        BuffReadAsChunkFiller::fill_async(self, target)
    }
}

pub struct BuffReadChunkFillAsync<'a, B, R, T>
where
    B: BorrowMut<R>,
    R: TrBuffIterRead<T>,
    T: Clone,
{
    filler_: &'a mut BuffReadAsChunkFiller<B, R, T>,
    target_: &'a mut [T],
}

impl<'a, B, R, T> BuffReadChunkFillAsync<'a, B, R, T>
where
    B: BorrowMut<R>,
    R: TrBuffIterRead<T>,
    T: Clone,
{
    pub fn new(
        filler: &'a mut BuffReadAsChunkFiller<B, R, T>,
        target: &'a mut [T],
    ) -> Self {
        BuffReadChunkFillAsync {
            filler_: filler,
            target_: target,
        }
    }

    pub fn may_cancel_with<C>(
        self,
        cancel: Pin<&'a mut C>,
    ) -> BuffReadChunkFillFuture<'a, C, B, R, T>
    where
        C: TrCancellationToken,
    {
        BuffReadChunkFillFuture::new(self.filler_, self.target_, cancel)
    }
}

impl<'a, B, R, T> IntoFuture for BuffReadChunkFillAsync<'a, B, R, T>
where
    B: BorrowMut<R>,
    R: TrBuffIterRead<T>,
    T: Clone,
{
    type IntoFuture = BuffReadChunkFillFuture<'a, NonCancellableToken, B, R, T>;
    type Output = <Self::IntoFuture as Future>::Output;

    fn into_future(self) -> Self::IntoFuture {
        let cancel = NonCancellableToken::pinned();
        BuffReadChunkFillFuture::new(self.filler_, self.target_, cancel)
    }
}

impl<'a, B, R, T> TrIntoFutureMayCancel<'a> for BuffReadChunkFillAsync<'a, B, R, T>
where
    B: BorrowMut<R>,
    R: TrBuffIterRead<T>,
    T: Clone,
{
    type MayCancelOutput = <Self as IntoFuture>::Output;

    #[inline(always)]
    fn may_cancel_with<C>(
        self,
        cancel: Pin<&'a mut C>,
    ) -> impl Future<Output = Self::MayCancelOutput>
    where
        C: TrCancellationToken,
    {
        BuffReadChunkFillAsync::may_cancel_with(self, cancel)
    }
}

#[pin_project]
pub struct BuffReadChunkFillFuture<'a, C, B, R, T>
where
    C: TrCancellationToken,
    B: BorrowMut<R>,
    R: TrBuffIterRead<T>,
    T: Clone,
{
    #[pin]filler_: &'a mut BuffReadAsChunkFiller<B, R, T>,
    #[pin]target_: &'a mut [T],
    cancel_: Pin<&'a mut C>,
}

impl<C, B, R, T> Future for BuffReadChunkFillFuture<'_, C, B, R, T>
where
    C: TrCancellationToken,
    B: BorrowMut<R>,
    R: TrBuffIterRead<T>,
    T: Clone,
{
    type Output = Result<
        usize,
        ChunkIoAbort<<R as TrBuffIterRead<T>>::Err>,
    >;

    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        let f = self.fill_async_();
        pin_mut!(f);
        f.poll(cx)
    }
}

impl<'a, C, B, R, T> BuffReadChunkFillFuture<'a, C, B, R, T>
where
    C: TrCancellationToken,
    B: BorrowMut<R>,
    R: TrBuffIterRead<T>,
    T: Clone,
{
    pub fn new(
        filler: &'a mut BuffReadAsChunkFiller<B, R, T>,
        target: &'a mut [T],
        cancel: Pin<&'a mut C>,
    ) -> Self {
        BuffReadChunkFillFuture {
            filler_: filler,
            target_: target,
            cancel_: cancel,
        }
    }

    async fn fill_async_(
        self: Pin<&mut Self>,
    ) -> Result<usize, ChunkIoAbort<<R as TrBuffIterRead<T>>::Err>> {
        let mut this = self.project();
        let mut filler = this.filler_.as_mut();
        let buffer = filler.buffer_.borrow_mut();
        let mut target = this.target_.as_mut();
        let target_len = target.len();
        let mut perform_len = 0usize;
        loop {
            if perform_len >= target_len {
                break Result::Ok(perform_len);
            }
            #[cfg(test)]
            log::trace!(
                "[BuffReadChunkFillFuture::fill_async_] \
                target_len({target_len}), perform_len({perform_len})"
            );
            let r = buffer
                .read_async(target_len - perform_len)
                .may_cancel_with(this.cancel_.as_mut())
                .await;
            let Result::Ok(src_iter) = r else {
                let Result::Err(last_error) = r else {
                    unreachable!("[ReaderFillFuture::fill_async_]")
                };
                break Result::Err(ChunkIoAbort::new(perform_len, last_error));
            };
            for src in src_iter.into_iter() {
                let src_len = src.len();
                let opr_len = cmp::min(src_len, target_len - perform_len);
                if opr_len == 0 {
                    break;
                } else {
                    #[cfg(test)]
                    log::trace!(
                        "[BuffReadChunkFillFuture::fill_async_] \
                        read_async src_len({src_len}), opr_len({opr_len})"
                    );
                }
                debug_assert!(opr_len + perform_len <= target_len);
                let dst = &mut target[perform_len..perform_len + opr_len];
                dst.clone_from_slice(&src);
                perform_len += opr_len;
            }
        }
    }
}
