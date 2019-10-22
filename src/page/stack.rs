use crate::cfg;
use crate::sync::atomic::{spin_loop_hint, AtomicUsize, Ordering};
use std::marker::PhantomData;

#[derive(Debug)]
pub(super) struct TransferStack<C: cfg::Config = cfg::DefaultConfig> {
    head: AtomicUsize,
    _cfg: PhantomData<fn(C)>,
}

impl<C: cfg::Config> TransferStack<C> {
    pub(super) fn new() -> Self {
        Self {
            head: AtomicUsize::new(super::Addr::<C>::NULL),
            _cfg: PhantomData,
        }
    }

    pub(super) fn pop_all(&self) -> Option<usize> {
        let val = self.head.swap(super::Addr::<C>::NULL, Ordering::Acquire);
        test_println!("-> pop {:#x}", val);
        if val == super::Addr::<C>::NULL {
            None
        } else {
            Some(val)
        }
    }

    pub(super) fn push(&self, value: usize) -> usize {
        let mut next = self.head.load(Ordering::Relaxed);
        loop {
            test_println!("-> next {:#x}", next);

            match self
                .head
                .compare_exchange(next, value, Ordering::AcqRel, Ordering::Acquire)
            {
                // lost the race!
                Err(actual) => {
                    test_println!("-> retry!");
                    next = actual;
                }
                Ok(_) => {
                    test_println!("-> successful; next={:#x}", next);
                    return next;
                }
            }
        }
    }
}

#[cfg(test)]
mod test {
    use super::*;
    use crate::{sync::CausalCell, test_util};
    use loom::thread;
    use std::sync::Arc;

    #[test]
    fn transfer_stack() {
        test_util::run_model("transfer_stack", || {
            let causalities = [CausalCell::new(999), CausalCell::new(999)];
            let shared = Arc::new((causalities, TransferStack::<cfg::DefaultConfig>::new()));
            let shared1 = shared.clone();
            let shared2 = shared.clone();

            let t1 = thread::spawn(move || {
                let (causalities, stack) = &*shared1;
                causalities[0].with_mut(|val| {
                    stack.push(0);
                    unsafe {
                        *val = 0;
                    }
                })
            });
            let t2 = thread::spawn(move || {
                let (causalities, stack) = &*shared2;
                causalities[1].with_mut(|val| {
                    stack.push(1);
                    unsafe {
                        *val = 1;
                    }
                })
            });

            let (causalities, stack) = &*shared;
            let mut idx = stack.pop_all();
            while idx == None {
                idx = stack.pop_all();
                thread::yield_now();
            }
            let idx = idx.unwrap();
            causalities[idx].with_mut(|val| unsafe {
                assert_eq!(*val, idx);
            });

            t1.join().unwrap();
            t2.join().unwrap();
        });
    }
}
