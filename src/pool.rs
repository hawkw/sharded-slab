use super::*;
use std::sync::{Mutex, RwLock};
use std::{collections, hash, ops::DerefMut};

/// A sharded slab.
///
/// See the [crate-level documentation](index.html) for details on using this type.
pub struct Slab<T, P, C: cfg::Config = DefaultConfig> {
    shards: Box<[CausalCell<Shard<T, C, P>>]>,
    _cfg: PhantomData<C>,
}

/// A heap-allocated type whose storage may be cleared of data, retaining any
/// allocated _capacity_.
///
/// Types must implement `Clear` to be pooled.
pub trait Clear {
    /// Clear all data in `self`, retaining the allocated capacithy.
    ///
    /// # Note
    ///
    /// This should only be implemented for types whose clear operation *retains
    /// any allocations* for that type. Types such as `BTreeMap`, whose
    /// `clear()` method releases the existing allocation, should *not*
    /// implement this trait.
    fn clear(&mut self);
}

impl<T, P> Slab<T, P>
where
    P: Default + Clear,
{
    /// Returns a new slab with the default configuration parameters.
    pub fn new() -> Self {
        Self::new_with_config()
    }

    /// Returns a new slab with the provided configuration parameters.
    pub fn new_with_config<C: Config>() -> Slab<T, C, P> {
        C::validate();
        let mut shards = Vec::with_capacity(C::MAX_SHARDS);

        #[allow(unused_mut)]
        let mut idx = 0;
        shards.resize_with(C::MAX_SHARDS, || {
            let shard = Shard::new(idx);

            #[cfg(debug_assertions)]
            {
                idx += 1;
            }

            CausalCell::new(shard)
        });
        Slab {
            shards: shards.into_boxed_slice(),
            _cfg: PhantomData,
        }
    }
}

impl<T, C, P> Slab<T, C, P>
where
    P: Default + Clear,
    C: Config,
{
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
        let tid = C::unpack_tid(idx);
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
        let tid = C::unpack_tid(key);
        #[cfg(test)]
        println!("get {:?}", tid);
        self.shards
            .get(tid.as_usize())?
            .with(|shard| unsafe { (*shard).get(key) })
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
    pub fn get_pooled(&self, key: usize) -> Option<&P> {
        let tid = C::unpack_tid(key);
        #[cfg(test)]
        println!("get {:?}", tid);
        self.shards
            .get(tid.as_usize())?
            .with(|shard| unsafe { (*shard).get_pooled(key) })
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

    /// Returns the number of items currently stored in the slab.
    pub fn len(&self) -> usize {
        self.shards
            .iter()
            .map(|shard| shard.with(|shard| unsafe { (*shard).len() }))
            .sum()
    }

    /// Returns the current number of items which may be stored in the slab
    /// without allocating.
    pub fn capacity(&self) -> usize {
        self.total_capacity() - self.len()
    }

    /// Returns an iterator over all the items in the slab.
    pub fn unique_iter<'a>(&'a mut self) -> iter::UniqueIter<'a, T, C, P> {
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

impl<T: fmt::Debug, C: cfg::Config, P> fmt::Debug for Slab<T, C, P> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Slab")
            // .field("shards", &self.shards)
            .field("Config", &C::debug())
            .finish()
    }
}

unsafe impl<T: Send, C: cfg::Config> Send for Slab<T, C, P> {}
unsafe impl<T: Sync, C: cfg::Config> Sync for Slab<T, C, P> {}

// ===== impl Clear =====

impl<T> Clear for RwLock<T>
where
    T: Clear,
{
    fn clear(&mut self) {
        let mut lock = match self.write() {
            Err(_) if std::thread::panicking() => return,
            res => res.unwrap(),
        };
        lock.clear();
    }
}

impl<T> Clear for Mutex<T>
where
    T: Clear,
{
    fn clear(&mut self) {
        let mut lock = match self.lock() {
            Err(_) if std::thread::panicking() => return,
            res => res.unwrap(),
        };
        lock.clear();
    }
}

impl<T> Clear for Box<T>
where
    T: Clear,
{
    #[inline]
    fn clear(&mut self) {
        self.deref_mut().clear()
    }
}

impl<T> Clear for Vec<T> {
    #[inline]
    fn clear(&mut self) {
        Vec::clear(self)
    }
}

impl<K, V, S> Clear for collections::HashMap<K, V, S>
where
    K: hash::Hash + Eq,
    S: hash::BuildHasher,
{
    #[inline]
    fn clear(&mut self) {
        collections::HashMap::clear(self)
    }
}

impl<T, S> Clear for collections::HashSet<T, S>
where
    T: hash::Hash + Eq,
    S: hash::BuildHasher,
{
    #[inline]
    fn clear(&mut self) {
        collections::HashSet::clear(self)
    }
}

impl Clear for String {
    #[inline]
    fn clear(&mut self) {
        String::clear(self)
    }
}

impl Clear for () {
    fn clear(&mut self) {
        // nop
    }
}
