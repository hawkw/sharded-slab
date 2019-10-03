use crate::{
    cfg::{self, Config},
    page::{self, Page},
    sync::CausalCell,
    Shard,
};
use std::slice;
pub struct UniqueIter<'a, T, C: Config = cfg::DefaultConfig, P = ()> {
    pub(super) shards: slice::IterMut<'a, CausalCell<Shard<T, C, P>>>,
    pub(super) pages: slice::Iter<'a, Page<T, C, P>>,
    pub(super) slots: page::Iter<'a, T, C, P>,
}
cfg_prefix! {
    #[cfg(feature = "pool")](impl<'a, T, P: Default + crate::pool::Clear, C: Config> Iterator for UniqueIter<'a, T, C, P>)
    #[cfg(else)](impl<'a, T, P, C: Config> Iterator for UniqueIter<'a, T, C, P>)
    {
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
}
