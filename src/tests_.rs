use core::{
    borrow::{Borrow, BorrowMut},
    mem::MaybeUninit,
};

use atomex::{
    x_deps::funty,
    TrCmpxchOrderings,
};
use core_malloc::CoreAlloc;
use mm_ptr::{Shared, Owned};

use spmv_oneshot::x_deps::atomex;

use buffex::ring_buffer::*;
use segm_buff::{SegmMut, SegmRef};

use crate::{BuffReadAsChunkFiller, ChunkDumper};

async fn writer_dumper_<B, P, T, O>(writer: BuffTx<B, P, T, O>)
where
    B: Borrow<RingBuffer<P, T, O>>,
    P: BorrowMut<[MaybeUninit<T>]>,
    T: funty::Unsigned + TryFrom<usize> + Copy,
    O: TrCmpxchOrderings,
{
    let capacity = writer.buffer().capacity();
    let mut dumper = ChunkDumper::new(writer);
    let mut span_length = 1usize;
    let init_each = |u, m: &mut MaybeUninit<_>| {
        let Result::Ok(x) = T::try_from(u) else { panic!() };
        m.write(x);
    };
    loop {
        if span_length > capacity {
            break;
        }
        let mut source = SegmRef::no_reclaim(Owned::new_slice(
            span_length,
            init_each,
            CoreAlloc::new(),
        ));
        let w = dumper.dump_async(&mut source).await;
        if let Result::Err(report) = w {
            log::error!("[tests_::writer_dumper_] span_len({span_length}) err: {report:?}");
            break;
        };
        let Result::Ok(copy_len) = w else { unreachable!() };
        log::trace!("[tests_::writer_dumper_] span_len({span_length}) copy len({copy_len})");
        assert_eq!(copy_len, span_length);
        span_length += 1;
    }
    log::info!("[tests_::writer_dumper_] exits");
}

async fn reader_filler_<B, P, T, O>(reader: BuffRx<B, P, T, O>)
where
    B: Borrow<RingBuffer<P, T, O>>,
    P: BorrowMut<[MaybeUninit<T>]>,
    T: funty::Unsigned + TryInto<usize> + Copy,
    O: TrCmpxchOrderings,
{
    let capacity = reader.as_ref().capacity();
    let mut filler = BuffReadAsChunkFiller::new(reader);
    let mut span_length = 1usize;
    loop {
        if span_length > capacity {
            break;
        }
        let mut target = SegmMut::no_reclaim(Owned::new_zeroed_slice(
            span_length,
            CoreAlloc::new(),
        ));
        let r = filler.fill_async(&mut target).await;
        if let Result::Err(report) = r {
            log::error!("[tests_::reader_filler_] span_len({span_length}) err: {report:?}");
            break;
        };
        let Result::Ok(fill_len) = r else { unreachable!() };
        log::trace!("[tests_::reader_filler_] span_len({span_length}) filled len({fill_len})");
        assert_eq!(fill_len, span_length);
        for (u, x) in target.iter().enumerate() {
            let v = unsafe { x.assume_init_read() };
            let Result::<usize, _>::Ok(x) = v.try_into() else {
                panic!()
            };
            assert_eq!(u, x, "#{span_length}: u({u}) != x({x})");
        }
        span_length += 1;
    }
    log::info!("[tests_::reader_filler_] exits");
}

#[tokio::test]
async fn u8_fill_copy_async_smoke() {
    const BUFF_SIZE: u8 = 255;

    let _ = env_logger::builder().is_test(true).try_init();

    let Result::Ok(ring_buff) =
        RingBuffer::<Owned<[MaybeUninit<u8>], CoreAlloc>>::try_new(
            Owned::new_zeroed_slice(
                usize::from(BUFF_SIZE),
                CoreAlloc::new(),
        ))
    else {
        panic!("[tests_::u8_fill_copy_async_smoke] try_new")
    };

    let ring_buff = Shared::new(ring_buff, CoreAlloc::new());
    let Result::Ok((writer, reader)) = RingBuffer
        ::try_split(ring_buff, Shared::strong_count, Shared::weak_count)
    else {
        panic!("[tests_::u8_fill_copy_async_smoke] try_split_shared");
    };
    let reader_handle = tokio::task::spawn(reader_filler_(reader));
    let writer_handle = tokio::task::spawn(writer_dumper_(writer));
    assert!(writer_handle.await.is_ok());
    assert!(reader_handle.await.is_ok());
}

#[tokio::test]
async fn u16_fill_copy_async_smoke() {
    const BUFF_SIZE: u16 = 255;

    let _ = env_logger::builder().is_test(true).try_init();

    let Result::Ok(ring_buff) =
        RingBuffer::<Owned<[MaybeUninit<u16>], CoreAlloc>, u16>::try_new(
            Owned::new_zeroed_slice(
                usize::from(BUFF_SIZE),
                CoreAlloc::new(),
        ))
    else {
        panic!("[tests_::u16_fill_copy_async_smoke] try_new")
    };

    let ring_buff = Shared::new(ring_buff, CoreAlloc::new());
    let Result::Ok((writer, reader)) = RingBuffer
        ::try_split(ring_buff, Shared::strong_count, Shared::weak_count)
    else {
        panic!("[tests_::u16_fill_copy_async_smoke] try_split_shared");
    };
    let writer_handle = tokio::task::spawn(writer_dumper_(writer));
    let reader_handle = tokio::task::spawn(reader_filler_(reader));
    assert!(writer_handle.await.is_ok());
    assert!(reader_handle.await.is_ok());
}

#[tokio::test]
async fn u32_fill_copy_async_smoke() {
    const BUFF_SIZE: u32 = 255;

    let _ = env_logger::builder().is_test(true).try_init();

    let Result::Ok(ring_buff) =
        RingBuffer::<Owned<[MaybeUninit<u32>], CoreAlloc>, u32>::try_new(
            Owned::new_zeroed_slice(
                usize::try_from(BUFF_SIZE).unwrap(),
                CoreAlloc::new(),
        ))
    else {
        panic!("[tests_::u32_fill_copy_async_smoke] try_new")
    };

    let ring_buff = Shared::new(ring_buff, CoreAlloc::new());
    let Result::Ok((writer, reader)) = RingBuffer
        ::try_split(ring_buff, Shared::strong_count, Shared::weak_count)
    else {
        panic!("[tests_::u32_fill_copy_async_smoke] try_split_shared");
    };
    let writer_handle = tokio::task::spawn(writer_dumper_(writer));
    let reader_handle = tokio::task::spawn(reader_filler_(reader));
    assert!(writer_handle.await.is_ok());
    assert!(reader_handle.await.is_ok());
}
