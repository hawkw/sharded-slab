use crate::sync::{
    atomic::{self, AtomicUsize, Ordering},
    CausalCell,
};
use crate::{cfg, Pack, Tid};
use std::{fmt, marker::PhantomData};

pub(crate) struct Slot<T, C> {
    lifecycle: AtomicUsize,
    /// The offset of the next item on the free list.
    next: CausalCell<usize>,
    /// The data stored in the slot.
    item: CausalCell<Option<T>>,
    _cfg: PhantomData<fn(C)>,
}

#[derive(Debug)]
pub(crate) struct Guard<'a, T, C = cfg::DefaultConfig> {
    item: &'a T,
    lifecycle: &'a AtomicUsize,
    _cfg: PhantomData<fn(C)>,
}

#[repr(transparent)]
pub(crate) struct Generation<C = cfg::DefaultConfig> {
    value: usize,
    _cfg: PhantomData<fn(C)>,
}

struct LifecycleGen<C>(Generation<C>);

#[repr(transparent)]
struct RefCount<C = cfg::DefaultConfig> {
    value: usize,
    _cfg: PhantomData<fn(C)>,
}

struct Lifecycle<C> {
    state: State,
    _cfg: PhantomData<fn(C)>,
}

#[derive(Debug, Eq, PartialEq, Copy, Clone)]
#[repr(usize)]
enum State {
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
            lifecycle: AtomicUsize::new(0),
            item: CausalCell::new(None),
            next: CausalCell::new(next),
            _cfg: PhantomData,
        }
    }

    #[inline(always)]
    pub(in crate::page) fn get(&self, gen: Generation<C>) -> Option<Guard<'_, T, C>> {
        let mut lifecycle = self.lifecycle.load(Ordering::Acquire);
        loop {
            let state = Lifecycle::<C>::from_packed(lifecycle);
            let current_gen = LifecycleGen::<C>::from_packed(lifecycle).0;
            let refs = RefCount::<C>::from_packed(lifecycle);

            test_println!(
                "-> get {:?}; current_gen={:?}; lifecycle={:#x}; state={:?}; refs={:?};",
                gen,
                current_gen,
                lifecycle,
                state,
                refs,
            );

            // Is the index's generation the same as the current generation? If not,
            // the item that index referred to was removed, so return `None`.
            if gen != current_gen || state != Lifecycle::NOT_REMOVED {
                test_println!("-> get: no longer exists!");
                return None;
            }

            let new_refs = refs.incr();
            match self.lifecycle.compare_exchange(
                lifecycle,
                new_refs.pack(current_gen.pack(state.pack(0))),
                Ordering::AcqRel,
                Ordering::Acquire,
            ) {
                Ok(_) => {
                    let item = self.value()?;

                    test_println!("-> {:?}", new_refs);

                    return Some(Guard {
                        item,
                        lifecycle: &self.lifecycle,
                        _cfg: PhantomData,
                    });
                }
                Err(actual) => {
                    test_println!("-> get: retrying; lifecycle={:#x};", actual);
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
    pub(super) fn insert(&self, value: &mut Option<T>) -> Option<Generation<C>> {
        debug_assert!(
            self.item.with(|item| unsafe { (*item).is_none() }),
            "inserted into full slot"
        );
        debug_assert!(value.is_some(), "inserted twice");

        let lifecycle = self.lifecycle.load(Ordering::Acquire);
        let gen = LifecycleGen::from_packed(lifecycle).0;
        let refs = RefCount::<C>::from_packed(lifecycle);

        test_println!(
            "-> insert; state={:?}; gen={:?}; refs={:?}",
            Lifecycle::<C>::from_packed(lifecycle),
            gen,
            refs
        );
        if refs.value != 0 {
            test_println!("-> insert while referenced! cancelling");
            return None;
        }
        let new_lifecycle = gen.pack(Lifecycle::<C>::NOT_REMOVED.pack(0));
        if let Err(_actual) = self.lifecycle.compare_exchange(
            lifecycle,
            new_lifecycle,
            Ordering::AcqRel,
            Ordering::Acquire,
        ) {
            test_println!(
                "-> modified during insert, cancelling! new={:#x}; expected={:#x}; actual={:#x};",
                new_lifecycle,
                lifecycle,
                _actual
            );
            return None;
        }

        // Set the new value.
        self.item.with_mut(|item| unsafe {
            *item = value.take();
        });

        test_println!("-> inserted at {:?}", gen);

        Some(gen)
    }

    #[inline(always)]
    pub(super) fn next(&self) -> usize {
        self.next.with(|next| unsafe { *next })
    }

    #[inline]
    pub(super) fn remove(&self, gen: Generation<C>) -> bool {
        let mut lifecycle = self.lifecycle.load(Ordering::Acquire);
        let mut curr_gen = LifecycleGen::from_packed(lifecycle).0;
        let prev;
        loop {
            test_println!(
                "-> remove deferred; gen={:?}; current_gen={:?};",
                gen,
                curr_gen
            );

            // Is the slot still at the generation we are trying to remove?
            if gen != curr_gen {
                return false;
            }

            let new_lifecycle = Lifecycle::<C>::MARKED.pack(lifecycle);
            test_println!(
                "-> remove deferred; old_lifecycle={:#x}; new_lifecycle={:#x};",
                lifecycle,
                new_lifecycle
            );
            match self.lifecycle.compare_exchange(
                lifecycle,
                new_lifecycle,
                Ordering::AcqRel,
                Ordering::Acquire,
            ) {
                Ok(actual) => {
                    prev = actual;
                    break;
                }
                Err(actual) => {
                    test_println!("-> remove deferred; retrying");
                    lifecycle = actual;
                    curr_gen = LifecycleGen::from_packed(lifecycle).0;
                }
            }
        }

        let refs = RefCount::<C>::from_packed(prev);
        test_println!(
            "-> remove deferred; marked, prev={:#2x}; refs={:?};",
            prev,
            refs
        );

        // Are there currently outstanding references to the slot? If so, it
        // will have to be removed when those references are dropped.
        if refs.value > 0 {
            return true;
        }

        // We are releasing the slot, acquire the memory now.
        atomic::fence(Ordering::Acquire);

        // Otherwise, we can remove the slot now!

        test_println!("-> remove deferred; can remove now");
        self.remove_inner(curr_gen).is_some()
    }

    #[inline]
    fn remove_inner(&self, current_gen: Generation<C>) -> Option<T> {
        let mut lifecycle = self.lifecycle.load(Ordering::Acquire);
        let mut advanced = false;
        let next_gen = current_gen.advance();
        loop {
            let gen = Generation::from_packed(lifecycle);
            test_println!("-> remove_inner; lifecycle={:#x}; expected_gen={:?}; current_gen={:?}; next_gen={:?};",
                lifecycle,
                current_gen,
                next_gen,
                gen
            );
            if (!advanced) && gen != current_gen {
                test_println!("-> already removed!");
                return None;
            }
            match self.lifecycle.compare_exchange(
                lifecycle,
                next_gen.pack(lifecycle),
                Ordering::AcqRel,
                Ordering::Acquire,
            ) {
                Ok(actual) => {
                    advanced = true;
                    let refs = RefCount::<C>::from_packed(actual);
                    test_println!("-> advanced gen; lifecycle={:#x}; refs={:?};", actual, refs);
                    if refs.value == 0 {
                        test_println!("-> ok to remove!");
                        return self.item.with_mut(|item| unsafe { (*item).take() });
                    } else {
                        test_println!("-> refs={:?}; spin...", refs);
                        atomic::spin_loop_hint();
                    }
                }
                Err(actual) => {
                    test_println!("-> retrying; lifecycle={:#x};", actual);
                    lifecycle = actual;
                }
            }
        }
    }

    #[inline]
    pub(super) fn remove_value(&self, gen: Generation<C>) -> Option<T> {
        self.remove_inner(gen)
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
        let lifecycle = self.lifecycle.load(Ordering::Relaxed);
        f.debug_struct("Slot")
            .field("lifecycle", &format_args!("{:#x}", lifecycle))
            .field("state", &Lifecycle::<C>::from_packed(lifecycle).state)
            .field("gen", &LifecycleGen::<C>::from_packed(lifecycle).0)
            .field("refs", &RefCount::<C>::from_packed(lifecycle))
            .field("next", &self.next())
            .finish()
    }
}

// === impl Generation ===

impl<C> fmt::Debug for Generation<C> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_tuple("Generation").field(&self.value).finish()
    }
}

impl<C: cfg::Config> Generation<C> {
    fn advance(self) -> Self {
        Self::from_usize((self.value + 1) % Self::BITS)
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

// === impl Guard ===

impl<'a, T, C: cfg::Config> Guard<'a, T, C> {
    pub(crate) fn release(&self) -> bool {
        let mut lifecycle = self.lifecycle.load(Ordering::Acquire);
        loop {
            let refs = RefCount::<C>::from_packed(lifecycle);
            let state = Lifecycle::<C>::from_packed(lifecycle).state;
            // if refs == 0 {
            //     #[cfg(test)]
            //     test_println!("drop on 0 refs; something is weird!");
            //     state = self.refs.load(Ordering::Acquire);
            //     continue;
            // }
            let dropping = refs.value == 1 && state == State::Marked;
            let new_state = if dropping {
                State::Removing as usize
            } else {
                refs.decr().pack(lifecycle)
            };

            test_println!(
                "-> drop guard; lifecycle={:?}; refs={:?}; new_state={:#x}; dropping={:?}",
                lifecycle,
                refs,
                new_state,
                dropping
            );
            match self.lifecycle.compare_exchange(
                lifecycle,
                new_state,
                Ordering::AcqRel,
                Ordering::Acquire,
            ) {
                Ok(_) => {
                    test_println!(
                        "-> drop guard: done; new_state={:#x}; dropping={:?}",
                        new_state,
                        dropping
                    );
                    return dropping;
                }
                Err(actual) => {
                    test_println!("-> drop guard; retry, actual={:?}", actual);
                    lifecycle = actual;
                }
            }
        }
    }

    pub(crate) fn item(&self) -> &T {
        self.item
    }
}

// === impl Lifecycle ===

impl<C: cfg::Config> Lifecycle<C> {
    const MARKED: Self = Self {
        state: State::Marked,
        _cfg: PhantomData,
    };

    const NOT_REMOVED: Self = Self {
        state: State::NotRemoved,
        _cfg: PhantomData,
    };
}

impl<C: cfg::Config> Pack<C> for Lifecycle<C> {
    const LEN: usize = 2;
    type Prev = ();

    fn from_usize(u: usize) -> Self {
        Self {
            state: match u & Self::MASK {
                0b00 => State::NotRemoved,
                0b01 => State::Marked,
                0b11 => State::Removing,
                bad => unreachable!("weird lifecycle {:#b}", bad),
            },
            _cfg: PhantomData,
        }
    }

    fn as_usize(&self) -> usize {
        self.state as usize
    }
}

impl<C> PartialEq for Lifecycle<C> {
    fn eq(&self, other: &Self) -> bool {
        self.state == other.state
    }
}

impl<C> Eq for Lifecycle<C> {}

impl<C> fmt::Debug for Lifecycle<C> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_tuple("Lifecycle").field(&self.state).finish()
    }
}

// === impl RefCount ===

impl<C: cfg::Config> Pack<C> for RefCount<C> {
    const LEN: usize = cfg::WIDTH - (Lifecycle::<C>::LEN + Generation::<C>::LEN);
    type Prev = Lifecycle<C>;

    fn from_usize(value: usize) -> Self {
        debug_assert!(value <= Self::BITS);
        Self {
            value,
            _cfg: PhantomData,
        }
    }

    fn as_usize(&self) -> usize {
        self.value
    }
}

impl<C: cfg::Config> RefCount<C> {
    const ONE: Self = Self {
        value: 1,
        _cfg: PhantomData,
    };

    fn incr(self) -> Self {
        Self::from_usize((self.value + 1) % Self::BITS)
    }

    fn decr(self) -> Self {
        Self::from_usize(self.value.saturating_sub(1))
    }
}

impl<C> fmt::Debug for RefCount<C> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_tuple("RefCount").field(&self.value).finish()
    }
}

impl<C: cfg::Config> PartialEq for RefCount<C> {
    fn eq(&self, other: &Self) -> bool {
        self.value == other.value
    }
}

impl<C: cfg::Config> Eq for RefCount<C> {}

impl<C: cfg::Config> PartialOrd for RefCount<C> {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        self.value.partial_cmp(&other.value)
    }
}

impl<C: cfg::Config> Ord for RefCount<C> {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        self.value.cmp(&other.value)
    }
}

impl<C: cfg::Config> Clone for RefCount<C> {
    fn clone(&self) -> Self {
        Self::from_usize(self.value)
    }
}

impl<C: cfg::Config> Copy for RefCount<C> {}

// === impl LifecycleGen ===

impl<C: cfg::Config> Pack<C> for LifecycleGen<C> {
    const LEN: usize = Generation::<C>::LEN;
    type Prev = RefCount<C>;

    fn from_usize(value: usize) -> Self {
        Self(Generation::from_usize(value))
    }

    fn as_usize(&self) -> usize {
        self.0.as_usize()
    }
}
