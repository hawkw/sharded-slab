use crate::{
    cfg::{self, CfgPrivate, DefaultConfig},
    clear::Clear,
    page,
    tid::Tid,
    Pack, Shard,
};

use std::{fmt, marker::PhantomData};

pub struct Pool<T, C = DefaultConfig>
where
    T: Clear + Default,
    C: cfg::Config,
{
    shards: Box<[Shard<T, C>]>,
    _cfg: PhantomData<C>,
}

impl<T> Pool<T>
where
    T: Clear + Default,
{
    pub fn new() -> Self {
        Self::new_with_config()
    }

    /// Returns a new `Pool` with the provided configuration parameters.
    pub fn new_with_config<C: cfg::Config>() -> Pool<T, C> {
        C::validate();
        let shards = (0..C::MAX_SHARDS).map(Shard::new).collect();
        Pool {
            shards,
            _cfg: PhantomData,
        }
    }
}

/// A guard that allows access to an object in a pool.
///
/// While the guard exists, it indicates to the pool that the item the guard references is
/// currently being accessed. If the item is removed from the pool while the guard exists, the
/// removal will be deferred until all guards are dropped.
pub struct PoolGuard<'a, T, C>
where
    T: Clear + Default,
    C: cfg::Config,
{
    inner: page::slot::Guard<'a, T, C>,
    shard: &'a Shard<T, C>,
    key: usize,
}

impl<T, C> Pool<T, C>
where
    T: Clear + Default,
    C: cfg::Config,
{
    /// The number of bits in each index which are used by the pool.
    ///
    /// If other data is packed into the `usize` indices returned by
    /// [`Pool::create`], user code is free to use any bits higher than the
    /// `USED_BITS`-th bit freely.
    ///
    /// This is determined by the [`Config`] type that configures the pool's
    /// parameters. By default, all bits are used; this can be changed by
    /// overriding the [`Config::RESERVED_BITS`][res] constant.
    ///
    /// [`Config`]: trait.Config.html
    /// [res]: trait.Config.html#associatedconstant.RESERVED_BITS
    /// [`Slab::insert`]: struct.Slab.html#method.insert
    pub const USED_BITS: usize = C::USED_BITS;

    /// Creates a new object in the pool, returning a key that can be used to access it.
    ///
    /// If this function returns `None`, then the shard for the current thread is full and no items
    /// can be added until some are removed, or the maximum number of shards has been reached.
    ///
    /// # Examples
    /// ```rust
    /// # use sharded_slab::Pool;
    /// let pool: Pool<String> = Pool::new();
    ///
    /// let key = pool.create(|item| *item = "hello".to_string()).unwrap();
    /// assert_eq!(pool.get(key).unwrap(), String::from("Hello"));
    /// ```
    pub fn create(&self, mut initilizer: impl FnMut(&mut T)) -> Option<usize> {
        let tid = Tid::<C>::current();
        test_println!("pool: create {:?}", tid);
        self.shards[tid.as_usize()]
            .get_initialized_slot(&mut initilizer)
            .map(|idx| tid.pack(idx))
    }

    pub fn create_with(&self, value: T) -> Option<usize>
    where
        T: Clone,
    {
        let tid = Tid::<C>::current();
        test_println!("pool: create_with {:?}", tid);
        self.shards[tid.as_usize()]
            .get_initialized_slot(&mut move |item| *item = value.clone())
            .map(|idx| tid.pack(idx))
    }

    /// Return a reference to the value associated with the given key.
    ///
    /// If the pool does not contain a value for the given key, `None` is returned instead.
    ///
    /// # Examples
    ///
    /// ```rust
    /// let pool: Pool<String> = sharded_slab::Pool::new();
    /// let key = pool.create().unwrap();
    ///
    /// assert_eq!(pool.get(key).unwrap(), String::from(""));
    /// assert!(pool.get(12345).is_none());
    /// ```
    pub fn get(&self, key: usize) -> Option<PoolGuard<'_, T, C>> {
        let tid = C::unpack_tid(key);

        test_println!("pool: get{:?}; current={:?}", tid, Tid::<C>::current());
        let inner = self.shards.get(tid.as_usize())?.get(key, |x| x)?;

        Some(PoolGuard {
            inner,
            // Safe access as previous line checks for validity
            shard: &self.shards[tid.as_usize()],
            key,
        })
    }

    /// Remove the value using the storage associated with the given key from the pool, reutrning
    /// `true` if the value was removed.
    ///
    /// Unlike [`clear`], this method does _not_ block the current thread until the value can be
    /// removed. Instead, if another thread is currently accessing that value, this marks it to be
    /// removed by that thread when it finishes accessing the value.
    ///
    /// # Examples
    ///
    /// [`clear`]: #method.clear
    pub fn remove(&self, key: usize) -> bool {
        let tid = C::unpack_tid(key);

        let shard = self.shards.get(tid.as_usize());
        if tid.is_current() {
            shard
                .map(|shard| shard.mark_clear_local(key))
                .unwrap_or(false)
        } else {
            shard
                .map(|shard| shard.mark_clear_remote(key))
                .unwrap_or(false)
        }
    }

    /// Clears the value in the storage associated with the given key from the pool, returning it.
    ///
    /// If the pool does not contain a value for that key, false is returned instead.
    ///
    /// **Note**: If the storage associated with the given key is being currently accessed by
    /// another thread, this method will block the current thread until the item is no longer
    /// accessed. if this is not desired, use [`remove`] instead.
    ///
    /// [`remove`]: #method.remove
    pub fn clear(&self, key: usize) -> bool {
        let tid = C::unpack_tid(key);

        let shard = self.shards.get(tid.as_usize());
        if tid.is_current() {
            shard.map(|shard| shard.clear_local(key)).unwrap_or(false)
        } else {
            shard.map(|shard| shard.clear_remote(key)).unwrap_or(false)
        }
    }
}

unsafe impl<T, C> Send for Pool<T, C>
where
    T: Send + Clear + Default,
    C: cfg::Config,
{
}
unsafe impl<T, C> Sync for Pool<T, C>
where
    T: Sync + Clear + Default,
    C: cfg::Config,
{
}

impl<'a, T, C> PoolGuard<'a, T, C>
where
    T: Clear + Default,
    C: cfg::Config,
{
    /// Returns the key used to access this guard
    pub fn key(&self) -> usize {
        self.key
    }
}

impl<'a, T, C> std::ops::Deref for PoolGuard<'a, T, C>
where
    T: Clear + Default,
    C: cfg::Config,
{
    type Target = T;

    fn deref(&self) -> &Self::Target {
        self.inner.item()
    }
}

impl<'a, T, C> Drop for PoolGuard<'a, T, C>
where
    T: Clear + Default,
    C: cfg::Config,
{
    fn drop(&mut self) {
        use crate::sync::atomic;
        test_println!(" -> drop PoolGuard: clearing data");
        atomic::fence(atomic::Ordering::Acquire);
        if Tid::<C>::current().as_usize() == self.shard.tid {
            self.shard.mark_clear_local(self.key);
        } else {
            self.shard.mark_clear_remote(self.key);
        }
    }
}

impl<'a, T, C> fmt::Debug for PoolGuard<'a, T, C>
where
    T: fmt::Debug + Clear + Default,
    C: cfg::Config,
{
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Debug::fmt(self.inner.item(), f)
    }
}

impl<'a, T, C> PartialEq<T> for PoolGuard<'a, T, C>
where
    T: PartialEq<T> + Clear + Default,
    C: cfg::Config,
{
    fn eq(&self, other: &T) -> bool {
        *self.inner.item() == *other
    }
}
