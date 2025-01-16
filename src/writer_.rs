use core::{
    borrow::BorrowMut,
    future::{Future, IntoFuture},
    iter::IntoIterator,
    marker::PhantomData,
    ops::AsyncFnOnce,
    pin::Pin,
    ptr::NonNull,
    task::{Context, Poll},
};

use abs_buff::{TrBuffIterWrite, TrBuffSegmMut, TrBuffSegmRef};
use abs_sync::cancellation::{
    NonCancellableToken, TrCancellationToken, TrMayCancel,
};
use segm_buff::x_deps::{abs_buff, abs_sync};

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

    pub fn may_cancel_with<'f, C: TrCancellationToken>(
        self,
        cancel: Pin<&'f mut C>,
    ) -> impl IntoFuture<Output = DumpResult<W, T>>
    where
        Self: 'f,
    {
        DumpFuture::new(self.dumper_, self.source_, cancel)
    }
}

impl<'a, S, B, W, T> IntoFuture for DumpAsync<'a, S, B, W, T>
where
    S: TrBuffSegmRef<T>,
    B: BorrowMut<W>,
    W: TrBuffIterWrite<T>,
{
    type Output = <Self::IntoFuture as Future>::Output;
    type IntoFuture = DumpFuture<'a, NonCancellableToken, S, B, W, T>;

    #[inline]
    fn into_future(self) -> Self::IntoFuture {
        let cancel = NonCancellableToken::pinned();
        DumpFuture::new(self.dumper_, self.source_, cancel)
    }
}

impl<'a, S, B, W, T> TrMayCancel<'a> for DumpAsync<'a, S, B, W, T>
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
    ) -> impl IntoFuture<Output = Self::MayCancelOutput>
    where
        Self: 'f,
    {
        DumpAsync::may_cancel_with(self, cancel)
    }
}

pub struct DumpFuture<'a, C, S, B, W, T>
where
    C: TrCancellationToken,
    S: TrBuffSegmRef<T>,
    B: BorrowMut<W>,
    W: TrBuffIterWrite<T>,
{
    dumper_: &'a mut ChunkDumper<B, W, T>,
    source_: &'a mut S,
    cancel_: Pin<&'a mut C>,
    future_: Option<FutImplType<'a, C, S, B, W, T>>,
}

impl<'a, C, S, B, W, T> DumpFuture<'a, C, S, B, W, T>
where
    C: TrCancellationToken,
    S: TrBuffSegmRef<T>,
    B: BorrowMut<W>,
    W: TrBuffIterWrite<T>,
{
    pub const fn new(
        dumper: &'a mut ChunkDumper<B, W, T>,
        source: &'a mut S,
        cancel: Pin<&'a mut C>,
    ) -> Self {
        DumpFuture {
            dumper_: dumper,
            source_: source,
            cancel_: cancel,
            future_: Option::None,
        }
    }
}

impl<C, S, B, W, T> Future for DumpFuture<'_, C, S, B, W, T>
where
    C: TrCancellationToken,
    S: TrBuffSegmRef<T>,
    B: BorrowMut<W>,
    W: TrBuffIterWrite<T>,
{
    type Output = DumpResult<W, T>;

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

unsafe impl<C, S, B, W, T> Send for DumpFuture<'_, C, S, B, W, T>
where
    C: Send + Sync + TrCancellationToken,
    S: Send + Sync + TrBuffSegmRef<T>,
    B: Send + Sync + BorrowMut<W>,
    W: Send + Sync + TrBuffIterWrite<T>,
{}

unsafe impl<C, S, B, W, T> Sync for DumpFuture<'_, C, S, B, W, T>
where
    C: Send + Sync + TrCancellationToken,
    S: Send + Sync + TrBuffSegmRef<T>,
    B: Send + Sync + BorrowMut<W>,
    W: Send + Sync + TrBuffIterWrite<T>,
{}

struct FutImpl<'a, C, S, B, W, T>(Pin<&'a mut DumpFuture<'a, C, S, B, W, T>>)
where
    C: TrCancellationToken,
    S: TrBuffSegmRef<T>,
    B: BorrowMut<W>,
    W: TrBuffIterWrite<T>;

impl<C, S, B, W, T> AsyncFnOnce<()> for FutImpl<'_, C, S, B, W, T>
where
    C: TrCancellationToken,
    S: TrBuffSegmRef<T>,
    B: BorrowMut<W>,
    W: TrBuffIterWrite<T>,
{
    type CallOnceFuture = impl Future<Output = DumpResult<W, T>>;
    type Output = <Self::CallOnceFuture as Future>::Output;

    extern "rust-call" fn async_call_once(
        self,
        _: (),
    ) -> Self::CallOnceFuture {
        let f = unsafe { self.0.get_unchecked_mut() };
        Self::may_cancel_impl(f.dumper_, f.source_, f.cancel_.as_mut())
    }
}

pub(super) type DumpResult<W, T> =
    Result<usize, ChunkIoAbort<<W as TrBuffIterWrite<T>>::Err>>;

type FutImplType<'a, C, S, B, W, T> =
    <FutImpl<'a, C, S, B, W, T> as AsyncFnOnce<()>>::CallOnceFuture;

impl<C, S, B, W, T> FutImpl<'_, C, S, B, W, T>
where
    C: TrCancellationToken,
    S: TrBuffSegmRef<T>,
    B: BorrowMut<W>,
    W: TrBuffIterWrite<T>,
{
    pub async fn may_cancel_impl<'f>(
        dumper: &'f mut ChunkDumper<B, W, T>,
        source: &'f mut S,
        mut cancel: Pin<&'f mut C>,
    ) -> DumpResult<W, T> {
        let src_init_len = source.len();
        let mut item_count = 0usize;
        loop {
            if item_count == src_init_len {
                break Result::Ok(item_count);
            }
            #[cfg(test)]
            log::trace!(
                "[writer_::FutImpl::may_cancel_impl] \
                src_init_len({src_init_len}), item_count({item_count})"
            );
            let writer = dumper.buffer_.borrow_mut();
            let res = writer
                .write_async(src_init_len - item_count)
                .may_cancel_with(cancel.as_mut())
                .await;
            match res {
                Result::Ok(dst_iter) => {
                    for mut dst_segm in dst_iter.into_iter() {
                        let c = dst_segm.dump_from_segm(source);
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
