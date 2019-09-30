use crate::{
    cfg::{self, CfgPrivate},
    page, Pack,
};
use std::{
    cell::{Cell, UnsafeCell},
    collections::VecDeque,
    fmt,
    marker::PhantomData,
    sync::{
        atomic::{AtomicUsize, Ordering},
        Mutex,
    },
};

use lazy_static::lazy_static;

/// Uniquely identifies a thread.
// #[repr(transparent)]
#[derive(Hash)]
pub(crate) struct Tid<P> {
    id: usize,
    _not_send: PhantomData<(UnsafeCell<()>, fn(P))>,
}

#[derive(Debug)]
struct Registration(Cell<Option<usize>>);

struct Registry {
    next: AtomicUsize,
    free: Mutex<VecDeque<usize>>,
}

lazy_static! {
    static ref REGISTRY: Registry = Registry {
        next: AtomicUsize::new(0),
        free: Mutex::new(VecDeque::new()),
    };
}
thread_local! {
    static REGISTRATION: Registration = Registration::new();
}

// === impl Tid ===

impl<C: cfg::Config> Pack<C> for Tid<C> {
    const LEN: usize = C::MAX_SHARDS.trailing_zeros() as usize + 1;
    const BITS: usize = cfg::make_mask(Self::LEN);

    type Prev = page::Addr<C>;

    fn as_usize(&self) -> usize {
        self.id
    }

    fn from_usize(id: usize) -> Self {
        debug_assert!(id <= Self::BITS);
        Self {
            id,
            _not_send: PhantomData,
        }
    }
}

impl<P: cfg::Config> Tid<P> {
    #[inline]
    pub(crate) fn current() -> Self {
        REGISTRATION
            .try_with(Registration::current)
            .unwrap_or_else(|_| Self::poisoned())
    }

    pub(crate) fn is_current(&self) -> bool {
        REGISTRATION
            .try_with(|r| self == &r.current::<P>())
            .unwrap_or(false)
    }
}

impl<P> Tid<P> {
    #[inline(always)]
    pub fn new(id: usize) -> Self {
        Self {
            id,
            _not_send: PhantomData,
        }
    }

    #[cold]
    fn poisoned() -> Self {
        Self {
            id: std::usize::MAX,
            _not_send: PhantomData,
        }
    }

    /// Returns true if the local thread ID was accessed while unwinding.
    pub(crate) fn is_poisoned(&self) -> bool {
        self.id == std::usize::MAX
    }
}

impl<P> fmt::Debug for Tid<P> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if self.is_poisoned() {
            f.debug_tuple("Tid")
                .field(&format_args!("<poisoned>"))
                .finish()
        } else {
            f.debug_tuple("Tid")
                .field(&format_args!("{:#x}", self.id))
                .finish()
        }
    }
}

impl<P> PartialEq for Tid<P> {
    fn eq(&self, other: &Self) -> bool {
        self.id == other.id
    }
}

impl<P> Eq for Tid<P> {}

impl<P: cfg::Config> Clone for Tid<P> {
    fn clone(&self) -> Self {
        Self {
            id: self.id,
            _not_send: PhantomData,
        }
    }
}

impl<P: cfg::Config> Copy for Tid<P> {}

// === impl Registration ===

impl Registration {
    fn new() -> Self {
        Self(Cell::new(None))
    }

    fn current<P: cfg::Config>(&self) -> Tid<P> {
        if let Some(tid) = self.0.get().map(Tid::new) {
            tid
        } else {
            self.register()
        }
    }

    #[cold]
    fn register<P: cfg::Config>(&self) -> Tid<P> {
        let next = REGISTRY.next.fetch_add(1, Ordering::AcqRel);
        let id = if next >= Tid::<P>::BITS {
            REGISTRY
                .free
                .lock()
                .ok()
                .and_then(|mut free| free.pop_front())
                .expect("maximum thread IDs reached!")
        } else {
            next
        };
        debug_assert!(id <= Tid::<P>::BITS, "thread ID overflow!");
        self.0.set(Some(id));
        Tid::new(id)
    }
}

impl Drop for Registration {
    fn drop(&mut self) {
        if let Some(id) = self.0.get() {
            if let Ok(mut free) = REGISTRY.free.lock() {
                free.push_back(id);
            }
        }
    }
}
