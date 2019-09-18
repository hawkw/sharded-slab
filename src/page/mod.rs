use crate::{Pack, Tid};

mod global;
pub(crate) mod slot;
use self::slot::Slot;

#[repr(transparent)]
#[derive(Debug, Copy, Clone, PartialEq, Eq)]
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
    const NULL: Self = Self(std::usize::MAX);
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

pub struct Page<T> {
    global: global::Stack,
    head: Offset,
    tail: Offset,
    slab: Box<[Slot<T>]>,
}

impl<T> Page<T> {
    pub(crate) fn new(size: usize) -> Self {
        let mut slab = Vec::with_capacity(size);
        slab.extend((1..size + 1).map(Slot::new));
        Self {
            global: global::Stack::new(),
            head: Offset::from_usize(0),
            tail: Offset::NULL,
            slab: slab.into_boxed_slice(),
        }
    }

    pub(crate) fn insert(&mut self, t: &mut Option<T>) -> Option<usize> {
        let head = self.head;
        println!("-> {:?}", head);
        if head.as_usize() <= self.slab.len() {
            // free slots remaining
            let slot = &mut self.slab[head.as_usize()];
            let gen = slot.insert(t);
            let next = slot.next();
            self.head = next;
            return Some(gen.pack(head.pack(0)));
        }

        unimplemented!("pop global free list")
    }

    pub(crate) fn get(&self, idx: usize) -> Option<&T> {
        let gen = slot::Generation::unpack(idx);
        let off = Offset::unpack(idx);
        println!("-> {:?}; {:?}", off, gen);
        self.slab[off.as_usize()].get(gen)
    }

    pub(crate) fn remove_local(&mut self, idx: usize) {
        debug_assert_eq!(Tid::unpack(idx), Tid::current());
        let offset = Offset::unpack(idx);
        let gen = slot::Generation::unpack(idx);

        self.slab[offset.0].remove(gen, self.head.as_usize());
        self.head = offset;

        if self.tail == Offset::NULL {
            self.tail = offset;
        }
    }

    pub(crate) fn remove_remote(&self, idx: usize) {
        unimplemented!()
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
            assert_eq!(pidx, Index::unpack(packed));
        }

        #[test]
        fn poff_roundtrips(poff in 0usize..Offset::BITS) {
            let poff = Offset::from_usize(poff);
            let packed = poff.pack(0);
            assert_eq!(poff, Offset::unpack(packed));
        }

        #[test]
        fn gen_roundtrips(gen in 0usize..slot::Generation::BITS) {
            let gen = slot::Generation::from_usize(gen);
            let packed = gen.pack(0);
            assert_eq!(gen, slot::Generation::unpack(packed));
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
            assert_eq!(poff, Offset::unpack(packed));
            assert_eq!(pidx, Index::unpack(packed));
            assert_eq!(gen, slot::Generation::unpack(packed));
        }
    }
}
