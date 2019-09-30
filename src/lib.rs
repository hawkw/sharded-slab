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
pub(crate) mod cfg;
mod iter;
use cfg::CfgPrivate;
pub use cfg::Config;

use self::sync::{
    atomic::{AtomicUsize, Ordering},
    CausalCell,
};
use page::Page;
use std::{fmt, marker::PhantomData};

/// A sharded slab.
pub struct Slab<T, P: cfg::Config = cfg::DefaultConfig> {
    shards: Box<[CausalCell<Shard<T, P>>]>,
    _cfg: PhantomData<P>,
}

#[derive(Clone, Debug)]
pub struct Builder<T> {
    threads: usize,
    pages: usize,
    initial_page_sz: usize,
    _t: PhantomData<fn(T)>,
}

struct Shard<T, P: cfg::Config> {
    tid: usize,
    sz: usize,
    len: AtomicUsize,
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
    pages: Vec<Page<T, P>>,
}

impl<T> Slab<T> {
    pub fn new() -> Self {
        Self::new_with_Config()
    }
}

impl<T, P: cfg::Config> Slab<T, P> {
    pub fn new_with_Config() -> Slab<T, P> {
        let mut shards = Vec::with_capacity(P::MAX_SHARDS);
        let mut idx = 0;
        shards.resize_with(P::MAX_SHARDS, || {
            let shard = Shard::new(idx);
            idx += 1;
            CausalCell::new(shard)
        });
        Self {
            shards: shards.into_boxed_slice(),
            _cfg: PhantomData,
        }
    }

    /// Inserts
    pub fn insert(&self, value: T) -> Option<usize> {
        let tid = Tid::<P>::current();
        #[cfg(test)]
        println!("insert {:?}", tid);
        self.shards[tid.as_usize()]
            .with_mut(|shard| unsafe {
                // we are guaranteed to only mutate the shard while on its thread.
                (*shard).insert(value)
            })
            .map(|idx| tid.pack(idx))
    }

    /// Removes the value associated with the given key from the slab, returning
    /// it.
    ///
    /// If the slab does not contain a value for that key, `None` is returned
    /// instead.
    pub fn remove(&self, idx: usize) -> Option<T> {
        let tid: Tid<P> = idx.unpack();
        #[cfg(test)]
        println!("rm {:?}", tid);
        if tid.is_current() {
            self.shards[tid.as_usize()].with_mut(|shard| unsafe {
                // only called if this is the current shard
                (*shard).remove_local(idx)
            })
        } else {
            self.shards[tid.as_usize()].with(|shard| unsafe { (*shard).remove_remote(idx) })
        }
    }

    /// Return a reference to the value associated with the given key.
    ///
    /// If the slab does not contain a value for the given key, `None` is
    /// returned instead.
    ///
    /// # Examples
    ///
    /// ```
    /// let slab = sharded_slab::Slab::new();
    /// let key = slab.insert("hello world").unwrap();
    ///
    /// assert_eq!(slab.get(key), Some(&"hello world"));
    /// assert_eq!(slab.get(12345), None);
    /// ```
    pub fn get(&self, key: usize) -> Option<&T> {
        let tid: Tid<P> = key.unpack();
        #[cfg(test)]
        println!("get {:?}", tid);
        self.shards
            .get(tid.as_usize())?
            .with(|shard| unsafe { (*shard).get(key) })
    }

    /// Returns `true` if the slab contains a value for the given key.
    ///
    /// # Examples
    ///
    /// ```
    /// let slab = sharded_slab::Slab::new();
    ///
    /// let key = slab.insert("hello world").unwrap();
    /// assert!(slab.contains(key));
    ///
    /// slab.remove(key).unwrap();
    /// assert!(!slab.contains(key));
    /// ```
    pub fn contains(&self, key: usize) -> bool {
        self.get(key).is_some()
    }

    pub fn len(&self) -> usize {
        self.shards
            .iter()
            .map(|shard| shard.with(|shard| unsafe { (*shard).len() }))
            .sum()
    }

    pub fn capacity(&self) -> usize {
        self.total_capacity() - self.len()
    }

    pub fn unique_iter<'a>(&'a mut self) -> iter::UniqueIter<'a, T, P> {
        let mut shards = self.shards.iter_mut();
        let shard = shards.next().expect("must be at least 1 shard");
        let mut pages = shard.with(|shard| unsafe { (*shard).iter() });
        let slots = pages.next().expect("must be at least 1 page").iter();
        iter::UniqueIter {
            shards,
            slots,
            pages,
        }
    }

    fn total_capacity(&self) -> usize {
        self.shards
            .iter()
            .map(|shard| shard.with(|shard| unsafe { (*shard).total_capacity() }))
            .sum()
    }
}

impl<T, P: cfg::Config> Shard<T, P> {
    fn new(tid: usize) -> Self {
        Self {
            tid,
            sz: P::INITIAL_SZ,
            len: AtomicUsize::new(0),
            pages: vec![Page::new(P::INITIAL_SZ, 0)],
        }
    }

    #[inline(always)]
    fn unpack_addr(idx: usize) -> page::Addr<P> {
        page::Addr::from_packed(idx)
    }

    fn insert(&mut self, value: T) -> Option<usize> {
        debug_assert_eq!(Tid::<P>::current().as_usize(), self.tid);

        let mut value = Some(value);
        for (_pidx, page) in self.pages.iter_mut().enumerate() {
            #[cfg(test)]
            println!("-> Index({:?}) ", _pidx);
            if let Some(poff) = page.insert(&mut value) {
                return Some(poff);
            }
        }

        let pidx = self.pages.len();
        if pidx >= P::MAX_PAGES {
            #[cfg(test)]
            println!("max pages (len={}, max={})", self.pages.len(), P::MAX_PAGES);
            // out of pages!
            return None;
        }
        // get new page
        let sz = P::page_size(pidx);
        let mut page = Page::new(sz, self.sz);
        self.sz += sz;
        let poff = page.insert(&mut value).expect("new page should be empty");
        self.pages.push(page);

        Some(poff)
    }

    fn get(&self, idx: usize) -> Option<&T> {
        debug_assert_eq!(Tid::<P>::from_packed(idx).as_usize(), self.tid);
        let addr = Self::unpack_addr(idx);
        let i = addr.index();
        #[cfg(test)]
        println!("-> {:?}; idx {:?}", addr, i);
        self.pages.get(i)?.get(idx)
    }

    fn remove_local(&mut self, idx: usize) -> Option<T> {
        debug_assert_eq!(Tid::<P>::current().as_usize(), self.tid);
        debug_assert_eq!(Tid::<P>::from_packed(idx).as_usize(), self.tid);
        let addr = Self::unpack_addr(idx);

        #[cfg(test)]
        println!("-> remove_local {:?}", addr);
        self.pages
            .get_mut(addr.index())?
            .remove_local(idx)
            .map(|item| {
                self.len.fetch_sub(1, Ordering::Release);
                item
            })
    }

    fn remove_remote(&self, idx: usize) -> Option<T> {
        debug_assert_eq!(Tid::<P>::from_packed(idx).as_usize(), self.tid);
        debug_assert!(Tid::<P>::current().as_usize() != self.tid);
        let addr = Self::unpack_addr(idx);

        #[cfg(test)]
        println!("-> remove_remote {:?}", addr);
        self.pages
            .get(addr.index())?
            .remove_remote(idx)
            .map(|item| {
                self.len.fetch_sub(1, Ordering::Release);
                item
            })
    }

    fn len(&self) -> usize {
        self.len.load(Ordering::Acquire)
    }

    fn total_capacity(&self) -> usize {
        self.iter().map(Page::total_capacity).sum()
    }

    fn iter<'a>(&'a self) -> std::slice::Iter<'a, Page<T, P>> {
        self.pages.iter()
    }
}

impl<T: fmt::Debug, P: cfg::Config> fmt::Debug for Slab<T, P> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Slab")
            // .field("shards", &self.shards)
            .field("Config", &P::debug())
            .finish()
    }
}

unsafe impl<T: Send, P: cfg::Config> Send for Slab<T, P> {}
unsafe impl<T: Sync, P: cfg::Config> Sync for Slab<T, P> {}

impl<T: fmt::Debug, P: cfg::Config> fmt::Debug for Shard<T, P> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Shard")
            .field("tid", &self.tid)
            .field("len", &self.len())
            .field("pages", &self.pages)
            .finish()
    }
}

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
pub(crate) trait Pack<C: cfg::Config>: Sized {
    const LEN: usize;

    const BITS: usize;
    const SHIFT: usize = Self::Prev::SHIFT + Self::Prev::LEN;
    const MASK: usize = Self::BITS << Self::SHIFT;

    type Prev: Pack<C>;

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

impl<C: cfg::Config> Pack<C> for () {
    const BITS: usize = 0;
    const LEN: usize = 0;
    const SHIFT: usize = 0;
    const MASK: usize = 0;

    type Prev = ();

    fn as_usize(&self) -> usize {
        unreachable!()
    }
    fn from_usize(val: usize) -> Self {
        unreachable!()
    }

    fn pack(&self, to: usize) -> usize {
        unreachable!()
    }

    fn from_packed(from: usize) -> Self {
        unreachable!()
    }
}

pub(crate) trait Unpack<P, T> {
    fn unpack(self) -> T;
}

impl<P: cfg::Config, T: Pack<P>> Unpack<P, T> for usize {
    #[inline(always)]
    fn unpack(self) -> T {
        T::from_packed(self)
    }
}

#[cfg(test)]
mod tests;
