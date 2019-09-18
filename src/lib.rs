mod page;
pub(crate) mod sync;
mod tid;
pub(crate) use tid::Tid;

use self::sync::CausalCell;
use page::Page;

/// Token bit allocation:
/// ```text
///
/// 32-bit:
///  rggg_gttt_ttti_iiio_oooo_oooo_oooo_oooo
///   │    │      │    └─────────page offset
///   │    │      └───────────────page index
///   │    └───────────────────────thread id
///   └───────────────────────────generation
/// ```
pub(crate) trait Pack: Sized {
    const BITS: usize;
    const LEN: usize;
    const SHIFT: usize;
    const MASK: usize = Self::BITS << Self::SHIFT;

    fn as_usize(&self) -> usize;
    fn from_usize(val: usize) -> Self;

    fn pack(&self, to: usize) -> usize {
        let value = self.as_usize();
        debug_assert!(value <= Self::BITS);

        (to & !Self::MASK) | (value << Self::SHIFT)
    }

    fn unpack(from: usize) -> Self {
        let value = (from & Self::MASK) >> Self::SHIFT;
        debug_assert!(value <= Self::BITS);
        Self::from_usize(value)
    }
}

pub struct Slab<T> {
    shards: Box<[CausalCell<Shard<T>>]>,
}

struct Shard<T> {
    tid: usize,
    // ┌─────────────┐      ┌────────┐
    // │ page 1      │      │        │
    // ├─────────────┤ ┌───▶│  next──┼─┐
    // │ page 2      │ │    ├────────┤ │
    // │             │ │    │XXXXXXXX│ │
    // │ local_free──┼─┘    ├────────┤ │
    // │ global_free─┼─┐    │        │◀┘
    // ├─────────────┤ └───▶│  next──┼─┐
    // │   page 3    │      ├────────┤ │
    // └─────────────┘      │XXXXXXXX│ │
    //       ...            ├────────┤ │
    // ┌─────────────┐      │XXXXXXXX│ │
    // │ page n      │      ├────────┤ │
    // └─────────────┘      │        │◀┘
    //                      │  next──┼───▶
    //                      ├────────┤
    //                      │XXXXXXXX│
    //                      └────────┘
    //                         ...
    pages: Vec<Page<T>>,
}

impl<T> Slab<T> {
    pub fn new(threads: usize) -> Self {
        let mut shards = Vec::with_capacity(threads);
        let mut idx = 0;
        shards.resize_with(threads, || {
            let shard = Shard::new(idx);
            idx += 1;
            CausalCell::new(shard)
        });
        Self {
            shards: shards.into_boxed_slice(),
        }
    }

    pub fn insert(&self, value: T) -> Option<usize> {
        let tid = Tid::current();
        print!("insert {:?} ", tid);
        self.shards[tid.as_usize()].with_mut(|shard| unsafe {
            // we are guaranteed to only mutate the shard while on its thread.
            (*shard).insert(value)
        })
    }

    pub fn remove(&self, idx: usize) {
        let tid = Tid::unpack(idx);
        if tid.is_current() {
            self.shards[tid.as_usize()].with_mut(|shard| unsafe {
                // only called if this is the current shard
                (*shard).remove_local(idx)
            })
        } else {
            self.shards[tid.as_usize()].with(|shard| unsafe { (*shard).remove_remote(idx) })
        }
    }

    pub fn get(&self, idx: usize) -> Option<&T> {
        let tid = Tid::unpack(idx);
        print!("get {:?} ", tid);
        self.shards[tid.as_usize()].with(|shard| unsafe { (*shard).get(idx) })
    }
}

impl<T> Shard<T> {
    fn new(tid: usize) -> Self {
        Self {
            tid,
            pages: vec![Page::new(32)],
        }
    }

    fn insert(&mut self, value: T) -> Option<usize> {
        debug_assert_eq!(Tid::current().as_usize(), self.tid);

        let mut value = Some(value);
        for (pidx, page) in self.pages.iter_mut().enumerate() {
            print!("-> Index({:?}) ", pidx);
            if let Some(poff) = page.insert(&mut value) {
                return Some(page::Index::from_usize(pidx).pack(poff));
            }
        }

        if self.pages.len() > page::Index::MASK {
            // out of pages!
            return None;
        }

        // get new page
        let pidx = self.pages.len();
        let mut page = Page::new(32 * 2usize.pow(pidx as u32));
        let poff = page.insert(&mut value).expect("new page should be empty");
        self.pages.push(page);

        Some(page::Index::from_usize(pidx).pack(poff))
    }

    fn get(&self, idx: usize) -> Option<&T> {
        debug_assert_eq!(Tid::unpack(idx).as_usize(), self.tid);
        let pidx = page::Index::unpack(idx);
        print!("-> {:?}", pidx);
        self.pages[pidx.as_usize()].get(idx)
    }

    fn remove_local(&mut self, idx: usize) {
        debug_assert_eq!(Tid::current().as_usize(), self.tid);
        debug_assert_eq!(Tid::unpack(idx).as_usize(), self.tid);

        let pidx = page::Index::unpack(idx).as_usize();
        self.pages[pidx].remove_local(idx)
    }

    fn remove_remote(&self, idx: usize) {
        debug_assert_eq!(Tid::unpack(idx).as_usize(), self.tid);

        let pidx = page::Index::unpack(idx).as_usize();
        self.pages[pidx].remove_remote(idx)
    }
}

#[cfg(test)]
mod test {
    use crate::{
        page::{self, slot},
        Pack, Tid,
    };
    use proptest::prelude::*;

    proptest! {
        #[test]
        fn tid_roundtrips(tid in 0usize..Tid::BITS) {
            let tid = Tid::from_usize(tid);
            let packed = tid.pack(0);
            assert_eq!(tid, Tid::unpack(packed));
        }

        #[test]
        fn idx_roundtrips(
            tid in 0usize..Tid::BITS,
            gen in 0usize..slot::Generation::BITS,
            pidx in 0usize..page::Index::BITS,
            poff in 0usize..page::Offset::BITS,
        ) {
            let tid = Tid::from_usize(tid);
            let gen = slot::Generation::from_usize(gen);
            let pidx = page::Index::from_usize(pidx);
            let poff = page::Offset::from_usize(poff);
            let packed = tid.pack(gen.pack(pidx.pack(poff.pack(0))));
            assert_eq!(poff, page::Offset::unpack(packed));
            assert_eq!(pidx, page::Index::unpack(packed));
            assert_eq!(gen, slot::Generation::unpack(packed));
            assert_eq!(tid, Tid::unpack(packed));
        }
    }
}
