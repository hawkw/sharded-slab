use crate::{
    cfg::{self, CfgPrivate, DefaultConfig},
    clear::Clear,
    page,
    tid::Tid,
    Pack, Shard,
};

use std::marker::PhantomData;

pub struct Pool<T: Clear + Default, C: cfg::Config = DefaultConfig> {
    shards: Box<[Shard<T, C>]>,
    _cfg: PhantomData<C>,
}

impl<T: Clear + Default> Pool<T> {
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

/// A guard that allows access to an object in a slab.
///
/// While the guard exists, it indicates to the slab that the item the guard references is
/// currently being accessed. If the item is removed from the pool while the guard exists, the
/// removal will be deferred until all guards are dropped.
pub struct PoolGuard<'a, T, C: cfg::Config = DefaultConfig> {
    inner: page::slot::Guard<'a, T, C>,
    shard: &'a Shard<T, C>,
    key: usize,
}

impl<T: Clear + Default, C: cfg::Config> Pool<T, C> {
    /// The number of bits in each index which are used by the slab.
    ///
    /// If other data is packed into the `usize` indices returned by
    /// [`Pool::create`], user code is free to use any bits higher than the
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
    /// let key = pool.create().unwrap();
    /// assert_eq!(pool.get(key).unwrap(), String::from(""));
    /// ```
    pub fn create(&self) -> Option<usize> {
        let tid = Tid::<C>::current();
        test_println!("pool: create {:?}", tid);
        let value = T::default();
        self.shards[tid.as_usize()]
            .insert(value)
            .map(|idx| tid.pack(idx))
    }

    pub fn create_with<F>(&self, fun: F) -> Option<usize>
    where
        F: FnOnce() -> T,
    {
        let tid = Tid::<C>::current();
        test_println!("pool: create {:?}", tid);
        let value = fun();
        self.shards[tid.as_usize()]
            .insert(value)
            .map(|idx| tid.pack(idx))
    }

    /// Return a reference to the value associated with the given key.
    ///
    /// If the slab does not contain a value for the given key, `None` is returned instead.
    ///
    /// # Examples
    ///
    /// ```rust
    /// let pool: Pool<String> = sharded_slab::Pool::new();
    /// let key = pool.create().unwrap();
    ///
    /// assert_eq!(pool.get(key).unwrap(), String::from(""));
    /// assert!(slab.get(12345).is_none());
    /// ```
    pub fn get(&self, key: usize) -> Option<PoolGuard<'_, T, C>> {
        let tid = C::unpack_tid(key);

        test_println!("pool: get{:?}; current={:?}", tid, Tid::<C>::current());
        let inner = self.shards.get(tid.as_usize())?.get(key)?;

        Some(PoolGuard {
            inner,
            // Safe access as previous line checks for validity
            shard: &self.shards[tid.as_usize()],
            key,
        })
    }
}
