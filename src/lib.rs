pub(crate) mod global;
mod page;
pub(crate) mod sync;
mod tid;
pub(crate) use tid::Tid;

use self::sync::CausalCell;
use page::Page;

#[cfg(target_pointer_width = "32")]
pub(crate) mod consts {
    //! Token bit allocation:
    //! ```text
    //!  gggg_gttt_ttti_iiio_oooo_oooo_oooo_oooo
    //!  │     │      │    └─────────page offset
    //!  │     │      └───────────────page index
    //!  │     └───────────────────────thread id
    //!  └────────────────────────────generation
    //! ```
    pub(crate) const POFF_MASK: usize = 0b1_1111_1111_1111_1111; // 17 bits

    pub(crate) const PIDX_MASK: usize = 0b1111 << PIDX_SHIFT; // 4 bits
    pub(crate) const PIDX_SHIFT: usize = 17;

    pub(crate) const TID_MASK: usize = 0x0011_1111 << TID_SHIFT; // 6 bits (max of 64 tids);
    pub(crate) const TID_SHIFT: usize = PIDX_SHIFT + 4;

    pub(crate) const GEN_MASK: usize = 0x0001_1111 << TID_SHIFT; // 5 bits
    pub(crate) const GEN_SHIFT: usize = TID_SHIFT + 6;
}

#[cfg(target_pointer_width = "64")]
pub(crate) mod consts {
    pub(crate) const POFF_MASK: usize = 0x3_FFFF_FFFF; // 34 bits

    pub(crate) const PIDX_MASK: usize = 0x8 << PIDX_SHIFT; // 8 bits
    pub(crate) const PIDX_SHIFT: usize = 34;

    pub(crate) const TID_MASK: usize = 0x1111_1111_1111 << TID_SHIFT; // 12 bits (max of 4096 tids);
    pub(crate) const TID_SHIFT: usize = PIDX_SHIFT + 8;

    pub(crate) const GEN_MASK: usize = 0x0011_1111_1111 << TID_SHIFT; // 10 bits
    pub(crate) const GEN_SHIFT: usize = TID_SHIFT + 12;
}

pub struct Slab<T> {
    shards: Box<[CausalCell<Shard<T>>]>,
}

#[derive(Clone)]
struct Shard<T> {
    tid: usize,
    pages: Vec<Page<T>>,
}

impl<T> Slab<T> {
    pub fn new(threads: usize) -> Self {
        let mut shards = Vec::with_capacity(threads);
        let mut idx = 0;
        shards.resize_with(threads, || {
            let shard = Shard::new(idx);
            idx += 1;
            shard
        });
        Self {
            shards: shards.into_boxed_slice(),
        }
    }

    pub fn insert(&self, value: T) -> usize {
        let tidx = Tid::current().as_usize();
        self.shards[tidx].with_mut(|shard| unsafe {
            // we are guaranteed to only mutate the shard while on its thread.
            (*shard).insert(value)
        })
    }
}

impl<T> Shard<T> {
    fn new(tid: usize) -> Self {
        Self {
            tid,
            pages: vec![Page::new(32)],
        }
    }

    fn pack_idxs(&self, pidx: usize, poff: usize) -> usize {
        debug_assert!(self.tid <= consts::TID_MASK);
        debug_assert!(pidx <= consts::PIDX_MASK);
        debug_assert!(poff <= consts::POFF_MASK);
        (self.tid << consts::TID_SHIFT) | (pidx << consts::PIDX_SHIFT) | poff
    }

    fn insert(&mut self, value: T) -> Option<usize> {
        debug_assert!(Tid::current().as_usize() == self.tid);

        let mut value = Some(value);
        for (pidx, mut page) in self.pages.iter_mut().enumerate() {
            if let Some(poff) = page.insert(&mut value) {
                return Some(self.pack_idxs(pidx, poff));
            }
        }

        if self.pages.len() == consts::PIDX_MASK {
            // out of pages!
            return None;
        }

        // get new page
        let pidx = self.pages.len();
        let mut page = Page::new(32 * 2.pow(pidx));
        let poff = page.insert(&mut value).expect("new page should be empty");
        self.pages.push(page);

        Some(self.pack_idxs(pidx, poff))
    }
}
