use crate::sync::atomic::{spin_loop_hint, AtomicUsize, Ordering};
use crate::{Pack, Tid, Unpack};

pub(crate) mod slot;
use self::slot::Slot;
use std::{fmt, ops};

#[repr(transparent)]
#[derive(Copy, Clone, PartialEq, Eq)]
pub(crate) struct Offset(usize);

impl Pack for Offset {
    #[cfg(target_pointer_width = "32")]
    const BITS: usize = 0b1_1111_1111_1111_1111;
    #[cfg(target_pointer_width = "32")]
    const LEN: usize = 17;

    #[cfg(target_pointer_width = "64")]
    const BITS: usize = 0x3_FFFF_FFFF;
    #[cfg(target_pointer_width = "64")]
    const LEN: usize = 34;

    const SHIFT: usize = 0;

    fn as_usize(&self) -> usize {
        self.0
    }

    fn from_usize(val: usize) -> Self {
        debug_assert!(val <= Self::BITS);
        Self(val)
    }
}

impl Offset {
    const NULL: Self = Self(std::usize::MAX & Self::MASK);
}

#[repr(transparent)]
#[derive(Debug, Copy, Clone, PartialEq, Eq)]
pub(crate) struct Index(usize);

impl Pack for Index {
    #[cfg(target_pointer_width = "32")]
    const BITS: usize = 0b1111;
    #[cfg(target_pointer_width = "32")]
    const LEN: usize = 4;

    #[cfg(target_pointer_width = "64")]
    const BITS: usize = 0b1111_1111;
    #[cfg(target_pointer_width = "64")]
    const LEN: usize = 8;

    const SHIFT: usize = Offset::LEN;

    fn as_usize(&self) -> usize {
        self.0
    }

    fn from_usize(val: usize) -> Self {
        debug_assert!(val <= Self::BITS);
        Self(val)
    }
}

#[derive(Debug)]
pub(crate) struct Page<T> {
    remote_head: AtomicUsize,
    local_head: Offset,
    slab: Box<[Slot<T>]>,
}

impl<T> Page<T> {
    pub(crate) fn new(size: usize) -> Self {
        let mut slab = Vec::with_capacity(size);
        slab.extend((1..size).map(Slot::new));
        slab.push(Slot::new(Offset::NULL.as_usize()));
        Self {
            remote_head: AtomicUsize::new(Offset::NULL.as_usize()),
            local_head: Offset::from_usize(0),
            slab: slab.into_boxed_slice(),
        }
    }

    pub(crate) fn insert(&mut self, t: &mut Option<T>) -> Option<usize> {
        let head = self.local_head;
        #[cfg(test)]
        println!("-> local {:?}", head);
        let head = if head.as_usize() <= self.slab.len() {
            head
        } else {
            let head = self
                .remote_head
                .swap(Offset::NULL.as_usize(), Ordering::Acquire);
            let head = Offset::from_usize(head);
            #[cfg(test)]
            println!("-> remote {:?}", head);
            head
        };

        if head != Offset::NULL {
            let slot = &mut self[head];
            let gen = slot.insert(t);
            self.local_head = slot.next();
            Some(gen.pack(head.pack(0)))
        } else {
            None
        }
    }

    pub(crate) fn get(&self, idx: usize) -> Option<&T> {
        let poff = Offset::from_packed(idx);
        #[cfg(test)]
        println!("-> {:?}", poff);

        self[poff].get(idx)
    }

    pub(crate) fn remove_local(&mut self, idx: usize) -> Option<T> {
        debug_assert!(Tid::from_packed(idx).is_current());
        let offset = Offset::from_packed(idx);

        #[cfg(test)]
        println!("-> {:?}", offset);

        let val = self[offset].remove(idx, self.local_head);
        self.local_head = offset;
        val
    }

    pub(crate) fn remove_remote(&self, idx: usize) -> Option<T> {
        debug_assert!(Tid::from_packed(idx) != Tid::current());
        let offset = Offset::from_packed(idx);

        #[cfg(test)]
        println!("-> {:?}", offset);

        let next = self.push_remote(offset);
        #[cfg(test)]
        println!("-> next={:?}", next);

        self[offset].remove(idx, next)
    }

    #[inline]
    fn push_remote(&self, offset: impl Unpack<Offset>) -> usize {
        let offset = offset.unpack().as_usize();
        loop {
            let next = self.remote_head.load(Ordering::Relaxed);
            let actual = self
                .remote_head
                .compare_and_swap(next, offset, Ordering::Release);
            if actual == next {
                return next;
            }
            spin_loop_hint();
        }
    }
}

impl<T, P: Unpack<Offset>> ops::Index<P> for Page<T> {
    type Output = Slot<T>;
    #[inline]
    fn index(&self, idx: P) -> &Self::Output {
        &self.slab[idx.unpack().as_usize()]
    }
}

impl<T, P: Unpack<Offset>> ops::IndexMut<P> for Page<T> {
    #[inline]
    fn index_mut(&mut self, idx: P) -> &mut Self::Output {
        &mut self.slab[idx.unpack().as_usize()]
    }
}

impl fmt::Debug for Offset {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if self == &Self::NULL {
            f.debug_tuple("page::Offset")
                .field(&format_args!("NULL"))
                .finish()
        } else {
            f.debug_tuple("page::Offset").field(&self.0).finish()
        }
    }
}

#[cfg(test)]
mod test {
    use super::*;
    use crate::Pack;
    use proptest::prelude::*;

    proptest! {
        #[test]
        fn pidx_roundtrips(pidx in 0usize..Index::BITS) {
            let pidx = Index::from_usize(pidx);
            let packed = pidx.pack(0);
            assert_eq!(pidx, Index::from_packed(packed));
        }

        #[test]
        fn poff_roundtrips(poff in 0usize..Offset::BITS) {
            let poff = Offset::from_usize(poff);
            let packed = poff.pack(0);
            assert_eq!(poff, Offset::from_packed(packed));
        }

        #[test]
        fn gen_roundtrips(gen in 0usize..slot::Generation::BITS) {
            let gen = slot::Generation::from_usize(gen);
            let packed = gen.pack(0);
            assert_eq!(gen, slot::Generation::from_packed(packed));
        }

        #[test]
        fn page_roundtrips(
            gen in 0usize..slot::Generation::BITS,
            pidx in 0usize..Index::BITS,
            poff in 0usize..Offset::BITS,
        ) {
            let gen = slot::Generation::from_usize(gen);
            let pidx = Index::from_usize(pidx);
            let poff = Offset::from_usize(poff);
            let packed = gen.pack(pidx.pack(poff.pack(0)));
            assert_eq!(poff, Offset::from_packed(packed));
            assert_eq!(pidx, Index::from_packed(packed));
            assert_eq!(gen, slot::Generation::from_packed(packed));
        }
    }
}
