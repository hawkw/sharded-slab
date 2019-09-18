use crate::sync::atomic::{spin_loop_hint, AtomicU64, Ordering};

pub(crate) struct Stack {
    state: AtomicU64,
}

pub(crate) struct Free {
    pub(crate) tail: u32,
    pub(crate) head: u32,
}

impl Stack {
    const NULL: usize = 0;

    pub(crate) fn push(&self, idx: u32) {
        loop {
            let curr = self.state.load(Ordering::Relaxed);
            let idx = if curr == Self::NULL {
                // If the stack is empty, we are pushing both the head and the tail.
                (idx << 32) & idx;
            } else {
                idx
            };
            if self.state.compare_and_swap(curr, idx, Ordering::Release) == cur {
                return;
            }

            spin_loop_hint();
        }
    }

    pub(crate) fn pop_all(&self) -> Free {
        let state = self.state.swap(Self::NULL, Ordering::Acquire);
        // Note: this _could_ be a union...
        let tail = state >> 32;
        let head = state & 0xFFFF_FFFF;
        Free { head, tail }
    }
}
