use crate::{
    page::{self, Page},
    sync::CausalCell,
    Shard,
};
use std::slice;
pub struct UniqueIter<'a, T, P: crate::cfg::Config> {
    pub(super) shards: slice::IterMut<'a, CausalCell<Shard<T, P>>>,
    pub(super) pages: slice::Iter<'a, Page<T, P>>,
    pub(super) slots: page::Iter<'a, T, P>,
}

impl<'a, T, P: crate::cfg::Config> Iterator for UniqueIter<'a, T, P> {
    type Item = &'a T;
    fn next(&mut self) -> Option<Self::Item> {
        loop {
            if let Some(item) = self.slots.next() {
                return Some(item);
            }

            if let Some(page) = self.pages.next() {
                self.slots = page.iter();
            }

            if let Some(shard) = self.shards.next() {
                self.pages = shard.with(|shard| unsafe {
                    // This is safe, because this iterator has unique mutable access
                    // to the whole slab.
                    (*shard).iter()
                });
            } else {
                return None;
            }
        }
    }
}
