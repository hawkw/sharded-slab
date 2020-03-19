use std::{sync::Arc, collections, hash, ops::DerefMut, sync};

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

impl<T> Clear for Option<T> {
    fn clear(&mut self) {
        let _ = self.take();
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

impl<T> Clear for Arc<T> where T: Clear {
    #[inline]
    fn clear(&mut self) {
        self.clear()
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

impl<T: Clear> Clear for sync::Mutex<T> {
    #[inline]
    fn clear(&mut self) {
        self.get_mut().unwrap().clear();
    }
}

impl<T: Clear> Clear for sync::RwLock<T> {
    #[inline]
    fn clear(&mut self) {
        self.write().unwrap().clear();
    }
}
