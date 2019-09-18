use crate::global;
use crate::sync::Arc;

pub struct Page<T> {
    // TODO: this can probably be a pointer, since the global could just be a
    // field on the owning struct...
    global: global::Stack,
    head: u32,
    tail: u32,
    slab: Box<[Slot<T>]>,
}

enum Slot<T> {
    Free(u32),
    Full(T),
}

impl<T> Page<T> {
    pub(crate) fn new(size: usize) -> Self {
        let mut slab = Vec::with_capacity(size);
        slab.extend((2..size + 2).map(Slot::Free));
        Self {
            global: global::Stack::new(),
            head: 1,
            tail: 1,
            slab: slab.into_boxed_slice(),
        }
    }

    pub(crate) insert(&mut self, t: &mut Option<T>) -> Option<u32> {
        unimplemented!();
    }

    pub(crate) fn deallocate(&self, idx: u32) {
        unimplemented!();
    }
}
