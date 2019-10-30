use crate::cfg::{self, CfgPrivate};
use crate::sync::CausalCell;
use crate::Pack;

pub(crate) mod slot;
mod stack;
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

    pub(crate) fn index(self) -> usize {
        // Since every page is twice as large as the previous page, and all page sizes
        // are powers of two, we can determine the page index that contains a given
        // address by counting leading zeros, which tells us what power of two
        // the offset fits into.
        //
        // First, we must shift down to the smallest page size, so that the last
        // offset on the first page becomes 0.
        let shifted = (self.addr + C::INITIAL_SZ) >> C::ADDR_INDEX_SHIFT;
        // Now, we can  determine the number of twos places by counting the
        // number of leading  zeros (unused twos places) in the number's binary
        // representation, and subtracting that count from the total number of bits in a word.
        cfg::WIDTH - shifted.leading_zeros() as usize
    }

    pub(crate) fn offset(self) -> usize {
        self.addr
    }
}

impl<C: cfg::Config> Pack<C> for Addr<C> {
    const LEN: usize = C::MAX_PAGES + C::ADDR_INDEX_SHIFT;

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

pub(crate) struct Local {
    head: CausalCell<usize>,
}

pub(crate) struct Shared<T, C> {
    remote: stack::TransferStack<C>,
    size: usize,
    prev_sz: usize,
    slab: CausalCell<Option<Slots<T, C>>>,
}

type Slots<T, C> = Box<[Slot<T, C>]>;

impl Local {
    pub(crate) fn new() -> Self {
        Self {
            head: CausalCell::new(0),
        }
    }

    #[inline(always)]
    fn head(&self) -> usize {
        self.head.with(|head| unsafe { *head })
    }

    #[inline(always)]
    fn set_head(&self, new_head: usize) {
        self.head.with_mut(|head| unsafe {
            *head = new_head;
        })
    }
}

impl<T, C: cfg::Config> Shared<T, C> {
    const NULL: usize = Addr::<C>::NULL;

    pub(crate) fn new(size: usize, prev_sz: usize) -> Self {
        Self {
            prev_sz,
            size,
            remote: stack::TransferStack::new(),
            slab: CausalCell::new(None),
        }
    }

    /// Returns `true` if storage is currently allocated for this page, `false`
    /// otherwise.
    #[inline]
    fn is_unallocated(&self) -> bool {
        self.slab.with(|s| unsafe { (*s).is_none() })
    }

    #[cold]
    fn fill(&self) {
        test_println!("-> alloc new page ({})", self.size);
        debug_assert!(self.is_unallocated());

        let mut slab = Vec::with_capacity(self.size);
        slab.extend((1..self.size).map(Slot::new));
        slab.push(Slot::new(Self::NULL));
        self.slab.with_mut(|s| {
            // this mut access is safe — it only occurs to initially
            // allocate the page, which only happens on this thread; if the
            // page has not yet been allocated, other threads will not try
            // to access it yet.
            unsafe {
                *s = Some(slab.into_boxed_slice());
            }
        });
    }

    #[inline]
    pub(crate) fn insert(&self, local: &Local, t: &mut Option<T>) -> Option<usize> {
        let head = local.head();

        test_println!("-> local head {:?}", head);

        // are there any items on the local free list? (fast path)
        let head = if head < self.size {
            head
        } else {
            // if the local free list is empty, pop all the items on the remote
            // free list onto the local free list.
            let head = self.remote.pop_all();

            test_println!("-> remote head {:?}", head);
            head?
        };

        // if the head is still null, both the local and remote free lists are
        // empty --- we can't fit any more items on this page.
        if head == Self::NULL {
            test_println!("-> NULL! {:?}", head);
            return None;
        }

        // do we need to allocate storage for this page?
        if self.is_unallocated() {
            self.fill();
        }

        let gen = self.slab.with(|slab| {
            let slab = unsafe { &*(slab) }
                .as_ref()
                .expect("page must have been allocated to insert!");
            let slot = &slab[head];
            let gen = slot.insert(t);
            local.set_head(slot.next());
            gen
        })?;

        let index = head + self.prev_sz;

        test_println!("insert at offset: {}", index);
        Some(gen.pack(index))
    }

    #[inline]
    pub(crate) fn get(&self, addr: Addr<C>, idx: usize) -> Option<slot::Guard<'_, T, C>> {
        let poff = addr.offset() - self.prev_sz;

        test_println!("-> offset {:?}", poff);

        self.slab.with(|slab| {
            unsafe { &*slab }
                .as_ref()?
                .get(poff)?
                .get(C::unpack_gen(idx))
        })
    }

    pub(crate) fn remove(&self, addr: Addr<C>, gen: slot::Generation<C>) -> bool {
        let offset = addr.offset() - self.prev_sz;

        test_println!("-> offset {:?}", offset);

        self.slab.with(|slab| {
            let slab = unsafe { &*slab }.as_ref();
            if let Some(slot) = slab.and_then(|slab| slab.get(offset)) {
                slot.remove(gen)
            } else {
                false
            }
        })
    }

    pub(crate) fn remove_local(
        &self,
        local: &Local,
        addr: Addr<C>,
        gen: slot::Generation<C>,
    ) -> Option<T> {
        let offset = addr.offset() - self.prev_sz;

        test_println!("-> offset {:?}", offset);

        self.slab.with(|slab| {
            let slab = unsafe { &*slab }.as_ref()?;
            let slot = slab.get(offset)?;
            let val = slot.remove_value(gen)?;
            slot.set_next(local.head());
            local.set_head(offset);
            Some(val)
        })
    }

    pub(crate) fn remove_remote(&self, addr: Addr<C>, gen: slot::Generation<C>) -> Option<T> {
        let offset = addr.offset() - self.prev_sz;

        test_println!("-> offset {:?}", offset);

        self.slab.with(|slab| {
            let slab = unsafe { &*slab }.as_ref()?;
            let slot = slab.get(offset)?;
            let val = slot.remove_value(gen)?;
            self.remote.push(offset, |next| slot.set_next(next));
            Some(val)
        })
    }

    pub(crate) fn iter(&self) -> Option<Iter<'_, T, C>> {
        let slab = self.slab.with(|slab| unsafe { (&*slab).as_ref() });
        slab.map(|slab| slab.iter().filter_map(Slot::value as fn(_) -> _))
    }
}

impl fmt::Debug for Local {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.head.with(|head| {
            let head = unsafe { *head };
            f.debug_struct("Local")
                .field("head", &format_args!("{:#0x}", head))
                .finish()
        })
    }
}

impl<C, T> fmt::Debug for Shared<C, T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Shared")
            .field("remote", &self.remote)
            .field("prev_sz", &self.prev_sz)
            .field("size", &self.size)
            // .field("slab", &self.slab)
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
