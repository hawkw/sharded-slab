//! A lock-free concurrent slab.
//!
//! Slabs provide pre-allocated storage for many instances of a single data
//! type. When a large number of values of a single type are required,
//! this can be more efficient than allocating each item individually. Since the
//! allocated items are the same size, memory fragmentation is reduced, and
//! creating and removing new items can be very cheap.
//!
//! This crate implements a lock-free concurrent slab, indexed by `usize`s.
//!
//! # Examples
//!
//! Inserting an item into the slab, returning an index:
//! ```rust
//! # use sharded_slab::Slab;
//! let slab = Slab::new();
//!
//! let key = slab.insert("hello world").unwrap();
//! assert_eq!(slab.get(key), Some(&"hello world"));
//! ```
//!
//! To share a slab across threads, it may be wrapped in an `Arc`:
//! ```rust
//! # use sharded_slab::Slab;
//! use std::sync::Arc;
//! let slab = Arc::new(Slab::new());
//!
//! let slab2 = slab.clone();
//! let thread2 = std::thread::spawn(move || {
//!     let key = slab2.insert("hello from thread two").unwrap();
//!     assert_eq!(slab2.get(key), Some(&"hello from thread two"));
//!     key
//! });
//!
//! let key1 = slab.insert("hello from thread one").unwrap();
//! assert_eq!(slab.get(key1), Some(&"hello from thread one"));
//!
//! // Wait for thread 2 to complete.
//! let key2 = thread2.join().unwrap();
//!
//! // The item inserted by thread 2 remains in the slab.
//! assert_eq!(slab.get(key2), Some(&"hello from thread two"));
//!```
//!
//! If items in the slab must be mutated, a `Mutex` or `RwLock` may be used for
//! each item, providing granular locking of items rather than of the slab:
//!
//! ```rust
//! # use sharded_slab::Slab;
//! use std::sync::{Arc, Mutex};
//! let slab = Arc::new(Slab::new());
//!
//! let key = slab.insert(Mutex::new(String::from("hello world"))).unwrap();
//!
//! let slab2 = slab.clone();
//! let thread2 = std::thread::spawn(move || {
//!     let mut hello = slab2.get(key).unwrap().lock().unwrap();
//!     *hello = String::from("hello everyone!");
//! });
//!
//! thread2.join().unwrap();
//!
//! let hello = slab.get(key).unwrap().lock().unwrap();
//! assert_eq!(hello.as_str(), "hello everyone!");
//! ```
//!
//! # Configuration
//!
//! For performance reasons, several values used by the slab are calculated as
//! constants. In order to allow users to tune the slab's parameters, we provide
//! a [`Config`] trait which defines these parameters as associated `consts`.
//! The `Slab` type is generic over a `C: Config` parameter.
//!
//! [`Config`]: trait.Config.html
//!
//! # Comparison with Similar Crates
//!
//! - [`slab`]: Carl Lerche's `slab` crate provides a slab implementation with a
//!   similar API, implemented by storing all data in a single vector.
//!
//!   Unlike `sharded_slab`, inserting and removing elements from the slab
//!   requires  mutable access. This means that if the slab is accessed
//!   concurrently by multiple threads, it is necessary for it to be protected
//!   by a `Mutex` or `RwLock`. Items may not be inserted or removed (or
//!   accessed, if a `Mutex` is used) concurrently, even when they are
//!   unrelated. In many cases, the lock can become a significant bottleneck. On
//!   the other hand, this crate allows separate indices in the slab to be
//!   accessed, inserted, and removed concurrently without requiring a global
//!   lock. Therefore, when the slab is shared across multiple threads, this
//!   crate offers significantly better performance than `slab`.
//!
//!   However, the lock free slab introduces some additional constant-factor
//!   overhead. This means that in use-cases where a slab is _not_ shared by
//!   multiple threads and locking is not required, this crate will likely offer
//!   slightly worse performance.
//!
//!   In summary: `sharded-slab` offers significantly improved performance in
//!   concurrent use-cases, while `slab` should be preferred in single-threaded
//!   use-cases.
//!
//! [`slab`]: https://crates.io/crates/loom
//!
//! # Safety and Correctness
//!
//! Most implementations of lock-free data structures in Rust require some
//! amount of unsafe code, and this crate is not an exception. In order to catch
//! potential bugs in this unsafe code, we make use of [`loom`], a
//! permutation-testing tool for concurrent Rust programs. All `unsafe` blocks
//! this crate occur in accesses to `loom` `CausalCell`s. This means that when
//! those accesses occur in this crate's tests, `loom` will assert that they are
//! valid under the C11 memory model across multiple permutations of concurrent
//! executions of those tests.
//!
//! In order to guard against the [ABA problem][aba], this crate makes use of
//! _generational indices_. Each slot in the slab tracks a generation counter
//! which is incremented every time a value is inserted into that slot, and the
//! indices returned by [`Slab::insert`] include the generation of the slot when
//! the value was inserted, packed into the high-order bits of the index. This
//! ensures that if a value is inserted, removed,  and a new value is inserted
//! into the same slot in the slab, the key returned by the first call to
//! `insert` will not map to the new value.
//!
//! Since a fixed number of bits are set aside to use for storing the generation
//! counter, the counter will wrap  around after being incremented a number of
//! times. To avoid situations where a returned index lives long enough to see the
//! generation counter wrap around to the same value, it is good to be fairly
//! generous when configuring the allocation of index bits.
//!
//! [`loom`]: https://crates.io/crates/loom
//! [aba]: https://en.wikipedia.org/wiki/ABA_problem
//! [`Slab::insert`]: struct.Slab.html#method.insert
//!
//! # Performance
//!
//! These graphs were produced by [benchmarks] of the sharded slab implementation,
//! using the [`criterion`] crate.
//!
//! The first shows the results of a benchmark where an increasing number of
//! items are inserted and then removed into a slab concurrently by five
//! threads. It compares the performance of the sharded slab implementation
//! with a `RwLock<slab::Slab>`:
//!
//! <img width="1124" alt="Screen Shot 2019-10-01 at 5 09 49 PM" src="https://user-images.githubusercontent.com/2796466/66078398-cd6c9f80-e516-11e9-9923-0ed6292e8498.png">
//!
//! The second graph shows the results of a benchmark where an increasing
//! number of items are inserted and then removed by a _single_ thread. It
//! compares the performance of the sharded slab implementation with an
//! `RwLock<slab::Slab>` and a `mut slab::Slab`.
//!
//! <img width="925" alt="Screen Shot 2019-10-01 at 5 13 45 PM" src="https://user-images.githubusercontent.com/2796466/66078469-f0974f00-e516-11e9-95b5-f65f0aa7e494.png">
//!
//! These benchmarks demonstrate that, while the sharded approach introduces
//! a small constant-factor overhead, it offers significantly better
//! performance across concurrent accesses.
//!
//! [benchmarks]: https://github.com/hawkw/sharded-slab/blob/master/benches/bench.rs
//! [`criterion`]: https://crates.io/crates/criterion
//!
//! # Implementation Notes
//!
//! See [this page](implementation/index.html) for details on this crate's design
//! and implementation.
//!
#![doc(html_root_url = "https://docs.rs/sharded-slab/0.0.2")]

#[cfg(test)]
macro_rules! thread_local {
    ($($tts:tt)+) => { loom::thread_local!{ $($tts)+ } }
}

#[cfg(not(test))]
macro_rules! thread_local {
    ($($tts:tt)+) => { std::thread_local!{ $($tts)+ } }
}

pub mod implementation;
mod page;
pub(crate) mod sync;
mod tid;
pub(crate) use tid::Tid;
pub(crate) mod cfg;
mod iter;
use cfg::CfgPrivate;
pub use cfg::{Config, DefaultConfig};

use std::{fmt, marker::PhantomData};

/// A sharded slab.
///
/// See the [crate-level documentation](index.html) for details on using this type.
pub struct Slab<T, C: cfg::Config = DefaultConfig> {
    shards: Box<[Shard<T, C>]>,
    _cfg: PhantomData<C>,
}

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
struct Shard<T, C: cfg::Config> {
    #[cfg(debug_assertions)]
    tid: usize,
    /// The local free list for each page.
    ///
    /// These are only ever accessed from this shard's thread, so they are
    /// stored separately from the shared state for the page that can be
    /// accessed concurrently, to minimize false sharing.
    local: Box<[page::Local]>,
    /// The shared state for each page in this shard.
    ///
    /// This consists of the page's metadata (size, previous size), remote free
    /// list, and a pointer to the actual array backing that page.
    shared: Box<[page::Shared<T, C>]>,
}

impl<T> Slab<T> {
    /// Returns a new slab with the default configuration parameters.
    pub fn new() -> Self {
        Self::new_with_config()
    }

    /// Returns a new slab with the provided configuration parameters.
    pub fn new_with_config<C: cfg::Config>() -> Slab<T, C> {
        C::validate();
        let shards = (0..C::MAX_SHARDS).map(Shard::new).collect();
        Slab {
            shards,
            _cfg: PhantomData,
        }
    }
}

impl<T, C: cfg::Config> Slab<T, C> {
    /// The number of bits in each index which are used by the slab.
    ///
    /// If other data is packed into the `usize` indices returned by
    /// [`Slab::insert`], user code is free to use any bits higher than the
    /// `USED_BITS`-th bit freely.
    ///
    /// This is determined by the [`Config`] type that configures the slab's
    /// parameters. By default, all bits are used; this can be changed by
    /// overriding the [`Config::RESERVED_BITS`][res] constant.
    ///
    /// [`Config`]: trait.Config.html
    /// [res]: trait.Config.html#associatedconstant.RESERVED_BITS
    /// [`Slab::insert`]: struct.Slab.html#method.insert
    pub const USED_BITS: usize = C::USED_BITS;

    /// Inserts a value into the slab, returning a key that can be used to
    /// access it.
    ///
    /// If this function returns `None`, then the shard for the current thread
    /// is full and no items can be added until some are removed, or the maximum
    /// number of shards has been reached.
    ///
    /// # Examples
    /// ```rust
    /// # use sharded_slab::Slab;
    /// let slab = Slab::new();
    ///
    /// let key = slab.insert("hello world").unwrap();
    /// assert_eq!(slab.get(key), Some(&"hello world"));
    /// ```
    pub fn insert(&self, value: T) -> Option<usize> {
        let tid = Tid::<C>::current();
        #[cfg(test)]
        println!("insert {:?}", tid);
        self.shards[tid.as_usize()]
            .insert(value)
            .map(|idx| tid.pack(idx))
    }

    /// Removes the value associated with the given key from the slab, returning
    /// it.
    ///
    /// If the slab does not contain a value for that key, `None` is returned
    /// instead.
    pub fn remove(&self, idx: usize) -> Option<T> {
        let tid = C::unpack_tid(idx);
        #[cfg(test)]
        println!("rm {:?}", tid);
        let shard = &self.shards[tid.as_usize()];
        if tid.is_current() {
            shard.remove_local(idx)
        } else {
            shard.remove_remote(idx)
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
        let tid = C::unpack_tid(key);
        #[cfg(test)]
        println!("get {:?}", tid);
        self.shards.get(tid.as_usize())?.get(key)
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

    /// Returns an iterator over all the items in the slab.
    pub fn unique_iter<'a>(&'a mut self) -> iter::UniqueIter<'a, T, C> {
        let mut shards = self.shards.iter_mut();
        let shard = shards.next().expect("must be at least 1 shard");
        let mut pages = shard.iter();
        let slots = pages.next().and_then(page::Shared::iter);
        iter::UniqueIter {
            shards,
            slots,
            pages,
        }
    }
}

impl<T, C: cfg::Config> Shard<T, C> {
    fn new(_idx: usize) -> Self {
        let mut total_sz = 0;
        let shared = (0..C::MAX_PAGES)
            .map(|page_num| {
                let sz = C::page_size(page_num);
                let prev_sz = total_sz;
                total_sz += sz;
                page::Shared::new(sz, prev_sz)
            })
            .collect();
        let local = (0..C::MAX_PAGES).map(|_| page::Local::new()).collect();
        Self {
            #[cfg(debug_assertions)]
            tid: _idx,
            local,
            shared,
        }
    }

    fn insert(&self, value: T) -> Option<usize> {
        let mut value = Some(value);

        // Can we fit the value into an existing page?
        for (page_idx, page) in self.shared.iter().enumerate() {
            let local = self.local(page_idx);
            #[cfg(test)]
            println!("-> page {}; {:?}; {:?}", page_idx, local, page);

            if let Some(poff) = page.insert(local, &mut value) {
                return Some(poff);
            }
        }

        None
    }

    #[inline(always)]
    fn get(&self, idx: usize) -> Option<&T> {
        #[cfg(debug_assertions)]
        debug_assert_eq!(Tid::<C>::from_packed(idx).as_usize(), self.tid);
        let addr = C::unpack_addr(idx);
        let i = addr.index();
        #[cfg(test)]
        println!("-> {:?}; idx {:?}", addr, i);
        if i > self.shared.len() {
            return None;
        }
        self.shared[i].get(addr, idx)
    }

    /// Remove an item on the shard's local thread.
    fn remove_local(&self, idx: usize) -> Option<T> {
        #[cfg(debug_assertions)]
        {
            debug_assert_eq!(Tid::<C>::from_packed(idx).as_usize(), self.tid);
        }
        let addr = C::unpack_addr(idx);
        let page = addr.index();

        #[cfg(test)]
        println!("-> remove_local {:?}; page {:?}", addr, page);

        self.shared
            .get(page)?
            .remove_local(self.local(page), addr, C::unpack_gen(idx))
    }

    /// Remove an item, while on a different thread from the shard's local thread.
    fn remove_remote(&self, idx: usize) -> Option<T> {
        #[cfg(debug_assertions)]
        {
            debug_assert_eq!(Tid::<C>::from_packed(idx).as_usize(), self.tid);
            debug_assert!(Tid::<C>::current().as_usize() != self.tid);
        }
        let addr = C::unpack_addr(idx);
        let page = addr.index();

        #[cfg(test)]
        println!("-> remove_remote {:?}; page {:?}", addr, page);

        self.shared
            .get(page)?
            .remove_remote(addr, C::unpack_gen(idx))
    }

    #[inline(always)]
    fn local(&self, i: usize) -> &page::Local {
        #[cfg(debug_assertions)]
        debug_assert_eq!(
            Tid::<C>::current().as_usize(),
            self.tid,
            "tried to access local data from another thread!"
        );

        &self.local[i]
    }

    fn iter<'a>(&'a self) -> std::slice::Iter<'a, page::Shared<T, C>> {
        self.shared.iter()
    }
}

impl<T: fmt::Debug, C: cfg::Config> fmt::Debug for Slab<T, C> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Slab")
            .field("shards", &self.shards)
            .field("Config", &C::debug())
            .finish()
    }
}

unsafe impl<T: Send, C: cfg::Config> Send for Slab<T, C> {}
unsafe impl<T: Sync, C: cfg::Config> Sync for Slab<T, C> {}

impl<T: fmt::Debug, C: cfg::Config> fmt::Debug for Shard<T, C> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let mut d = f.debug_struct("Shard");

        #[cfg(debug_assertions)]
        d.field("tid", &self.tid);
        d.field("shared", &self.shared).finish()
    }
}

pub(crate) trait Pack<C: cfg::Config>: Sized {
    const LEN: usize;

    const BITS: usize;
    const SHIFT: usize = Self::Prev::SHIFT + Self::Prev::LEN;
    const MASK: usize = Self::BITS << Self::SHIFT;

    type Prev: Pack<C>;

    fn as_usize(&self) -> usize;
    fn from_usize(val: usize) -> Self;

    #[inline(always)]
    fn pack(&self, to: usize) -> usize {
        let value = self.as_usize();
        debug_assert!(value <= Self::BITS);

        (to & !Self::MASK) | (value << Self::SHIFT)
    }

    #[inline(always)]
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
    fn from_usize(_val: usize) -> Self {
        unreachable!()
    }

    fn pack(&self, _to: usize) -> usize {
        unreachable!()
    }

    fn from_packed(_from: usize) -> Self {
        unreachable!()
    }
}

#[cfg(test)]
mod tests;
