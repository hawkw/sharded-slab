use crate::{page, Pack};
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
#[derive(Copy, Clone, Eq, PartialEq, Ord, PartialOrd, Hash)]
pub(crate) struct Tid {
    id: usize,
    _not_send: PhantomData<UnsafeCell<()>>,
}

#[derive(Debug)]
struct Registration(Cell<Option<Tid>>);

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

impl Pack for Tid {
    #[cfg(target_pointer_width = "32")]
    const BITS: usize = 0b0011_1111;
    #[cfg(target_pointer_width = "32")]
    const LEN: usize = 6;

    #[cfg(target_pointer_width = "64")]
    const BITS: usize = 0b1111_1111_1111;
    #[cfg(target_pointer_width = "64")]
    const LEN: usize = 12;

    const SHIFT: usize = page::Index::SHIFT + page::Index::LEN;

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

impl Tid {
    #[inline]
    pub(crate) fn current() -> Self {
        REGISTRATION
            .try_with(Registration::current)
            .unwrap_or_else(|_| Self::poisoned())
    }

    pub(crate) fn is_current(&self) -> bool {
        REGISTRATION
            .try_with(|r| self == &r.current())
            .unwrap_or(false)
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

impl fmt::Debug for Tid {
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

// === impl Registration ===

impl Registration {
    fn new() -> Self {
        Self(Cell::new(None))
    }

    fn current(&self) -> Tid {
        if let Some(tid) = self.0.get() {
            tid
        } else {
            self.register()
        }
    }

    #[cold]
    fn register(&self) -> Tid {
        let next = REGISTRY.next.fetch_add(1, Ordering::AcqRel);
        let id = if next >= Tid::BITS {
            REGISTRY
                .free
                .lock()
                .ok()
                .and_then(|mut free| free.pop_front())
                .expect("maximum thread IDs reached!")
        } else {
            next
        };
        debug_assert!(id <= Tid::BITS, "thread ID overflow!");
        let tid = Tid {
            id,
            _not_send: PhantomData,
        };
        self.0.set(Some(tid));
        tid
    }
}

impl Drop for Registration {
    fn drop(&mut self) {
        if let Some(Tid { id, .. }) = self.0.get() {
            if let Ok(mut free) = REGISTRY.free.lock() {
                println!("drop tid: {}", id);
                free.push_back(id);
            }
        }
    }
}
