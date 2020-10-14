use crate::{page, Shard, Tid};
use std::slice;

pub struct UniqueIter<'a, T, C: crate::cfg::Config> {
    pub(super) shards: slice::IterMut<'a, Shard<Option<T>, C>>,
    pub(super) pages: slice::Iter<'a, page::Shared<Option<T>, C>>,
    pub(super) slots: Option<page::IterUnique<'a, T, C>>,
}

pub struct Iter<'a, T, C>
where
    C: crate::cfg::Config,
{
    pub(super) shards: slice::Iter<'a, Shard<Option<T>, C>>,
    pub(super) current_shard: &'a Shard<Option<T>, C>,
    pub(super) pages: slice::Iter<'a, page::Shared<Option<T>, C>>,
    pub(super) current_page_sz: usize,
    pub(super) slots: Option<page::Iter<'a, T, C>>,
}

impl<'a, T, C: crate::cfg::Config> Iterator for UniqueIter<'a, T, C> {
    type Item = &'a T;
    fn next(&mut self) -> Option<Self::Item> {
        loop {
            if let Some(item) = self.slots.as_mut().and_then(|slots| slots.next()) {
                return Some(item);
            }

            if let Some(page) = self.pages.next() {
                self.slots = page.iter_unique();
                continue;
            }

            if let Some(shard) = self.shards.next() {
                self.pages = shard.iter();
            } else {
                return None;
            }
        }
    }
}

impl<'a, T, C> Iterator for Iter<'a, T, C>
where
    C: crate::cfg::Config,
{
    type Item = crate::Guard<'a, T, C>;
    fn next(&mut self) -> Option<Self::Item> {
        use crate::Pack;

        loop {
            if let Some((idx, inner, gen)) = self.slots.as_mut().and_then(|slots| slots.next()) {
                let shard = self.current_shard;
                let key = shard.tid().pack(
                    gen.pack(page::Addr::<C>::from_usize(idx + self.current_page_sz).pack(0)),
                );
                test_println!(
                    "shard: {:?}; pg {} has ix {}",
                    self.current_shard.tid(),
                    self.current_page_sz,
                    idx
                );
                return Some(crate::Guard { inner, shard, key });
            } else {
                test_println!(
                    "shard: {:?}; pg {} empty",
                    self.current_shard.tid(),
                    self.current_page_sz
                );
                self.slots = None;
            }

            if let Some(page) = self.pages.next() {
                self.current_page_sz = page.prev_sz();
                // test_println!("SLOT ITER: {:?}", page);
                self.slots = page.iter();
                continue;
            }

            if let Some(shard) = self.shards.next() {
                if shard.tid() > Tid::<C>::max_active() {
                    return None;
                }
                self.pages = shard.iter();
                self.current_shard = shard;
                self.current_page_sz = 0;
                self.slots = None;
            // test_println!("iter: next shard {:?}", shard.tid());
            } else {
                // test_println!("iter empty");
                return None;
            }
        }
    }
}
