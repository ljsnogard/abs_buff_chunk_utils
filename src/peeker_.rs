use core::{
    borrow::{Borrow, BorrowMut},
    future::{Future, IntoFuture},
    marker::PhantomData,
    ops::AsyncFnOnce,
    pin::Pin,
    ptr::NonNull,
    task::{Context, Poll},
};

use abs_buff::{TrBuffIterPeek, TrBuffSegmMut};
use abs_sync::cancellation::{
    NonCancellableToken, TrCancellationToken, TrMayCancel,
};
use segm_buff::x_deps::{abs_buff, abs_sync};

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

    pub fn may_cancel_with<'f, C: TrCancellationToken>(
        self,
        cancel: Pin<&'f mut C>,
    ) -> FillFuture<'f, C, S, B, P, T>
    where
        Self: 'f,
    {
        FillFuture::new(self.filler_, self.target_, cancel)
    }
}

impl<'a, S, B, P, T> IntoFuture for FillAsync<'a, S, B, P, T>
where
    S: TrBuffSegmMut<T>,
    B: BorrowMut<P>,
    P: TrBuffIterPeek<T>,
    T: Clone,
{
    type IntoFuture = FillFuture<'a, NonCancellableToken, S, B, P, T>;
    type Output = <Self::IntoFuture as Future>::Output;

    #[inline]
    fn into_future(self) -> Self::IntoFuture {
        let cancel = NonCancellableToken::pinned();
        FillFuture::new(self.filler_, self.target_, cancel)
    }
}

impl<'a, S, B, P, T> TrMayCancel<'a> for FillAsync<'a, S, B, P, T>
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
    ) -> impl IntoFuture<Output = Self::MayCancelOutput>
    where
        Self: 'f,
    {
        FillAsync::may_cancel_with(self, cancel)
    }
}

pub struct FillFuture<'a, C, S, B, P, T>
where
    C: TrCancellationToken,
    S: TrBuffSegmMut<T>,
    B: BorrowMut<P>,
    P: TrBuffIterPeek<T>,
    T: Clone,
{
    filler_: &'a mut BuffPeekAsChunkFiller<B, P, T>,
    target_: &'a mut S,
    cancel_: Pin<&'a mut C>,
    future_: Option<FutImplType<'a, C, S, B, P, T>>,
}

impl<'a, C, S, B, P, T> FillFuture<'a, C, S, B, P, T>
where
    C: TrCancellationToken,
    S: TrBuffSegmMut<T>,
    B: BorrowMut<P>,
    P: TrBuffIterPeek<T>,
    T: Clone,
{
    pub const fn new(
        filler: &'a mut BuffPeekAsChunkFiller<B, P, T>,
        target: &'a mut S,
        cancel: Pin<&'a mut C>,
    ) -> Self {
        FillFuture {
            filler_: filler,
            target_: target,
            cancel_: cancel,
            future_: Option::None,
        }
    }
}

impl<C, S, B, P, T> Future for FillFuture<'_, C, S, B, P, T>
where
    C: TrCancellationToken,
    S: TrBuffSegmMut<T>,
    B: BorrowMut<P>,
    P: TrBuffIterPeek<T>,
    T: Clone,
{
    type Output = FillResult<P, T>;

    fn poll(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
    ) -> Poll<Self::Output> {
        let mut this = unsafe {
            let p = self.as_mut().get_unchecked_mut();
            NonNull::new_unchecked(p)
        };
        loop {
            let mut p = unsafe {
                let ptr = &mut this.as_mut().future_;
                NonNull::new_unchecked(ptr)
            };
            let opt_f = unsafe { p.as_mut() };
            if let Option::Some(f) = opt_f {
                let f_pinned = unsafe { Pin::new_unchecked(f) };
                break f_pinned.poll(cx)
            } else {
                let h = FutImpl(unsafe { 
                    Pin::new_unchecked(this.as_mut())
                });
                let f = h();
                let opt = unsafe { p.as_mut() };
                *opt = Option::Some(f);
            }
        }
    }
}

unsafe impl<C, S, B, P, T> Send for FillFuture<'_, C, S, B, P, T>
where
    C: Send + Sync + TrCancellationToken,
    S: Send + Sync + TrBuffSegmMut<T>,
    B: Send + Sync + BorrowMut<P>,
    P: Send + Sync + TrBuffIterPeek<T>,
    T: Send + Sync + Clone,
{}

unsafe impl<C, S, B, P, T> Sync for FillFuture<'_, C, S, B, P, T>
where
    C: Send + Sync + TrCancellationToken,
    S: Send + Sync + TrBuffSegmMut<T>,
    B: Send + Sync + BorrowMut<P>,
    P: Send + Sync + TrBuffIterPeek<T>,
    T: Send + Sync + Clone,
{}

struct FutImpl<'a, C, S, B, P, T>(Pin<&'a mut FillFuture<'a, C, S, B, P, T>>)
where
    C: TrCancellationToken,
    S: TrBuffSegmMut<T>,
    B: BorrowMut<P>,
    P: TrBuffIterPeek<T>,
    T: Clone;

impl<C, S, B, P, T> AsyncFnOnce<()> for FutImpl<'_, C, S, B, P, T>
where
    C: TrCancellationToken,
    S: TrBuffSegmMut<T>,
    B: BorrowMut<P>,
    P: TrBuffIterPeek<T>,
    T: Clone,
{
    type CallOnceFuture = impl Future<Output = Self::Output>;
    type Output = FillResult<P, T>;

    #[inline]
    extern "rust-call" fn async_call_once(
        self,
        _: (),
    ) -> Self::CallOnceFuture {
        let future = unsafe { self.0.get_unchecked_mut() };
        Self::may_cancel_impl(
            future.filler_,
            future.target_,
            future.cancel_.as_mut(),
        )
    }
}

pub(super) type FillResult<P, T> = 
    Result<usize, ChunkIoAbort<<P as TrBuffIterPeek<T>>::Err>>;

type FutImplType<'a, C, S, B, P, T> =
    <FutImpl<'a, C, S, B, P, T> as AsyncFnOnce<()>>::CallOnceFuture;

impl<C, S, B, P, T> FutImpl<'_, C, S, B, P, T>
where
    C: TrCancellationToken,
    S: TrBuffSegmMut<T>,
    B: BorrowMut<P>,
    P: TrBuffIterPeek<T>,
    T: Clone,
{
    pub async fn may_cancel_impl<'f>(
        filler: &'f mut BuffPeekAsChunkFiller<B, P, T>,
        target: &'f mut S,
        mut cancel: Pin<&'f mut C>,
    ) -> Result<usize, ChunkIoAbort<<P as TrBuffIterPeek<T>>::Err>> {
        let buffer = filler.buffer_.borrow_mut();
        let mut item_count = 0usize;
        loop {
            let target_len = target.len();
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
                        let opr_len = target.clone_from_slice(src.borrow());
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
