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

use abs_buff::{x_deps::abs_sync, TrBuffIterPeek};
use abs_sync::{cancellation::*, x_deps::pin_utils};

use crate::{ChunkIoAbort, TrChunkFiller};

pub struct BuffPeekAsChunkFiller<B, P, T>
where
    B: BorrowMut<P>,
    P: TrBuffIterPeek<T>,
{
    _use_p_: PhantomData<P>,
    _use_t_: PhantomData<[T]>,
    buffer_: B,
}

impl<B, P, T> BuffPeekAsChunkFiller<B, P, T>
where
    B: BorrowMut<P>,
    P: TrBuffIterPeek<T>,
{
    pub const fn new(buffer: B) -> Self {
        BuffPeekAsChunkFiller {
            _use_p_: PhantomData,
            _use_t_: PhantomData,
            buffer_: buffer,
        }
    }

    pub fn fill_async<'a>(
        &'a mut self,
        target: &'a mut [T],
    ) -> BuffPeekChunkFillAsync<'a, B, P, T> {
        BuffPeekChunkFillAsync::new(self, target)
    }
}

impl<'a, P, T> From<&'a mut P> for BuffPeekAsChunkFiller<&'a mut P, P, T>
where
    P: TrBuffIterPeek<T>,
{
    fn from(value: &'a mut P) -> Self {
        BuffPeekAsChunkFiller::new(value)
    }
}

impl<B, P, T> TrChunkFiller<T> for BuffPeekAsChunkFiller<B, P, T>
where
    B: BorrowMut<P>,
    P: TrBuffIterPeek<T>,
{
    type IoAbort = ChunkIoAbort<<P as TrBuffIterPeek<T>>::Err>;
    type FillAsync<'a> = BuffPeekChunkFillAsync<'a, B, P, T> where Self: 'a;

    #[inline(always)]
    fn fill_async<'a>(
        &'a mut self,
        target: &'a mut [T],
    ) -> Self::FillAsync<'a> {
        BuffPeekAsChunkFiller::fill_async(self, target)
    }
}

pub struct BuffPeekChunkFillAsync<'a, B, P, T>
where
    B: BorrowMut<P>,
    P: TrBuffIterPeek<T>,
{
    filler_: &'a mut BuffPeekAsChunkFiller<B, P, T>,
    target_: &'a mut [T],
}

impl<'a, B, P, T> BuffPeekChunkFillAsync<'a, B, P, T>
where
    B: BorrowMut<P>,
    P: TrBuffIterPeek<T>,
{
    pub fn new(
        filler: &'a mut BuffPeekAsChunkFiller<B, P, T>,
        target: &'a mut [T],
    ) -> Self {
        BuffPeekChunkFillAsync {
            filler_: filler,
            target_: target,
        }
    }

    fn may_cancel_with<C>(
        self,
        cancel: Pin<&'a mut C>,
    ) -> BuffPeekChunkFillFuture<'a, C, B, P, T>
    where
        C: TrCancellationToken,
    {
        BuffPeekChunkFillFuture::new(self.filler_, self.target_, cancel)
    }
}

impl<'a, B, P, T> IntoFuture for BuffPeekChunkFillAsync<'a, B, P, T>
where
    B: BorrowMut<P>,
    P: TrBuffIterPeek<T>,
{
    type IntoFuture = BuffPeekChunkFillFuture<'a, NonCancellableToken, B, P, T>;
    type Output = <Self::IntoFuture as Future>::Output;

    fn into_future(self) -> Self::IntoFuture {
        let cancel = NonCancellableToken::pinned();
        BuffPeekChunkFillFuture::new(self.filler_, self.target_, cancel)
    }
}

impl<'a, B, P, T> TrIntoFutureMayCancel<'a> for BuffPeekChunkFillAsync<'a, B, P, T>
where
    B: BorrowMut<P>,
    P: TrBuffIterPeek<T>,
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
        BuffPeekChunkFillAsync::may_cancel_with(self, cancel)
    }
}

#[pin_project]
pub struct BuffPeekChunkFillFuture<'a, C, B, P, T>
where
    C: TrCancellationToken,
    B: BorrowMut<P>,
    P: TrBuffIterPeek<T>,
{
    #[pin]filler_: &'a mut BuffPeekAsChunkFiller<B, P, T>,
    #[pin]target_: &'a mut [T],
    cancel_: Pin<&'a mut C>,
}

impl<C, B, P, T> Future for BuffPeekChunkFillFuture<'_, C, B, P, T>
where
    C: TrCancellationToken,
    B: BorrowMut<P>,
    P: TrBuffIterPeek<T>,
{
    type Output = Result<
        usize,
        ChunkIoAbort<<P as TrBuffIterPeek<T>>::Err>,
    >;

    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        let f = self.fill_async_();
        pin_mut!(f);
        f.poll(cx)
    }
}

impl<'a, C, B, P, T> BuffPeekChunkFillFuture<'a, C, B, P, T>
where
    C: TrCancellationToken,
    B: BorrowMut<P>,
    P: TrBuffIterPeek<T>,
{
    pub fn new(
        filler: &'a mut BuffPeekAsChunkFiller<B, P, T>,
        target: &'a mut [T],
        cancel: Pin<&'a mut C>,
    ) -> Self {
        BuffPeekChunkFillFuture {
            filler_: filler,
            target_: target,
            cancel_: cancel,
        }
    }

    async fn fill_async_(
        self: Pin<&mut Self>,
    ) -> Result<usize, ChunkIoAbort<<P as TrBuffIterPeek<T>>::Err>> {
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
            let r = buffer
                .peek_async()
                .may_cancel_with(this.cancel_.as_mut())
                .await;
            let Result::Ok(src_iter) = r else {
                let Result::Err(last_error) = r else {
                    unreachable!("[BuffPeekChunkFillFuture::fill_async_]")
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
                        "[BuffPeekChunkFillFuture::fill_async_] \
                        src_len({src_len}) opr_len({opr_len})"
                    );
                }
                let dst = &mut target[perform_len..perform_len + opr_len];
                dst.clone_from_slice(&src[..opr_len]);
                perform_len += opr_len;
            }
        }
    }
}
