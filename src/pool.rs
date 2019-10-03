use std::sync::{Mutex, RwLock};
use std::{collections, hash, ops::DerefMut};

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
