use crate::sync::{
    atomic::{self, AtomicUsize, Ordering},
    CausalCell,
};
use crate::{cfg, Pack, Tid};
use std::{fmt, marker::PhantomData};

pub(crate) struct Slot<T, C> {
    /// ABA guard generation counter incremented every time a value is inserted
    /// into the slot.
    gen: AtomicUsize,
    refs: AtomicUsize,
    /// The offset of the next item on the free list.
    next: CausalCell<usize>,
    /// The data stored in the slot.
    item: CausalCell<Option<T>>,
    _cfg: PhantomData<fn(C)>,
}

#[derive(Debug)]
pub(crate) struct Guard<'a, T> {
    item: &'a T,
    refs: &'a AtomicUsize,
}

#[repr(transparent)]
pub(crate) struct Generation<C = cfg::DefaultConfig> {
    value: usize,
    _cfg: PhantomData<fn(C)>,
}

#[derive(Debug, Eq, PartialEq, Copy, Clone)]
#[repr(usize)]
enum Lifecycle {
    NotRemoved = 0b00,
    Marked = 0b01,
    Removing = 0b11,
}

impl<C: cfg::Config> Pack<C> for Generation<C> {
    /// Use all the remaining bits in the word for the generation counter, minus
    /// any bits reserved by the user.
    const LEN: usize = (cfg::WIDTH - C::RESERVED_BITS) - Self::SHIFT;

    type Prev = Tid<C>;

    #[inline(always)]
    fn from_usize(u: usize) -> Self {
        debug_assert!(u <= Self::BITS);
        Self::new(u)
    }

    #[inline(always)]
    fn as_usize(&self) -> usize {
        self.value
    }
}

impl<C: cfg::Config> Generation<C> {
    fn new(value: usize) -> Self {
        Self {
            value,
            _cfg: PhantomData,
        }
    }
}

impl<T, C: cfg::Config> Slot<T, C> {
    pub(in crate::page) fn new(next: usize) -> Self {
        Self {
            gen: AtomicUsize::new(0),
            refs: AtomicUsize::new(0),
            item: CausalCell::new(None),
            next: CausalCell::new(next),
            _cfg: PhantomData,
        }
    }

    #[inline(always)]
    pub(in crate::page) fn get(&self, gen: Generation<C>) -> Option<Guard<'_, T>> {
        let mut lifecycle = self.refs.load(Ordering::Acquire);
        loop {
            let state = Lifecycle::from(lifecycle);

            let current_gen = self.gen.load(Ordering::Acquire);
            #[cfg(test)]
            println!(
                "-> get {:?}; current_gen={:?}; lifecycle={:#x}; state={:?}; refs={:?};",
                gen,
                current_gen,
                lifecycle,
                state,
                lifecycle >> Lifecycle::REFS_SHIFT,
            );

            // Is the index's generation the same as the current generation? If not,
            // the item that index referred to was removed, so return `None`.
            if gen.value != current_gen || state != Lifecycle::NotRemoved {
                return None;
            }

            let new_refs = lifecycle + 4;
            match self.refs.compare_exchange(
                lifecycle,
                new_refs | state as usize,
                Ordering::AcqRel,
                Ordering::Acquire,
            ) {
                Ok(_) => {
                    let item = self.value()?;
                    #[cfg(test)]
                    println!("-> {:?} + 1 refs", lifecycle >> Lifecycle::REFS_SHIFT,);

                    return Some(Guard {
                        item,
                        refs: &self.refs,
                    });
                }
                Err(actual) => {
                    #[cfg(test)]
                    println!("-> retry; lifecycle={:#x};", actual);
                    lifecycle = actual;
                }
            };
        }
    }

    #[inline(always)]
    pub(super) fn value<'a>(&'a self) -> Option<&'a T> {
        self.item.with(|item| unsafe { (&*item).as_ref() })
    }

    #[inline]
    pub(super) fn insert(&self, value: &mut Option<T>) -> Generation<C> {
        debug_assert!(
            self.item.with(|item| unsafe { (*item).is_none() }),
            "inserted into full slot"
        );
        debug_assert!(value.is_some(), "inserted twice");

        // If the `is_removed` bit was not previously set, that's fine; this
        // might have been removed via `remove_now`.
        self.refs
            .store(Lifecycle::NotRemoved as usize, Ordering::Release);

        // Set the new value.
        self.item.with_mut(|item| unsafe {
            *item = value.take();
        });

        let gen = self.gen.load(Ordering::Acquire);

        #[cfg(test)]
        println!("-> {:?}", gen);

        Generation::new(gen)
    }

    #[inline(always)]
    pub(super) fn next(&self) -> usize {
        self.next.with(|next| unsafe { *next })
    }

    #[inline]
    pub(super) fn remove(&self, gen: Generation<C>) -> bool {
        let curr_gen = self.gen.load(Ordering::Acquire);

        #[cfg(test)]
        println!("-> remove deferred; gen={:?};", curr_gen);

        // Is the slot still at the generation we are trying to remove?
        if gen.value != curr_gen {
            return false;
        }

        let prev = self
            .refs
            .fetch_or(Lifecycle::Marked as usize, Ordering::Release);
        #[cfg(test)]
        println!("-> remove deferred; marked, prev={:#2x};", prev);

        // Are there currently outstanding references to the slot? If so, it
        // will have to be removed when those references are dropped.
        if prev & Lifecycle::REFS_MASK > 0 {
            return true;
        }

        // We are releasing the slot, acquire the memory now.
        atomic::fence(Ordering::Acquire);

        // Otherwise, we can remove the slot now!
        #[cfg(test)]
        println!("-> remove deferred; can remove now");
        self.remove_inner(curr_gen).is_some()
    }

    #[inline]
    fn remove_inner(&self, current_gen: usize) -> Option<T> {
        let next_gen = (current_gen + 1) % Generation::<C>::BITS;

        if let Err(_actual) =
            self.gen
                .compare_exchange(current_gen, next_gen, Ordering::AcqRel, Ordering::Acquire)
        {
            #[cfg(test)]
            println!(
                "-> already removed; actual_gen={:?}; previous={:?};",
                _actual, current_gen
            );
            return None;
        }

        #[cfg(test)]
        println!("-> next generation={:?};", next_gen);

        loop {
            let refs = self.refs.load(Ordering::Acquire);

            #[cfg(test)]
            print!("-> refs={:?}", refs);

            if refs & Lifecycle::REFS_MASK == 0 {
                #[cfg(test)]
                println!("; ok to remove!");
                return self.item.with_mut(|item| unsafe { (*item).take() });
            }

            #[cfg(test)]
            println!("; spin");
            atomic::spin_loop_hint();
        }
    }

    #[inline]
    pub(super) fn remove_value(&self, gen: Generation<C>) -> Option<T> {
        let current = self.gen.load(Ordering::Acquire);
        #[cfg(test)]
        println!("-> remove={:?}; current={:?};", gen, current);

        // Is the index's generation the same as the current generation? If not,
        // the item that index referred to was already removed.
        if gen.value != current {
            return None;
        }

        self.remove_inner(current)
    }

    #[inline(always)]
    pub(super) fn set_next(&self, next: usize) {
        self.next.with_mut(|n| unsafe {
            (*n) = next;
        })
    }
}

impl<T, C: cfg::Config> fmt::Debug for Slot<T, C> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Slot")
            .field("gen", &self.gen.load(Ordering::Relaxed))
            .field("next", &self.next())
            .finish()
    }
}

impl<C> fmt::Debug for Generation<C> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_tuple("Generation").field(&self.value).finish()
    }
}

impl<C: cfg::Config> PartialEq for Generation<C> {
    fn eq(&self, other: &Self) -> bool {
        self.value == other.value
    }
}

impl<C: cfg::Config> Eq for Generation<C> {}

impl<C: cfg::Config> PartialOrd for Generation<C> {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        self.value.partial_cmp(&other.value)
    }
}

impl<C: cfg::Config> Ord for Generation<C> {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        self.value.cmp(&other.value)
    }
}

impl<C: cfg::Config> Clone for Generation<C> {
    fn clone(&self) -> Self {
        Self::new(self.value)
    }
}

impl<C: cfg::Config> Copy for Generation<C> {}

impl<'a, T> Guard<'a, T> {
    pub(crate) fn release(&self) -> bool {
        let mut state = self.refs.load(Ordering::Relaxed);
        loop {
            let refs = state >> Lifecycle::REFS_SHIFT;
            let lifecycle = Lifecycle::from(state);
            // if refs == 0 {
            //     #[cfg(test)]
            //     println!("drop on 0 refs; something is weird!");
            //     state = self.refs.load(Ordering::Acquire);
            //     continue;
            // }
            let dropping = refs == 1 && lifecycle == Lifecycle::Marked;
            let new_state = if dropping {
                Lifecycle::Removing as usize
            } else {
                (refs - 1) << Lifecycle::REFS_SHIFT | lifecycle as usize
            };
            #[cfg(test)]
            println!(
                "-> drop guard; lifecycle={:?}; refs={:?}; new_state={:#x}; dropping={:?}",
                lifecycle, refs, new_state, dropping
            );
            match self
                .refs
                .compare_exchange(state, new_state, Ordering::Release, Ordering::Relaxed)
            {
                Ok(_) => return dropping,
                Err(actual) => {
                    #[cfg(test)]
                    println!("-> drop guard; retry, actual={:?}", actual);
                    state = actual;
                }
            }
        }
    }

    pub(crate) fn item(&self) -> &T {
        self.item
    }
}

impl Lifecycle {
    const MASK: usize = 0b11;
    const REFS_MASK: usize = !Self::MASK;
    const REFS_SHIFT: usize = cfg::WIDTH - (Self::MASK.leading_zeros() as usize);
}

impl From<usize> for Lifecycle {
    #[inline(always)]
    fn from(u: usize) -> Self {
        match u & Self::MASK {
            0b00 => Lifecycle::NotRemoved,
            0b01 => Lifecycle::Marked,
            0b11 => Lifecycle::Removing,
            bad => unreachable!("weird lifecycle {:#b}", bad),
        }
    }
}
