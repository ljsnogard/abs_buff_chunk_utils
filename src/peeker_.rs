use core::{
    borrow::{Borrow, BorrowMut},
    future::{Future, IntoFuture},
    marker::PhantomData,
    ops::AsyncFnOnce,
    pin::Pin,
};

use abs_buff::{
    x_deps::abs_sync,
    TrBuffIterPeek, TrBuffSegmMut,
};
use abs_sync::cancellation::{
    NonCancellableToken, TrCancellationToken, TrIntoFutureMayCancel,
};

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

    pub fn fill_async<'a, S: TrBuffSegmMut<T>>(
        &'a mut self,
        target: &'a mut S,
    ) -> FillAsync<'a, S, B, P, T> {
        FillAsync::new(self, target)
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
    type IoAbort = ChunkIoAbort<<P as TrBuffIterPeek<T>>::Err>;

    type FillAsync<'a, S> = FillAsync<'a, S, B, P, T>
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
        BuffPeekAsChunkFiller::fill_async(self, target)
    }
}

pub struct FillAsync<'a, S, B, P, T>
where
    S: TrBuffSegmMut<T>,
    B: BorrowMut<P>,
    P: TrBuffIterPeek<T>,
    T: Clone,
{
    filler_: &'a mut BuffPeekAsChunkFiller<B, P, T>,
    target_: &'a mut S,
}

impl<'a, S, B, P, T> FillAsync<'a, S, B, P, T>
where
    S: TrBuffSegmMut<T>,
    B: BorrowMut<P>,
    P: TrBuffIterPeek<T>,
    T: Clone,
{
    pub fn new(
        filler: &'a mut BuffPeekAsChunkFiller<B, P, T>,
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
    ) -> Result<usize, ChunkIoAbort<<P as TrBuffIterPeek<T>>::Err>> {
        let buffer = self.filler_.buffer_.borrow_mut();
        let mut item_count = 0usize;
        loop {
            let target_len = self.target_.len();
            if item_count >= target_len {
                break Result::Ok(item_count);
            }
            let r = buffer
                .peek_async()
                .may_cancel_with(cancel.as_mut())
                .await;
            match r {
                Result::Ok(src_iter) => {
                    for src in src_iter.into_iter() {
                        let opr_len = self.target_.clone_from_slice(src.borrow());
                        item_count += opr_len;
                    }
                },
                Result::Err(last_error) => {
                    break Result::Err(ChunkIoAbort::new(last_error, item_count));
                }
            }
        }
    }
}

impl<'a, C, S, B, P, T> AsyncFnOnce<(Pin<&'a mut C>,)>
for FillAsync<'a, S, B, P, T>
where
    C: TrCancellationToken,
    S: TrBuffSegmMut<T>,
    B: BorrowMut<P>,
    P: TrBuffIterPeek<T>,
    T: Clone,
{
    type CallOnceFuture = impl Future<Output = Self::Output>;
    type Output = Result<usize, ChunkIoAbort<<P as TrBuffIterPeek<T>>::Err>>;

    #[inline]
    extern "rust-call" fn async_call_once(
        self,
        args: (Pin<&'a mut C>,),
    ) -> Self::CallOnceFuture {
        let cancel = args.0;
        self.may_cancel_with(cancel)
    }
}


impl<'a, S, B, P, T> IntoFuture for FillAsync<'a, S, B, P, T>
where
    S: TrBuffSegmMut<T>,
    B: BorrowMut<P>,
    P: TrBuffIterPeek<T>,
    T: Clone,
{
    type IntoFuture = <Self as
        AsyncFnOnce<(Pin<&'a mut NonCancellableToken>,)>>::CallOnceFuture;

    type Output = <Self::IntoFuture as Future>::Output;

    #[inline]
    fn into_future(self) -> Self::IntoFuture {
        self(NonCancellableToken::pinned())
    }
}

impl<S, B, P, T> TrIntoFutureMayCancel for FillAsync<'_, S, B, P, T>
where
    S: TrBuffSegmMut<T>,
    B: BorrowMut<P>,
    P: TrBuffIterPeek<T>,
    T: Clone,
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
