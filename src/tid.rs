use crate::sync::{AtomicUsize, Ordering};
use std::{cell::UnsafeCell, marker::PhantomData};

/// Uniquely identifies a thread.
#[repr(transparent)]
#[derive(Copy, Clone, Eq, PartialEq, Ord, PartialOrd, Hash)]
pub(crate) struct Tid {
    id: usize,
    _not_send: PhantomData<UnsafeCell<()>>,
}

// === impl Tid ===

impl Tid {
    pub(crate) const MAX: usize = 1024;

    pub(crate) fn current() -> Self {
        thread_local! {
            static MY_ID: Cell<Option<Tid>> = Cell::new(None);
        }

        MY_ID
            .try_with(|my_id| my_id.get().unwrap_or_else(|| Self::new_thread(my_id)))
            .unwrap_or_else(|_| Self::poisoned())
    }

    pub(crate) fn as_usize(&self) -> usize {
        debug_assert!(self.id <= Self::MAX);
        self.id
    }

    #[cold]
    fn new_thread(local: &Cell<Option<Tid>>) -> Self {
        static NEXT_ID: AtomicUsize = AtomicUsize::new(0);
        let id = NEXT_ID.fetch_add(1, Ordering::AcqRel);
        debug_assert!(id <= Self::MAX);
        let tid = Self {
            id,
            _not_send: PhantomData,
        };
        local.set(Some(tid));
        tid
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
            f.debug_tuple("Tid").field(&self.id).finish()
        }
    }
}
