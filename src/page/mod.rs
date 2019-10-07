use crate::cfg::{self, CfgPrivate};
use crate::sync::atomic::{spin_loop_hint, AtomicUsize, Ordering};
use crate::Pack;

pub(crate) mod slot;
use self::slot::Slot;
use std::{fmt, marker::PhantomData};

/// A page address encodes the location of a slot within a shard (the page
/// number and offset within that page) as a single linear value.
#[repr(transparent)]
pub(crate) struct Addr<C: cfg::Config = cfg::DefaultConfig> {
    addr: usize,
    _cfg: PhantomData<fn(C)>,
}

impl<C: cfg::Config> Addr<C> {
    const NULL: usize = Self::BITS + 1;

    pub(crate) fn index(&self) -> usize {
        // Since every page is twice as large as the previous page, and all page sizes
        // are powers of two, we can determine the page index that contains a given
        // address by shifting the address down by the smallest page size and
        // looking at how many twos places necessary to represent that number,
        // telling us what power of two page size it fits inside of. We can
        // determine the number of twos places by counting the number of leading
        // zeros (unused twos places) in the number's binary representation, and
        // subtracting that count from the total number of bits in a word.
        cfg::WIDTH - (self.addr + C::INITIAL_SZ >> C::ADDR_INDEX_SHIFT).leading_zeros() as usize
    }

    pub(crate) fn offset(&self) -> usize {
        self.addr
    }
}

impl<C: cfg::Config> Pack<C> for Addr<C> {
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

pub(crate) type Iter<'a, T, C> =
    std::iter::FilterMap<std::slice::Iter<'a, Slot<T, C>>, fn(&'a Slot<T, C>) -> Option<&'a T>>;

pub(crate) struct Page<T, C> {
    prev_sz: usize,
    remote_head: AtomicUsize,
    local_head: usize,
    slab: Box<[Slot<T, C>]>,
}

impl<T, C: cfg::Config> Page<T, C> {
    const NULL: usize = Addr::<C>::NULL;

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

    #[inline]
    pub(crate) fn insert(&mut self, t: &mut Option<T>) -> Option<usize> {
        let head = self.local_head;
        #[cfg(test)]
        println!("-> local head {:?}", head);

        // are there any items on the local free list? (fast path)
        let head = if head < self.slab.len() {
            head
        } else {
            // if the local free list is empty, pop all the items on the remote
            // free list onto the local free list.
            let head = self.remote_head.swap(Self::NULL, Ordering::Acquire);
            #[cfg(test)]
            println!("-> remote head {:?}", head);
            head
        };

        // if the head is still null, both the local and remote free lists are
        // empty --- we can't fit any more items on this page.
        if head == Self::NULL {
            #[cfg(test)]
            println!("-> NULL! {:?}", head);
            return None;
        }

        let slot = &mut self.slab[head];
        let gen = slot.insert(t);
        self.local_head = slot.next();
        let index = head + self.prev_sz;
        #[cfg(test)]
        println!("insert at offset: {}", index);
        Some(gen.pack(index))
    }

    #[inline]
    pub(crate) fn get(&self, addr: Addr<C>, idx: usize) -> Option<&T> {
        let poff = addr.offset() - self.prev_sz;
        #[cfg(test)]
        println!("-> offset {:?}", poff);

        self.slab.get(poff)?.get(C::unpack_gen(idx))
    }

    pub(crate) fn remove_local(&mut self, addr: Addr<C>, gen: slot::Generation<C>) -> Option<T> {
        let offset = addr.offset() - self.prev_sz;

        #[cfg(test)]
        println!("-> offset {:?}", offset);

        let slot = self.slab.get(offset)?;
        if !slot.try_remove(gen, self.local_head) {
            return None;
        }
        self.local_head = offset;
        slot.remove_value()
    }

    pub(crate) fn remove_remote(&self, addr: Addr<C>, gen: slot::Generation<C>) -> Option<T> {
        let offset = addr.offset() - self.prev_sz;

        #[cfg(test)]
        println!("-> offset {:?}", offset);
        let slot = self.slab.get(offset)?;

        loop {
            let next = self.remote_head.load(Ordering::Relaxed);

            #[cfg(test)]
            println!("-> next={:?}", next);

            if !slot.try_remove(gen, next) {
                break;
            }

            let actual = self
                .remote_head
                .compare_and_swap(next, offset, Ordering::Release);
            if actual == next {
                return slot.remove_value();
            }

            spin_loop_hint();
        }

        None
    }

    pub(crate) fn total_capacity(&self) -> usize {
        self.slab.len()
    }

    pub(crate) fn iter<'a>(&'a self) -> Iter<'a, T, C> {
        self.slab.iter().filter_map(Slot::value)
    }

    #[inline(always)]
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

impl<C, T> fmt::Debug for Page<C, T> {
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

impl<C: cfg::Config> fmt::Debug for Addr<C> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Addr")
            .field("addr", &format_args!("{:#0x}", &self.addr))
            .field("index", &self.index())
            .field("offset", &self.offset())
            .finish()
    }
}

impl<C: cfg::Config> PartialEq for Addr<C> {
    fn eq(&self, other: &Self) -> bool {
        self.addr == other.addr
    }
}

impl<C: cfg::Config> Eq for Addr<C> {}

impl<C: cfg::Config> PartialOrd for Addr<C> {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        self.addr.partial_cmp(&other.addr)
    }
}

impl<C: cfg::Config> Ord for Addr<C> {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        self.addr.cmp(&other.addr)
    }
}

impl<C: cfg::Config> Clone for Addr<C> {
    fn clone(&self) -> Self {
        Self::from_usize(self.addr)
    }
}

impl<C: cfg::Config> Copy for Addr<C> {}

#[cfg(test)]
mod test {
    use super::*;
    use crate::Pack;
    use proptest::prelude::*;

    proptest! {
        #[test]
        fn addr_roundtrips(pidx in 0usize..Addr::<cfg::DefaultConfig>::BITS) {
            let addr = Addr::<cfg::DefaultConfig>::from_usize(pidx);
            let packed = addr.pack(0);
            assert_eq!(addr, Addr::from_packed(packed));
        }
        #[test]
        fn gen_roundtrips(gen in 0usize..slot::Generation::<cfg::DefaultConfig>::BITS) {
            let gen = slot::Generation::<cfg::DefaultConfig>::from_usize(gen);
            let packed = gen.pack(0);
            assert_eq!(gen, slot::Generation::from_packed(packed));
        }

        #[test]
        fn page_roundtrips(
            gen in 0usize..slot::Generation::<cfg::DefaultConfig>::BITS,
            addr in 0usize..Addr::<cfg::DefaultConfig>::BITS,
        ) {
            let gen = slot::Generation::<cfg::DefaultConfig>::from_usize(gen);
            let addr = Addr::<cfg::DefaultConfig>::from_usize(addr);
            let packed = gen.pack(addr.pack(0));
            assert_eq!(addr, Addr::from_packed(packed));
            assert_eq!(gen, slot::Generation::from_packed(packed));
        }
    }
}
