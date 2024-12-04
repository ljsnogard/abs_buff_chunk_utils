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

use abs_buff::{x_deps::abs_sync, TrBuffIterWrite};
use abs_sync::{cancellation::*, x_deps::pin_utils};

use crate::{ChunkIoAbort, TrChunkLoader};

pub struct BuffWriteAsChunkLoader<B, W, T>
where
    B: BorrowMut<W>,
    W: TrBuffIterWrite<T>,
{
    _use_w_: PhantomData<W>,
    _use_t_: PhantomData<[T]>,
    buffer_: B,
}

impl<B, W, T> BuffWriteAsChunkLoader<B, W, T>
where
    B: BorrowMut<W>,
    W: TrBuffIterWrite<T>,
{
    pub const fn new(buffer: B) -> Self {
        BuffWriteAsChunkLoader {
            _use_w_: PhantomData,
            _use_t_: PhantomData,
            buffer_: buffer,
        }
    }

    pub fn load_async<'a>(
        &'a mut self,
        source: &'a [T],
    ) -> BuffWriteChunkLoadAsync<'a, B, W, T> {
        BuffWriteChunkLoadAsync::new(self, source)
    }
}

impl<'a, W, T> From<&'a mut W> for BuffWriteAsChunkLoader<&'a mut W, W, T>
where
    W: TrBuffIterWrite<T>,
{
    fn from(value: &'a mut W) -> Self {
        BuffWriteAsChunkLoader::new(value)
    }
}

impl<B, W, T> TrChunkLoader<T> for BuffWriteAsChunkLoader<B, W, T>
where
    B: BorrowMut<W>,
    W: TrBuffIterWrite<T>,
{
    type IoAbort = ChunkIoAbort<<W as TrBuffIterWrite<T>>::Err>;
    type LoadAsync<'a> = BuffWriteChunkLoadAsync<'a, B, W, T> where Self: 'a;

    #[inline(always)]
    fn load_async<'a>(&'a mut self, source: &'a [T]) -> Self::LoadAsync<'a> {
        BuffWriteAsChunkLoader::load_async(self, source)
    }
}

pub struct BuffWriteChunkLoadAsync<'a, B, W, T>
where
    B: BorrowMut<W>,
    W: TrBuffIterWrite<T>,
{
    loader_: &'a mut BuffWriteAsChunkLoader<B, W, T>,
    source_: &'a [T],
}

impl<'a, B, W, T> BuffWriteChunkLoadAsync<'a, B, W, T>
where
    B: BorrowMut<W>,
    W: TrBuffIterWrite<T>,
{
    pub fn new(
        loader: &'a mut BuffWriteAsChunkLoader<B, W, T>,
        source: &'a [T],
    ) -> Self {
        BuffWriteChunkLoadAsync {
            loader_: loader,
            source_: source,
        }
    }

    pub fn may_cancel_with<C>(
        self,
        cancel: Pin<&'a mut C>,
    ) -> BuffWriteChunkLoadFuture<'a, C, B, W, T>
    where
        C: TrCancellationToken,
    {
        BuffWriteChunkLoadFuture::new(self.loader_, self.source_, cancel)
    }
}

impl<'a, B, W, T> IntoFuture for BuffWriteChunkLoadAsync<'a, B, W, T>
where
    B: BorrowMut<W>,
    W: TrBuffIterWrite<T>,
{
    type IntoFuture = BuffWriteChunkLoadFuture<'a, NonCancellableToken, B, W, T>;
    type Output = <Self::IntoFuture as Future>::Output;

    fn into_future(self) -> Self::IntoFuture {
        let cancel = NonCancellableToken::pinned();
        BuffWriteChunkLoadFuture::new(self.loader_, self.source_, cancel)
    }
}

impl<'a, B, W, T> TrIntoFutureMayCancel<'a> for BuffWriteChunkLoadAsync<'a, B, W, T>
where
    B: BorrowMut<W>,
    W: TrBuffIterWrite<T>,
{
    type MayCancelOutput = <<Self as IntoFuture>::IntoFuture as Future>::Output;

    #[inline(always)]
    fn may_cancel_with<C>(
        self,
        cancel: Pin<&'a mut C>,
    ) -> impl Future<Output = Self::MayCancelOutput>
    where
        C: TrCancellationToken,
    {
        BuffWriteChunkLoadAsync::may_cancel_with(self, cancel)
    }
}

#[pin_project]
pub struct BuffWriteChunkLoadFuture<'a, C, B, W, T>
where
    C: TrCancellationToken,
    B: BorrowMut<W>,
    W: TrBuffIterWrite<T>,
{
    #[pin]loader_: &'a mut BuffWriteAsChunkLoader<B, W, T>,
    source_: &'a [T],
    cancel_: Pin<&'a mut C>,
}

impl<C, B, W, T> Future for BuffWriteChunkLoadFuture<'_, C, B, W, T>
where
    C: TrCancellationToken,
    B: BorrowMut<W>,
    W: TrBuffIterWrite<T>,
{
    type Output = Result<usize, ChunkIoAbort<<W as TrBuffIterWrite<T>>::Err>>;

    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        let f = self.load_async_();
        pin_mut!(f);
        f.poll(cx)
    }
}

impl<'a, C, B, W, T> BuffWriteChunkLoadFuture<'a, C, B, W, T>
where
    C: TrCancellationToken,
    B: BorrowMut<W>,
    W: TrBuffIterWrite<T>,
{
    pub fn new(
        loader: &'a mut BuffWriteAsChunkLoader<B, W, T>,
        source: &'a [T],
        cancel: Pin<&'a mut C>,
    ) -> Self {
        BuffWriteChunkLoadFuture {
            loader_: loader,
            source_: source,
            cancel_: cancel,
        }
    }

    async fn load_async_(
        self: Pin<&mut Self>,
    ) -> Result<usize, ChunkIoAbort<<W as TrBuffIterWrite<T>>::Err>> {
        let mut this = self.project();
        let mut loader = this.loader_.as_mut();
        let buffer = loader.buffer_.borrow_mut();
        let source = *this.source_;
        let source_len = source.len();
        let mut perform_len = 0usize;
        loop {
            if perform_len >= source_len {
                break Result::Ok(perform_len);
            }
            #[cfg(test)]
            log::trace!(
                "[BuffWriteChunkLoadFuture::load_async_] \
                source_len({source_len}), perform_len({perform_len})"
            );
            let w = buffer
                .write_async(source_len - perform_len)
                .may_cancel_with(this.cancel_.as_mut())
                .await;
            let Result::Ok(dst_iter) = w else {
                let Result::Err(last_error) = w else {
                    unreachable!("[BuffWriteChunkLoadFuture::load_async_]")
                };
                break Result::Err(ChunkIoAbort::new(perform_len, last_error));
            };
            for mut dst in dst_iter.into_iter() {
                let dst_len = dst.len();
                let opr_len = cmp::min(dst_len, source_len - perform_len);
                debug_assert!(opr_len + perform_len <= source_len);
                let src = &source[perform_len..perform_len + opr_len];
                dst.clone_from_slice(src);
                perform_len += opr_len;
            }
        }
    }
}
