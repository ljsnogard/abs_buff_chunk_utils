use core::{
    borrow::{Borrow, BorrowMut},
    future::{Future, IntoFuture},
    marker::PhantomData,
    ops::AsyncFnOnce,
    pin::Pin,
    ptr::NonNull,
    task::{Context, Poll},
};

use abs_buff::{TrBuffIterRead, TrBuffSegmMut};
use abs_sync::cancellation::{
    NonCancellableToken, TrCancellationToken, TrMayCancel,
};
use segm_buff::x_deps::{abs_buff, abs_sync};

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

    pub fn may_cancel_with<'f, C: TrCancellationToken>(
        self,
        cancel: Pin<&'f mut C>,
    ) -> FillFuture<'f, C, S, B, R, T>
    where
        Self: 'f,
    {
        FillFuture::new(self.filler_, self.target_, cancel)
    }
}

impl<'a, S, B, R, T> IntoFuture for FillAsync<'a, S, B, R, T>
where
    S: TrBuffSegmMut<T>,
    B: BorrowMut<R>,
    R: TrBuffIterRead<T>,
{
    type Output = <Self::IntoFuture as Future>::Output;
    type IntoFuture = FillFuture<'a, NonCancellableToken, S, B, R, T>;

    #[inline]
    fn into_future(self) -> Self::IntoFuture {
        let cancel = NonCancellableToken::pinned();
        FillFuture::new(self.filler_, self.target_, cancel)
    }
}

impl<'a, S, B, R, T> TrMayCancel<'a> for FillAsync<'a, S, B, R, T>
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
    ) -> impl IntoFuture<Output = Self::MayCancelOutput>
    where
        Self: 'f,
    {
        FillAsync::may_cancel_with(self, cancel)
    }
}

pub struct FillFuture<'a, C, S, B, R, T>
where
    C: TrCancellationToken,
    S: TrBuffSegmMut<T>,
    B: BorrowMut<R>,
    R: TrBuffIterRead<T>,
{
    filler_: &'a mut BuffReadAsChunkFiller<B, R, T>,
    target_: &'a mut S,
    cancel_: Pin<&'a mut C>,
    future_: Option<FutImplType<'a, C, S, B, R, T>>,
}

impl<'a, C, S, B, R, T> FillFuture<'a, C, S, B, R, T>
where
    C: TrCancellationToken,
    S: TrBuffSegmMut<T>,
    B: BorrowMut<R>,
    R: TrBuffIterRead<T>,
{
    pub const fn new(
        filler: &'a mut BuffReadAsChunkFiller<B, R, T>,
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

impl<C, S, B, R, T> Future for FillFuture<'_, C, S, B, R, T>
where
    C: TrCancellationToken,
    S: TrBuffSegmMut<T>,
    B: BorrowMut<R>,
    R: TrBuffIterRead<T>,
{
    type Output = FillResult<R, T>;

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

unsafe impl<C, S, B, R, T> Send for FillFuture<'_, C, S, B, R, T>
where
    C: Send + Sync + TrCancellationToken,
    S: Send + Sync + TrBuffSegmMut<T>,
    B: Send + Sync + BorrowMut<R>,
    R: Send + Sync + TrBuffIterRead<T>,
{}

unsafe impl<C, S, B, R, T> Sync for FillFuture<'_, C, S, B, R, T>
where
    C: Send + Sync + TrCancellationToken,
    S: Send + Sync + TrBuffSegmMut<T>,
    B: Send + Sync + BorrowMut<R>,
    R: Send + Sync + TrBuffIterRead<T>,
{}

struct FutImpl<'a, C, S, B, R, T>(Pin<&'a mut FillFuture<'a, C, S, B, R, T>>)
where
    C: TrCancellationToken,
    S: TrBuffSegmMut<T>,
    B: BorrowMut<R>,
    R: TrBuffIterRead<T>;

impl<C, S, B, R, T> AsyncFnOnce<()> for FutImpl<'_, C, S, B, R, T>
where
    C: TrCancellationToken,
    S: TrBuffSegmMut<T>,
    B: BorrowMut<R>,
    R: TrBuffIterRead<T>,
{
    type CallOnceFuture = impl Future<Output = Self::Output>;
    type Output = FillResult<R, T>;

    #[inline]
    extern "rust-call" fn async_call_once(
        self,
        _: (),
    ) -> Self::CallOnceFuture {
        let f = unsafe { self.0.get_unchecked_mut() };
        Self::may_cancel_impl(f.filler_, f.target_, f.cancel_.as_mut())
    }
}

pub(super) type FillResult<R, T> = 
    Result<usize, ChunkIoAbort<<R as TrBuffIterRead<T>>::Err>>;

type FutImplType<'a, C, S, B, P, T> =
    <FutImpl<'a, C, S, B, P, T> as AsyncFnOnce<()>>::CallOnceFuture;

impl<C, S, B, R, T> FutImpl<'_, C, S, B, R, T>
where
    C: TrCancellationToken,
    S: TrBuffSegmMut<T>,
    B: BorrowMut<R>,
    R: TrBuffIterRead<T>,
{
    pub async fn may_cancel_impl<'f>(
        filler: &'f mut BuffReadAsChunkFiller<B, R, T>,
        target: &'f mut S,
        mut cancel: Pin<&'f mut C>,
    ) -> FillResult<R, T> {
        let dst_init_len = target.len();
        let mut item_count = 0usize;
        loop {
            if item_count == dst_init_len {
                break Result::Ok(item_count);
            }
            #[cfg(test)]
            log::trace!(
                "[reader_::FutImpl::may_cancel_impl] \
                dst_init_len({dst_init_len}), item_count({item_count})"
            );
            let reader = filler.buffer_.borrow_mut();
            let r = reader
                .read_async(dst_init_len - item_count)
                .may_cancel_with(cancel.as_mut())
                .await;
            match r {
                Result::Ok(src_iter) => {
                    for mut src in src_iter.into_iter() {
                        let opr_len = target.dump_from_segm(&mut src);
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
