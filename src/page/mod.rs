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

    type Prev = ();

    fn as_usize(&self) -> usize {
        self.0
    }

    fn from_usize(val: usize) -> Self {
        debug_assert!(val <= Self::BITS);
        Self(val)
    }
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

    type Prev = Offset;

    fn as_usize(&self) -> usize {
        self.0
    }

    fn from_usize(val: usize) -> Self {
        debug_assert!(val <= Self::BITS);
        Self(val)
    }
}

#[repr(transparent)]
#[derive(Debug, Copy, Clone, PartialEq, Eq)]
pub(crate) struct Addr(usize);

impl Addr {
    const NULL: usize = Self::BITS + 1;

    pub(crate) fn index(&self) -> usize {
        64 - (self.0 + 32 >> 6).leading_zeros() as usize
    }

    pub(crate) const fn offset(&self) -> usize {
        self.0
    }
}

impl Pack for Addr {
    #[cfg(target_pointer_width = "32")]
    const LEN: usize = 16;
    #[cfg(target_pointer_width = "64")]
    const LEN: usize = 32;

    #[cfg(target_pointer_width = "32")]
    const BITS: usize = 0xFFFF;
    #[cfg(target_pointer_width = "64")]
    const BITS: usize = 0xFFFF_FFFF;

    type Prev = ();

    fn as_usize(&self) -> usize {
        self.0
    }

    fn from_usize(val: usize) -> Self {
        debug_assert!(val <= Self::BITS);
        Self(val)
    }
}

pub(crate) type Iter<'a, T> =
    std::iter::FilterMap<std::slice::Iter<'a, Slot<T>>, fn(&'a Slot<T>) -> Option<&'a T>>;

#[derive(Debug)]
pub(crate) struct Page<T> {
    prev_sz: usize,
    remote_head: AtomicUsize,
    local_head: usize,
    slab: Box<[Slot<T>]>,
}

impl<T> Page<T> {
    pub(crate) fn new(size: usize, prev_sz: usize) -> Self {
        let mut slab = Vec::with_capacity(size);
        slab.extend((1..size).map(Slot::new));
        slab.push(Slot::new(Addr::NULL));
        Self {
            prev_sz,
            remote_head: AtomicUsize::new(Addr::NULL),
            local_head: 0,
            slab: slab.into_boxed_slice(),
        }
    }

    pub(crate) fn insert(&mut self, t: &mut Option<T>) -> Option<usize> {
        let head = self.local_head;
        #[cfg(test)]
        println!("-> local head {:?}", head);
        let head = if head < self.slab.len() {
            head
        } else {
            let head = self.remote_head.swap(Addr::NULL, Ordering::Acquire);
            #[cfg(test)]
            println!("-> remote head {:?}", head);
            head
        };

        if head != Addr::NULL {
            let slot = &mut self.slab[head];
            let gen = slot.insert(t);
            self.local_head = slot.next();
            let index = head + self.prev_sz;
            #[cfg(test)]
            println!("insert at offset: {}", index);
            Some(gen.pack(head + self.prev_sz))
        } else {
            None
        }
    }

    pub(crate) fn get(&self, idx: usize) -> Option<&T> {
        let poff = Addr::from_packed(idx).offset() - self.prev_sz;
        #[cfg(test)]
        println!("-> offset {:?}", poff);

        self.slab.get(poff)?.get(idx)
    }

    pub(crate) fn remove_local(&mut self, idx: usize) -> Option<T> {
        debug_assert!(Tid::from_packed(idx).is_current());
        let offset = Addr::from_packed(idx).offset() - self.prev_sz;

        #[cfg(test)]
        println!("-> offset {:?}", offset);

        let val = self.slab.get(offset)?.remove(idx, self.local_head);
        self.local_head = offset;
        val
    }

    pub(crate) fn remove_remote(&self, idx: usize) -> Option<T> {
        debug_assert!(Tid::from_packed(idx) != Tid::current());
        let offset = Addr::from_packed(idx).offset() - self.prev_sz;

        #[cfg(test)]
        println!("-> offset {:?}", offset);

        let next = self.push_remote(offset);
        #[cfg(test)]
        println!("-> next={:?}", next);

        self.slab.get(offset)?.remove(idx, next)
    }

    pub(crate) fn total_capacity(&self) -> usize {
        self.slab.len()
    }

    pub(crate) fn iter<'a>(&'a self) -> Iter<'a, T> {
        self.slab.iter().filter_map(Slot::value)
    }

    #[inline]
    fn push_remote(&self, offset: usize) -> usize {
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

// impl<T, P: Unpack<Offset>> ops::Index<P> for Page<T> {
//     type Output = Slot<T>;
//     #[inline]
//     fn index(&self, idx: P) -> &Self::Output {
//         &self.slab[idx.unpack().as_usize()]
//     }
// }

// impl<T, P: Unpack<Offset>> ops::IndexMut<P> for Page<T> {
//     #[inline]
//     fn index_mut(&mut self, idx: P) -> &mut Self::Output {
//         &mut self.slab[idx.unpack().as_usize()]
//     }
// }

// impl fmt::Debug for Offset {
//     fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
//         if self == &Self::NULL {
//             f.debug_tuple("page::Offset")
//                 .field(&format_args!("NULL"))
//                 .finish()
//         } else {
//             f.debug_tuple("page::Offset").field(&self.0).finish()
//         }
//     }
// }

#[cfg(test)]
mod test {
    use super::*;
    use crate::Pack;
    use proptest::prelude::*;

    proptest! {
        #[test]
        fn addr_roundtrips(pidx in 0usize..Index::BITS) {
            let addr = Addr::from_usize(pidx);
            let packed = addr.pack(0);
            assert_eq!(addr, Addr::from_packed(packed));
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
            addr in 0usize..Addr::BITS,
        ) {
            let gen = slot::Generation::from_usize(gen);
            let addr = Addr::from_usize(addr);
            let packed = gen.pack(addr.pack(0));
            assert_eq!(addr, Addr::from_packed(packed));
            assert_eq!(gen, slot::Generation::from_packed(packed));
        }
    }
}
