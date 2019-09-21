use crate::sync::atomic::{spin_loop_hint, AtomicU64, Ordering};
use crate::{page, Pack};

pub(crate) struct Stack {
    state: AtomicU64,
}

pub(crate) struct Free {
    pub(crate) tail: page::Offset,
    pub(crate) head: page::Offset,
}

impl Stack {
    const NULL: u64 = 0;

    pub(crate) fn new() -> Self {
        Self {
            state: AtomicU64::new(Self::NULL),
        }
    }

    pub(crate) fn push(&self, idx: usize) -> usize {
        let idx = idx as u64;
        debug_assert!(idx <= std::u32::MAX as u64);

        loop {
            let curr = self.state.load(Ordering::Relaxed);
            let idx = if curr == Self::NULL {
                // If the stack is empty, we are pushing both the head and the tail.
                (idx << 32) & idx
            } else {
                idx
            };
            if self.state.compare_and_swap(curr, idx, Ordering::Release) == curr {
                return (curr >> 32) as usize;
            }

            spin_loop_hint();
        }
    }

    pub(crate) fn pop_all(&self) -> Free {
        let state = self.state.swap(Self::NULL, Ordering::Acquire);
        // Note: this _could_ be a union...
        let tail = page::Offset::from_usize((state >> 32) as usize);
        let head = page::Offset::from_usize(state as usize & page::Offset::MASK);
        Free { head, tail }
    }
}
