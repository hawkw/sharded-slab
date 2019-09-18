use crate::sync::atomic::Ordering;
use crate::{page, Pack};
use std::sync::atomic::AtomicUsize;
use std::{
    cell::{Cell, UnsafeCell},
    fmt,
    marker::PhantomData,
};

/// Uniquely identifies a thread.
// #[repr(transparent)]
#[derive(Copy, Clone, Eq, PartialEq, Ord, PartialOrd, Hash)]
pub(crate) struct Tid {
    id: usize,
    _not_send: PhantomData<UnsafeCell<()>>,
}

// === impl Tid ===
thread_local! {
    static MY_ID: Cell<Option<Tid>> = Cell::new(None);
}

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
    pub(crate) fn current() -> Self {
        MY_ID
            .try_with(|my_id| my_id.get().unwrap_or_else(|| Self::new_thread(my_id)))
            .unwrap_or_else(|_| Self::poisoned())
    }

    pub(crate) fn is_current(&self) -> bool {
        MY_ID
            .try_with(|my_id| {
                let curr = my_id.get().unwrap_or_else(|| Self::new_thread(my_id));
                self == &curr
            })
            .unwrap_or(false)
    }

    #[cold]
    fn new_thread(local: &Cell<Option<Tid>>) -> Self {
        static NEXT_ID: AtomicUsize = AtomicUsize::new(0);
        let id = NEXT_ID.fetch_add(1, Ordering::AcqRel);
        let tid = Self::from_usize(id);
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
            f.debug_tuple("Tid")
                .field(&format_args!("{:#x}", self.id))
                .finish()
        }
    }
}
