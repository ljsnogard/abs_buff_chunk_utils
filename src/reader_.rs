use core::{
    borrow::BorrowMut,
    cmp,
    future::{Future, IntoFuture},
    marker::PhantomData,
    mem::MaybeUninit,
    ops::AsyncFnOnce,
    pin::Pin,
};

use abs_buff::{x_deps::abs_sync, TrBuffIterRead};
use abs_sync::cancellation::{
    NonCancellableToken, TrCancellationToken, TrIntoFutureMayCancel,
};

use crate::{
    chunk_io_::bitwise_copy_items_,
    ChunkIoAbort, TrChunkFiller,
};

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

    pub fn fill_async<'a>(
        &'a mut self,
        target: &'a mut [MaybeUninit<T>],
    ) -> FillAsync<'a, B, R, T> {
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
    type IoAbort<'a> = ChunkIoAbort<<R as TrBuffIterRead<T>>::Err>
    where
        Self: 'a;

    type FillAsync<'a> = FillAsync<'a, B, R, T> where Self: 'a;

    #[inline]
    fn fill_async<'a>(
        &'a mut self,
        target: &'a mut [MaybeUninit<T>],
    ) -> Self::FillAsync<'a> {
        BuffReadAsChunkFiller::fill_async(self, target)
    }
}

pub struct FillAsync<'a, B, R, T>
where
    B: BorrowMut<R>,
    R: TrBuffIterRead<T>,
{
    filler_: &'a mut BuffReadAsChunkFiller<B, R, T>,
    target_: &'a mut [MaybeUninit<T>],
}

impl<'a, B, R, T> FillAsync<'a, B, R, T>
where
    B: BorrowMut<R>,
    R: TrBuffIterRead<T>,
{
    pub fn new(
        filler: &'a mut BuffReadAsChunkFiller<B, R, T>,
        target: &'a mut [MaybeUninit<T>],
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
        let buffer = self.filler_.buffer_.borrow_mut();
        let target_len = self.target_.len();
        let mut perform_len = 0usize;
        loop {
            if perform_len >= target_len {
                break Result::Ok(perform_len);
            }
            #[cfg(test)]
            log::trace!(
                "[reader_::FillAsync::fill_clone_async_] \
                target_len({target_len}), perform_len({perform_len})"
            );
            let r = buffer
                .read_async(target_len - perform_len)
                .may_cancel_with(cancel.as_mut())
                .await;
            let Result::Ok(src_iter) = r else {
                let Result::Err(last_error) = r else {
                    unreachable!("[reader_::FillAsync::fill_clone_async_]")
                };
                let remnants = target_len - perform_len;
                break Result::Err(ChunkIoAbort::new(last_error, remnants));
            };
            for src in src_iter.into_iter() {
                let src_len = src.len();
                let opr_len = cmp::min(src_len, target_len - perform_len);
                if opr_len == 0 {
                    break;
                } else {
                    #[cfg(test)]
                    log::trace!(
                        "[reader_::FillAsync::fill_clone_async_] \
                        read_async src_len({src_len}), opr_len({opr_len})"
                    );
                }
                debug_assert!(opr_len + perform_len <= target_len);
                let dst = &mut
                    self.target_[perform_len..perform_len + opr_len];
                let src = &src[..opr_len];
                unsafe { bitwise_copy_items_(src, dst) };
                perform_len += opr_len;
            }
        }
    }
}

impl<'a, C, B, R, T> AsyncFnOnce<(Pin<&'a mut C>,)>
for FillAsync<'a, B, R, T>
where
    C: TrCancellationToken,
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

impl<'a, B, R, T> IntoFuture for FillAsync<'a, B, R, T>
where
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

impl<'a, B, R, T> TrIntoFutureMayCancel<'a> for FillAsync<'a, B, R, T>
where
    B: BorrowMut<R>,
    R: TrBuffIterRead<T>,
{
    type MayCancelOutput = <Self as IntoFuture>::Output;

    #[inline]
    fn may_cancel_with<C>(
        self,
        cancel: Pin<&'a mut C>,
    ) -> impl Future<Output = Self::MayCancelOutput>
    where
        C: TrCancellationToken,
    {
        FillAsync::may_cancel_with(self, cancel)
    }
}
