use crate::{
    cfg::{self, CfgPrivate},
    clear::Clear,
    page,
    sync::{
        alloc,
        atomic::{AtomicPtr, AtomicUsize, Ordering::*},
    },
    tid::Tid,
    Pack,
};

use std::{fmt, ptr};

pub(crate) struct Array<T, C: cfg::Config> {
    shards: Box<[AtomicPtr<alloc::Track<Shard<T, C>>>]>,
    max: AtomicUsize,
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
pub(crate) struct Shard<T, C: cfg::Config> {
    /// The shard's parent thread ID.
    pub(crate) tid: usize,
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

impl<T, C> Shard<T, C>
where
    C: cfg::Config,
{
    #[inline(always)]
    pub(crate) fn get<U>(
        &self,
        idx: usize,
        f: impl FnOnce(&T) -> &U,
    ) -> Option<page::slot::Guard<'_, U, C>> {
        debug_assert_eq!(Tid::<C>::from_packed(idx).as_usize(), self.tid);
        let (addr, page_index) = page::indices::<C>(idx);

        test_println!("-> {:?}", addr);
        if page_index > self.shared.len() {
            return None;
        }

        self.shared[page_index].get(addr, idx, f)
    }

    pub(crate) fn new(tid: usize) -> Self {
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
        Self { tid, local, shared }
    }
}

impl<T, C> Shard<Option<T>, C>
where
    C: cfg::Config,
{
    /// Remove an item on the shard's local thread.
    pub(crate) fn take_local(&self, idx: usize) -> Option<T> {
        debug_assert_eq!(Tid::<C>::from_packed(idx).as_usize(), self.tid);
        let (addr, page_index) = page::indices::<C>(idx);

        test_println!("-> remove_local {:?}", addr);

        self.shared
            .get(page_index)?
            .take(addr, C::unpack_gen(idx), self.local(page_index))
    }

    /// Remove an item, while on a different thread from the shard's local thread.
    pub(crate) fn take_remote(&self, idx: usize) -> Option<T> {
        debug_assert_eq!(Tid::<C>::from_packed(idx).as_usize(), self.tid);
        debug_assert!(Tid::<C>::current().as_usize() != self.tid);

        let (addr, page_index) = page::indices::<C>(idx);

        test_println!("-> take_remote {:?}; page {:?}", addr, page_index);

        let shared = self.shared.get(page_index)?;
        shared.take(addr, C::unpack_gen(idx), shared.free_list())
    }

    pub(crate) fn remove_local(&self, idx: usize) -> bool {
        debug_assert_eq!(Tid::<C>::from_packed(idx).as_usize(), self.tid);
        let (addr, page_index) = page::indices::<C>(idx);

        if page_index > self.shared.len() {
            return false;
        }

        self.shared[page_index].remove(addr, C::unpack_gen(idx), self.local(page_index))
    }

    pub(crate) fn remove_remote(&self, idx: usize) -> bool {
        debug_assert_eq!(Tid::<C>::from_packed(idx).as_usize(), self.tid);
        let (addr, page_index) = page::indices::<C>(idx);

        if page_index > self.shared.len() {
            return false;
        }

        let shared = &self.shared[page_index];
        shared.remove(addr, C::unpack_gen(idx), shared.free_list())
    }

    pub(crate) fn iter<'a>(&'a self) -> std::slice::Iter<'a, page::Shared<Option<T>, C>> {
        self.shared.iter()
    }
}

impl<T, C> Shard<T, C>
where
    T: Clear + Default,
    C: cfg::Config,
{
    pub(crate) fn init_with<F>(&self, mut func: F) -> Option<usize>
    where
        F: FnMut(&page::slot::Slot<T, C>) -> Option<page::slot::Generation<C>>,
    {
        // Can we fit the value into an existing page?
        for (page_idx, page) in self.shared.iter().enumerate() {
            let local = self.local(page_idx);

            test_println!("-> page {}; {:?}; {:?}", page_idx, local, page);

            if let Some(poff) = page.init_with(local, &mut func) {
                return Some(poff);
            }
        }

        None
    }

    pub(crate) fn mark_clear_local(&self, idx: usize) -> bool {
        debug_assert_eq!(Tid::<C>::from_packed(idx).as_usize(), self.tid);
        let (addr, page_index) = page::indices::<C>(idx);

        if page_index > self.shared.len() {
            return false;
        }

        self.shared[page_index].mark_clear(addr, C::unpack_gen(idx), self.local(page_index))
    }

    pub(crate) fn mark_clear_remote(&self, idx: usize) -> bool {
        debug_assert_eq!(Tid::<C>::from_packed(idx).as_usize(), self.tid);
        let (addr, page_index) = page::indices::<C>(idx);

        if page_index > self.shared.len() {
            return false;
        }

        let shared = &self.shared[page_index];
        shared.mark_clear(addr, C::unpack_gen(idx), shared.free_list())
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
}

impl<T: fmt::Debug, C: cfg::Config> fmt::Debug for Shard<T, C> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let mut d = f.debug_struct("Shard");

        #[cfg(debug_assertions)]
        d.field("tid", &self.tid);
        d.field("shared", &self.shared).finish()
    }
}

impl<T, C> Array<T, C>
where
    C: cfg::Config,
{
    pub(crate) fn new() -> Self {
        let mut shards = Vec::with_capacity(C::MAX_SHARDS);
        for _ in 0..C::MAX_SHARDS {
            // XXX(eliza): T_T this could be avoided with maybeuninit or something...
            shards.push(AtomicPtr::new(ptr::null_mut()));
        }
        Self {
            shards: shards.into(),
            max: AtomicUsize::new(0),
        }
    }

    #[inline]
    pub(crate) fn get<'a>(&'a self, idx: usize) -> Option<&'a Shard<T, C>> {
        let ptr = self.shards.get(idx)?.load(Acquire);
        let ptr = ptr::NonNull::new(ptr)?;
        let shard = unsafe {
            // Safety: the returned pointer will not outlive the shards array.
            &*ptr.as_ptr()
        };
        Some(shard.get_ref())
    }

    #[inline]
    pub(crate) fn current<'a>(&'a self) -> (Tid<C>, &'a Shard<T, C>) {
        let tid = Tid::<C>::current();
        test_println!("current: {:?}", tid);
        let idx = tid.as_usize();
        // It's okay for this to be relaxed. The value is only ever stored by
        // the thread that corresponds to the index, and we are that thread.
        let ptr = self.shards[idx].load(Relaxed);
        let shard = ptr::NonNull::new(ptr)
            .map(|shard| unsafe {
                test_println!("-> shard exists");
                // Safety: `NonNull::as_ref` is unsafe because it creates a
                // reference with a potentially unbounded lifetime. However, the
                // reference does not outlive this function, so it's fine to use it
                // here.
                &*shard.as_ptr()
            })
            .unwrap_or_else(|| {
                let shard = Box::new(alloc::Track::new(Shard::new(idx)));
                let ptr = Box::into_raw(shard);
                test_println!("-> allocated new shard at {:p}", ptr);
                self.shards[idx]
                    .compare_exchange(ptr::null_mut(), ptr, AcqRel, Acquire)
                    .expect(
                        "a shard can only be inserted by the thread that owns it, this is a bug!",
                    );

                test_println!("-> ...and set shard {} to point to {:p}", idx, ptr);
                let mut max = self.max.load(Acquire);
                while max < idx {
                    match self.max.compare_exchange(max, idx, AcqRel, Acquire) {
                        Ok(_) => break,
                        Err(actual) => max = actual,
                    }
                }
                test_println!("-> highest index={}, prev={}", std::cmp::max(max, idx), max);
                unsafe {
                    // Safety: we just put it there!
                    &*ptr
                }
            })
            .get_ref();
        (tid, shard)
    }
}

impl<T, C: cfg::Config> Drop for Array<T, C> {
    fn drop(&mut self) {
        // XXX(eliza): this could be `with_mut` if we wanted to impl a wrapper for std atomics to change `get_mut` to `with_mut`...
        let max = self.max.load(Acquire);
        for shard in &self.shards[0..=max] {
            // XXX(eliza): this could be `with_mut` if we wanted to impl a wrapper for std atomics to change `get_mut` to `with_mut`...
            let ptr = shard.load(Acquire);
            if ptr.is_null() {
                continue;
            }
            let shard = unsafe {
                // Safety: this is the only place where these boxes are
                // deallocated, and we have exclusive access to the shard array,
                // because...we are dropping it...
                Box::from_raw(ptr)
            };
            drop(shard)
        }
    }
}

impl<T, C: cfg::Config> fmt::Debug for Array<T, C> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let max = self.max.load(Acquire);
        let mut list = f.debug_list();
        for shard in &self.shards[0..=max] {
            list.entry(&format_args!("{:p}", shard.load(Acquire)));
        }
        list.finish()
    }
}
