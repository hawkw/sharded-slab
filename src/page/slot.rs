use crate::sync::{
    atomic::{AtomicUsize, Ordering},
    CausalCell,
};
use crate::{page, Pack, Tid, Unpack};

#[derive(Debug)]
pub(crate) struct Slot<T> {
    gen: Generation,
    item: CausalCell<Option<T>>,
    next: AtomicUsize,
}

#[repr(transparent)]
#[derive(Debug, Copy, Clone, PartialEq, Eq)]
pub(crate) struct Generation(usize);

impl Pack for Generation {
    #[cfg(target_pointer_width = "32")]
    const BITS: usize = 0b1111;
    #[cfg(target_pointer_width = "32")]
    const LEN: usize = 4;

    #[cfg(target_pointer_width = "64")]
    const BITS: usize = 0b1111_1111;
    #[cfg(target_pointer_width = "64")]
    const LEN: usize = 8;

    const SHIFT: usize = Tid::SHIFT + Tid::LEN;

    #[inline(always)]
    fn from_usize(u: usize) -> Self {
        debug_assert!(u <= Self::BITS);
        Self(u)
    }

    #[inline(always)]
    fn as_usize(&self) -> usize {
        self.0
    }
}

impl Generation {
    #[inline]
    fn advance(&mut self) -> Self {
        self.0 = (self.0 + 1) % Self::BITS;
        debug_assert!(self.0 <= Self::BITS);
        *self
    }
}

impl<T> Slot<T> {
    pub(in crate::page) fn new(next: usize) -> Self {
        Self {
            gen: Generation(0),
            item: CausalCell::new(None),
            next: AtomicUsize::new(next),
        }
    }

    pub(in crate::page) fn get(&self, gen: impl Unpack<Generation>) -> Option<&T> {
        let gen = gen.unpack();
        #[cfg(test)]
        println!("-> get {:?}; current={:?}", gen, self.gen);
        if gen != self.gen {
            return None;
        }

        self.item.with(|item| unsafe { (&*item).as_ref() })
    }

    pub(in crate::page) fn insert(&mut self, value: &mut Option<T>) -> Generation {
        debug_assert!(self.item.with(|item| unsafe { (*item).is_none() }), "inserted into full slot");
        debug_assert!(value.is_some(), "inserted twice");
        self.item.with_mut(|item| unsafe {
            *item = value.take();
        });

        let gen = self.gen.advance();
        #[cfg(test)]
        println!("-> {:?}", gen);
        gen
    }

    pub(in crate::page) fn next(&self) -> page::Offset {
        page::Offset::from_usize(self.next.load(Ordering::Acquire))
    }

    pub(in crate::page) fn remove(&self, gen: impl Unpack<Generation>, next: impl Unpack<page::Offset>) -> Option<T> {
        let gen = gen.unpack();
        let next = next.unpack().as_usize();

        #[cfg(test)]
        println!("-> remove={:?}; current={:?}", gen, self.gen);

        debug_assert_eq!(gen, self.gen);
        if gen == self.gen {
            let val = self.item.with_mut(|item| unsafe {
                (*item).take()
            });
            debug_assert!(val.is_some());

            self.next.store(next, Ordering::Release);
            val
        } else {
            None
        }
    }
}
