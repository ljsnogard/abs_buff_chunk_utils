use core::{
    borrow::BorrowMut,
    cmp,
    future::{Future, IntoFuture},
    marker::PhantomData,
    mem::MaybeUninit,
    ops::AsyncFnOnce,
    pin::Pin,
};

use abs_buff::{x_deps::abs_sync, TrBuffIterPeek};
use abs_sync::cancellation::{
    NonCancellableToken, TrCancellationToken, TrIntoFutureMayCancel};
use recl_slices::{NoReclaim, SliceMut};

use crate::{ChunkIoAbort, TrChunkFiller};

pub struct BuffPeekAsChunkFiller<B, P, T>
where
    B: BorrowMut<P>,
    P: TrBuffIterPeek<T>,
    T: Clone,
{
    _use_p_: PhantomData<P>,
    _use_t_: PhantomData<[T]>,
    buffer_: B,
}

impl<B, P, T> BuffPeekAsChunkFiller<B, P, T>
where
    B: BorrowMut<P>,
    P: TrBuffIterPeek<T>,
    T: Clone,
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
        target: &'a mut [MaybeUninit<T>],
    ) -> BuffPeekChunkFillAsync<'a, B, P, T> {
        BuffPeekChunkFillAsync::new(self, target)
    }
}

impl<'a, P, T> From<&'a mut P> for BuffPeekAsChunkFiller<&'a mut P, P, T>
where
    P: TrBuffIterPeek<T>,
    T: Clone,
{
    fn from(value: &'a mut P) -> Self {
        BuffPeekAsChunkFiller::new(value)
    }
}

impl<B, P, T> TrChunkFiller<T> for BuffPeekAsChunkFiller<B, P, T>
where
    B: BorrowMut<P>,
    P: TrBuffIterPeek<T>,
    T: Clone,
{
    type IoAbort<'a> = ChunkIoAbort<<P as TrBuffIterPeek<T>>::Err>
    where
        T: 'a,
        Self: 'a;

    type FillAsync<'a> = BuffPeekChunkFillAsync<'a, B, P, T>
    where
        T: 'a,
        Self: 'a;

    #[inline(always)]
    fn fill_async<'a>(
        &'a mut self,
        target: &'a mut [MaybeUninit<T>],
    ) -> Self::FillAsync<'a> {
        BuffPeekAsChunkFiller::fill_async(self, target)
    }
}

pub struct BuffPeekChunkFillAsync<'a, B, P, T>
where
    B: BorrowMut<P>,
    P: TrBuffIterPeek<T>,
    T: Clone,
{
    filler_: &'a mut BuffPeekAsChunkFiller<B, P, T>,
    target_: &'a mut [MaybeUninit<T>],
}

impl<'a, B, P, T> BuffPeekChunkFillAsync<'a, B, P, T>
where
    B: BorrowMut<P>,
    P: TrBuffIterPeek<T>,
    T: Clone,
{
    pub fn new(
        filler: &'a mut BuffPeekAsChunkFiller<B, P, T>,
        target: &'a mut [MaybeUninit<T>],
    ) -> Self {
        BuffPeekChunkFillAsync {
            filler_: filler,
            target_: target,
        }
    }

    pub async fn may_cancel_with<C: TrCancellationToken>(
        self,
        mut cancel: Pin<&'a mut C>,
    ) -> Result<usize, ChunkIoAbort<<P as TrBuffIterPeek<T>>::Err>> {
        let buffer = self.filler_.buffer_.borrow_mut();
        let target_len = self.target_.len();
        let mut perform_len = 0usize;
        loop {
            if perform_len >= target_len {
                break Result::Ok(perform_len);
            }
            let r = buffer
                .peek_async()
                .may_cancel_with(cancel.as_mut())
                .await;
            let Result::Ok(src_iter) = r else {
                let Result::Err(last_error) = r else {
                    unreachable!("[BuffPeekChunkFillAsync::may_cancel_with]")
                };
                let remnants = target_len - perform_len;
                break Result::Err(ChunkIoAbort::new(last_error, remnants));
            };
            for src in src_iter.into_iter() {
                let src_len = src.len();
                let opr_len = cmp::min(src_len, target_len - perform_len);
                if opr_len == 0 {
                    break;
                }

                #[cfg(test)]
                log::trace!(
                    "[BuffPeekChunkFillAsync::may_cancel_with] \
                    src_len({src_len}) opr_len({opr_len})");

                let dst = &mut
                    self.target_[perform_len..perform_len + opr_len];
                let mut dst = unsafe {
                    // SliceMut will properly handle the problem of dropping 
                    // items of the slice caused by transmuting from
                    // &mut [MaybeUninit<T>] to &mut [T] during overwritting.
                    SliceMut::new(
                        dst, 
                        Option::<NoReclaim>::None)
                };
                dst.clone_from_slice(&src[..opr_len]);
                perform_len += opr_len;
            }
        }
    }
}

impl<'a, C, B, P, T> AsyncFnOnce<(Pin<&'a mut C>,)>
for BuffPeekChunkFillAsync<'a, B, P, T>
where
    C: TrCancellationToken,
    B: BorrowMut<P>,
    P: TrBuffIterPeek<T>,
    T: Clone,
{
    type CallOnceFuture = impl Future<Output = Self::Output>;
    type Output =
        Result<usize, ChunkIoAbort<<P as TrBuffIterPeek<T>>::Err>>;

    #[inline]
    extern "rust-call" fn async_call_once(
        self,
        args: (Pin<&'a mut C>,),
    ) -> Self::CallOnceFuture {
        let cancel = args.0;
        self.may_cancel_with(cancel)
    }
}


impl<'a, B, P, T> IntoFuture for BuffPeekChunkFillAsync<'a, B, P, T>
where
    B: BorrowMut<P>,
    P: TrBuffIterPeek<T>,
    T: Clone,
{
    type IntoFuture = <Self as AsyncFnOnce<(Pin<&'a mut NonCancellableToken>,)>>
        ::CallOnceFuture;

    type Output = <Self::IntoFuture as Future>::Output;

    #[inline]
    fn into_future(self) -> Self::IntoFuture {
        self(NonCancellableToken::pinned())
    }
}

impl<'a, B, P, T> TrIntoFutureMayCancel<'a> for BuffPeekChunkFillAsync<'a, B, P, T>
where
    B: BorrowMut<P>,
    P: TrBuffIterPeek<T>,
    T: Clone,
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
        BuffPeekChunkFillAsync::may_cancel_with(self, cancel)
    }
}
