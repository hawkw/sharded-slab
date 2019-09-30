use crate::cfg::{self, CfgPrivate, Unpack};
use crate::sync::atomic::{spin_loop_hint, AtomicUsize, Ordering};
use crate::{Pack, Tid};

pub(crate) mod slot;
use self::slot::Slot;
use std::{fmt, marker::PhantomData, ops};

#[repr(transparent)]
pub(crate) struct Addr<C: cfg::Params = cfg::DefaultParams> {
    addr: usize,
    _cfg: PhantomData<fn(C)>,
}

impl<C: cfg::Params> Addr<C> {
    const NULL: usize = Self::BITS + 1;

    pub(crate) fn index(&self) -> usize {
        cfg::WIDTH - (self.addr + C::INITIAL_SZ >> C::ADDR_INDEX_SHIFT).leading_zeros() as usize
    }

    pub(crate) fn offset(&self) -> usize {
        self.addr
    }
}

impl<C: cfg::Params> Pack<C> for Addr<C> {
    const LEN: usize = C::MAX_PAGES + C::ADDR_INDEX_SHIFT;
    const BITS: usize = cfg::make_mask(Self::LEN);

    type Prev = ();

    fn as_usize(&self) -> usize {
        self.addr
    }

    fn from_usize(addr: usize) -> Self {
        debug_assert!(addr <= Self::BITS);
        Self {
            addr,
            _cfg: PhantomData,
        }
    }
}

pub(crate) type Iter<'a, T, P> =
    std::iter::FilterMap<std::slice::Iter<'a, Slot<T, P>>, fn(&'a Slot<T, P>) -> Option<&'a T>>;

pub(crate) struct Page<T, P> {
    prev_sz: usize,
    remote_head: AtomicUsize,
    local_head: usize,
    slab: Box<[Slot<T, P>]>,
}

impl<T, P: cfg::Params> Page<T, P> {
    const NULL: usize = Addr::<P>::NULL;

    pub(crate) fn new(size: usize, prev_sz: usize) -> Self {
        let mut slab = Vec::with_capacity(size);
        slab.extend((1..size).map(Slot::new));
        slab.push(Slot::new(Self::NULL));
        Self {
            prev_sz,
            remote_head: AtomicUsize::new(Self::NULL),
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
            let head = self.remote_head.swap(Self::NULL, Ordering::Acquire);
            #[cfg(test)]
            println!("-> remote head {:?}", head);
            head
        };

        if head != Self::NULL {
            let slot = &mut self.slab[head];
            let gen = slot.insert(t);
            self.local_head = slot.next();
            let index = head + self.prev_sz;
            #[cfg(test)]
            println!("insert at offset: {}", index);
            Some(gen.pack(head + self.prev_sz))
        } else {
            #[cfg(test)]
            println!("-> NULL! {:?}", head);
            None
        }
    }

    pub(crate) fn get(&self, idx: usize) -> Option<&T> {
        let poff = P::unpack_addr(idx).offset() - self.prev_sz;
        #[cfg(test)]
        println!("-> offset {:?}", poff);

        self.slab.get(poff)?.get(idx)
    }

    pub(crate) fn remove_local(&mut self, idx: usize) -> Option<T> {
        debug_assert!(P::unpack_tid(idx).is_current());
        let offset = P::unpack_addr(idx).offset() - self.prev_sz;

        #[cfg(test)]
        println!("-> offset {:?}", offset);

        let val = self.slab.get(offset)?.remove(idx, self.local_head);
        self.local_head = offset;
        val
    }

    pub(crate) fn remove_remote(&self, idx: usize) -> Option<T> {
        debug_assert!(P::unpack_tid(idx) != Tid::current());
        let offset = P::unpack_addr(idx).offset() - self.prev_sz;

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

    pub(crate) fn iter<'a>(&'a self) -> Iter<'a, T, P> {
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

impl<P, T> fmt::Debug for Page<P, T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Page")
            .field(
                "remote_head",
                &format_args!("{:#0x}", &self.remote_head.load(Ordering::Relaxed)),
            )
            .field("local_head", &format_args!("{:#0x}", &self.local_head))
            .field("prev_sz", &self.prev_sz)
            .field("slab", &self.slab)
            .finish()
    }
}

impl<P: cfg::Params> fmt::Debug for Addr<P> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Addr")
            .field("addr", &format_args!("{:#0x}", &self.addr))
            .field("index", &format_args!("{:#0x}", &self.index()))
            .field("offset", &format_args!("{:#0x}", &self.offset()))
            .finish()
    }
}

impl<P: cfg::Params> PartialEq for Addr<P> {
    fn eq(&self, other: &Self) -> bool {
        self.addr == other.addr
    }
}

impl<P: cfg::Params> Eq for Addr<P> {}

impl<P: cfg::Params> PartialOrd for Addr<P> {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        self.addr.partial_cmp(&other.addr)
    }
}

impl<P: cfg::Params> Ord for Addr<P> {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        self.addr.cmp(&other.addr)
    }
}

impl<P: cfg::Params> Clone for Addr<P> {
    fn clone(&self) -> Self {
        Self::from_usize(self.addr)
    }
}

impl<P: cfg::Params> Copy for Addr<P> {}

#[cfg(test)]
mod test {
    use super::*;
    use crate::Pack;
    use proptest::prelude::*;

    proptest! {
        #[test]
        fn addr_roundtrips(pidx in 0usize..Addr::<cfg::DefaultParams>::BITS) {
            let addr = Addr::<cfg::DefaultParams>::from_usize(pidx);
            let packed = addr.pack(0);
            assert_eq!(addr, Addr::from_packed(packed));
        }
        #[test]
        fn gen_roundtrips(gen in 0usize..slot::Generation::<cfg::DefaultParams>::BITS) {
            let gen = slot::Generation::<cfg::DefaultParams>::from_usize(gen);
            let packed = gen.pack(0);
            assert_eq!(gen, slot::Generation::from_packed(packed));
        }

        #[test]
        fn page_roundtrips(
            gen in 0usize..slot::Generation::<cfg::DefaultParams>::BITS,
            addr in 0usize..Addr::<cfg::DefaultParams>::BITS,
        ) {
            let gen = slot::Generation::<cfg::DefaultParams>::from_usize(gen);
            let addr = Addr::<cfg::DefaultParams>::from_usize(addr);
            let packed = gen.pack(addr.pack(0));
            assert_eq!(addr, Addr::from_packed(packed));
            assert_eq!(gen, slot::Generation::from_packed(packed));
        }
    }
}
