use core::{
    borrow::{Borrow, BorrowMut},
    mem::MaybeUninit,
    ptr,
};

pub(super) unsafe fn bitwise_copy_items_<'a, TSrc, TDst, TDat>(
    source: TSrc,
    mut target: TDst)
where
    TSrc: 'a + Borrow<[TDat]>,
    TDst: 'a + BorrowMut<[MaybeUninit<TDat>]>,
{
    let source: &[TDat] = source.borrow();
    let target: &mut [MaybeUninit<TDat>]  = target.borrow_mut();
    debug_assert_eq!(source.len(), target.len());
    if source.is_empty() {
        return;
    }
    let src = &source[0] as *const TDat;
    let dst = &mut target[0] as *mut _ as *mut TDat;
    ptr::copy_nonoverlapping(src, dst, source.len());
}
