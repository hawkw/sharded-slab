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
mod iter;

use self::sync::{
    atomic::{AtomicUsize, Ordering},
    CausalCell,
};
use page::Page;
use std::{marker::PhantomData, ops};

/// A sharded slab.
#[derive(Debug)]
pub struct Slab<T> {
    shards: Box<[CausalCell<Shard<T>>]>,
}

#[derive(Clone, Debug)]
pub struct Builder<T> {
    threads: usize,
    pages: usize,
    initial_page_sz: usize,
    _t: PhantomData<fn(T)>,
}

#[derive(Debug)]
struct Shard<T> {
    tid: usize,
    max_pages: usize,
    initial_page_sz: usize,
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

    /// Inserts
    pub fn insert(&self, value: T) -> Option<usize> {
        let tid = Tid::current();
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
        let tid: Tid = idx.unpack();
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
        let tid: Tid = key.unpack();
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

    pub fn unique_iter<'a>(&'a mut self) -> iter::UniqueIter<'a, T> {
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
            len: AtomicUsize::new(0),
            pages: vec![Page::new(initial_page_sz)],
        }
    }

    fn insert(&mut self, value: T) -> Option<usize> {
        debug_assert_eq!(Tid::current().as_usize(), self.tid);

        let mut value = Some(value);
        for (pidx, page) in self.pages.iter_mut().enumerate() {
            #[cfg(test)]
            println!("-> Index({:?}) ", pidx);
            if let Some(poff) = page.insert(&mut value) {
                return Some(page::Index::from_usize(pidx).pack(poff));
            }
        }

        let pidx = self.pages.len();
        if pidx >= self.max_pages {
            #[cfg(test)]
            println!(
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
        println!("-> {:?}", pidx);
        self.pages.get(pidx.as_usize())?.get(idx)
    }

    fn remove_local(&mut self, idx: usize) -> Option<T> {
        debug_assert_eq!(Tid::current().as_usize(), self.tid);
        debug_assert_eq!(Tid::from_packed(idx).as_usize(), self.tid);
        let pidx = page::Index::from_packed(idx);

        #[cfg(test)]
        println!("-> remove_local {:?}", pidx);
        self.pages
            .get_mut(pidx.as_usize())?
            .remove_local(idx)
            .map(|item| {
                self.len.fetch_sub(1, Ordering::Release);
                item
            })
    }

    fn remove_remote(&self, idx: usize) -> Option<T> {
        debug_assert_eq!(Tid::from_packed(idx).as_usize(), self.tid);
        debug_assert!(Tid::current().as_usize() != self.tid);
        let pidx = page::Index::from_packed(idx);

        #[cfg(test)]
        println!("-> remove_remote {:?}", pidx);
        self.pages
            .get(pidx.as_usize())?
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

    fn iter<'a>(&'a self) -> std::slice::Iter<'a, Page<T>> {
        self.pages.iter()
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

unsafe impl<T: Send> Send for Slab<T> {}
unsafe impl<T: Sync> Sync for Slab<T> {}

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
    const SHIFT: usize = Self::Prev::SHIFT + Self::Prev::LEN;
    const MASK: usize = Self::BITS << Self::SHIFT;

    type Prev: Pack;

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

impl Pack for () {
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

#[cfg(test)]
mod tests;
