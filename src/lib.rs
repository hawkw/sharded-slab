#[cfg(test)]
macro_rules! thread_local {
    ($($tts:tt)+) => { loom::thread_local!{ $($tts)+ } }
}

#[cfg(not(test))]
macro_rules! thread_local {
    ($($tts:tt)+) => { std::thread_local!{ $($tts)+ } }
}

mod page;
pub(crate) mod sync;
mod tid;
pub(crate) use tid::Tid;

use self::sync::CausalCell;
use page::Page;
use std::{marker::PhantomData, ops};

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

    fn from_packed(from: usize) -> Self {
        let value = (from & Self::MASK) >> Self::SHIFT;
        debug_assert!(value <= Self::BITS);
        Self::from_usize(value)
    }
}

#[derive(Clone, Debug)]
pub struct Builder<T> {
    threads: usize,
    pages: usize,
    initial_page_sz: usize,
    _t: PhantomData<fn(T)>,
}

pub(crate) trait Unpack<T: Pack> {
    fn unpack(self) -> T;
}

impl<T: Pack> Unpack<T> for usize {
    #[inline(always)]
    fn unpack(self) -> T {
        T::from_packed(self)
    }
}
impl<T: Pack> Unpack<T> for T {
    #[inline(always)]
    fn unpack(self) -> T {
        self
    }
}

pub struct Slab<T> {
    shards: Box<[CausalCell<Shard<T>>]>,
}

struct Shard<T> {
    tid: usize,
    max_pages: usize,
    initial_page_sz: usize,
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
    pub fn builder() -> Builder<T> {
        Builder::default()
    }

    pub fn new() -> Self {
        Self::default()
    }

    fn from_builder(builder: Builder<T>) -> Self {
        let Builder {
            threads,
            initial_page_sz,
            pages,
            ..
        } = builder;
        let mut shards = Vec::with_capacity(threads);
        let mut idx = 0;
        shards.resize_with(threads, || {
            let shard = Shard::new(idx, initial_page_sz, pages);
            idx += 1;
            CausalCell::new(shard)
        });
        Self {
            shards: shards.into_boxed_slice(),
        }
    }

    pub fn insert(&self, value: T) -> Option<usize> {
        let tid = Tid::current();
        #[cfg(test)]
        print!("insert {:?}", tid);
        self.shards[tid.as_usize()]
            .with_mut(|shard| unsafe {
                // we are guaranteed to only mutate the shard while on its thread.
                (*shard).insert(value)
            })
            .map(|idx| tid.pack(idx))
    }

    pub fn remove(&self, idx: usize) {
        let tid: Tid = idx.unpack();
        #[cfg(test)]
        print!("rm {:?}", tid);
        if tid.is_current() {
            self.shards[tid.as_usize()].with_mut(|shard| unsafe {
                // only called if this is the current shard
                (*shard).remove_local(idx)
            })
        } else {
            self.shards[tid.as_usize()].with(|shard| unsafe { (*shard).remove_remote(idx) })
        }
    }

    #[inline]
    pub fn get(&self, idx: usize) -> Option<&T> {
        let tid: Tid = idx.unpack();
        #[cfg(test)]
        print!("get {:?}", tid);
        self.shards[tid.as_usize()].with(|shard| unsafe { (*shard).get(idx) })
    }
}

impl<T> Default for Slab<T> {
    #[inline]
    fn default() -> Self {
        Slab::<T>::builder().finish()
    }
}

impl<T> Builder<T> {
    pub fn max_threads(self, threads: usize) -> Self {
        assert!(threads <= Tid::BITS);
        Self { threads, ..self }
    }

    pub fn max_pages(self, pages: usize) -> Self {
        assert!(pages <= page::Index::BITS);
        Self { pages, ..self }
    }

    pub fn initial_page_size(self, initial_page_sz: usize) -> Self {
        assert!(initial_page_sz.is_power_of_two());
        assert!(initial_page_sz <= page::Offset::BITS);

        Self {
            initial_page_sz,
            ..self
        }
    }

    pub fn finish(self) -> Slab<T> {
        Slab::from_builder(self)
    }
}

impl<T> Default for Builder<T> {
    fn default() -> Self {
        Self {
            threads: Tid::BITS,
            initial_page_sz: 32,
            pages: page::Index::BITS,
            _t: PhantomData,
        }
    }
}

impl<T> Shard<T> {
    fn new(tid: usize, initial_page_sz: usize, max_pages: usize) -> Self {
        Self {
            tid,
            max_pages,
            initial_page_sz,
            pages: vec![Page::new(initial_page_sz)],
        }
    }

    fn insert(&mut self, value: T) -> Option<usize> {
        debug_assert_eq!(Tid::current().as_usize(), self.tid);

        let mut value = Some(value);
        for (pidx, page) in self.pages.iter_mut().enumerate() {
            #[cfg(test)]
            print!("-> Index({:?}) ", pidx);
            if let Some(poff) = page.insert(&mut value) {
                return Some(page::Index::from_usize(pidx).pack(poff));
            }
        }

        let pidx = self.pages.len();
        if pidx > self.max_pages {
            #[cfg(test)]
            print!(
                "max pages (len={}, max={})",
                self.pages.len(),
                self.max_pages
            );
            // out of pages!
            return None;
        }

        // get new page
        let mut page = Page::new(self.initial_page_sz * 2usize.pow(pidx as u32));
        let poff = page.insert(&mut value).expect("new page should be empty");
        self.pages.push(page);

        Some(page::Index::from_usize(pidx).pack(poff))
    }

    fn get(&self, idx: usize) -> Option<&T> {
        debug_assert_eq!(Tid::from_packed(idx).as_usize(), self.tid);
        let pidx = page::Index::from_packed(idx);

        #[cfg(test)]
        print!("-> {:?}", pidx);
        self[pidx].get(idx)
    }

    fn remove_local(&mut self, idx: usize) {
        debug_assert_eq!(Tid::current().as_usize(), self.tid);
        debug_assert_eq!(Tid::from_packed(idx).as_usize(), self.tid);
        let pidx = page::Index::from_packed(idx);

        #[cfg(test)]
        print!(" -> remove_local {:?}", pidx);
        self[pidx].remove_local(idx)
    }

    fn remove_remote(&self, idx: usize) {
        debug_assert_eq!(Tid::from_packed(idx).as_usize(), self.tid);
        debug_assert!(Tid::current().as_usize() != self.tid);
        let pidx = page::Index::from_packed(idx);

        #[cfg(test)]
        print!(" -> remove_remote {:?}", pidx);
        self[pidx].remove_remote(idx)
    }
}

impl<T, P: Unpack<page::Index>> ops::Index<P> for Shard<T> {
    type Output = Page<T>;
    #[inline]
    fn index(&self, idx: P) -> &Self::Output {
        &self.pages[idx.unpack().as_usize()]
    }
}

impl<T, P: Unpack<page::Index>> ops::IndexMut<P> for Shard<T> {
    #[inline]
    fn index_mut(&mut self, idx: P) -> &mut Self::Output {
        &mut self.pages[idx.unpack().as_usize()]
    }
}

#[cfg(test)]
mod tests;
