use crate::sync::{
    atomic::{AtomicUsize, Ordering},
    CausalCell,
};
use crate::{cfg, page, Pack, Tid, Unpack};
use std::{fmt, marker::PhantomData};
#[derive(Debug)]
pub(crate) struct Slot<T, P: cfg::Params> {
    gen: Generation<P>,
    next: AtomicUsize,
    item: CausalCell<Option<T>>,
}

#[repr(transparent)]
pub(crate) struct Generation<C: cfg::Params = cfg::DefaultParams> {
    value: usize,
    _cfg: PhantomData<fn(C)>,
}

impl<C: cfg::Params> Pack<C> for Generation<C> {
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

impl<C: cfg::Params> Generation<C> {
    fn new(value: usize) -> Self {
        Self {
            value,
            _cfg: PhantomData,
        }
    }

    #[inline]
    fn advance(&mut self) -> Self {
        self.value = (self.value + 1) % Self::BITS;
        debug_assert!(self.value <= Self::BITS);
        *self
    }
}

impl<T, P: cfg::Params> Slot<T, P> {
    pub(in crate::page) fn new(next: usize) -> Self {
        Self {
            gen: Generation::new(0),
            item: CausalCell::new(None),
            next: AtomicUsize::new(next),
        }
    }

    pub(in crate::page) fn get(&self, gen: impl Unpack<P, Generation<P>>) -> Option<&T> {
        let gen = gen.unpack();
        #[cfg(test)]
        println!("-> get {:?}; current={:?}", gen, self.gen);
        if gen != self.gen {
            return None;
        }

        self.value()
    }

    pub(super) fn value<'a>(&'a self) -> Option<&'a T> {
        self.item.with(|item| unsafe { (&*item).as_ref() })
    }

    pub(in crate::page) fn insert(&mut self, value: &mut Option<T>) -> Generation<P> {
        debug_assert!(
            self.item.with(|item| unsafe { (*item).is_none() }),
            "inserted into full slot"
        );
        debug_assert!(value.is_some(), "inserted twice");
        self.item.with_mut(|item| unsafe {
            *item = value.take();
        });

        let gen = self.gen.advance();
        #[cfg(test)]
        println!("-> {:?}", gen);
        gen
    }

    pub(in crate::page) fn next(&self) -> usize {
        self.next.load(Ordering::Acquire)
    }

    pub(in crate::page) fn remove(
        &self,
        gen: impl Unpack<P, Generation<P>>,
        next: usize,
    ) -> Option<T> {
        let gen = gen.unpack();

        #[cfg(test)]
        println!("-> remove={:?}; current={:?}", gen, self.gen);

        debug_assert_eq!(gen, self.gen);
        if gen == self.gen {
            let val = self.item.with_mut(|item| unsafe { (*item).take() });
            debug_assert!(val.is_some());

            self.next.store(next, Ordering::Release);
            val
        } else {
            None
        }
    }
}

impl<P: cfg::Params> fmt::Debug for Generation<P> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_tuple("Generation").field(&self.value).finish()
    }
}

impl<P: cfg::Params> PartialEq for Generation<P> {
    fn eq(&self, other: &Self) -> bool {
        self.value == other.value
    }
}

impl<P: cfg::Params> Eq for Generation<P> {}

impl<P: cfg::Params> PartialOrd for Generation<P> {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        self.value.partial_cmp(&other.value)
    }
}

impl<P: cfg::Params> Ord for Generation<P> {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        self.value.cmp(&other.value)
    }
}

impl<P: cfg::Params> Clone for Generation<P> {
    fn clone(&self) -> Self {
        Self::new(self.value)
    }
}

impl<P: cfg::Params> Copy for Generation<P> {}
